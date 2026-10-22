//! Running actions that leave the deck (commands, apps, keys, media, audio).

pub mod audio;
pub mod command;
pub mod keys;
pub mod launch;
pub mod media;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ulanzi_core::model::{Action, Step};

use self::keys::VirtualKeyboard;
use self::media::MediaClient;

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

/// Run a multi-action's steps in order, waiting each step's delay before it.
pub async fn run_multi(runner: &dyn ActionRunner, steps: &[Step]) -> Result<(), ActionError> {
    for step in steps {
        if step.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(step.delay_ms as u64)).await;
        }
        runner.run(&step.action).await?;
    }
    Ok(())
}

/// The real action runner.
pub struct Executor {
    media: Option<MediaClient>,
    keyboard: Arc<Mutex<Option<VirtualKeyboard>>>,
}

impl Executor {
    /// `session` is the session bus, needed for media control; without it media actions report unavailable.
    pub fn new(session: Option<zbus::Connection>) -> Executor {
        Executor {
            media: session.map(MediaClient::new),
            keyboard: Arc::default(),
        }
    }

    /// Create the virtual keyboard now. The compositor needs a moment to pick up a new input
    /// device, so creating it on the first shortcut would lose that shortcut. Blocking; call from
    /// `spawn_blocking`.
    pub fn prepare_keyboard(&self) {
        let Ok(mut guard) = self.keyboard.lock() else {
            return;
        };
        if guard.is_none() {
            match VirtualKeyboard::new() {
                Ok(kb) => *guard = Some(kb),
                Err(e) => tracing::warn!("{e}"),
            }
        }
    }

    pub fn keyboard_handle(&self) -> Arc<Mutex<Option<VirtualKeyboard>>> {
        self.keyboard.clone()
    }

    pub fn media(&self) -> Option<&MediaClient> {
        self.media.as_ref()
    }
}

/// Press a combo on the shared virtual keyboard, creating it on first use.
fn shortcut(kb: &Mutex<Option<VirtualKeyboard>>, combo: &str) -> Result<(), ActionError> {
    let keys = keys::parse_combo(combo)?;
    let mut guard = kb
        .lock()
        .map_err(|_| ActionError::Failed("keyboard lock poisoned".into()))?;
    if guard.is_none() {
        *guard = Some(VirtualKeyboard::new()?);
    }
    match guard.as_mut() {
        Some(kb) => kb.press_combo(&keys),
        None => Err(ActionError::Unavailable("virtual keyboard unavailable".into())),
    }
}

#[async_trait::async_trait]
impl ActionRunner for Executor {
    async fn run(&self, action: &Action) -> Result<(), ActionError> {
        match action {
            Action::Command { cmd, terminal } => command::spawn_shell(cmd, *terminal),
            Action::LaunchApp { desktop_id, exec } => {
                if let Some(id) = desktop_id {
                    let path = launch::find_desktop(id, &launch::application_dirs())
                        .ok_or_else(|| ActionError::Failed(format!("app {id} is not installed")))?;
                    let text =
                        std::fs::read_to_string(&path).map_err(|e| ActionError::Failed(e.to_string()))?;
                    let argv = launch::exec_from_desktop(&text)
                        .ok_or_else(|| ActionError::Failed(format!("{} has no Exec line", path.display())))?;
                    command::spawn_detached(&argv)
                } else if let Some(exec) = exec {
                    command::spawn_shell(exec, false)
                } else {
                    Err(ActionError::Failed("no app chosen".into()))
                }
            }
            Action::OpenUrl { url } => command::spawn_detached(&["xdg-open".to_string(), url.clone()]),
            Action::Shortcut { keys } => {
                // uinput writes block (with short sleeps between keys), so run them off the async threads.
                let (kb, keys) = (self.keyboard.clone(), keys.clone());
                tokio::task::spawn_blocking(move || shortcut(&kb, &keys))
                    .await
                    .map_err(|e| ActionError::Failed(e.to_string()))?
            }
            Action::TypeText { .. } => Err(ActionError::NotYet("M2")),
            Action::PlaySound { path, device } => audio::play_sound(path, device.as_deref()),
            Action::Media { control, player } => match &self.media {
                Some(m) => m.control(*control, player.as_deref()).await,
                None => Err(ActionError::Unavailable(
                    "media control needs the session bus".into(),
                )),
            },
            Action::Volume {
                target,
                op,
                step,
                limit_100,
            } => audio::volume(target, op, *step, *limit_100).await,
            Action::Multi { steps } => run_multi(self, steps).await,
            Action::Obs(_) => Err(ActionError::NotYet("M3")),
            Action::NextPage
            | Action::PrevPage
            | Action::GoToPage { .. }
            | Action::OpenFolder { .. }
            | Action::Back
            | Action::SwitchProfile { .. }
            | Action::AutoProfile
            | Action::Brightness { .. }
            | Action::SleepNow
            | Action::ScreenNext
            | Action::Timer { .. } => Err(ActionError::Failed(
                "this action only works from the deck itself".into(),
            )),
        }
    }
}
