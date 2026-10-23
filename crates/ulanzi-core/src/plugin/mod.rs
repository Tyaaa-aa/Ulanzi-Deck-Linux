//! WebAssembly plugins: the manifest (`plugin.toml`) and the permissions it asks for.

mod manifest;
mod permission;

pub use manifest::{CONTRACT, Manifest, ManifestAction, ManifestError, Permissions};
pub use permission::{host_allowed, is_lan};
