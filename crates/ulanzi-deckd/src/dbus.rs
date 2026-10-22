//! The session-bus API (`io.github.tyaaa_aa.UlanziDeck1`) used by the GUI and `ulanzictl`.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::{broadcast, mpsc, oneshot};
use ulanzi_core::model::{BrightnessOp, Model, Profile, Settings};
use ulanzi_core::store::{ProfileLoad, Store, parse_profile};
use ulanzi_proto::InputKind;
use uuid::Uuid;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

use crate::deck::{DeckEvent, DeckMsg, DeckSnapshot};

pub const BUS_NAME: &str = "io.github.tyaaa_aa.UlanziDeck.Service";
pub const OBJECT_PATH: &str = "/io/github/tyaaa_aa/UlanziDeck";
pub const INTERFACE: &str = "io.github.tyaaa_aa.UlanziDeck1";
pub const API_VERSION: u32 = 1;
/// What the GUI sees instead of a stored OBS password; sending it back keeps the stored one.
pub const PASSWORD_PLACEHOLDER: &str = "••••";

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "io.github.tyaaa_aa.UlanziDeck1.Error")]
pub enum DeckError {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// Another client changed the profile first; reload and retry.
    StaleRevision(String),
    /// The profile or argument is not valid.
    Invalid(String),
    NotFound(String),
    /// The daemon is shutting down or busy.
    Unavailable(String),
}

type Result<T> = std::result::Result<T, DeckError>;

pub struct DeckService {
    tx: mpsc::Sender<DeckMsg>,
    store: Store,
    revisions: Mutex<HashMap<Uuid, u64>>,
    /// The window in front, for the app's "add the app in front" button.
    focused: Mutex<ulanzi_core::autoswitch::Window>,
}

impl DeckService {
    pub fn new(tx: mpsc::Sender<DeckMsg>, store: Store) -> DeckService {
        DeckService {
            tx,
            store,
            revisions: Mutex::default(),
            focused: Mutex::default(),
        }
    }

    async fn send(&self, m: DeckMsg) -> Result<()> {
        self.tx
            .send(m)
            .await
            .map_err(|_| DeckError::Unavailable("the deck has stopped".into()))
    }

    async fn snapshot(&self) -> Result<DeckSnapshot> {
        let (tx, rx) = oneshot::channel();
        self.send(DeckMsg::Query(tx)).await?;
        rx.await
            .map_err(|_| DeckError::Unavailable("the deck has stopped".into()))
    }

    fn revision(&self, id: Uuid) -> u64 {
        self.revisions
            .lock()
            .map(|r| r.get(&id).copied().unwrap_or(1))
            .unwrap_or(1)
    }

    /// Every profile's revision moves on, so an open editor reloads instead of saving over a change.
    fn bump(&self, ids: &[Uuid]) {
        if let Ok(mut r) = self.revisions.lock() {
            for id in ids {
                let next = r.get(id).copied().unwrap_or(1) + 1;
                r.insert(*id, next);
            }
        }
    }

    /// Tell the deck about profiles that appeared, changed or went away on disk.
    async fn resync(&self) -> Result<()> {
        let on_disk: Vec<Uuid> = self
            .store
            .list_profiles()
            .into_iter()
            .filter_map(|l| match l {
                ProfileLoad::Ok(p) => Some(p.id),
                _ => None,
            })
            .collect();
        reload(&self.tx, &self.store).await;
        for (id, _) in self.snapshot().await?.profiles {
            if !on_disk.contains(&id) {
                self.send(DeckMsg::RemoveProfile(id)).await?;
            }
        }
        Ok(())
    }

    fn snap(&self, reason: &str) -> Result<String> {
        ulanzi_core::snapshot::take(&self.store, reason)
            .map(|s| s.name)
            .map_err(|e| DeckError::Unavailable(format!("could not back up first: {e}")))
    }

