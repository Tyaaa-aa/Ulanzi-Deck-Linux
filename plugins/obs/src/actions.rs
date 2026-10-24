//! The deck's OBS actions: parsing a key's action id and params, and planning the requests.

use serde_json::{Value, json};

use crate::protocol::Tracked;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeOp {
    Up,
    Down,
    Mute,
    Set { percent: u8 },
}

/// What a key can do in OBS Studio (obs-websocket 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsAction {
    /// Show a scene (on the preview instead, in studio mode, with `preview`).
    SetScene {
        scene: String,
        preview: bool,
    },
    ToggleScenes {
        a: String,
        b: String,
    },
    ToggleSource {
        scene: String,
        source: String,
    },
    ToggleRecord,
    StartRecord,
    StopRecord,
    /// Pause or resume the recording.
    PauseRecord,
    ToggleStream,
    StartStream,
    StopStream,
    ToggleReplay,
    SaveReplay,
    ToggleVirtualCam,
    ToggleMute {
        input: String,
    },
    /// Change an input's volume; `step` is in dB.
    InputVolume {
        input: String,
        op: VolumeOp,
        step: u8,
    },
    ToggleFilter {
        source: String,
        filter: String,
    },
    /// Studio mode: send the preview to the program.
    StudioTransition,
    SetCollection {
        name: String,
    },
    SetProfile {
        name: String,
    },
    /// OBS's own screenshot of the program (saved where OBS saves them).
    Screenshot,
}

/// How to carry out an action.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    Requests(Vec<(String, Value)>),
    /// Read the input's volume, then set it ([`volume_db`]).
    InputVolume {
        input: String,
        op: VolumeOp,
        step: u8,
    },
    /// Read the filter's state, then flip it.
    ToggleFilter {
        source: String,
        filter: String,
    },
}

fn text(params: &Value, key: &str, what: &str) -> Result<String, String> {
    match params.get(key).and_then(Value::as_str) {
        Some(v) if !v.trim().is_empty() => Ok(v.to_string()),
        _ => Err(format!("Choose {what} for this key first")),
    }
}

/// Turn a plugin action id and its params into an [`ObsAction`].
pub fn parse_action(action: &str, params: &Value) -> Result<ObsAction, String> {
    use ObsAction::*;
    Ok(match action {
        "set_scene" => SetScene {
            scene: text(params, "scene", "a scene")?,
            preview: params.get("preview").and_then(Value::as_bool).unwrap_or(false),
        },
        "toggle_scenes" => {
            let a = text(params, "a", "two scenes")?;
            ToggleScenes {
                a,
                b: text(params, "b", "two scenes")?,
            }
        }
        "toggle_source" => {
            let scene = text(params, "scene", "a scene")?;
            ToggleSource {
                scene,
                source: text(params, "source", "a source")?,
            }
        }
        "toggle_record" => ToggleRecord,
        "start_record" => StartRecord,
        "stop_record" => StopRecord,
        "pause_record" => PauseRecord,
        "toggle_stream" => ToggleStream,
        "start_stream" => StartStream,
        "stop_stream" => StopStream,
        "toggle_replay" => ToggleReplay,
        "save_replay" => SaveReplay,
        "toggle_virtual_cam" => ToggleVirtualCam,
        "studio_transition" => StudioTransition,
        "screenshot" => Screenshot,
        "toggle_mute" => ToggleMute {
            input: text(params, "input", "an audio input")?,
        },
        "input_volume" => {
            let input = text(params, "input", "an audio input")?;
            let op = match params.get("op").and_then(Value::as_str).unwrap_or_default() {
                "up" => VolumeOp::Up,
                "down" => VolumeOp::Down,
                "mute" => VolumeOp::Mute,
                "set" => VolumeOp::Set {
                    percent: params
                        .get("percent")
                        .and_then(Value::as_f64)
                        .map_or(100, |p| p.round().clamp(0.0, 100.0) as u8),
                },
                _ => return Err("op must be up, down, mute or set".to_string()),
            };
            let step = params
                .get("step")
                .and_then(Value::as_f64)
                .map_or(2, |s| s.round().clamp(0.0, 255.0) as u8);
            InputVolume { input, op, step }
        }
        "toggle_filter" => {
            let source = text(params, "source", "a source")?;
            ToggleFilter {
                source,
                filter: text(params, "filter", "a filter")?,
            }
        }
        "set_collection" => SetCollection {
            name: text(params, "name", "a scene collection")?,
        },
        "set_profile" => SetProfile {
            name: text(params, "name", "an OBS profile")?,
        },
        other => return Err(format!("unknown action: {other}")),
    })
}

