//! Finding, checking and downloading new versions of the app. What to show and which file to pick
//! are in `ulanzi_core::update`.

pub mod check;
pub mod download;
pub mod kind;
pub mod verify;

use std::time::Duration;

use ulanzi_core::store::Store;

/// Time after the service starts before the first check.
const FIRST_CHECK: Duration = Duration::from_secs(120);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const HOUR_MS: u64 = 60 * 60 * 1000;

/// Check for updates once a day until the service stops, so long as checking is on.
pub async fn run(store: Store) {
    tokio::time::sleep(FIRST_CHECK).await;
    loop {
        let (st, source) = (store.clone(), check::Source::real());
        if ulanzi_core::update::UpdateState::load(&st).check {
            let _ = tokio::task::spawn_blocking(move || check::run(&st, &source, false)).await;
        }
        // Spread out, so everyone does not ask at the same moment.
        let jitter = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64 % HOUR_MS)
            .unwrap_or(0);
        tokio::time::sleep(DAY + Duration::from_millis(jitter)).await;
    }
}

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
