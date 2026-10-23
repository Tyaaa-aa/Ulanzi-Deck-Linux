//! Volume and sound playback through PipeWire's `wpctl` and the PulseAudio-compatible `pactl`.

use ulanzi_core::model::{AudioTarget, VolumeOp};

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

/// Make the next output the default.
pub async fn switch_output(chosen: &[String]) -> Result<(), ActionError> {
    let sinks = parse_sinks(&pactl(&["-f", "json", "list", "sinks"]).await?);
    let names: Vec<String> = sinks.iter().map(|s| s.0.clone()).collect();
    let current = pactl(&["get-default-sink"]).await.unwrap_or_default();
    let next = next_output(&current, &names, chosen)
        .ok_or_else(|| ActionError::Unavailable("none of this key's outputs is connected".into()))?;
    pactl(&["set-default-sink", &next]).await?;
    let desc = sinks
        .iter()
        .find(|s| s.0 == next)
        .map(|s| s.1.as_str())
        .unwrap_or(&next);
    tracing::info!("default output: {desc}");
    Ok(())
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
pub async fn current_level(input: bool) -> Option<(u8, bool)> {
    let target = if input {
        "@DEFAULT_AUDIO_SOURCE@"
    } else {
        "@DEFAULT_AUDIO_SINK@"
    };
    let out = tokio::process::Command::new("wpctl")
        .args(["get-volume", target])
        .output()
        .await
        .ok()?;
    parse_wpctl_volume(&String::from_utf8_lossy(&out.stdout))
}

pub fn play_sound_argv(path: &str, device: Option<&str>) -> Vec<String> {
    match device {
        Some(d) => s(&["pw-play", "--target", d, path]),
        None => s(&["pw-play", path]),
    }
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

pub fn play_sound(path: &str, device: Option<&str>) -> Result<(), ActionError> {
    if !std::path::Path::new(path).is_file() {
        return Err(ActionError::Failed(format!("sound file {path} is missing")));
    }
    if which("pw-play").is_some() {
        return spawn_detached(&play_sound_argv(path, device));
    }
    let mut argv = s(&["paplay"]);
    if let Some(d) = device {
        argv.push(format!("--device={d}"));
    }
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