fn one(ty: &str, data: Value) -> Result<Plan, String> {
    Ok(Plan::Requests(vec![(ty.to_string(), data)]))
}

fn need(what: &str, v: &str) -> Result<(), String> {
    if v.trim().is_empty() {
        Err(format!("Choose {what} for this key first"))
    } else {
        Ok(())
    }
}

pub fn plan(a: &ObsAction, t: &Tracked) -> Result<Plan, String> {
    use ObsAction::*;
    match a {
        SetScene { scene, preview } => {
            need("a scene", scene)?;
            let ty = if *preview {
                "SetCurrentPreviewScene"
            } else {
                "SetCurrentProgramScene"
            };
            one(ty, json!({"sceneName": scene}))
        }
        ToggleScenes { a, b } => {
            need("two scenes", a)?;
            need("two scenes", b)?;
            let to = if t.live.program == *a { b } else { a };
            one("SetCurrentProgramScene", json!({"sceneName": to}))
        }
        ToggleSource { scene, source } => {
            need("a scene", scene)?;
            need("a source", source)?;
            let (id, on) = t
                .items
                .iter()
                .find(|((s, _), (name, _))| s == scene && name == source)
                .map(|((_, id), (_, on))| (*id, *on))
                .ok_or_else(|| format!("“{source}” is not in the scene “{scene}”"))?;
            one(
                "SetSceneItemEnabled",
                json!({"sceneName": scene, "sceneItemId": id, "sceneItemEnabled": !on}),
            )
        }
        ToggleRecord => one("ToggleRecord", json!({})),
        StartRecord => one("StartRecord", json!({})),
        StopRecord => one("StopRecord", json!({})),
        PauseRecord => one("ToggleRecordPause", json!({})),
        ToggleStream => one("ToggleStream", json!({})),
        StartStream => one("StartStream", json!({})),
        StopStream => one("StopStream", json!({})),
        ToggleReplay => one("ToggleReplayBuffer", json!({})),
        SaveReplay => one("SaveReplayBuffer", json!({})),
        ToggleVirtualCam => one("ToggleVirtualCam", json!({})),
        ToggleMute { input } => {
            need("an audio input", input)?;
            one("ToggleInputMute", json!({"inputName": input}))
        }
        InputVolume { input, op, step } => {
            need("an audio input", input)?;
            if *op == VolumeOp::Mute {
                return one("ToggleInputMute", json!({"inputName": input}));
            }
            Ok(Plan::InputVolume {
                input: input.clone(),
                op: *op,
                step: *step,
            })
        }
        ToggleFilter { source, filter } => {
            need("a source", source)?;
            need("a filter", filter)?;
            Ok(Plan::ToggleFilter {
                source: source.clone(),
                filter: filter.clone(),
            })
        }
        StudioTransition => one("TriggerStudioModeTransition", json!({})),
        SetCollection { name } => {
            need("a scene collection", name)?;
            one("SetCurrentSceneCollection", json!({"sceneCollectionName": name}))
        }
        SetProfile { name } => {
            need("an OBS profile", name)?;
            one("SetCurrentProfile", json!({"profileName": name}))
        }
        Screenshot => one(
            "TriggerHotkeyByName",
            json!({"hotkeyName": "OBSBasic.Screenshot"}),
        ),
    }
}

/// The new volume in dB: steps clamp to −100…0 dB; `Set` is percent of full (linear) volume.
pub fn volume_db(current_db: f64, op: VolumeOp, step: u8) -> f64 {
    let v = match op {
        VolumeOp::Up => current_db + step as f64,
        VolumeOp::Down => current_db - step as f64,
        VolumeOp::Set { percent: 0 } => -100.0,
        VolumeOp::Set { percent } => 20.0 * (percent.min(100) as f64 / 100.0).log10(),
        VolumeOp::Mute => current_db,
    };
    v.clamp(-100.0, 0.0)
}
