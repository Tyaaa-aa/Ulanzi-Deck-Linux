//! Installing a downloaded update. The window does this because it has the password prompt;
//! downloading and checking the file are the service's.

use std::path::{Path, PathBuf};

use ulanzi_core::update::InstallKind;

/// Why an install did not finish. Cancelling the password prompt is not a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallFailure {
    Cancelled,
    Failed(String),
}

/// The command that installs `file`; `None` for kinds not installed through a package manager.
/// `have` says whether a program is on `PATH`. `file` must be absolute: `apt-get` takes anything
/// else for a package name.
pub fn install_command(kind: InstallKind, file: &Path, have: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    let file = file.to_string_lossy().into_owned();
    let cmd = |args: &[&str]| {
        let mut out: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        out.push(file.clone());
        Some(out)
    };
    match kind {
        InstallKind::Arch => cmd(&["pkexec", "pacman", "-U", "--noconfirm"]),
        InstallKind::Deb => cmd(&["pkexec", "apt-get", "install", "-y"]),
        InstallKind::Rpm if have("dnf") => cmd(&["pkexec", "dnf", "install", "-y"]),
        InstallKind::Rpm => cmd(&[
            "pkexec",
            "zypper",
            "--non-interactive",
            "install",
            "--allow-unsigned-rpm",
        ]),
        InstallKind::AppImage | InstallKind::Tarball | InstallKind::Other => None,
    }
}

/// `pkexec` exits with 126 when the password prompt is dismissed.
const PROMPT_DISMISSED: i32 = 126;

fn outcome(code: Option<i32>, stderr: &str) -> Result<(), InstallFailure> {
    match code {
        Some(0) => Ok(()),
        Some(PROMPT_DISMISSED) => Err(InstallFailure::Cancelled),
        _ => Err(InstallFailure::Failed(
            stderr
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("The install failed.")
                .to_string(),
        )),
    }
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

async fn run(args: &[String]) -> Result<(), InstallFailure> {
    let (program, rest) = args
        .split_first()
        .ok_or_else(|| InstallFailure::Failed("Nothing to run.".into()))?;
    let out = tokio::process::Command::new(program)
        .args(rest)
        .output()
        .await
        .map_err(|e| InstallFailure::Failed(format!("{program}: {e}")))?;
    outcome(out.status.code(), &String::from_utf8_lossy(&out.stderr))
}

/// Install the downloaded `file`. An AppImage needs nothing here: the service already swapped it.
pub async fn install(kind: InstallKind, file: &Path) -> Result<(), InstallFailure> {
    if kind == InstallKind::Tarball {
        return install_tarball(file).await;
    }
    match install_command(kind, file, on_path) {
        Some(args) => run(&args).await,
        None => Ok(()),
    }
}

async fn install_tarball(file: &Path) -> Result<(), InstallFailure> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return Err(InstallFailure::Failed("The download has no folder.".into()));
    };
    let name = name.to_string_lossy();
    let folder = dir.join(name.strip_suffix(".tar.gz").unwrap_or(&name));
    let text = |p: &Path| p.to_string_lossy().into_owned();
    let tar = [
        "tar".to_string(),
        "-xzf".into(),
        text(file),
        "-C".into(),
        text(dir),
    ];
    run(&tar).await?;
    run(&[text(&folder.join("install.sh")), "--no-udev".into()]).await
}

/// Start `exe` a moment after this window has gone: a second copy started while this one still
/// runs would hand over to it and quit.
pub fn relaunch_command(exe: &str) -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        "sleep 1; exec \"$0\"".into(),
        exe.to_string(),
    ]
}

/// The program to start again. After a package update Linux reports this program as
/// `… (deleted)`, and an AppImage's own file is what the menu runs. `APPIMAGE` counts only with this
/// program inside the mounted image: a window started from another AppImage's terminal inherits it.
fn program_to_start(exe: &Path, appimage: Option<&Path>, appdir: Option<&Path>) -> PathBuf {
    match (appimage, appdir) {
        (Some(image), Some(dir)) if exe.starts_with(dir) => image.to_path_buf(),
        _ => {
            let text = exe.to_string_lossy();
            PathBuf::from(text.strip_suffix(" (deleted)").unwrap_or(&text))
        }
    }
}

