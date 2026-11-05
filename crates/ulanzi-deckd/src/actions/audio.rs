//! Volume and sound playback through PipeWire's `wpctl` and the PulseAudio-compatible `pactl`.

use std::collections::BTreeMap;

use ulanzi_core::model::{AudioTarget, OutputSettings, VolumeOp};

use super::ActionError;
use super::command::{run_capture, spawn_detached, which};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Commands for the default output/input (wpctl) or a named output device (pactl).
pub fn volume_argv(target: &AudioTarget, op: &VolumeOp, step: u8) -> Vec<Vec<String>> {
    match target {
        AudioTarget::Default | AudioTarget::Input => {
            let node = if *target == AudioTarget::Input {
                "@DEFAULT_AUDIO_SOURCE@"
            } else {
                "@DEFAULT_AUDIO_SINK@"
            };
            let cmd = match op {
                VolumeOp::Up => s(&["wpctl", "set-volume", node, &format!("{step}%+")]),
                VolumeOp::Down => s(&["wpctl", "set-volume", node, &format!("{step}%-")]),
                VolumeOp::Mute => s(&["wpctl", "set-mute", node, "toggle"]),
                VolumeOp::Set { percent } => s(&["wpctl", "set-volume", node, &format!("{percent}%")]),
            };
            vec![cmd]
        }
        AudioTarget::Device { name } => {
            let cmd = match op {
                VolumeOp::Up => s(&["pactl", "set-sink-volume", name, &format!("+{step}%")]),
                VolumeOp::Down => s(&["pactl", "set-sink-volume", name, &format!("-{step}%")]),
                VolumeOp::Mute => s(&["pactl", "set-sink-mute", name, "toggle"]),
                VolumeOp::Set { percent } => s(&["pactl", "set-sink-volume", name, &format!("{percent}%")]),
            };
            vec![cmd]
        }
        AudioTarget::App { .. } => Vec::new(),
    }
}

/// Sink inputs (app streams) whose app name, binary or media name contains `name` (case-insensitive),
/// with their current volume in percent, from `pactl -f json list sink-inputs`.
pub fn match_sink_inputs(pactl_json: &str, name: &str) -> Vec<(u32, u8)> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(pactl_json) else {
        return Vec::new();
    };
    let needle = name.to_lowercase();
    items
        .iter()
        .filter_map(|it| {
            let props = it.get("properties")?;
            let hit = ["application.name", "application.process.binary", "media.name"]
                .iter()
                .any(|k| {
                    props
                        .get(*k)
                        .and_then(|v| v.as_str())
                        .is_some_and(|v| v.to_lowercase().contains(&needle))
                });
            if !hit {
                return None;
            }
            let id = it.get("index")?.as_u64()? as u32;
            let pct = it
                .get("volume")
                .and_then(|v| v.as_object())
                .and_then(|ch| ch.values().next())
                .and_then(|c| c.get("value_percent"))
                .and_then(|p| p.as_str())
                .and_then(|p| p.trim_end_matches('%').trim().parse::<u32>().ok())
                .unwrap_or(0)
                .min(255) as u8;
            Some((id, pct))
        })
        .collect()
}

/// Commands for app streams; with `limit_100`, streams already at 100% are not raised.
pub fn app_volume_argv(inputs: &[(u32, u8)], op: &VolumeOp, step: u8, limit_100: bool) -> Vec<Vec<String>> {
    inputs
        .iter()
        .filter(|(_, pct)| !(limit_100 && *op == VolumeOp::Up && *pct >= 100))
        .map(|(id, _)| {
            let id = id.to_string();
            match op {
                VolumeOp::Up => s(&["pactl", "set-sink-input-volume", &id, &format!("+{step}%")]),
                VolumeOp::Down => s(&["pactl", "set-sink-input-volume", &id, &format!("-{step}%")]),
                VolumeOp::Mute => s(&["pactl", "set-sink-input-mute", &id, "toggle"]),
                VolumeOp::Set { percent } => {
                    s(&["pactl", "set-sink-input-volume", &id, &format!("{percent}%")])
                }
            }
        })
        .collect()
}

