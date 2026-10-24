//! OBS Studio as a plugin: scenes, recording, streaming, replay buffer, virtual camera and audio,
//! with live state on key labels.
use ulanzi_plugin::{Cx, Plugin};

pub mod actions;
pub mod protocol;

#[derive(Default)]
pub struct Obs;

impl Plugin for Obs {
    fn action(&mut self, _cx: &mut Cx, _action: &str, _params: ulanzi_plugin::Value) -> Result<(), String> {
        Err("not yet".into())
    }
}

ulanzi_plugin::export_plugin!(Obs);