    fn read_bundle(path: &str) -> Result<ulanzi_core::bundle::Bundle> {
        let bytes = std::fs::read(path).map_err(|e| DeckError::NotFound(format!("{path}: {e}")))?;
        ulanzi_core::bundle::read(&bytes).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    fn id(s: &str) -> Result<Uuid> {
        Uuid::parse_str(s).map_err(|_| DeckError::Invalid(format!("{s:?} is not a profile id")))
    }
}

/// Re-read settings and profiles from disk and hand them to the deck.
pub async fn reload(tx: &mpsc::Sender<DeckMsg>, store: &Store) {
    match store.load_settings() {
        Ok(s) => {
            let _ = tx.send(DeckMsg::Settings(Box::new(s))).await;
        }
        Err(e) => tracing::error!("reload: {e}"),
    }
    for l in store.list_profiles() {
        if let ProfileLoad::Ok(p) = l {
            let _ = tx.send(DeckMsg::Apply { profile: p }).await;
        }
    }
}

#[interface(name = "io.github.tyaaa_aa.UlanziDeck1")]
impl DeckService {
    /// Handshake: API version and current state.
    async fn hello(
        &self,
        client_version: String,
    ) -> Result<(u32, String, String, String, String, String, u32)> {
        tracing::debug!("hello from client {client_version}");
        let s = self.snapshot().await?;
        Ok((
            API_VERSION,
            env!("CARGO_PKG_VERSION").to_string(),
            s.device.code().to_string(),
            s.device.detail(),
            format!("{:?}", s.model),
            s.profile.to_string(),
            s.page as u32,
        ))
    }

    async fn list_profiles(&self) -> Result<Vec<(String, String)>> {
        Ok(self
            .snapshot()
            .await?
            .profiles
            .into_iter()
            .map(|(id, n)| (id.to_string(), n))
            .collect())
    }

