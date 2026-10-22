//! Running actions that leave the deck (commands, apps, keys, media, audio).

use ulanzi_core::model::Action;

/// Why an action did not run. `Display` is the sentence shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("this action arrives in {0}")]
    NotYet(&'static str),
}

/// Runs one action. Navigation, brightness and sleep are handled by the deck itself and never reach this.
#[async_trait::async_trait]
pub trait ActionRunner: Send + Sync {
    async fn run(&self, action: &Action) -> Result<(), ActionError>;
}
