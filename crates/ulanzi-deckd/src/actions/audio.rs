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
