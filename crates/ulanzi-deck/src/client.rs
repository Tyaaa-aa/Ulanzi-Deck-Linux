//! The GUI's connection to `ulanzi-deckd`: calls, debounced profile saves, conflict handling and
//! reconnection. Runs on tokio; the GTK side talks to it through a relm4 worker.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::{Notify, broadcast};
use ulanzi_core::model::{Profile, Settings};
use ulanzi_core::store::parse_profile;
use uuid::Uuid;
use zbus::Connection;

pub const BUS_NAME: &str = "io.github.tyaaa_aa.UlanziDeck.Service";
pub const OBJECT_PATH: &str = "/io/github/tyaaa_aa/UlanziDeck";
pub const INTERFACE: &str = "io.github.tyaaa_aa.UlanziDeck1";
/// Edits are saved once nothing changed for this long.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("the Ulanzi Deck service is not running")]
    NotRunning,
    #[error("{0}")]
    Service(String),
    #[error("the service sent something this app does not understand: {0}")]
    Protocol(String),
}

impl From<zbus::Error> for ClientError {
    fn from(e: zbus::Error) -> Self {
        let s = e.to_string();
        // Spawn.*: the bus tried to auto-start the service and that failed (it is stopped or broken).
        let name = match &e {
            zbus::Error::MethodError(name, _, _) => name.as_str().to_string(),
            _ => String::new(),
        };
        let not_running = ["ServiceUnknown", "NameHasNoOwner", "NoReply", "Error.Spawn."]
            .iter()
            .any(|k| s.contains(k) || name.contains(k));
        if not_running {
            ClientError::NotRunning
        } else if let zbus::Error::MethodError(_, Some(msg), _) = &e {
            ClientError::Service(msg.clone())
        } else {
            ClientError::Service(s)
        }
    }
}

type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug, Clone, PartialEq)]
pub struct Hello {
    pub api_version: u32,
    pub daemon_version: String,
    pub device_state: String,
    pub device_detail: String,
    pub model: String,
    pub active_profile: Uuid,
    pub active_page: u32,
}

/// Things the GUI reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    State {
        state: String,
        detail: String,
    },
    Input {
        control: String,
        kind: String,
    },
    Page {
        profile: Uuid,
        page: u32,
    },
    ActionFailed {
        control: String,
        message: String,
    },
    ProfilesChanged,
    /// The service went away; edits are kept and sent when it returns.
    Disconnected,
    Reconnected,
    /// Another app changed this profile; our edit was applied on top of it.
    Conflict {
        profile: Uuid,
    },
    /// Saving failed for a reason other than the service being away.
    SaveFailed {
        message: String,
    },
}

struct Inner {
    conn: Connection,
    proxy: zbus::Proxy<'static>,
    revisions: Mutex<HashMap<Uuid, u64>>,
    pending: Mutex<Option<Profile>>,
    kick: Notify,
    /// Serialises saves so a flush and the debounce timer never race.
    saving: tokio::sync::Mutex<()>,
    connected: Mutex<bool>,
    events: broadcast::Sender<ClientEvent>,
}

