//! Home Assistant as a plugin: toggle, run and set entities and call any service, with live entity
//! state on key labels. The connection logic arrives in a later task; this holds the pure model.
use serde_json::Value;
use ulanzi_plugin::{Cx, Plugin};

pub mod actions;
pub mod protocol;

#[derive(Default)]
pub struct Ha;

impl Plugin for Ha {
    fn action(&mut self, _cx: &mut Cx, action: &str, _params: Value) -> Result<(), String> {
        Err(format!("not implemented yet: {action}"))
    }
}

ulanzi_plugin::export_plugin!(Ha);
