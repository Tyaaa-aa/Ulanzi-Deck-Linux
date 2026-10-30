//! Finding, checking and downloading new versions of the app. What to show and which file to pick
//! are in `ulanzi_core::update`.

pub mod verify;

/// Why an update step did not work. `Display` is the sentence shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("Can't reach GitHub. Check your connection and try again.")]
    Offline,
    #[error("This download could not be verified, so it was not installed.")]
    Unverified,
    #[error("Ulanzi Deck can't replace its AppImage in this folder.")]
    NotWritable,
    #[error("{0}")]
    Failed(String),
}
