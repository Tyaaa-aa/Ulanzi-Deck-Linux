//! Running actions that leave the deck (commands, apps, keys, media, audio, plugins).

pub mod audio;
pub mod command;
pub mod keys;
pub mod launch;
pub mod media;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ulanzi_core::model::{Action, Step, table_to_json};

use self::keys::VirtualKeyboard;
use self::media::MediaClient;
use crate::plugins::PluginError;

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
    obs: Option<crate::obs::ObsHandle>,
    plugins: Option<crate::plugins::PluginSlot>,
}

impl Executor {
    /// `session` is the session bus, needed for media control; without it media actions report unavailable.
    pub fn new(session: Option<zbus::Connection>) -> Executor {
        Executor {
            media: session.map(MediaClient::new),
            keyboard: Arc::default(),
            obs: None,
            plugins: None,
        }
    }

    /// Run OBS actions through this connection.
    pub fn with_obs(mut self, obs: crate::obs::ObsHandle) -> Executor {
        self.obs = Some(obs);
        self
    }

    /// Run plugin actions through this host.
    pub fn with_plugins(self, host: crate::plugins::PluginHost) -> Executor {
        self.with_plugin_slot(crate::plugins::PluginSlot::ready(host))
    }

    /// Run plugin actions through the host in `slot` once it is ready (it starts in the background).
    pub fn with_plugin_slot(mut self, slot: crate::plugins::PluginSlot) -> Executor {
        self.plugins = Some(slot);
        self
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
    with_keyboard(kb, |k| k.press_combo(&keys))
}

fn with_keyboard<T>(
    kb: &Mutex<Option<VirtualKeyboard>>,
    f: impl FnOnce(&mut VirtualKeyboard) -> Result<T, ActionError>,
) -> Result<T, ActionError> {
    let mut guard = kb
        .lock()
        .map_err(|_| ActionError::Failed("keyboard lock poisoned".into()))?;
    if guard.is_none() {
        *guard = Some(VirtualKeyboard::new()?);
    }
    match guard.as_mut() {
        Some(kb) => f(kb),
        None => Err(ActionError::Unavailable("virtual keyboard unavailable".into())),
    }
}

/// Type `text`: key by key when a US layout has every character, else through the clipboard.
fn type_text(kb: &Mutex<Option<VirtualKeyboard>>, text: &str) -> Result<(), ActionError> {
    match keys::plan_typing(text) {
        keys::Typing::Keys(list) => with_keyboard(kb, |k| {
            for keys in &list {
                k.press_combo(keys)?;
            }
            Ok(())
        }),
        keys::Typing::Paste => {
            copy_to_clipboard(text)?;
            // Give the clipboard owner a moment before the app asks for the text.
            std::thread::sleep(Duration::from_millis(120));
            let paste = keys::parse_combo("ctrl+v")?;
            with_keyboard(kb, |k| k.press_combo(&paste))
        }
    }
}

/// Put text on the clipboard with wl-copy (Wayland) or xclip (X11).
fn copy_to_clipboard(text: &str) -> Result<(), ActionError> {
    use std::io::Write;
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let argv: &[&str] = if wayland {
        &["wl-copy"]
    } else {
        &["xclip", "-selection", "clipboard"]
    };
    let mut child = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| {
            ActionError::Unavailable(format!(
                "this text has characters a key press can't make; install {} to type it through the clipboard",
                if wayland { "wl-clipboard" } else { "xclip" }
            ))
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| ActionError::Failed(e.to_string()))?;
    }
    child.wait().map_err(|e| ActionError::Failed(e.to_string()))?;
    Ok(())
}

/// A plugin that is missing or turned off is "unavailable"; anything else went wrong.
fn plugin_error(e: PluginError) -> ActionError {
    match e {
        PluginError::NotInstalled(_) | PluginError::Off(_) => ActionError::Unavailable(e.to_string()),
        _ => ActionError::Failed(e.to_string()),
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
            Action::TypeText { text } => {
                let (kb, text) = (self.keyboard.clone(), text.clone());
                tokio::task::spawn_blocking(move || type_text(&kb, &text))
                    .await
                    .map_err(|e| ActionError::Failed(e.to_string()))?
            }
            Action::PlaySound { path, device } => audio::play_sound(path, device.as_deref()),
            Action::Media { control, player } => match &self.media {
                Some(m) => m.control(*control, player.as_deref()).await,
                None => Err(ActionError::Unavailable(
                    "media control needs the session bus".into(),
                )),
            },
            Action::Seek { seconds, player } => match &self.media {
                Some(m) => m.seek(*seconds, player.as_deref()).await,
                None => Err(ActionError::Unavailable(
                    "media control needs the session bus".into(),
                )),
            },
            Action::SwitchOutput { devices } => audio::switch_output(devices).await,
            Action::Volume {
                target,
                op,
                step,
                limit_100,
            } => audio::volume(target, op, *step, *limit_100).await,
            Action::Multi { steps } => run_multi(self, steps).await,
            Action::Plugin {
                plugin,
                action,
                params,
            } => match &self.plugins {
                Some(slot) => match slot.get() {
                    Some(host) => host
                        .action(plugin, action, table_to_json(params))
                        .await
                        .map_err(plugin_error),
                    None if slot.is_starting() => {
                        Err(ActionError::Unavailable("plugins are still starting".into()))
                    }
                    None => Err(ActionError::Unavailable("plugins are not running".into())),
                },
                None => Err(plugin_error(PluginError::NotInstalled(plugin.clone()))),
            },
            Action::Obs(o) => match &self.obs {
                Some(obs) => obs.action(o).await.map_err(ActionError::Failed),
                None => Err(ActionError::Unavailable("OBS support is off".into())),
            },
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
