//! Asking GitHub for the latest release. An automatic check that fails says nothing; **Check Now**
//! reports it.

use std::time::Duration;

use ulanzi_core::store::Store;
use ulanzi_core::update::{self, InstallKind, Release, UpdateState, UpdateStatus};

use super::UpdateError;

const LATEST: &str = "https://api.github.com/repos/Tyaaa-aa/Ulanzi-Deck-Linux/releases/latest";

/// Where to ask, and what this copy is.
#[derive(Debug, Clone)]
pub struct Source {
    pub url: String,
    pub current: String,
    pub kind: InstallKind,
}

impl Source {
    /// This copy of the app. `ULANZI_RELEASES_URL` replaces the address, for tests.
    pub fn real() -> Source {
        Source {
            url: std::env::var("ULANZI_RELEASES_URL").unwrap_or_else(|_| LATEST.into()),
            current: env!("CARGO_PKG_VERSION").into(),
            kind: super::kind::detect(),
        }
    }
}

/// The release to offer: newer than this copy and not skipped.
fn to_offer(state: &UpdateState, src: &Source) -> Option<Release> {
    state
        .offered
        .clone()
        .filter(|r| update::is_newer(&r.tag, &src.current))
        .filter(|r| state.skipped.as_deref() != Some(r.version()))
}

fn can_install(release: &Release, kind: InstallKind) -> bool {
    let arch = std::env::consts::ARCH;
    update::asset_name(kind, release.version(), arch).is_some_and(|n| release.asset(&n).is_some())
        && release.asset("SHA256SUMS").is_some()
        && release.asset("SHA256SUMS.minisig").is_some()
}

/// What is known, without asking anyone.
pub fn status(store: &Store, src: &Source) -> UpdateStatus {
    let state = UpdateState::load(store);
    let offered = to_offer(&state, src);
    UpdateStatus {
        current: src.current.clone(),
        can_install: offered.as_ref().is_some_and(|r| can_install(r, src.kind)),
        offered,
        kind: src.kind,
        check: state.check,
        last_check: state.last_check,
        whats_new_shown: state.whats_new_shown,
    }
}

/// Ask for the latest release. With `force` (**Check Now**), a failure is an error; otherwise the
/// last offer simply stands.
pub fn run(store: &Store, src: &Source, force: bool) -> Result<UpdateStatus, UpdateError> {
    let failed = |store: &Store| {
        if force {
            Err(UpdateError::Offline)
        } else {
            Ok(status(store, src))
        }
    };
    let mut state = UpdateState::load(store);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent
        .get(&src.url)
        .header("User-Agent", format!("ulanzi-deck/{}", src.current))
        .header("Accept", "application/vnd.github+json");
    if let Some(tag) = &state.etag {
        request = request.header("If-None-Match", tag);
    }
    let Ok(mut response) = request.call() else {
        return failed(store);
    };
    match response.status().as_u16() {
        304 => {}
        200 => {
            let etag = response
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(String::from);
            let Ok(body) = response.body_mut().read_to_string() else {
                return failed(store);
            };
            state.etag = etag;
            state.offered = update::parse_latest(&body).filter(|r| update::is_newer(&r.tag, &src.current));
        }
        _ => return failed(store),
    }
    state.last_check = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    save(store, &state)?;
    Ok(status(store, src))
}

fn save(store: &Store, state: &UpdateState) -> Result<(), UpdateError> {
    state.save(store).map_err(|e| UpdateError::Failed(e.to_string()))
}

/// Stop offering `version`; a newer one is offered again.
pub fn skip(store: &Store, version: &str) -> Result<(), UpdateError> {
    let mut state = UpdateState::load(store);
    state.skipped = Some(version.to_string());
    save(store, &state)
}

/// Turn the daily check on or off.
pub fn set_checking(store: &Store, on: bool) -> Result<(), UpdateError> {
    let mut state = UpdateState::load(store);
    state.check = on;
    save(store, &state)
}

/// Remember that What's New was shown for `version`.
pub fn whats_new_shown(store: &Store, version: &str) -> Result<(), UpdateError> {
    let mut state = UpdateState::load(store);
    state.whats_new_shown = Some(version.to_string());
    save(store, &state)
}
