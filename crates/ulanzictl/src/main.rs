//! `ulanzictl`: control and inspect the Ulanzi Deck service from the command line.

mod probe;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use ulanzi_core::plugin::PickItem;
use ulanzi_core::store::{ProfileLoad, Store, parse_profile};
use ulanzi_deckd::dbus::{BUS_NAME, INTERFACE, OBJECT_PATH};

#[derive(Parser, Debug)]
#[command(name = "ulanzictl", version, about = "Control the Ulanzi Deck service")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug, PartialEq)]
enum Cmd {
    /// Show device and service status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Set brightness (0-100).
    Brightness {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Go to a page: next, prev, or a page number starting at 1.
    Page { target: PageTarget },
    /// List or switch profiles.
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
    /// List, install, remove, turn on or off and restart plugins.
    Plugin {
        #[command(subcommand)]
        cmd: PluginCmd,
    },
    /// Import the old Python configuration (~/.config/ulanzi/config.yaml).
    Migrate {
        #[arg(long)]
        force: bool,
    },
    /// Import profiles from a .ulanzideck file (a backup is taken first).
    Import {
        file: PathBuf,
        /// Replace profiles that already exist (default: keep both).
        #[arg(long, conflicts_with = "skip")]
        replace: bool,
        /// Skip profiles that already exist.
        #[arg(long)]
        skip: bool,
        /// Switch to the first imported profile.
        #[arg(long)]
        switch: bool,
    },
    /// Export profiles to a .ulanzideck file (all of them unless --profile is given).
    Export {
        file: PathBuf,
        /// A profile name or id; repeat for several.
        #[arg(long = "profile")]
        profiles: Vec<String>,
        /// Include the settings (never the OBS password).
        #[arg(long)]
        settings: bool,
    },
    /// List backups, newest first.
    Snapshots,
    /// Restore a backup by name (the current state is backed up first).
    Restore { name: String },
    /// Check profile files for errors (all profiles when no file is given).
    Validate { file: Option<PathBuf> },
    /// Print key presses, page changes and action errors as they happen.
    Monitor,
    /// Print versions, session, device state and recent logs for bug reports.
    Diagnostics,
    /// Talk to the device directly (the service must be stopped).
    Probe {
        #[command(subcommand)]
        what: probe::Probe,
    },
}

#[derive(Subcommand, Debug, PartialEq)]
enum ProfileCmd {
    List,
    Switch { name_or_id: String },
}

#[derive(Subcommand, Debug, PartialEq)]
enum PluginCmd {
    /// Show installed plugins and how they are doing.
    List,
    /// Install (or replace) the plugin in a folder (needs Developer mode).
    Install { dir: PathBuf },
    /// Delete a plugin and its saved secrets (its settings are kept).
    Remove { id: String },
    /// Turn a plugin on.
    Enable { id: String },
    /// Turn a plugin off.
    Disable { id: String },
    /// Stop and start a plugin again (also after it stopped working).
    Restart { id: String },
    /// Allow or refuse installing plugins from a folder.
    DevMode { state: OnOff },
    /// List what a plugin's picker offers (starts the plugin if needed).
    Items {
        id: String,
        picker: String,
        /// Only entries matching this text.
        #[arg(default_value = "")]
        query: String,
        /// Only entries in this group.
        #[arg(long, default_value = "")]
        group: String,
    },
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq)]
enum OnOff {
    On,
    Off,
}

/// `PluginListItems`: the entries of a plugin's picker.
async fn plugin_list_items(
    p: &zbus::Proxy<'_>,
    id: &str,
    picker: &str,
    query: &str,
    group: &str,
) -> Result<Vec<PickItem>, String> {
    let json: String = p
        .call("PluginListItems", &(id, picker, query, group))
        .await
        .map_err(not_running)?;
    serde_json::from_str(&json).map_err(|e| format!("unexpected answer from the service: {e}"))
}

