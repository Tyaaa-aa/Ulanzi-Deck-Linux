//! Turn a key press into a Home Assistant service call. No I/O.
use serde_json::{Map, Value};

use crate::protocol::{Tracked, valid_entity_id};

/// Largest `data` text for the "Call a service" action.
const MAX_DATA_BYTES: usize = 4096;
const DEFAULT_STEP: f64 = 5.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceCall {
    pub domain: String,
    pub service: String,
    pub entity_id: Option<String>,
    pub data: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    Call(ServiceCall),
    Reconnect,
    ForgetToken,
}

fn call(domain: &str, service: &str, entity: &str, data: Map<String, Value>) -> Plan {
    Plan::Call(ServiceCall {
        domain: domain.into(),
        service: service.into(),
        entity_id: Some(entity.into()),
        data,
    })
}

fn data_of(key: &str, value: Value) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(key.into(), value);
    m
}

fn entity_of(params: &Value) -> Result<&str, String> {
    let id = params
        .get("entity_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if id.is_empty() {
        return Err("Choose an entity for this key first".to_string());
    }
    if valid_entity_id(id) {
        Ok(id)
    } else {
        Err(format!(
            "entity_id \"{}\" is not valid: use the form light.living_room (lowercase letters, digits and _)",
            id.chars().take(60).collect::<String>()
        ))
    }
}

fn number(params: &Value, key: &str) -> Option<f64> {
    match params.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

fn is_word(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

pub fn plan(action: &str, params: &Value, tracked: &Tracked) -> Result<Plan, String> {
    match action {
        "toggle" | "turn_on" | "turn_off" => {
            Ok(call("homeassistant", action, entity_of(params)?, Map::new()))
        }
        "run" => run(entity_of(params)?),
        "set_level" => set_level(params, tracked),
        "call_service" => call_service(params),
        "reconnect" => Ok(Plan::Reconnect),
        "forget_token" => Ok(Plan::ForgetToken),
        other => Err(format!("unknown action: {other}")),
    }
}

fn run(entity: &str) -> Result<Plan, String> {
    let domain = entity.split('.').next().unwrap_or_default();
    let service = match domain {
        "scene" | "script" => "turn_on",
        "automation" => "trigger",
        "button" | "input_button" => "press",
        _ => {
            return Err(format!(
                "run works on scene, script, automation, button and input_button entities, not {entity} (use toggle, turn on or turn off)"
            ));
        }
    };
    Ok(call(domain, service, entity, Map::new()))
}

fn set_level(params: &Value, tracked: &Tracked) -> Result<Plan, String> {
    let entity = entity_of(params)?;
    let domain = entity.split('.').next().unwrap_or_default();
    let op = params.get("op").and_then(Value::as_str).unwrap_or("set");
    let step = number(params, "step")
        .unwrap_or(DEFAULT_STEP)
        .round()
        .clamp(1.0, 50.0) as i64;
    let percent = || {
        number(params, "percent")
            .map(|p| p.round().clamp(0.0, 100.0) as i64)
            .ok_or_else(|| "set needs a percent from 0 to 100".to_string())
    };
    let signed = match op {
        "set" => 0,
        "up" => step,
        "down" => -step,
        other => return Err(format!("op must be set, up or down, not \"{other}\"")),
    };
    match domain {
        "light" => Ok(if op == "set" {
            call(
                "light",
                "turn_on",
                entity,
                data_of("brightness_pct", percent()?.into()),
            )
        } else {
            call(
                "light",
                "turn_on",
                entity,
                data_of("brightness_step_pct", signed.into()),
            )
        }),
        "media_player" => Ok(match op {
            "set" => {
                let level = percent()? as f64 / 100.0;
                call(
                    "media_player",
                    "volume_set",
                    entity,
                    data_of("volume_level", level.into()),
                )
            }
            "up" => call("media_player", "volume_up", entity, Map::new()),
            _ => call("media_player", "volume_down", entity, Map::new()),
        }),
        "fan" => Ok(match op {
            "set" => call(
                "fan",
                "set_percentage",
                entity,
                data_of("percentage", percent()?.into()),
            ),
            "up" => call(
                "fan",
                "increase_speed",
                entity,
                data_of("percentage_step", step.into()),
            ),
            _ => call(
                "fan",
                "decrease_speed",
                entity,
                data_of("percentage_step", step.into()),
            ),
        }),
        "cover" => {
            let position = if op == "set" {
                percent()?
            } else {
                let now = tracked
                    .entities
                    .get(entity)
                    .and_then(|e| e.attrs.get("current_position"))
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        format!("{entity} has no known position yet (list it under entities so its position is tracked)")
                    })?;
                (now.round() as i64 + signed).clamp(0, 100)
            };
            Ok(call(
                "cover",
                "set_cover_position",
                entity,
                data_of("position", position.into()),
            ))
        }
        _ => Err(format!(
            "set_level works on light, media_player, fan and cover entities, not {entity}"
        )),
    }
}

fn call_service(params: &Value) -> Result<Plan, String> {
    let text = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or_default().trim();
    let (domain, service) = (text("domain"), text("service"));
    if !is_word(domain) || !is_word(service) {
        return Err("domain and service must be lowercase letters, digits and _ (for example notify and mobile_app_phone)".into());
    }
    let entity_id = match text("entity_id") {
        "" => None,
        e if valid_entity_id(e) => Some(e.to_string()),
        _ => return Err("entity_id is not valid: use the form light.living_room, or leave it empty".into()),
    };
    let data = match params.get("data") {
        Some(Value::Object(m)) => {
            if Value::Object(m.clone()).to_string().len() > MAX_DATA_BYTES {
                return Err(format!("the JSON data is longer than {MAX_DATA_BYTES} bytes"));
            }
            m.clone()
        }
        Some(Value::String(s)) if !s.trim().is_empty() => {
            if s.len() > MAX_DATA_BYTES {
                return Err(format!("the JSON data is longer than {MAX_DATA_BYTES} bytes"));
            }
            match serde_json::from_str::<Value>(s) {
                Ok(Value::Object(m)) => m,
                Ok(_) => return Err("the JSON data must be an object like {\"message\": \"hi\"}".into()),
                Err(_) => return Err("the JSON data is not valid JSON".into()),
            }
        }
        None | Some(Value::Null) | Some(Value::String(_)) => Map::new(),
        Some(_) => return Err("the JSON data must be an object like {\"message\": \"hi\"}".into()),
    };
    Ok(Plan::Call(ServiceCall {
        domain: domain.into(),
        service: service.into(),
        entity_id,
        data,
    }))
}