    async fn get_profile(&self, id: String) -> Result<String> {
        let p = self
            .store
            .load_profile(Self::id(&id)?)
            .map_err(|e| DeckError::NotFound(e.to_string()))?;
        toml::to_string_pretty(&p).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// Replace a profile. `revision` must be the one the client last saw (1 for a fresh daemon).
    async fn apply_profile(&self, toml: String, revision: u64) -> Result<u64> {
        let profile = parse_profile(&toml).map_err(|e| DeckError::Invalid(e.to_string()))?;
        let current = self.revision(profile.id);
        if revision != current {
            return Err(DeckError::StaleRevision(format!(
                "profile is at revision {current}, not {revision}"
            )));
        }
        self.store
            .save_profile(&profile)
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        let next = current + 1;
        if let Ok(mut r) = self.revisions.lock() {
            r.insert(profile.id, next);
        }
        self.send(DeckMsg::Apply {
            profile: Box::new(profile),
        })
        .await?;
        Ok(next)
    }

    async fn switch_profile(&self, id: String) -> Result<()> {
        self.send(DeckMsg::SwitchProfile(Self::id(&id)?)).await
    }

    async fn set_page(&self, index: u32) -> Result<()> {
        self.send(DeckMsg::SetPage(index as usize)).await
    }

    async fn next_page(&self) -> Result<()> {
        self.send(DeckMsg::NextPage).await
    }

    async fn prev_page(&self) -> Result<()> {
        self.send(DeckMsg::PrevPage).await
    }

    async fn set_brightness(&self, percent: u8) -> Result<()> {
        if percent > 100 {
            return Err(DeckError::Invalid("brightness is 0-100".into()));
        }
        self.send(DeckMsg::Brightness(BrightnessOp::Set { percent }))
            .await
    }

    async fn reload(&self) -> Result<()> {
        reload(&self.tx, &self.store).await;
        Ok(())
    }

    /// Called by the desktop's window manager script when another window comes to the front.
    async fn window_activated(&self, class: String, desktop_id: String, title: String) -> Result<()> {
        let w = ulanzi_core::autoswitch::Window {
            class,
            desktop_id,
            title,
        };
        if let Ok(mut f) = self.focused.lock() {
            *f = w.clone();
        }
        self.send(DeckMsg::Focus(w)).await
    }

    /// The window in front: (class, desktop id, title).
    async fn focused_window(&self) -> Result<(String, String, String)> {
        let w = self.focused.lock().map(|f| f.clone()).unwrap_or_default();
        Ok((w.class, w.desktop_id, w.title))
    }

    /// Check a `.ulanzideck` file: its manifest (TOML) and the names of profiles that already exist.
    async fn preview_bundle(&self, path: String) -> Result<(String, Vec<String>)> {
        let b = Self::read_bundle(&path)?;
        let have: Vec<Uuid> = self
            .store
            .list_profiles()
            .into_iter()
            .filter_map(|l| match l {
                ProfileLoad::Ok(p) => Some(p.id),
                _ => None,
            })
            .collect();
        let conflicts = b
            .profiles
            .iter()
            .filter(|p| have.contains(&p.profile.id))
            .map(|p| p.profile.name.clone())
            .collect();
        let manifest = toml::to_string_pretty(&b.manifest).map_err(|e| DeckError::Invalid(e.to_string()))?;
        Ok((manifest, conflicts))
    }

    /// Import a `.ulanzideck` file. `conflict` is keep_both, replace or skip. Returns the imported
    /// profile ids, warnings, and the snapshot taken first (restore it to undo).
    async fn import_bundle(
        &self,
        path: String,
        conflict: String,
        keep_rules: bool,
        settings: bool,
        switch_to: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(Vec<String>, Vec<String>, String)> {
        use ulanzi_core::bundle::{Conflict, ImportOptions};
        let b = Self::read_bundle(&path)?;
        let conflict = match conflict.as_str() {
            "replace" => Conflict::Replace,
            "skip" => Conflict::Skip,
            _ => Conflict::KeepBoth,
        };
        let undo = self.snap("before-import")?;
        let device = self.snapshot().await.ok().map(|s| s.model);
        let r = ulanzi_core::bundle::import(
            &self.store,
            &b,
            &ImportOptions {
                conflict,
                keep_rules,
                settings,
                device,
            },
        )
        .map_err(|e| DeckError::Invalid(e.to_string()))?;
        let touched: Vec<Uuid> = r.imported.iter().chain(&r.replaced).copied().collect();
        self.bump(&touched);
        self.resync().await?;
        if switch_to && let Some(id) = touched.first() {
            self.send(DeckMsg::SwitchProfile(*id)).await?;
        }
        let _ = Self::profiles_changed(&emitter).await;
        Ok((touched.iter().map(Uuid::to_string).collect(), r.warnings, undo))
    }

    /// Write profiles (all when `ids` is empty) to a `.ulanzideck` file.
    async fn export_bundle(&self, path: String, ids: Vec<String>, include_settings: bool) -> Result<()> {
        let ids: Vec<Uuid> = ids.iter().map(|i| Self::id(i)).collect::<Result<_>>()?;
        let bytes = ulanzi_core::bundle::export(&self.store, &ids, include_settings)
            .map_err(|e| DeckError::Invalid(e.to_string()))?;
        ulanzi_core::store::write_atomic(std::path::Path::new(&path), &bytes)
            .map_err(|e| DeckError::Unavailable(format!("{path}: {e}")))
    }

    /// Backups, newest first: (name, created, reason, profile names).
    async fn list_snapshots(&self) -> Result<Vec<(String, String, String, Vec<String>)>> {
        Ok(ulanzi_core::snapshot::list(&self.store)
            .into_iter()
            .map(|s| (s.name, s.created, s.reason, s.profiles))
            .collect())
    }

    async fn take_snapshot(&self, reason: String) -> Result<String> {
        self.snap(&reason)
    }

    /// Put everything back as in snapshot `name`. Returns the snapshot taken just before (to undo).
    async fn restore_snapshot(
        &self,
        name: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<String> {
        let before = ulanzi_core::snapshot::restore(&self.store, &name)
            .map_err(|e| DeckError::Invalid(e.to_string()))?;
        let ids: Vec<Uuid> = self
            .store
            .list_profiles()
            .into_iter()
            .filter_map(|l| match l {
                ProfileLoad::Ok(p) => Some(p.id),
                _ => None,
            })
            .collect();
        self.bump(&ids);
        self.resync().await?;
        let _ = Self::profiles_changed(&emitter).await;
        Ok(before.name)
    }

    /// Import the old Python config. Returns (profile id, keys, warnings).
    async fn migrate(&self, force: bool) -> Result<(String, u32, Vec<String>)> {
        let path = ulanzi_core::migrate::legacy_config_path()
            .filter(|p| p.is_file())
            .ok_or_else(|| {
                DeckError::NotFound("no old configuration at ~/.config/ulanzi/config.yaml".into())
            })?;
        self.snap("before-migration")?;
        let r = ulanzi_core::migrate::migrate_legacy(&path, &self.store, force)
            .map_err(|e| DeckError::Invalid(e.to_string()))?;
        reload(&self.tx, &self.store).await;
        let _ = self.tx.send(DeckMsg::SwitchProfile(r.profile_id)).await;
        Ok((r.profile_id.to_string(), r.keys as u32, r.warnings))
    }

    /// Current settings as TOML; a stored OBS password is replaced by a placeholder.
    async fn get_settings(&self) -> Result<String> {
        let mut s = self
            .store
            .load_settings()
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        if s.obs.password.as_deref().is_some_and(|p| !p.is_empty()) {
            s.obs.password = Some(PASSWORD_PLACEHOLDER.into());
        }
        toml::to_string_pretty(&s).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// Replace the settings. A password equal to the placeholder keeps the stored one.
    async fn apply_settings(&self, toml_text: String) -> Result<()> {
        let mut s: Settings = toml::from_str(&toml_text).map_err(|e| DeckError::Invalid(e.to_string()))?;
        if s.obs.password.as_deref() == Some(PASSWORD_PLACEHOLDER) {
            let old = self
                .store
                .load_settings()
                .map_err(|e| DeckError::Unavailable(e.to_string()))?;
            s.obs.password = old.obs.password;
        }
        self.store
            .save_settings(&s)
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        self.send(DeckMsg::Settings(Box::new(s))).await
    }

    /// Create an empty profile and return its id.
    async fn create_profile(
        &self,
        name: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DeckError::Invalid("a profile needs a name".into()));
        }
        let model = self.snapshot().await.map(|s| s.model).unwrap_or(Model::D200X);
        let p = Profile::new_default(name, model);
        self.store
            .save_profile(&p)
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        let id = p.id.to_string();
        self.send(DeckMsg::Apply { profile: Box::new(p) }).await?;
        let _ = Self::profiles_changed(&emitter).await;
        Ok(id)
    }

    /// Delete a profile (its files are moved aside). The last profile cannot be deleted.
    async fn delete_profile(
        &self,
        id: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        let id = Self::id(&id)?;
        if self.snapshot().await?.profiles.len() <= 1 {
            return Err(DeckError::Invalid("the last profile cannot be deleted".into()));
        }
        self.snap("before-delete")?;
        self.store
            .delete_profile(id)
            .map_err(|e| DeckError::NotFound(e.to_string()))?;
        self.send(DeckMsg::RemoveProfile(id)).await?;
        let _ = Self::profiles_changed(&emitter).await;
        Ok(())
    }

    /// The revision a client must pass to ApplyProfile for this profile.
    async fn profile_revision(&self, id: String) -> Result<u64> {
        Ok(self.revision(Self::id(&id)?))
    }

    #[zbus(signal)]
    async fn profiles_changed(e: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn device_state_changed(e: &SignalEmitter<'_>, state: &str, detail: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn input_event(e: &SignalEmitter<'_>, control: &str, kind: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn page_changed(e: &SignalEmitter<'_>, profile: &str, page: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_failed(e: &SignalEmitter<'_>, control: &str, message: &str) -> zbus::Result<()>;
}

fn kind_name(k: InputKind) -> &'static str {
    match k {
        InputKind::Press => "press",
        InputKind::Release => "release",
        InputKind::TurnLeft => "left",
        InputKind::TurnRight => "right",
    }
}

/// Turn deck events into D-Bus signals until the deck stops.
pub async fn forward_signals(conn: Connection, mut events: broadcast::Receiver<DeckEvent>) {
    let emitter = match SignalEmitter::new(&conn, OBJECT_PATH) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("cannot emit signals: {e}");
            return;
        }
    };
    loop {
        let ev = match events.recv().await {
            Ok(ev) => ev,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("dropped {n} deck events");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let r = match ev {
            DeckEvent::State(s) => DeckService::device_state_changed(&emitter, s.code(), &s.detail()).await,
            DeckEvent::Input(i) => {
                DeckService::input_event(&emitter, &i.control.as_str(), kind_name(i.kind)).await
            }
            DeckEvent::Page { profile, page } => {
                DeckService::page_changed(&emitter, &profile.to_string(), page as u32).await
            }
            DeckEvent::ActionFailed { control, message } => {
                DeckService::action_failed(&emitter, &control, &message).await
            }
        };
        if let Err(e) = r {
            tracing::debug!("signal not sent: {e}");
        }
    }
}
