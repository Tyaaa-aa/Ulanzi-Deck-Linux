//! WebAssembly plugins: the manifest (`plugin.toml`) and the permissions it asks for.

mod field;
pub mod key_set;
mod manifest;
pub mod permission;

pub use field::{Choice, Field, FieldKind, ShowIf};
pub use manifest::{
    CONTRACT, Inspected, KeySet, KeySetKey, KeySetVariant, MAX_KEY_SET_KEYS, Manifest, ManifestAction,
    ManifestError, ManifestLabel, ManifestState, Permissions, PickItem, Picker, PickerStyle, Track, is_asset,
};
pub use permission::{describe, host_allowed, is_lan};