/// Cheap to clone; all clones share one connection.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    /// Connect on the session bus.
    pub async fn connect() -> Result<Client> {
        Self::with(Connection::session().await.map_err(ClientError::from)?).await
    }

    /// Connect on a given bus address (tests).
    pub async fn connect_to(address: &str) -> Result<Client> {
        let conn = zbus::connection::Builder::address(address)?.build().await?;
        Self::with(conn).await
    }

    async fn with(conn: Connection) -> Result<Client> {
        let proxy = zbus::Proxy::new(&conn, BUS_NAME, OBJECT_PATH, INTERFACE).await?;
        let (events, _) = broadcast::channel(256);
        let inner = Arc::new(Inner {
            conn,
            proxy,
            revisions: Mutex::default(),
            pending: Mutex::default(),
            kick: Notify::new(),
            saving: tokio::sync::Mutex::new(()),
            connected: Mutex::new(true),
            events,
        });
        let client = Client { inner };
        tokio::spawn(client.clone().debounce_loop());
        tokio::spawn(client.clone().watch_owner());
        tokio::spawn(client.clone().forward_signals());
        Ok(client)
    }

    pub fn events(&self) -> broadcast::Receiver<ClientEvent> {
        self.inner.events.subscribe()
    }

    fn emit(&self, e: ClientEvent) {
        let _ = self.inner.events.send(e);
    }

    pub async fn hello(&self) -> Result<Hello> {
        let (api_version, daemon_version, device_state, device_detail, model, profile, active_page): (
            u32,
            String,
            String,
            String,
            String,
            String,
            u32,
        ) = self
            .inner
            .proxy
            .call("Hello", &(env!("CARGO_PKG_VERSION"),))
            .await?;
        let active_profile = Uuid::parse_str(&profile).map_err(|e| ClientError::Protocol(e.to_string()))?;
        Ok(Hello {
            api_version,
            daemon_version,
            device_state,
            device_detail,
            model,
            active_profile,
            active_page,
        })
    }

    pub async fn profiles(&self) -> Result<Vec<(Uuid, String)>> {
        let list: Vec<(String, String)> = self.inner.proxy.call("ListProfiles", &()).await?;
        Ok(list
            .into_iter()
            .filter_map(|(id, n)| Uuid::parse_str(&id).ok().map(|id| (id, n)))
            .collect())
    }

    /// Load a profile and remember its revision for the next save.
    pub async fn profile(&self, id: Uuid) -> Result<Profile> {
        let text: String = self.inner.proxy.call("GetProfile", &(id.to_string(),)).await?;
        let rev: u64 = self
            .inner
            .proxy
            .call("ProfileRevision", &(id.to_string(),))
            .await?;
        self.set_revision(id, rev);
        parse_profile(&text).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub fn revision(&self, id: Uuid) -> u64 {
        self.inner
            .revisions
            .lock()
            .map(|r| r.get(&id).copied().unwrap_or(1))
            .unwrap_or(1)
    }

    fn set_revision(&self, id: Uuid, rev: u64) {
        if let Ok(mut r) = self.inner.revisions.lock() {
            r.insert(id, rev);
        }
    }

    /// Queue a profile to save; it is sent [`DEBOUNCE`] after the last call.
    pub fn apply(&self, profile: Profile) {
        if let Ok(mut p) = self.inner.pending.lock() {
            *p = Some(profile);
        }
        self.inner.kick.notify_one();
    }

    /// Send any queued profile now.
    pub async fn flush(&self) {
        self.save_pending().await;
    }

    pub async fn settings(&self) -> Result<Settings> {
        let text: String = self.inner.proxy.call("GetSettings", &()).await?;
        toml::from_str(&text).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub async fn apply_settings(&self, s: &Settings) -> Result<()> {
        let text = toml::to_string(s).map_err(|e| ClientError::Protocol(e.to_string()))?;
        Ok(self
            .inner
            .proxy
            .call::<_, _, ()>("ApplySettings", &(text,))
            .await?)
    }

    pub async fn create_profile(&self, name: &str) -> Result<Uuid> {
        let id: String = self.inner.proxy.call("CreateProfile", &(name,)).await?;
        Uuid::parse_str(&id).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    pub async fn delete_profile(&self, id: Uuid) -> Result<()> {
        Ok(self
            .inner
            .proxy
            .call::<_, _, ()>("DeleteProfile", &(id.to_string(),))
            .await?)
    }

    pub async fn switch_profile(&self, id: Uuid) -> Result<()> {
        Ok(self
            .inner
            .proxy
            .call::<_, _, ()>("SwitchProfile", &(id.to_string(),))
            .await?)
    }

    pub async fn set_page(&self, index: u32) -> Result<()> {
        Ok(self.inner.proxy.call::<_, _, ()>("SetPage", &(index,)).await?)
    }

    pub async fn set_brightness(&self, percent: u8) -> Result<()> {
        Ok(self
            .inner
            .proxy
            .call::<_, _, ()>("SetBrightness", &(percent,))
            .await?)
    }

    /// Import the Python version's config. Returns (profile, key count, warnings).
    pub async fn migrate(&self, force: bool) -> Result<(Uuid, u32, Vec<String>)> {
        let (id, keys, warnings): (String, u32, Vec<String>) =
            self.inner.proxy.call("Migrate", &(force,)).await?;
        let id = Uuid::parse_str(&id).map_err(|e| ClientError::Protocol(e.to_string()))?;
        Ok((id, keys, warnings))
    }

    async fn debounce_loop(self) {
        loop {
            self.inner.kick.notified().await;
            // Wait until edits stop arriving for DEBOUNCE.
            while tokio::time::timeout(DEBOUNCE, self.inner.kick.notified())
                .await
                .is_ok()
            {}
            self.save_pending().await;
        }
    }

    fn set_connected(&self, up: bool) {
        let was = self
            .inner
            .connected
            .lock()
            .map(|mut c| std::mem::replace(&mut *c, up))
            .unwrap_or(up);
        if was != up {
            self.emit(if up {
                ClientEvent::Reconnected
            } else {
                ClientEvent::Disconnected
            });
        }
    }

    async fn save_pending(&self) {
        let _guard = self.inner.saving.lock().await;
        let Some(profile) = self.inner.pending.lock().ok().and_then(|mut p| p.take()) else {
            return;
        };
        match self.send_profile(&profile).await {
            Ok(()) => self.set_connected(true),
            Err(ClientError::NotRunning) => {
                self.set_connected(false);
                // Keep it for when the service returns, unless a newer edit arrived meanwhile.
                if let Ok(mut p) = self.inner.pending.lock()
                    && p.is_none()
                {
                    *p = Some(profile);
                }
            }
            Err(e) => self.emit(ClientEvent::SaveFailed {
                message: e.to_string(),
            }),
        }
    }

    /// ApplyProfile; on a stale revision, take the current one and apply on top (once).
    async fn send_profile(&self, profile: &Profile) -> Result<()> {
        let text = toml::to_string(profile).map_err(|e| ClientError::Protocol(e.to_string()))?;
        for attempt in 0..2 {
            let rev = self.revision(profile.id);
            match self
                .inner
                .proxy
                .call::<_, _, u64>("ApplyProfile", &(text.as_str(), rev))
                .await
            {
                Ok(new) => {
                    self.set_revision(profile.id, new);
                    return Ok(());
                }
                Err(zbus::Error::MethodError(name, _, _))
                    if name.as_str().ends_with("StaleRevision") && attempt == 0 =>
                {
                    let current: u64 = self
                        .inner
                        .proxy
                        .call("ProfileRevision", &(profile.id.to_string(),))
                        .await?;
                    self.set_revision(profile.id, current);
                    self.emit(ClientEvent::Conflict { profile: profile.id });
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(ClientError::Service(
            "the profile kept changing underneath; try again".into(),
        ))
    }

    async fn watch_owner(self) {
        let Ok(dbus) = zbus::fdo::DBusProxy::new(&self.inner.conn).await else {
            return;
        };
        let Ok(mut changes) = dbus.receive_name_owner_changed_with_args(&[(0, BUS_NAME)]).await else {
            return;
        };
        while let Some(change) = changes.next().await {
            let Ok(args) = change.args() else {
                continue;
            };
            if args.new_owner().is_some() {
                self.set_connected(true);
                self.save_pending().await;
            } else {
                self.set_connected(false);
            }
        }
    }

    async fn forward_signals(self) {
        let Ok(mut signals) = self.inner.proxy.receive_all_signals().await else {
            return;
        };
        while let Some(m) = signals.next().await {
            let member = m.header().member().map(|m| m.to_string()).unwrap_or_default();
            let body = m.body();
            let event = match member.as_str() {
                "DeviceStateChanged" => body
                    .deserialize::<(String, String)>()
                    .ok()
                    .map(|(state, detail)| ClientEvent::State { state, detail }),
                "InputEvent" => body
                    .deserialize::<(String, String)>()
                    .ok()
                    .map(|(control, kind)| ClientEvent::Input { control, kind }),
                "ActionFailed" => body
                    .deserialize::<(String, String)>()
                    .ok()
                    .map(|(control, message)| ClientEvent::ActionFailed { control, message }),
                "PageChanged" => body.deserialize::<(String, u32)>().ok().and_then(|(p, page)| {
                    Uuid::parse_str(&p)
                        .ok()
                        .map(|profile| ClientEvent::Page { profile, page })
                }),
                "ProfilesChanged" => Some(ClientEvent::ProfilesChanged),
                _ => None,
            };
            if let Some(e) = event {
                self.emit(e);
            }
        }
    }
}
