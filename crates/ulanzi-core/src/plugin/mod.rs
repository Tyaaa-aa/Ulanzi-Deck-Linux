//! WebAssembly plugins: the manifest (`plugin.toml`) and the permissions it asks for.

mod field;
mod manifest;
pub mod permission;

pub use field::{Choice, Field, FieldKind};
pub use manifest::{CONTRACT, Inspected, Manifest, ManifestAction, ManifestError, Permissions};
pub use permission::{describe, host_allowed, is_lan};