/// `plugin items` output: one entry per line, id first, then the label and what else it has.
fn items_table(items: &[PickItem]) -> String {
    if items.is_empty() {
        return "nothing to pick\n".into();
    }
    let width = items.iter().map(|i| i.id.chars().count()).max().unwrap_or(0);
    let mut out = String::new();
    for i in items {
        let mut line = format!("{:<width$}  {}", i.id, i.label);
        for extra in [&i.subtitle, &i.group, &i.state].into_iter().flatten() {
            line.push_str(&format!("  [{extra}]"));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// One row of `PluginList`: id, name, version, status, detail.
type PluginRow = (String, String, String, String, String);

/// `plugin list` output: aligned columns, the detail (a reason or message) last.
fn plugin_table(rows: &[PluginRow]) -> String {
    if rows.is_empty() {
        return "no plugins installed\n".into();
    }
    let header = ("ID", "NAME", "VERSION", "STATUS", "DETAIL");
    let width = |f: fn(&PluginRow) -> &str, title: &str| {
        rows.iter()
            .map(|r| f(r).chars().count())
            .chain([title.len()])
            .max()
            .unwrap_or(0)
    };
    let w = [
        width(|r| &r.0, header.0),
        width(|r| &r.1, header.1),
        width(|r| &r.2, header.2),
        width(|r| &r.3, header.3),
    ];
    let line = |c: [&str; 5]| {
        let text = format!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}",
            c[0],
            c[1],
            c[2],
            c[3],
            c[4],
            w0 = w[0],
            w1 = w[1],
            w2 = w[2],
            w3 = w[3]
        );
        format!("{}\n", text.trim_end())
    };
    let mut out = line([header.0, header.1, header.2, header.3, header.4]);
    for r in rows {
        out += &line([&r.0, &r.1, &r.2, &r.3, &r.4]);
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
enum PageTarget {
    Next,
    Prev,
    /// Zero-based index.
    Index(u32),
}

impl std::str::FromStr for PageTarget {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "next" => Ok(PageTarget::Next),
            "prev" | "previous" => Ok(PageTarget::Prev),
            n => match n.parse::<u32>() {
                Ok(i) if i >= 1 => Ok(PageTarget::Index(i - 1)),
                _ => Err(format!("{n:?} is not next, prev or a page number (1, 2, ...)")),
            },
        }
    }
}

/// Replace whatever follows "password" (any case) on a log line.
fn redact(log: &str) -> String {
    log.lines()
        .map(|l| {
            match l
                .as_bytes()
                .windows(8)
                .position(|w| w.eq_ignore_ascii_case(b"password"))
            {
                // The match starts with ASCII 'p', so the byte index is a char boundary.
                Some(i) => format!("{}password <redacted>", &l[..i]),
                None => l.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse one profile file; the error includes the line and column.
fn validate_file(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_profile(&text)
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", path.display()))
}

async fn proxy() -> Result<zbus::Proxy<'static>, String> {
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| format!("no session bus: {e}"))?;
    zbus::Proxy::new(&conn, BUS_NAME, OBJECT_PATH, INTERFACE)
        .await
        .map_err(|e| e.to_string())
}

/// The service resolves paths itself, so hand it an absolute one.
fn absolute(p: &Path) -> Result<String, String> {
    std::path::absolute(p)
        .map(|a| a.to_string_lossy().into_owned())
        .map_err(|e| format!("{}: {e}", p.display()))
}

fn not_running(e: zbus::Error) -> String {
    // The service's own errors carry a sentence meant for the user; show just that.
    if let zbus::Error::MethodError(name, Some(msg), _) = &e
        && name.starts_with("io.github.tyaaa_aa.UlanziDeck1.Error.")
    {
        return msg.clone();
    }
    let s = e.to_string();
    if s.contains("ServiceUnknown") || s.contains("NameHasNoOwner") {
        "the Ulanzi Deck service is not running (systemctl --user start ulanzi-deckd)".into()
    } else {
        s
    }
}

type Hello = (u32, String, String, String, String, String, u32);

async fn run(cli: Cli) -> Result<(), String> {
    match cli.cmd {
        Cmd::Status { json } => {
            let p = proxy().await?;
            let (api, ver, state, detail, model, profile, page): Hello =
                p.call("Hello", &("ulanzictl",)).await.map_err(not_running)?;
            let profiles: Vec<(String, String)> = p.call("ListProfiles", &()).await.map_err(not_running)?;
            // Older services don't have Firmware: leave it out then.
            let firmware: String = p.call("Firmware", &()).await.unwrap_or_default();
            let name = profiles
                .iter()
                .find(|(id, _)| *id == profile)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            if json {
                let v = serde_json::json!({
                    "api": api, "daemon_version": ver, "device_state": state, "device_detail": detail,
                    "model": model, "firmware": firmware, "profile_id": profile, "profile": name, "page": page + 1,
                });
                println!("{v}");
            } else {
                println!("service   ulanzi-deckd {ver} (API {api})");
                println!("device    {state}: {detail}");
                println!("model     {model}");
                if !firmware.is_empty() {
                    println!("firmware  {firmware}");
                }
                println!("profile   {name} ({profile})");
                println!("page      {}", page + 1);
            }
        }
        Cmd::Brightness { percent } => proxy()
            .await?
            .call::<_, _, ()>("SetBrightness", &(percent,))
            .await
            .map_err(not_running)?,
        Cmd::Page { target } => {
            let p = proxy().await?;
            match target {
                PageTarget::Next => p.call::<_, _, ()>("NextPage", &()).await,
                PageTarget::Prev => p.call::<_, _, ()>("PrevPage", &()).await,
                PageTarget::Index(i) => p.call::<_, _, ()>("SetPage", &(i,)).await,
            }
            .map_err(not_running)?
        }
        Cmd::Profile { cmd } => {
            let p = proxy().await?;
            let profiles: Vec<(String, String)> = p.call("ListProfiles", &()).await.map_err(not_running)?;
            match cmd {
                ProfileCmd::List => {
                    for (id, name) in profiles {
                        println!("{id}  {name}");
                    }
                }
                ProfileCmd::Switch { name_or_id } => {
                    let id = profiles
                        .iter()
                        .find(|(id, n)| *id == name_or_id || n.eq_ignore_ascii_case(&name_or_id))
                        .map(|(id, _)| id.clone())
                        .ok_or_else(|| format!("no profile called {name_or_id:?}"))?;
                    p.call::<_, _, ()>("SwitchProfile", &(id,))
                        .await
                        .map_err(not_running)?;
                }
            }
        }
        Cmd::Plugin { cmd } => plugin(cmd).await?,
        Cmd::Migrate { force } => {
            let (id, keys, warnings): (String, u32, Vec<String>) = proxy()
                .await?
                .call("Migrate", &(force,))
                .await
                .map_err(not_running)?;
            println!("imported {keys} keys into profile {id}");
            for w in warnings {
                println!("  warning: {w}");
            }
        }
        Cmd::Import {
            file,
            replace,
            skip,
            switch,
        } => {
            let path = absolute(&file)?;
            let conflict = if replace {
                "replace"
            } else if skip {
                "skip"
            } else {
                "keep_both"
            };
            let (ids, warnings, undo): (Vec<String>, Vec<String>, String) = proxy()
                .await?
                .call("ImportBundle", &(path.as_str(), conflict, true, false, switch))
                .await
                .map_err(not_running)?;
            println!("imported {} profile(s)", ids.len());
            for w in warnings {
                println!("  warning: {w}");
            }
            println!("to undo: ulanzictl restore {undo}");
        }
        Cmd::Export {
            file,
            profiles,
            settings,
        } => {
            let p = proxy().await?;
            let list: Vec<(String, String)> = p.call("ListProfiles", &()).await.map_err(not_running)?;
            let ids = profiles
                .iter()
                .map(|want| {
                    list.iter()
                        .find(|(id, name)| id == want || name.eq_ignore_ascii_case(want))
                        .map(|(id, _)| id.clone())
                        .ok_or_else(|| format!("no profile called {want:?}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let path = absolute(&file)?;
            p.call::<_, _, ()>("ExportBundle", &(path.as_str(), ids, settings))
                .await
                .map_err(not_running)?;
            println!("wrote {path}");
        }
        Cmd::Snapshots => {
            let snaps: Vec<(String, String, String, Vec<String>)> = proxy()
                .await?
                .call("ListSnapshots", &())
                .await
                .map_err(not_running)?;
            if snaps.is_empty() {
                println!("no backups yet");
            }
            for (name, created, reason, profiles) in snaps {
                println!("{name}\n  {created} · {reason} · {}", profiles.join(", "));
            }
        }
        Cmd::Restore { name } => {
            let undo: String = proxy()
                .await?
                .call("RestoreSnapshot", &(name.as_str(),))
                .await
                .map_err(not_running)?;
            println!("restored {name}; to undo: ulanzictl restore {undo}");
        }
        Cmd::Validate { file } => match file {
            Some(f) => {
                validate_file(&f)?;
                println!("{}: ok", f.display());
            }
            None => {
                let store = Store::xdg().map_err(|e| e.to_string())?;
                let mut bad = 0;
                for l in store.list_profiles() {
                    match l {
                        ProfileLoad::Ok(p) => println!("ok       {} ({})", p.name, p.id),
                        ProfileLoad::Invalid { path, error } => {
                            bad += 1;
                            println!("INVALID  {}: {error}", path.display());
                        }
                    }
                }
                if bad > 0 {
                    return Err(format!("{bad} invalid profile(s) were moved aside"));
                }
            }
        },
        Cmd::Monitor => {
            let p = proxy().await?;
            let mut all = p.receive_all_signals().await.map_err(|e| e.to_string())?;
            println!("watching; press Ctrl+C to stop");
            while let Some(m) = all.next().await {
                let name = m.header().member().map(|m| m.to_string()).unwrap_or_default();
                let body = m.body();
                let text = match name.as_str() {
                    "InputEvent" | "DeviceStateChanged" | "ActionFailed" => body
                        .deserialize::<(String, String)>()
                        .map(|(a, b)| format!("{a} {b}"))
                        .unwrap_or_default(),
                    "PageChanged" => body
                        .deserialize::<(String, u32)>()
                        .map(|(a, b)| format!("{a} page {}", b + 1))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                println!("{name:<20} {text}");
            }
        }
        Cmd::Diagnostics => diagnostics().await,
        Cmd::Probe { what } => probe::run(what).await?,
    }
    Ok(())
}

async fn plugin(cmd: PluginCmd) -> Result<(), String> {
    let p = proxy().await?;
    match cmd {
        PluginCmd::List => {
            let rows: Vec<PluginRow> = p.call("PluginList", &()).await.map_err(not_running)?;
            print!("{}", plugin_table(&rows));
        }
        PluginCmd::Install { dir } => {
            let dir = std::fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = dir.to_string_lossy().into_owned();
            let id: String = p
                .call("PluginInstallDir", &(path.as_str(),))
                .await
                .map_err(not_running)?;
            println!("installed {id}");
        }
        PluginCmd::Remove { id } => p
            .call::<_, _, ()>("PluginRemove", &(id.as_str(),))
            .await
            .map_err(not_running)?,
        PluginCmd::Enable { id } => p
            .call::<_, _, ()>("PluginSetEnabled", &(id.as_str(), true))
            .await
            .map_err(not_running)?,
        PluginCmd::Disable { id } => p
            .call::<_, _, ()>("PluginSetEnabled", &(id.as_str(), false))
            .await
            .map_err(not_running)?,
        PluginCmd::Restart { id } => p
            .call::<_, _, ()>("PluginRestart", &(id.as_str(),))
            .await
            .map_err(not_running)?,
        PluginCmd::Items {
            id,
            picker,
            query,
            group,
        } => print!(
            "{}",
            items_table(&plugin_list_items(&p, &id, &picker, &query, &group).await?)
        ),
        PluginCmd::DevMode { state } => {
            let text: String = p.call("GetSettings", &()).await.map_err(not_running)?;
            let mut s: ulanzi_core::model::Settings = toml::from_str(&text).map_err(|e| e.to_string())?;
            s.developer_mode = state == OnOff::On;
            let text = toml::to_string_pretty(&s).map_err(|e| e.to_string())?;
            p.call::<_, _, ()>("ApplySettings", &(text,))
                .await
                .map_err(not_running)?;
            println!(
                "Developer mode is {}",
                if s.developer_mode { "on" } else { "off" }
            );
        }
    }
    Ok(())
}

async fn diagnostics() {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "-".into());
    println!("ulanzictl       {}", env!("CARGO_PKG_VERSION"));
    println!(
        "session         {} / {}",
        env("XDG_SESSION_TYPE"),
        env("XDG_CURRENT_DESKTOP")
    );
    match proxy().await {
        Ok(p) => match p.call::<_, _, Hello>("Hello", &("ulanzictl",)).await {
            Ok((api, ver, state, detail, model, ..)) => {
                println!("service         ulanzi-deckd {ver} (API {api})");
                println!("device          {state}: {detail} ({model})");
                if let Ok(fw) = p.call::<_, _, String>("Firmware", &()).await
                    && !fw.is_empty()
                {
                    println!("firmware        {fw}");
                }
            }
            Err(e) => println!("service         {}", not_running(e)),
        },
        Err(e) => println!("service         {e}"),
    }
    println!(
        "kernel          {}",
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .unwrap_or_default()
            .trim()
    );
    let direct = ulanzi_deckd::transport::probe(
        Path::new("/sys/class/hidraw"),
        Path::new("/dev"),
        Path::new("/proc"),
    );
    println!("device (direct) {}: {}", direct.code(), direct.detail());
    println!(
        "uinput          {}",
        if std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .is_ok()
        {
            "writable"
        } else {
            "not writable"
        }
    );
    println!("\nrecent log:");
    let journal = std::process::Command::new("journalctl")
        .args([
            "--user",
            "-u",
            "ulanzi-deckd",
            "-n",
            "200",
            "--no-pager",
            "-o",
            "cat",
        ])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .filter(|s| !s.trim().is_empty() && !s.contains("-- No entries --"));
    let log = journal.or_else(|| {
        let dir = Store::xdg().ok()?.state_dir().to_path_buf();
        let mut logs: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("daemon"))
            })
            .collect();
        logs.sort();
        let text = std::fs::read_to_string(logs.last()?).ok()?;
        let lines: Vec<&str> = text.lines().collect();
        Some(lines[lines.len().saturating_sub(200)..].join("\n"))
    });
    println!("{}", redact(&log.unwrap_or_else(|| "(no log found)".into())));
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ulanzictl: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Result<Cmd, clap::Error> {
        Cli::try_parse_from(std::iter::once("ulanzictl").chain(args.iter().copied())).map(|c| c.cmd)
    }

    #[test]
    fn import_and_export_flags() {
        assert_eq!(
            parse(&["import", "a.ulanzideck", "--replace", "--switch"]).expect("ok"),
            Cmd::Import {
                file: "a.ulanzideck".into(),
                replace: true,
                skip: false,
                switch: true
            }
        );
        assert!(parse(&["import", "a.ulanzideck", "--replace", "--skip"]).is_err());
        assert_eq!(
            parse(&[
                "export",
                "b.ulanzideck",
                "--profile",
                "Photo Editing",
                "--profile",
                "Default"
            ])
            .expect("ok"),
            Cmd::Export {
                file: "b.ulanzideck".into(),
                profiles: vec!["Photo Editing".into(), "Default".into()],
                settings: false
            }
        );
    }

    #[test]
    fn plugin_commands() {
        let plugin = |args: &[&str]| match parse(args) {
            Ok(Cmd::Plugin { cmd }) => Ok(cmd),
            Ok(other) => panic!("{other:?}"),
            Err(e) => Err(e),
        };
        assert_eq!(plugin(&["plugin", "list"]).expect("ok"), PluginCmd::List);
        assert_eq!(
            plugin(&["plugin", "install", "./hello"]).expect("ok"),
            PluginCmd::Install {
                dir: "./hello".into()
            }
        );
        assert_eq!(
            plugin(&["plugin", "disable", "hello"]).expect("ok"),
            PluginCmd::Disable { id: "hello".into() }
        );
        assert_eq!(
            plugin(&["plugin", "dev-mode", "on"]).expect("ok"),
            PluginCmd::DevMode { state: OnOff::On }
        );
        assert!(plugin(&["plugin", "dev-mode", "maybe"]).is_err());
        assert_eq!(
            plugin(&["plugin", "items", "obs", "scenes", "--group", "x"]).expect("ok"),
            PluginCmd::Items {
                id: "obs".into(),
                picker: "scenes".into(),
                query: String::new(),
                group: "x".into(),
            }
        );
        assert_eq!(
            plugin(&["plugin", "items", "ha", "entities", "kitchen"]).expect("ok"),
            PluginCmd::Items {
                id: "ha".into(),
                picker: "entities".into(),
                query: "kitchen".into(),
                group: String::new(),
            }
        );
        assert!(plugin(&["plugin", "items", "obs"]).is_err());
        assert!(plugin(&["plugin", "remove"]).is_err());
    }

    #[test]
    fn items_lines() {
        let item = |id: &str, label: &str| ulanzi_core::plugin::PickItem {
            id: id.into(),
            label: label.into(),
            subtitle: None,
            group: None,
            icon: None,
            state: None,
        };
        let mut lamp = item("light.kitchen", "Kitchen");
        lamp.state = Some("on".into());
        let text = items_table(&[item("a", "A"), lamp]);
        assert_eq!(text, "a              A\nlight.kitchen  Kitchen  [on]\n");
        assert_eq!(items_table(&[]), "nothing to pick\n");
    }

    #[test]
    fn plugin_list_columns() {
        let rows = vec![
            (
                "hello".to_string(),
                "Hello".to_string(),
                "0.1.0".to_string(),
                "running".to_string(),
                String::new(),
            ),
            (
                "broken_one".to_string(),
                "broken_one".to_string(),
                String::new(),
                "incompatible".to_string(),
                "plugin.wasm is missing".to_string(),
            ),
        ];
        let text = plugin_table(&rows);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].starts_with("ID "), "{text}");
        // Columns line up: every row's status starts where the header's does.
        let col = lines[0].find("STATUS").expect("header");
        assert_eq!(&lines[1][col..col + 7], "running");
        assert_eq!(&lines[2][col..col + 12], "incompatible");
        assert!(lines[2].ends_with("plugin.wasm is missing"), "{text}");
        assert_eq!(plugin_table(&[]), "no plugins installed\n");
    }

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn page_numbers_are_one_based() {
        assert_eq!(
            parse(&["page", "3"]).expect("ok"),
            Cmd::Page {
                target: PageTarget::Index(2)
            }
        );
        assert_eq!(
            parse(&["page", "next"]).expect("ok"),
            Cmd::Page {
                target: PageTarget::Next
            }
        );
        assert!(parse(&["page", "0"]).is_err());
    }

    #[test]
    fn brightness_range() {
        assert_eq!(
            parse(&["brightness", "40"]).expect("ok"),
            Cmd::Brightness { percent: 40 }
        );
        assert!(parse(&["brightness", "101"]).is_err());
    }

    #[test]
    fn diagnostics_redacts_password() {
        let log = "INFO connected\nDEBUG obs password=hunter2 port=4455\nPassword: secret";
        let r = redact(log);
        assert!(!r.contains("hunter2") && !r.contains("secret"), "{r}");
        assert!(r.contains("INFO connected"));
        let tricky = redact("İİİ obs Password=hunter2");
        assert!(
            tricky.starts_with("İİİ obs ") && !tricky.contains("hunter2"),
            "{tricky}"
        );
    }

    #[test]
    fn validate_reports_line() {
        let d = tempfile::tempdir().expect("tmp");
        let f = d.path().join("p.toml");
        std::fs::write(&f, "version = 1\nname = [\n").expect("write");
        let e = validate_file(&f).expect_err("broken");
        assert!(e.contains("line"), "{e}");
    }
}
