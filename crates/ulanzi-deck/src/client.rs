//! The GUI's connection to `ulanzi-deckd`: calls, debounced profile saves, conflict handling and
//! reconnection. Runs on tokio; the GTK side talks to it through a relm4 worker.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::{Notify, broadcast};
use ulanzi_core::model::{Profile, Settings};
use ulanzi_core::plugin::{Inspected, Manifest, PickItem};
use ulanzi_core::store::parse_profile;
use uuid::Uuid;
use zbus::Connection;

use crate::plugins::PluginRow;

/// What to do with profiles that already exist when importing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportChoice {
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BundlePreview {
    pub manifest: ulanzi_core::bundle::Manifest,
    /// Names of profiles that already exist.
    pub conflicts: Vec<String>,
    /// Shell commands its keys and widgets would run.
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportDone {
    pub imported: Vec<Uuid>,
    pub warnings: Vec<String>,
    /// Restore this snapshot to undo the import.
    pub undo: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub name: String,
    pub created: String,
    pub reason: String,
    pub profiles: Vec<String>,
}

pub const BUS_NAME: &str = "io.github.tyaaa_aa.UlanziDeck.Service";
pub const OBJECT_PATH: &str = "/io/github/tyaaa_aa/UlanziDeck";
pub const INTERFACE: &str = "io.github.tyaaa_aa.UlanziDeck1";
/// Edits are saved once nothing changed for this long.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("the Ulanzi Deck service is not running")]
    NotRunning,
    #[error("the Ulanzi Deck service did not answer in time")]
    NoReply,
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
        let not_running = ["ServiceUnknown", "NameHasNoOwner", "Error.Spawn."]
            .iter()
            .any(|k| s.contains(k) || name.contains(k));
        if not_running {
            ClientError::NotRunning
        } else if s.contains("NoReply") || name.contains("NoReply") {
            ClientError::NoReply
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
    /// Settings changed outside the app.
    SettingsChanged,
    /// The plugin list or a plugin's status changed: fetch the list again.
    PluginsChanged,
    /// How far installing `id` from the store is: `stage` is preparing, downloading (`done` of
    /// `total` bytes), checking or installing.
    InstallProgress {
        id: String,
        stage: String,
        done: u64,
        total: u64,
    },
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
    /// Save a profile other than the one being edited, right away (fetch it with `profile` first).
    pub async fn apply_now(&self, profile: &Profile) -> Result<()> {
        self.send_profile(profile).await
    }

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

    /// What a `.ulanzideck` file holds, and which of its profiles already exist here.
    pub async fn preview_bundle(&self, path: &std::path::Path) -> Result<BundlePreview> {
        let (manifest, conflicts): (String, Vec<String>) = self
            .inner
            .proxy
            .call("PreviewBundle", &(path.to_string_lossy().as_ref(),))
            .await?;
        let manifest = toml::from_str(&manifest).map_err(|e| ClientError::Protocol(e.to_string()))?;
        // Older services can't list commands; the dialog then shows none.
        let commands: Vec<String> = self
            .inner
            .proxy
            .call("BundleCommands", &(path.to_string_lossy().as_ref(),))
            .await
            .unwrap_or_default();
        Ok(BundlePreview {
            manifest,
            conflicts,
            commands,
        })
    }

    pub async fn import_bundle(
        &self,
        path: &std::path::Path,
        conflict: ImportChoice,
        keep_rules: bool,
        settings: bool,
        switch_to: bool,
    ) -> Result<ImportDone> {
        let conflict = match conflict {
            ImportChoice::KeepBoth => "keep_both",
            ImportChoice::Replace => "replace",
            ImportChoice::Skip => "skip",
        };
        let (ids, warnings, undo): (Vec<String>, Vec<String>, String) = self
            .inner
            .proxy
            .call(
                "ImportBundle",
                &(
                    path.to_string_lossy().as_ref(),
                    conflict,
                    keep_rules,
                    settings,
                    switch_to,
                ),
            )
            .await?;
        Ok(ImportDone {
            imported: ids.iter().filter_map(|i| Uuid::parse_str(i).ok()).collect(),
            warnings,
            undo,
        })
    }

    pub async fn export_bundle(
        &self,
        path: &std::path::Path,
        ids: &[Uuid],
        include_settings: bool,
    ) -> Result<()> {
        let ids: Vec<String> = ids.iter().map(Uuid::to_string).collect();
        Ok(self
            .inner
            .proxy
            .call(
                "ExportBundle",
                &(path.to_string_lossy().as_ref(), ids, include_settings),
            )
            .await?)
    }

    pub async fn list_snapshots(&self) -> Result<Vec<Snapshot>> {
        let list: Vec<(String, String, String, Vec<String>)> =
            self.inner.proxy.call("ListSnapshots", &()).await?;
        Ok(list
            .into_iter()
            .map(|(name, created, reason, profiles)| Snapshot {
                name,
                created,
                reason,
                profiles,
            })
            .collect())
    }

    pub async fn take_snapshot(&self, reason: &str) -> Result<String> {
        Ok(self.inner.proxy.call("TakeSnapshot", &(reason,)).await?)
    }

    /// Returns the snapshot taken just before (restore it to undo).
    pub async fn restore_snapshot(&self, name: &str) -> Result<String> {
        Ok(self.inner.proxy.call("RestoreSnapshot", &(name,)).await?)
    }

    /// How auto-switch learns the app in front: (backend, state, detail).
    pub async fn focus_status(&self) -> Result<(String, String, String)> {
        Ok(self.inner.proxy.call("FocusStatus", &()).await?)
    }

    /// Towns for the weather widget's place search: (label, latitude, longitude).
    pub async fn weather_places(&self, query: &str) -> Result<Vec<(String, f64, f64)>> {
        Ok(self.inner.proxy.call("WeatherPlaces", &(query,)).await?)
    }

    /// The weather now for the preview (coordinates 0,0 = look the place up by name).
    pub async fn weather_now(
        &self,
        place: &str,
        at: Option<(f64, f64)>,
        fahrenheit: bool,
    ) -> Result<ulanzi_core::render::screen::WeatherData> {
        let (lat, lon) = at.unwrap_or((0.0, 0.0));
        let (ok, temp, code, high, low, text): (bool, f64, u8, f64, f64, String) = self
            .inner
            .proxy
            .call("WeatherNow", &(place, lat, lon, fahrenheit))
            .await?;
        Ok(if ok {
            ulanzi_core::render::screen::WeatherData::Ok {
                temp: temp as f32,
                code,
                place: text,
                high: high as f32,
                low: low as f32,
            }
        } else {
            ulanzi_core::render::screen::WeatherData::Err(text)
        })
    }

    /// What the service sees playing, for the Now Playing preview.
    pub async fn now_playing(&self) -> Result<Option<ulanzi_core::render::screen::MediaData>> {
        let (any, title, artist, art, pos, len, playing): (bool, String, String, String, u32, u32, bool) =
            self.inner.proxy.call("NowPlaying", &()).await?;
        Ok(any.then(|| ulanzi_core::render::screen::MediaData {
            title,
            artist,
            art: (!art.is_empty()).then(|| art.into()),
            position_s: (len > 0).then_some(pos),
            length_s: (len > 0).then_some(len),
            playing,
        }))
    }

    /// Installed plugins (and plugin folders that could not be loaded).
    pub async fn plugin_list(&self) -> Result<Vec<PluginRow>> {
        let list: Vec<(String, String, String, String, String)> =
            self.inner.proxy.call("PluginList", &()).await?;
        Ok(list
            .into_iter()
            .map(|(id, name, version, status, detail)| PluginRow {
                id,
                name,
                version,
                status,
                detail,
            })
            .collect())
    }

    /// Every installed plugin's manifest, by id.
    pub async fn plugin_manifests(&self) -> Result<BTreeMap<String, Manifest>> {
        let text: String = self.inner.proxy.call("PluginManifests", &()).await?;
        serde_json::from_str(&text).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// What installing the plugin folder at `path` (absolute) would install, writing nothing.
    pub async fn plugin_inspect_dir(&self, path: &Path) -> Result<Inspected> {
        let text: String = self
            .inner
            .proxy
            .call("PluginInspectDir", &(path.to_string_lossy().as_ref(),))
            .await?;
        serde_json::from_str(&text).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// Install (or replace) the plugin in the folder at `path` (absolute); returns its id.
    pub async fn plugin_install_dir(&self, path: &Path) -> Result<String> {
        Ok(self
            .inner
            .proxy
            .call("PluginInstallDir", &(path.to_string_lossy().as_ref(),))
            .await?)
    }

    /// What installing the `.ulanziplugin` file at `path` (absolute) would install, writing nothing
    /// that stays. Works without Developer mode.
    pub async fn plugin_inspect_file(&self, path: &Path) -> Result<Inspected> {
        let text: String = self
            .inner
            .proxy
            .call("PluginInspectFile", &(path.to_string_lossy().as_ref(),))
            .await?;
        serde_json::from_str(&text).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// Install (or replace) the plugin in the `.ulanziplugin` file at `path` (absolute); returns
    /// its id. Unsigned packages need Developer mode.
    pub async fn plugin_install_file(&self, path: &Path) -> Result<String> {
        Ok(self
            .inner
            .proxy
            .call("PluginInstallFile", &(path.to_string_lossy().as_ref(),))
            .await?)
    }

    /// The plugin store's signed list (the last one seen when the store can't be reached).
    /// A copy under 30 minutes old is used unless `refresh`.
    pub async fn plugin_store_index(&self, refresh: bool) -> Result<ulanzi_core::plugin::store::Listing> {
        let text: String = self.inner.proxy.call("PluginStoreIndex", &(refresh,)).await?;
        serde_json::from_str(&text).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// Install or update plugin `id` from the store; returns its id.
    pub async fn plugin_store_install(&self, id: &str) -> Result<String> {
        Ok(self.inner.proxy.call("PluginStoreInstall", &(id,)).await?)
    }

    /// Play a sound file as a Play Sound key would (preview).
    pub async fn play_sound(&self, path: &str, device: Option<&str>, volume: u8) -> Result<()> {
        Ok(self
            .inner
            .proxy
            .call("PlaySound", &(path, device.unwrap_or(""), volume))
            .await?)
    }

    /// Installed plugins the plugin store lists as official.
    pub async fn plugin_official(&self) -> Result<Vec<String>> {
        Ok(self.inner.proxy.call("PluginOfficial", &()).await?)
    }

    /// Installed plugins whose package was signed with a key the app trusts.
    pub async fn plugin_verified(&self) -> Result<Vec<String>> {
        Ok(self.inner.proxy.call("PluginVerified", &()).await?)
    }

    /// The entries of the plugin's picker matching `query` (`""`: all), only `group` unless it is
    /// `""`. Starts the plugin if needed; an error is the plugin's own message.
    pub async fn plugin_list_items(
        &self,
        id: &str,
        picker: &str,
        query: &str,
        group: &str,
    ) -> Result<Vec<PickItem>> {
        let text: String = self
            .inner
            .proxy
            .call("PluginListItems", &(id, picker, query, group))
            .await?;
        serde_json::from_str(&text).map_err(|e| ClientError::Protocol(e.to_string()))
    }

    /// Stop the plugin and delete it and its secrets (its settings are kept).
    pub async fn plugin_remove(&self, id: &str) -> Result<()> {
        Ok(self.inner.proxy.call::<_, _, ()>("PluginRemove", &(id,)).await?)
    }

    pub async fn plugin_set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        Ok(self
            .inner
            .proxy
            .call::<_, _, ()>("PluginSetEnabled", &(id, enabled))
            .await?)
    }

    pub async fn plugin_restart(&self, id: &str) -> Result<()> {
        Ok(self.inner.proxy.call::<_, _, ()>("PluginRestart", &(id,)).await?)
    }

    /// The plugin's declared secret keys that are set (never their values).
    pub async fn plugin_secret_keys(&self, id: &str) -> Result<Vec<String>> {
        Ok(self.inner.proxy.call("PluginSecretKeys", &(id,)).await?)
    }

    /// Write a secret straight into the plugin's secret store.
    pub async fn plugin_set_secret(&self, id: &str, key: &str, value: &str) -> Result<()> {
        Ok(self
            .inner
            .proxy
            .call::<_, _, ()>("PluginSetSecret", &(id, key, value))
            .await?)
    }

    pub async fn plugin_clear_secret(&self, id: &str, key: &str) -> Result<()> {
        Ok(self
            .inner
            .proxy
            .call::<_, _, ()>("PluginClearSecret", &(id, key))
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
            let event =
                match member.as_str() {
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
                    "SettingsChanged" => Some(ClientEvent::SettingsChanged),
                    "PluginsChanged" => Some(ClientEvent::PluginsChanged),
                    "PluginInstallProgress" => body.deserialize::<(String, String, u64, u64)>().ok().map(
                        |(id, stage, done, total)| ClientEvent::InstallProgress {
                            id,
                            stage,
                            done,
                            total,
                        },
                    ),
                    _ => None,
                };
            if let Some(e) = event {
                self.emit(e);
            }
        }
    }
}
