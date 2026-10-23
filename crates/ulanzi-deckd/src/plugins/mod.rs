//! WebAssembly plugin host.

// The generated bindings cover the whole contract; not every item is used yet.
#[allow(dead_code)]
mod bindings {
    wasmtime::component::bindgen!({ world: "plugin", path: "../ulanzi-plugin/wit" });
}