async fn restart_unit() -> bool {
    let systemctl = |args: &'static [&'static str]| {
        tokio::process::Command::new("systemctl")
            .arg("--user")
            .args(args)
            .status()
    };
    let _ = systemctl(&["daemon-reload"]).await;
    systemctl(&["restart", "ulanzi-deckd"])
        .await
        .is_ok_and(|s| s.success())
}

/// Restart the service, then start this window again: the caller quits it on success. An AppImage
/// that was never installed has no systemd unit, so its service is told to stop, and the new window
/// starts the new one.
pub async fn restart(client: &crate::client::Client) -> Result<(), String> {
    if !restart_unit().await {
        client.quit_service().await.map_err(|e| e.to_string())?;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (image, dir) = (std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR"));
    let exe = program_to_start(
        &exe,
        image.as_deref().map(Path::new),
        dir.as_deref().map(Path::new),
    );
    if !exe.exists() {
        return Err(format!("{} is missing", exe.display()));
    }
    let args = relaunch_command(&exe.to_string_lossy());
    std::process::Command::new(&args[0])
        .args(&args[1..])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: &str = "/home/a/.cache/ulanzi-deck/updates/2.1.2/file";

    fn cmd(kind: InstallKind, tools: &[&str]) -> Vec<String> {
        install_command(kind, Path::new(F), |t| tools.contains(&t)).expect("command")
    }

    #[test]
    fn arch_uses_pacman() {
        assert_eq!(
            cmd(InstallKind::Arch, &[]),
            ["pkexec", "pacman", "-U", "--noconfirm", F]
        );
    }

    #[test]
    fn debian_uses_apt() {
        assert_eq!(
            cmd(InstallKind::Deb, &[]),
            ["pkexec", "apt-get", "install", "-y", F]
        );
    }

    #[test]
    fn fedora_uses_dnf_and_opensuse_zypper() {
        assert_eq!(
            cmd(InstallKind::Rpm, &["dnf"]),
            ["pkexec", "dnf", "install", "-y", F]
        );
        assert_eq!(
            cmd(InstallKind::Rpm, &["zypper"]),
            [
                "pkexec",
                "zypper",
                "--non-interactive",
                "install",
                "--allow-unsigned-rpm",
                F
            ]
        );
    }

    #[test]
    fn nothing_to_run_for_the_other_kinds() {
        for k in [InstallKind::AppImage, InstallKind::Tarball, InstallKind::Other] {
            assert_eq!(install_command(k, Path::new(F), |_| true), None);
        }
    }

    #[test]
    fn pkexec_exit_126_is_a_cancel() {
        assert_eq!(outcome(Some(126), ""), Err(InstallFailure::Cancelled));
        assert_eq!(outcome(Some(0), "noise"), Ok(()));
    }

    #[test]
    fn failure_text_is_the_first_line_of_stderr() {
        assert_eq!(
            outcome(Some(1), "\n  error: conflicting files \nmore\n"),
            Err(InstallFailure::Failed("error: conflicting files".into()))
        );
        assert_eq!(
            outcome(None, ""),
            Err(InstallFailure::Failed("The install failed.".into()))
        );
    }

    #[test]
    fn a_replaced_program_is_found_again() {
        let none = None;
        assert_eq!(
            program_to_start(Path::new("/usr/bin/ulanzi-deck (deleted)"), none, none),
            Path::new("/usr/bin/ulanzi-deck")
        );
        assert_eq!(
            program_to_start(Path::new("/usr/bin/ulanzi-deck"), none, none),
            Path::new("/usr/bin/ulanzi-deck")
        );
    }

    #[test]
    fn an_appimage_restarts_itself_but_never_another_one() {
        let (image, dir) = (Path::new("/home/a/Ulanzi.AppImage"), Path::new("/tmp/.mount_u"));
        let inside = Path::new("/tmp/.mount_u/usr/bin/ulanzi-deck");
        assert_eq!(program_to_start(inside, Some(image), Some(dir)), image);
        let outside = Path::new("/usr/bin/ulanzi-deck");
        assert_eq!(program_to_start(outside, Some(image), Some(dir)), outside);
    }

    #[test]
    fn relaunching_waits_for_this_window_to_go() {
        let c = relaunch_command("/usr/bin/ulanzi deck");
        assert_eq!(c[0], "sh");
        assert!(c[2].starts_with("sleep 1"));
        assert_eq!(c[3], "/usr/bin/ulanzi deck");
    }
}
