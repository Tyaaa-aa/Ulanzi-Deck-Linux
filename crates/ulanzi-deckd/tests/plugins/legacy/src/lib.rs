//! A plugin built against the frozen 1.0 contract (`wit/plugin.wit`, a copy of the WIT as it was
//! at 1.0.0), without the SDK: it proves later hosts still load and run 1.0 components. Never
//! update its WIT.
//!
//! Action `ping` sets the state `ping` to `pong`; anything else is refused.

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::all, missing_docs)] // the generated export glue contains `unsafe`
mod bindings {
    wit_bindgen::generate!({ path: "wit", world: "plugin" });
}

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)] // `export!` expands to `unsafe` glue
mod guest {
    use super::bindings::ulanzi::plugin::{core, types};
    use super::bindings::{Guest, export};

    struct Legacy;

    impl Guest for Legacy {
        fn init(_settings: String) -> Result<(), String> {
            Ok(())
        }
        fn on_action(action: String, _params: String) -> Result<(), String> {
            match action.as_str() {
                "ping" => {
                    core::set_state("ping", "pong");
                    Ok(())
                }
                other => Err(format!("unknown action: {other}")),
            }
        }
        fn on_open(_socket: u32) {}
        fn on_message(_socket: u32, _data: types::Payload) {}
        fn on_closed(_socket: u32, _reason: String) {}
        fn on_http(_request: u32, _response: Result<types::HttpResponse, String>) {}
        fn on_timer(_timer: u32) {}
    }

    export!(Legacy with_types_in super::bindings);
}