/// (name, description) of each output from `pactl -f json list sinks`.
pub fn parse_sinks(json: &str) -> Vec<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let name = s.get("name")?.as_str()?.to_string();
            let desc = s
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or(&name)
                .to_string();
            Some((name, desc))
        })
        .collect()
}

/// The output after `current`, among `chosen` (every output when empty) that are plugged in.
pub fn next_output(current: &str, available: &[String], chosen: &[String]) -> Option<String> {
    let ring: Vec<&String> = if chosen.is_empty() {
        available.iter().collect()
    } else {
        chosen.iter().filter(|c| available.contains(c)).collect()
    };
    let at = ring.iter().position(|n| *n == current);
    let next = match at {
        Some(i) => ring.get((i + 1) % ring.len()),
        None => ring.first(),
    };
    next.map(|n| (*n).clone())
}

async fn pactl(args: &[&str]) -> Result<String, ActionError> {
    let out = tokio::process::Command::new("pactl")
        .args(args)
        .output()
        .await
        .map_err(|_| ActionError::Unavailable("switching outputs needs pactl (pipewire-pulse)".into()))?;
    if !out.status.success() {
        return Err(ActionError::Failed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Make the next output the default. Its volume and its apps' volumes are set first, so nothing
/// plays loud on it; apps making no sound now get theirs when they next do.
pub async fn switch_output(
    chosen: &[String],
    outputs: &BTreeMap<String, OutputSettings>,
) -> Result<(), ActionError> {
    let sinks = parse_sinks(&pactl(&["-f", "json", "list", "sinks"]).await?);
    let names: Vec<String> = sinks.iter().map(|s| s.0.clone()).collect();
    let current = pactl(&["get-default-sink"]).await.unwrap_or_default();
    let next = next_output(&current, &names, chosen)
        .ok_or_else(|| ActionError::Unavailable("none of this key's outputs is connected".into()))?;
    let inputs = match outputs.get(&next) {
        Some(o) if !o.apps.is_empty() => pactl(&["-f", "json", "list", "sink-inputs"])
            .await
            .unwrap_or_default(),
        _ => String::new(),
    };
    let (cmds, waiting) = switch_commands(&next, outputs.get(&next), &inputs);
    set_pending(PendingApps {
        output: next.clone(),
        apps: waiting,
    });
    // App streams may end meanwhile; only the switch itself must work.
    let last = cmds.len() - 1;
    for (i, c) in cmds.iter().enumerate() {
        match run_capture(c).await {
            Err(e) if i == last => return Err(e),
            Err(e) => tracing::debug!("{}: {e}", c.join(" ")),
            Ok(_) => {}
        }
    }
    let desc = sinks
        .iter()
        .find(|s| s.0 == next)
        .map(|s| s.1.as_str())
        .unwrap_or(&next);
    tracing::info!("default output: {desc}");
    Ok(())
}

/// The commands that switch to output `next` with `settings`, levels first and the switch last,
/// and the apps of `settings` with no stream in `inputs_json` (`pactl -f json list sink-inputs`).
pub fn switch_commands(
    next: &str,
    settings: Option<&OutputSettings>,
    inputs_json: &str,
) -> (Vec<Vec<String>>, BTreeMap<String, u8>) {
    let mut cmds = Vec::new();
    let mut waiting = BTreeMap::new();
    if let Some(o) = settings {
        waiting = o.apps.clone();
        for (id, pct) in take_playing(&mut waiting, inputs_json) {
            cmds.push(s(&[
                "pactl",
                "set-sink-input-volume",
                &id.to_string(),
                &format!("{pct}%"),
            ]));
        }
        if let Some(v) = o.volume {
            cmds.push(s(&["pactl", "set-sink-volume", next, &format!("{v}%")]));
        }
    }
    cmds.push(s(&["pactl", "set-default-sink", next]));
    (cmds, waiting)
}

async fn set_stream_volume(id: u32, percent: u8) {
    if let Err(e) = pactl(&["set-sink-input-volume", &id.to_string(), &format!("{percent}%")]).await {
        tracing::debug!("cannot set the volume of sound stream {id}: {e}");
    }
}

/// App volumes a switch could not set because the app made no sound then: each is set on the
/// app's next sound, while `output` is still the default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingApps {
    pub output: String,
    pub apps: BTreeMap<String, u8>,
}

static PENDING: std::sync::Mutex<Option<PendingApps>> = std::sync::Mutex::new(None);

fn set_pending(p: PendingApps) {
    if let Ok(mut slot) = PENDING.lock() {
        *slot = (!p.apps.is_empty()).then_some(p);
    }
}

/// The streams in `inputs_json` (`pactl -f json list sink-inputs`) of the apps in `apps`, each
/// with its app's volume. Apps with a stream are taken out of `apps`.
pub fn take_playing(apps: &mut BTreeMap<String, u8>, inputs_json: &str) -> Vec<(u32, u8)> {
    let mut out = Vec::new();
    apps.retain(|name, pct| {
        let streams = match_sink_inputs(inputs_json, name);
        out.extend(streams.iter().map(|(id, _)| (*id, *pct)));
        streams.is_empty()
    });
    out
}

/// The default output changed to `now`: app volumes waiting for another output are dropped.
fn output_changed(now: &str) {
    if let Ok(mut slot) = PENDING.lock()
        && slot.as_ref().is_some_and(|p| p.output != now)
    {
        *slot = None;
    }
}

/// A sound stream started: set the volumes of the apps waiting for one.
async fn stream_started() {
    let mut apps = match PENDING.lock() {
        Ok(slot) => match slot.as_ref() {
            Some(p) => p.apps.clone(),
            None => return,
        },
        Err(_) => return,
    };
    let Ok(json) = pactl(&["-f", "json", "list", "sink-inputs"]).await else {
        return;
    };
    let streams = take_playing(&mut apps, &json);
    if streams.is_empty() {
        return;
    }
    if let Ok(mut slot) = PENDING.lock()
        && let Some(p) = slot.as_mut()
    {
        p.apps.retain(|name, _| apps.contains_key(name));
        if p.apps.is_empty() {
            *slot = None;
        }
    }
    for (id, pct) in &streams {
        set_stream_volume(*id, *pct).await;
    }
    // The sound server may still put back the app's last volume as the stream settles.
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    for (id, pct) in &streams {
        set_stream_volume(*id, *pct).await;
    }
}

/// What a `pactl subscribe` line is about.
#[derive(Debug, PartialEq, Eq)]
pub enum SoundEvent {
    /// The server changed, which includes its default output.
    Server,
    /// An output came or went.
    Output,
    /// An app's sound stream started.
    StreamStarted,
    Other,
}

/// Reads a `pactl subscribe` line, such as "Event 'new' on sink-input #42".
pub fn parse_sound_event(line: &str) -> SoundEvent {
    let mut words = line.split_whitespace();
    let (Some("Event"), Some(kind), Some("on"), Some(facility)) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return SoundEvent::Other;
    };
    match (kind.trim_matches('\''), facility) {
        (_, "server") => SoundEvent::Server,
        ("new" | "remove", "sink") => SoundEvent::Output,
        ("new", "sink-input") => SoundEvent::StreamStarted,
        _ => SoundEvent::Other,
    }
}

/// The default output: (name, description).
pub async fn default_output() -> Option<(String, String)> {
    let name = pactl(&["get-default-sink"]).await.ok()?;
    let sinks = parse_sinks(&pactl(&["-f", "json", "list", "sinks"]).await.ok()?);
    let desc = sinks
        .into_iter()
        .find(|s| s.0 == name)
        .map(|s| s.1)
        .unwrap_or_else(|| name.clone());
    Some((name, desc))
}

/// Follow the sound server: `changed` gets the default output whenever it changes, and apps
/// waiting for a volume get it when they start making sound. Starts over when the server restarts.
pub async fn watch_outputs(changed: impl Fn(String, String) + Send + 'static) {
    use tokio::io::AsyncBufReadExt;
    let mut last: Option<String> = None;
    loop {
        let child = tokio::process::Command::new("pactl")
            .arg("subscribe")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();
        let Ok(mut child) = child else {
            tracing::info!("pactl is missing: keys do not follow the default output");
            return;
        };
        let mut check = true;
        if let Some(out) = child.stdout.take() {
            let mut lines = tokio::io::BufReader::new(out).lines();
            loop {
                if check {
                    check = false;
                    if let Some((name, desc)) = default_output().await
                        && last.as_deref() != Some(name.as_str())
                    {
                        output_changed(&name);
                        last = Some(name.clone());
                        changed(name, desc);
                    }
                }
                let Ok(Some(line)) = lines.next_line().await else {
                    break;
                };
                match parse_sound_event(&line) {
                    SoundEvent::Server | SoundEvent::Output => check = true,
                    SoundEvent::StreamStarted => {
                        tokio::spawn(stream_started());
                    }
                    SoundEvent::Other => {}
                }
            }
        }
        let _ = child.kill().await;
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// Percent and mute state from `wpctl get-volume` ("Volume: 0.45 [MUTED]").
pub fn parse_wpctl_volume(out: &str) -> Option<(u8, bool)> {
    let v: f32 = out.split_whitespace().nth(1)?.parse().ok()?;
    Some((
        (v * 100.0).round().clamp(0.0, 255.0) as u8,
        out.contains("[MUTED]"),
    ))
}

/// The current level of the default output (or the microphone), for the volume overlay.
/// The average of a pactl `volume` object's channels, in percent.
fn channel_percent(v: &serde_json::Value) -> Option<u8> {
    let ch: Vec<u32> = v
        .as_object()?
        .values()
        .filter_map(|c| {
            c.get("value_percent")?
                .as_str()?
                .trim_end_matches('%')
                .trim()
                .parse()
                .ok()
        })
        .collect();
    (!ch.is_empty()).then(|| (ch.iter().sum::<u32>() / ch.len() as u32).min(255) as u8)
}

/// Level, mute and description of the output `name`, from `pactl -f json list sinks`.
pub fn sink_level(json: &str, name: &str) -> Option<(u8, bool, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let s = v
        .as_array()?
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some(name))?;
    Some((
        channel_percent(s.get("volume")?)?,
        s.get("mute").and_then(|m| m.as_bool()).unwrap_or(false),
        s.get("description")
            .and_then(|d| d.as_str())
            .unwrap_or(name)
            .to_string(),
    ))
}

/// Level, mute and name of the first sound stream of app `name`, from
/// `pactl -f json list sink-inputs` (matched like the volume action matches).
pub fn app_level(json: &str, name: &str) -> Option<(u8, bool, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let needle = name.to_lowercase();
    let it = v.as_array()?.iter().find(|it| {
        let props = it.get("properties");
        ["application.name", "application.process.binary", "media.name"]
            .iter()
            .any(|k| {
                props
                    .and_then(|p| p.get(*k))
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| v.to_lowercase().contains(&needle))
            })
    })?;
    let app = it
        .get("properties")
        .and_then(|p| p.get("application.name"))
        .and_then(|v| v.as_str())
        .unwrap_or(name);
    // "VLC media player (LibVLC 3.0.24)" → "VLC media player"
    let app = app.split(" (").next().unwrap_or(app).trim().to_string();
    Some((
        channel_percent(it.get("volume")?)?,
        it.get("mute").and_then(|m| m.as_bool()).unwrap_or(false),
        app,
    ))
}

/// The level after a volume action, for the overlay: (percent, muted, what it is).
/// The default output has no name; the others are named on the overlay.
pub async fn current_level(target: &AudioTarget) -> Option<(u8, bool, String)> {
    match target {
        AudioTarget::Default | AudioTarget::Input => {
            let node = if *target == AudioTarget::Input {
                "@DEFAULT_AUDIO_SOURCE@"
            } else {
                "@DEFAULT_AUDIO_SINK@"
            };
            let out = tokio::process::Command::new("wpctl")
                .args(["get-volume", node])
                .output()
                .await
                .ok()?;
            let (p, m) = parse_wpctl_volume(&String::from_utf8_lossy(&out.stdout))?;
            let label = if *target == AudioTarget::Input {
                "Microphone"
            } else {
                ""
            };
            Some((p, m, label.to_string()))
        }
        AudioTarget::Device { name } => {
            let json = run_capture(&s(&["pactl", "-f", "json", "list", "sinks"]))
                .await
                .ok()?;
            sink_level(&json, name)
        }
        AudioTarget::App { name } => {
            let json = run_capture(&s(&["pactl", "-f", "json", "list", "sink-inputs"]))
                .await
                .ok()?;
            app_level(&json, name)
        }
    }
}

/// Volume of Play Sound keys from Preferences, percent; each key's own volume is a share of it.
static SOUND_VOLUME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(100);

/// Set the Preferences sound volume (percent).
pub fn set_sound_volume(percent: u8) {
    SOUND_VOLUME.store(percent.min(100), std::sync::atomic::Ordering::Relaxed);
}

/// A key's volume (percent) with the Preferences one applied, from 0.0 to 1.0.
pub fn sound_level(percent: u8) -> f32 {
    let global = SOUND_VOLUME.load(std::sync::atomic::Ordering::Relaxed);
    (percent.min(100) as f32 / 100.0) * (global.min(100) as f32 / 100.0)
}

/// pw-play for `path` at `level` (0.0 to 1.0).
pub fn play_sound_argv(path: &str, device: Option<&str>, level: f32) -> Vec<String> {
    let mut argv = s(&["pw-play"]);
    if let Some(d) = device {
        argv.extend(s(&["--target", d]));
    }
    if level < 1.0 {
        argv.push(format!("--volume={:.2}", level.max(0.0)));
    }
    argv.push(path.to_string());
    argv
}

/// Current volume of the default output/input from `wpctl get-volume` (1.0 = 100%).
async fn wpctl_level(target: &AudioTarget) -> Option<f32> {
    let node = if *target == AudioTarget::Input {
        "@DEFAULT_AUDIO_SOURCE@"
    } else {
        "@DEFAULT_AUDIO_SINK@"
    };
    let out = run_capture(&s(&["wpctl", "get-volume", node])).await.ok()?;
    out.split_whitespace().nth(1)?.parse().ok()
}

async fn device_percent(name: &str) -> Option<u32> {
    let out = run_capture(&s(&["pactl", "get-sink-volume", name])).await.ok()?;
    out.split('%')
        .next()?
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// Change a volume, as configured.
pub async fn volume(
    target: &AudioTarget,
    op: &VolumeOp,
    step: u8,
    limit_100: bool,
) -> Result<(), ActionError> {
    let at_max = limit_100
        && *op == VolumeOp::Up
        && match target {
            AudioTarget::Default | AudioTarget::Input => wpctl_level(target).await.is_some_and(|v| v >= 1.0),
            AudioTarget::Device { name } => device_percent(name).await.is_some_and(|p| p >= 100),
            AudioTarget::App { .. } => false,
        };
    if at_max {
        return Ok(());
    }
    let cmds = match target {
        AudioTarget::App { name } => {
            let json = run_capture(&s(&["pactl", "-f", "json", "list", "sink-inputs"])).await?;
            let inputs = match_sink_inputs(&json, name);
            if inputs.is_empty() {
                return Err(ActionError::Failed(format!(
                    "{name} is not playing any sound right now"
                )));
            }
            app_volume_argv(&inputs, op, step, limit_100)
        }
        _ => volume_argv(target, op, step),
    };
    for c in cmds {
        run_capture(&c).await?;
    }
    Ok(())
}

/// Play `path` at `volume` percent of the Preferences sound volume.
pub fn play_sound(path: &str, device: Option<&str>, volume: u8) -> Result<(), ActionError> {
    if !std::path::Path::new(path).is_file() {
        return Err(ActionError::Failed(format!("sound file {path} is missing")));
    }
    let level = sound_level(volume);
    if which("pw-play").is_some() {
        return spawn_detached(&play_sound_argv(path, device, level));
    }
    let mut argv = s(&["paplay"]);
    if let Some(d) = device {
        argv.push(format!("--device={d}"));
    }
    // paplay: 65536 is full volume.
    argv.push(format!("--volume={}", (level * 65536.0).round() as u32));
    argv.push(path.to_string());
    spawn_detached(&argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpctl_volume_parses() {
        assert_eq!(parse_wpctl_volume("Volume: 0.45\n"), Some((45, false)));
        assert_eq!(parse_wpctl_volume("Volume: 1.20 [MUTED]"), Some((120, true)));
        assert_eq!(parse_wpctl_volume("nothing"), None);
    }
}
