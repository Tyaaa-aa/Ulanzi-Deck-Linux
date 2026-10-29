//! The deck actor: owns the connected transport and the active profile, turns input into actions,
//! and keeps the device showing the right page.

mod nav;
mod press;
mod push;
mod screen_rt;
mod sleep;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::{Instant, MissedTickBehavior};
use ulanzi_core::model::{
    Action, AudioTarget, BrightnessOp, ClockStyle, Model, Profile, ScreenConfig, Settings,
};
use ulanzi_core::render::{IconLibrary, RenderCtx};
use ulanzi_core::store::{ProfileLoad, Store};
use ulanzi_proto::{
    ButtonEntry, Command, Control, DeviceReport, InputEvent, InputKind, ScreenMode, SmallWindow, brightness,
    button_zip, file_packets, label_style, parse_report, request_device_info, sleep_time, small_window,
    unlock,
};
use uuid::Uuid;

pub use nav::NavState;
pub use press::{Gesture, PressTracker};
pub use push::{BuiltPage, DynamicState, RenderCache, SCREEN_INDEX, build_page};
pub use sleep::SleepTimer;

use crate::actions::ActionRunner;
use crate::transport::{DeviceState, Transport};
use screen_rt::ScreenRuntime;

/// The wide-screen update goes out this often; like Ulanzi Studio's, it also keeps the deck from
/// locking itself (it locks after ~2 minutes without host traffic).
const TICK: Duration = Duration::from_secs(1);
/// Re-send unlock this often, in case the deck locked anyway.
const UNLOCK_EVERY: Duration = Duration::from_secs(30);
/// Minimum time between uploads. With correct framing (report ID 0) the deck took back-to-back
/// full and partial uploads on hardware; the gap is a safety margin that also folds rapid changes.
const MIN_UPLOAD_GAP: Duration = Duration::from_millis(500);
/// How long to wait for the device-info reply (the deck's "ready") before uploading anyway.
const INFO_TIMEOUT: Duration = Duration::from_secs(3);

/// Time to wait after an upload before the next one (flat; size made no difference on hardware).
pub fn upload_cooldown(_bytes: usize) -> Duration {
    MIN_UPLOAD_GAP
}

/// The page last uploaded: (profile, page index) and its manifest entries.
struct Shown {
    at: (Uuid, usize),
    entries: BTreeMap<u8, ButtonEntry>,
}

/// Commands for the deck.
pub enum DeckMsg {
    /// A transport opened on the given hidraw node.
    DeviceUp(Box<dyn Transport>, std::path::PathBuf),
    DeviceDown(DeviceState),
    /// A profile was changed (already validated and saved by the caller).
    Apply {
        profile: Box<Profile>,
    },
    /// Settings changed (already saved by the caller).
    Settings(Box<Settings>),
    SwitchProfile(Uuid),
    /// A profile was deleted (the caller already moved its files aside).
    RemoveProfile(Uuid),
    SetPage(usize),
    NextPage,
    PrevPage,
    Brightness(BrightnessOp),
    Dynamic(Box<DynamicState>),
    Query(oneshot::Sender<DeckSnapshot>),
    /// The desktop says another window is in front (auto profile switching).
    Focus(ulanzi_core::autoswitch::Window),
    /// Turn auto profile switching on or off (tray).
    ToggleAutoSwitch,
    /// A plugin set a state value (`<plugin>.<key>` in labels).
    PluginState {
        plugin: String,
        key: String,
        value: String,
    },
    /// A plugin instance ended (stopped, crashed, replaced or removed): forget its state values.
    PluginCleared(String),
    Shutdown,
}

/// Things that happened, for D-Bus signals and the GUI.
#[derive(Debug, Clone, PartialEq)]
pub enum DeckEvent {
    State(DeviceState),
    Page {
        profile: Uuid,
        page: usize,
    },
    Input(InputEvent),
    ActionFailed {
        control: String,
        message: String,
    },
    /// The deck changed settings itself (auto-switch toggle, brightness keys).
    SettingsChanged,
}

/// Current state, for `Hello` and `ulanzictl status`.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckSnapshot {
    pub device: DeviceState,
    pub model: Model,
    pub profile: Uuid,
    pub profile_name: String,
    pub page: usize,
    pub profiles: Vec<(Uuid, String)>,
    /// Auto profile switching is on.
    pub auto_switch: bool,
    /// Firmware version the deck reported, once it answered the info request.
    pub firmware: Option<String>,
}

/// Start the deck. Profiles are read from `store`; if there are none a default one is created.
pub fn spawn_deck(
    store: Store,
    settings: Settings,
    runner: Arc<dyn ActionRunner>,
) -> (mpsc::Sender<DeckMsg>, broadcast::Receiver<DeckEvent>) {
    let (tx, rx) = mpsc::channel(256);
    let events = start_deck(store, settings, runner, rx);
    (tx, events)
}

/// Start the deck actor on an existing channel (messages sent before this wait in the channel).
pub fn start_deck(
    store: Store,
    settings: Settings,
    runner: Arc<dyn ActionRunner>,
    rx: mpsc::Receiver<DeckMsg>,
) -> broadcast::Receiver<DeckEvent> {
    let (events, events_rx) = broadcast::channel(256);
    let deck = Deck::new(store, settings, runner, events);
    tokio::spawn(deck.run(rx));
    events_rx
}

/// Reads the volume after volume actions, off the action queue, one read at a time: a burst of
/// dial steps gives one read now and one more after the burst.
/// A volume level for the overlay: percent, muted, and what it belongs to (empty: the default output).
type Level = (u8, bool, String);

#[derive(Clone)]
struct LevelReader {
    tx: mpsc::UnboundedSender<Level>,
    busy: Arc<std::sync::atomic::AtomicBool>,
    again: Arc<std::sync::atomic::AtomicBool>,
}

impl LevelReader {
    fn new(tx: mpsc::UnboundedSender<Level>) -> LevelReader {
        LevelReader {
            tx,
            busy: Arc::default(),
            again: Arc::default(),
        }
    }

    fn request(&self, target: AudioTarget) {
        use std::sync::atomic::Ordering::SeqCst;
        if self.busy.swap(true, SeqCst) {
            self.again.store(true, SeqCst);
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                me.again.store(false, SeqCst);
                if let Some(level) = crate::actions::audio::current_level(&target).await {
                    let _ = me.tx.send(level);
                }
                if !me.again.load(SeqCst) {
                    break;
                }
            }
            me.busy.store(false, SeqCst);
        });
    }
}

struct Deck {
    store: Store,
    settings: Settings,
    profiles: Vec<Profile>,
    active: usize,
    nav: NavState,
    device: DeviceState,
    transport: Option<Box<dyn Transport>>,
    events: broadcast::Sender<DeckEvent>,
    runner: Arc<dyn ActionRunner>,
    queues: HashMap<Control, mpsc::UnboundedSender<Action>>,
    press: PressTracker,
    sleep: SleepTimer,
    /// Controls whose press woke the deck; their release is swallowed.
    swallowed: Vec<Control>,
    dynamic: DynamicState,
    cache: RenderCache,
    /// What the deck shows now, for partial updates.
    shown: Option<Shown>,
    /// No upload before this (the deck is unpacking the previous one).
    next_upload_at: Option<Instant>,
    /// Set from connect until the deck answers the info request.
    awaiting_info: Option<Instant>,
    upload_pending: bool,
    /// The pending upload must be a full page (a connect or page change asked for it).
    pending_full: bool,
    last_unlock: Instant,
    firmware: Option<String>,
    /// Counts partial uploads, for fresh image names.
    upload_seq: u32,
    /// The window in front, as last reported by the desktop.
    focused: Option<ulanzi_core::autoswitch::Window>,
    /// Widgets and the volume overlay on the wide screen.
    screen: ScreenRuntime,
    /// Hash of the wide-screen picture last handed to the page.
    screen_hash: Option<u64>,
    /// Volume levels read after volume actions, for the overlay.
    level_tx: LevelReader,
    level_rx: Option<mpsc::UnboundedReceiver<Level>>,
}

impl Deck {
    fn new(
        store: Store,
        settings: Settings,
        runner: Arc<dyn ActionRunner>,
        events: broadcast::Sender<DeckEvent>,
    ) -> Deck {
        let mut profiles: Vec<Profile> = store
            .list_profiles()
            .into_iter()
            .filter_map(|l| match l {
                ProfileLoad::Ok(p) => Some(*p),
                ProfileLoad::Invalid { .. } => None,
            })
            .collect();
        if profiles.is_empty() {
            let p = Profile::new_default("Default", Model::D200X);
            if let Err(e) = store.save_profile(&p) {
                tracing::error!("could not save the default profile: {e}");
            }
            profiles.push(p);
        }
        let active = settings
            .default_profile
            .and_then(|id| profiles.iter().position(|p| p.id == id))
            .unwrap_or(0);
        let now = Instant::now();
        let (level_tx, level_rx) = mpsc::unbounded_channel();
        let level_tx = LevelReader::new(level_tx);
        Deck {
            press: PressTracker::new(settings.hold_ms),
            sleep: SleepTimer::new(settings.sleep_after_min, now),
            store,
            settings,
            profiles,
            active,
            nav: NavState::default(),
            device: DeviceState::NotFound,
            transport: None,
            events,
            runner,
            queues: HashMap::new(),
            swallowed: Vec::new(),
            dynamic: DynamicState::default(),
            cache: RenderCache::default(),
            shown: None,
            next_upload_at: None,
            awaiting_info: None,
            upload_pending: false,
            pending_full: false,
            last_unlock: now,
            firmware: None,
            upload_seq: 0,
            focused: None,
            screen: ScreenRuntime::new(now),
            screen_hash: None,
            level_tx,
            level_rx: Some(level_rx),
        }
    }

    fn profile(&self) -> &Profile {
        &self.profiles[self.active]
    }

    fn emit(&self, e: DeckEvent) {
        let _ = self.events.send(e);
    }

    async fn run(mut self, mut rx: mpsc::Receiver<DeckMsg>) {
        let mut tick = tokio::time::interval_at(Instant::now() + TICK, TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let wake = self.screen.wake.clone();
        let mut levels = self.level_rx.take().expect("run once");
        loop {
            let hold_deadline = self.press.next_deadline();
            // Only arm the upload timer when an upload can actually happen; otherwise a deadline in
            // the past would fire on every loop and spin a core.
            let can_upload = self.transport.is_some() && self.awaiting_info.is_none();
            let upload_at = if self.upload_pending && can_upload {
                self.next_upload_at
            } else {
                None
            };
            let transport = self.transport.as_mut();
            tokio::select! {
                msg = rx.recv() => match msg {
                    None | Some(DeckMsg::Shutdown) => break,
                    Some(m) => self.on_msg(m).await,
                },
                report = async {
                    match transport {
                        Some(t) => t.recv().await,
                        None => std::future::pending().await,
                    }
                } => match report {
                    Ok(buf) => self.on_report(parse_report(&buf)).await,
                    Err(e) => self.lost(&e).await,
                },
                _ = async {
                    match hold_deadline {
                        Some(d) => tokio::time::sleep_until(d).await,
                        None => std::future::pending().await,
                    }
                } => {
                    for c in self.press.poll(Instant::now()) {
                        self.gesture(c, Gesture::Hold).await;
                    }
                },
                _ = async {
                    match upload_at {
                        Some(d) => tokio::time::sleep_until(d).await,
                        None => std::future::pending().await,
                    }
                } => self.push_page(false).await,
                _ = tick.tick() => self.on_tick().await,
                _ = wake.notified() => self.refresh_screen().await,
                Some((pct, muted, label)) = levels.recv() => {
                    if self.settings.volume_overlay {
                        self.screen.show_overlay(pct, muted, label, Instant::now());
                    }
                    self.refresh_screen().await;
                },
            }
        }
    }

    async fn on_msg(&mut self, m: DeckMsg) {
        match m {
            DeckMsg::DeviceUp(t, node) => {
                self.transport = Some(t);
                self.sleep.wake(Instant::now());
                self.set_device(DeviceState::Ready { node });
                self.configure().await;
            }
            DeckMsg::DeviceDown(state) => {
                self.transport = None;
                self.upload_pending = false;
                self.set_device(state);
            }
            DeckMsg::Apply { profile } => {
                let mut profile = *profile;
                profile.normalize();
                match self.profiles.iter().position(|p| p.id == profile.id) {
                    Some(i) => self.profiles[i] = profile,
                    None => self.profiles.push(profile),
                }
                self.nav.clamp(self.profile().pages.len());
                self.update_screen();
                self.send_window().await;
                self.push_page(false).await;
            }
            DeckMsg::Settings(s) => {
                let label_changed =
                    s.hide_labels != self.settings.hide_labels || s.label_style != self.settings.label_style;
                self.settings = *s;
                self.press = PressTracker::new(self.settings.hold_ms);
                self.sleep = SleepTimer::new(self.settings.sleep_after_min, Instant::now());
                self.send(&[brightness(self.settings.brightness)]).await;
                if label_changed {
                    self.send_label_style().await;
                    // Labels drawn into key pictures follow "Hide All Labels" too.
                    self.push_page(false).await;
                }
            }
            DeckMsg::SwitchProfile(id) => self.switch_profile(id).await,
            DeckMsg::RemoveProfile(id) => {
                if self.profiles.len() > 1
                    && let Some(i) = self.profiles.iter().position(|p| p.id == id)
                {
                    let was_active = i == self.active;
                    self.profiles.remove(i);
                    if self.active >= i && self.active > 0 {
                        self.active -= 1;
                    }
                    if was_active {
                        let id = self.profiles[self.active].id;
                        self.switch_profile(id).await;
                    }
                }
            }
            DeckMsg::SetPage(i) => {
                let pages = self.profile().pages.clone();
                self.nav.goto(i, &pages);
                self.page_changed().await;
            }
            DeckMsg::NextPage => {
                self.internal(&Action::NextPage).await;
            }
            DeckMsg::PrevPage => {
                self.internal(&Action::PrevPage).await;
            }
            DeckMsg::Brightness(op) => {
                self.internal(&Action::Brightness { op }).await;
            }
            DeckMsg::Dynamic(d) => {
                let mut d = *d;
                // The sampler sends values only; the screen picture is ours.
                d.keep_deck_fields(&self.dynamic);
                let changed = d != self.dynamic;
                self.dynamic = d;
                if self.update_screen() || changed {
                    self.push_page(false).await;
                }
            }
            DeckMsg::Query(reply) => {
                let p = self.profile();
                let _ = reply.send(DeckSnapshot {
                    device: self.device.clone(),
                    model: p.model,
                    profile: p.id,
                    profile_name: p.name.clone(),
                    page: self.nav.page,
                    profiles: {
                        let mut list: Vec<(Uuid, String)> =
                            self.profiles.iter().map(|p| (p.id, p.name.clone())).collect();
                        ulanzi_core::model::order_profiles(&mut list, &self.settings.profile_order);
                        list
                    },
                    firmware: self.firmware.clone(),
                    auto_switch: self.settings.auto_switch,
                });
            }
            DeckMsg::ToggleAutoSwitch => {
                self.internal(&Action::AutoProfile).await;
            }
            DeckMsg::Focus(w) => {
                self.focused = Some(w);
                self.auto_switch().await;
            }
            DeckMsg::PluginState { plugin, key, value } => {
                if self.dynamic.set_plugin_var(&plugin, &key, value) {
                    // The OBS widget on the wide screen shows the OBS plugin's values.
                    if plugin == "obs" {
                        self.update_screen();
                    }
                    self.push_page(false).await;
                }
            }
            DeckMsg::PluginCleared(plugin) => {
                if self.dynamic.clear_plugin(&plugin) {
                    self.push_page(false).await;
                }
            }
            DeckMsg::Shutdown => {}
        }
    }

    fn set_device(&mut self, s: DeviceState) {
        if s != self.device {
            self.device = s.clone();
            self.emit(DeckEvent::State(s));
        }
    }

    /// Full setup after connecting, in Ulanzi Studio's order: unlock (a locked deck reports no keys),
    /// label style, device info request, brightness, sleep time, page, wide screen.
    async fn configure(&mut self) {
        self.shown = None;
        // New connection, new image names: the deck must not match them against anything it cached.
        self.cache.generation = self.cache.generation.wrapping_add(1);
        self.last_unlock = Instant::now();
        if !self.send(&[unlock()]).await {
            return;
        }
        self.send_label_style().await;
        // Dimming is done by our own sleep timer, so the deck's built-in sleep stays off.
        self.send(&[
            request_device_info(),
            brightness(self.settings.brightness),
            sleep_time(0),
        ])
        .await;
        // The page goes out once the deck answers the info request (it is ready then), or after
        // INFO_TIMEOUT. Uploading the moment a rebooted deck reappears leaves it on its factory page.
        self.awaiting_info = Some(Instant::now());
    }

    /// The deck is ready (or we stopped waiting): upload the page and the wide screen.
    async fn deck_ready(&mut self) {
        self.awaiting_info = None;
        self.push_page(true).await;
        self.send_window().await;
    }

    async fn on_report(&mut self, r: DeviceReport) {
        match r {
            DeviceReport::Input(ev) => self.on_input(ev).await,
            DeviceReport::LabelStyleRequest => self.send_label_style().await,
            DeviceReport::DeviceInfo(json) => {
                let v: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
                let fw = v.get("Dversion").and_then(|f| f.as_str()).map(str::to_string);
                // The deck sends info frames for more than our request (unlock triggers one too), so
                // they are only recorded. Reconfiguring here looped on real hardware; a deck that
                // really restarts re-enumerates on USB and gets a full setup from the hot-plug path.
                if fw != self.firmware {
                    tracing::info!(
                        "deck info: {}",
                        json.split_whitespace().collect::<Vec<_>>().join(" ")
                    );
                    self.firmware = fw;
                }
                if self.awaiting_info.is_some() {
                    self.deck_ready().await;
                }
            }
            DeviceReport::Other(code) => tracing::debug!("unhandled report {code:#06x}"),
        }
    }

    async fn send_label_style(&mut self) {
        let style = self.settings.label_style.to_proto(!self.settings.hide_labels);
        self.send(&[label_style(&style)]).await;
    }

    async fn send_window(&mut self) {
        let now = chrono::Local::now();
        let (mode, date) = match self.profile().screen {
            ScreenConfig::FirmwareClock { style } => match style {
                ClockStyle::Classic => (ScreenMode::Clock, String::new()),
                ClockStyle::DateWeekday => (
                    ScreenMode::ClockDateWeekday,
                    now.format("%Y-%m-%d %a").to_string(),
                ),
                ClockStyle::Weekday => (ScreenMode::ClockWeekday, now.format("%a").to_string()),
                ClockStyle::Date => (ScreenMode::ClockDate, now.format("%Y-%m-%d").to_string()),
                ClockStyle::Large => (ScreenMode::ClockLarge, String::new()),
            },
            ScreenConfig::FirmwareStats => (ScreenMode::Stats, String::new()),
            ScreenConfig::Custom { .. } => (ScreenMode::Image, String::new()),
        };
        // The volume overlay is a picture, so a firmware mode gives way to it for a moment.
        let (mode, date) = if self.screen.overlay_active(Instant::now()) {
            (ScreenMode::Image, String::new())
        } else {
            (mode, date)
        };
        let pct = |k: &str| {
            self.dynamic
                .vars
                .get(k)
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(0)
        };
        let w = SmallWindow {
            mode,
            cpu: pct("cpu"),
            mem: pct("ram"),
            time: now.format("%H:%M:%S").to_string(),
            gpu: pct("gpu"),
            clock_24h: true,
            date,
        };
        self.send(&[small_window(&w)]).await;
    }

    /// Upload the current page if anything changed: the whole page when the page or profile changed
    /// (or with `force`), otherwise only the changed keys. After each upload the deck needs time to
    /// unpack it ([`upload_cooldown`]); a request inside that time is remembered and sent, with the
    /// latest state, when it ends.
    async fn push_page(&mut self, force: bool) {
        // Before the deck is ready, deck_ready() does the upload.
        if self.transport.is_none() || self.awaiting_info.is_some() {
            return;
        }
        let now = Instant::now();
        // Even a forced (full) upload waits; a second upload while the deck unpacks crashes it.
        if self.next_upload_at.is_some_and(|t| now < t) {
            self.upload_pending = true;
            self.pending_full |= force;
            return;
        }
        let force = force || std::mem::take(&mut self.pending_full);
        self.upload_pending = false;
        let hidden;
        let mut profile = &self.profiles[self.active];
        if self.settings.hide_labels {
            hidden = {
                let mut p = profile.clone();
                p.hide_drawn_labels();
                p
            };
            profile = &hidden;
        }
        let dir = self.store.profile_dir(profile.id);
        let ctx = RenderCtx {
            profile_dir: &dir,
            icons: IconLibrary::bundled(),
        };
        let mut dynamic = self.dynamic.clone();
        dynamic
            .vars
            .insert("page".into(), (self.nav.page + 1).to_string());
        dynamic.vars.insert("profile".into(), profile.name.clone());
        let here = (profile.id, self.nav.page);
        let built = build_page(profile, self.nav.page, &ctx, &dynamic, &mut self.cache);
        let same_page = self.shown.as_ref().filter(|s| !force && s.at == here);
        let packets = match same_page {
            // Same page: send only the keys that changed (what Ulanzi Studio does).
            Some(shown) => {
                let changed: BTreeMap<u8, ButtonEntry> = built
                    .entries
                    .iter()
                    .filter(|(i, e)| shown.entries.get(i) != Some(e))
                    .map(|(i, e)| (*i, e.clone()))
                    .collect();
                if changed.is_empty() {
                    return;
                }
                self.upload_seq = self.upload_seq.wrapping_add(1);
                let (changed, images) = push::with_fresh_names(&changed, &built.images, self.upload_seq);
                tracing::debug!("partial upload: keys {:?}", changed.keys().collect::<Vec<_>>());
                file_packets(Command::PartialUpdate, &button_zip(&changed, &images))
            }
            None => {
                tracing::info!(
                    "full upload: page {}, {} images",
                    self.nav.page + 1,
                    built.images.len()
                );
                file_packets(Command::SetButtons, &button_zip(&built.entries, &built.images))
            }
        };
        let bytes = packets.len() * ulanzi_proto::PACKET_SIZE;
        if self.send(&packets).await {
            let done = Instant::now();
            tracing::debug!(
                "upload of {} KB sent in {} ms",
                bytes / 1024,
                done.duration_since(now).as_millis()
            );
            self.next_upload_at = Some(done + upload_cooldown(bytes));
            self.shown = Some(Shown {
                at: here,
                entries: built.entries,
            });
        }
    }

    /// Send packets; on I/O failure drop the transport and report the device as gone.
    async fn send(&mut self, packets: &[ulanzi_proto::Packet]) -> bool {
        let Some(t) = self.transport.as_mut() else {
            return false;
        };
        match t.send(packets).await {
            Ok(()) => true,
            Err(e) => {
                self.lost(&e).await;
                false
            }
        }
    }

    async fn lost(&mut self, e: &std::io::Error) {
        tracing::warn!("device connection lost: {e}");
        self.transport = None;
        self.shown = None;
        self.firmware = None;
        // configure() uploads the whole page on reconnect anyway.
        self.upload_pending = false;
        self.set_device(DeviceState::NotFound);
    }

    async fn on_tick(&mut self) {
        let now = Instant::now();
        if self
            .awaiting_info
            .is_some_and(|t| now.duration_since(t) >= INFO_TIMEOUT)
        {
            tracing::warn!("the deck did not answer the info request; uploading anyway");
            self.deck_ready().await;
        }
        if self.sleep.should_sleep(now) {
            self.sleep.sleep();
            tracing::info!("sleeping after inactivity");
            self.send(&[brightness(self.settings.sleep_brightness)]).await;
        }
        if now.duration_since(self.last_unlock) >= UNLOCK_EVERY {
            self.last_unlock = now;
            self.send(&[unlock()]).await;
        }
        self.refresh_screen().await;
        self.send_window().await;
    }

    /// Redraw the wide-screen picture; true if it changed.
    fn update_screen(&mut self) -> bool {
        // Timer keys show the time left, so a ticking timer counts as a change.
        let t = self.screen.timer_now(Instant::now());
        let text = format!("{:02}:{:02}", t.remaining_s / 60, t.remaining_s % 60);
        let timer_changed = t.running && self.dynamic.vars.get("timer") != Some(&text);
        self.dynamic.vars.insert("timer".into(), text);
        self.dynamic
            .vars
            .insert("timer_running".into(), if t.running { "1" } else { "0" }.into());
        let p = &self.profiles[self.active];
        let dir = self.store.profile_dir(p.id);
        let png = self.screen.picture(p, &dir, &self.dynamic, Instant::now());
        if !screen_rt::changed(&mut self.screen_hash, png.as_deref()) {
            return timer_changed;
        }
        self.dynamic.screen_png = png;
        true
    }

    /// Redraw the wide screen and upload it if it changed.
    async fn refresh_screen(&mut self) {
        let was = self.dynamic.screen_png.is_some();
        if self.update_screen() {
            if was != self.dynamic.screen_png.is_some() {
                // Overlay came or went on a firmware mode.
                self.send_window().await;
            }
            self.push_page(false).await;
        }
    }

    async fn on_input(&mut self, ev: InputEvent) {
        let now = Instant::now();
        self.emit(DeckEvent::Input(ev));
        self.sleep.touch(now);
        if self.sleep.is_asleep() {
            self.sleep.wake(now);
            tracing::info!("waking up");
            self.send(&[brightness(self.settings.brightness)]).await;
            if ev.kind == InputKind::Press {
                self.swallowed.push(ev.control);
            }
            return;
        }
        match ev.kind {
            InputKind::Press => {
                let has_hold = self.hold_action(ev.control).is_some();
                self.press.on_press(ev.control, now, has_hold);
            }
            InputKind::Release => {
                if let Some(i) = self.swallowed.iter().position(|c| *c == ev.control) {
                    self.swallowed.remove(i);
                    self.press.forget(ev.control);
                    return;
                }
                if let Some(g) = self.press.on_release(ev.control, now) {
                    self.gesture(ev.control, g).await;
                }
            }
            InputKind::TurnLeft | InputKind::TurnRight => {
                if let Control::Dial(i) = ev.control {
                    let dial = self
                        .profile()
                        .pages
                        .get(self.nav.page)
                        .and_then(|p| p.dials.get(i as usize));
                    let a = if ev.kind == InputKind::TurnLeft {
                        dial.and_then(|d| d.left.clone())
                    } else {
                        dial.and_then(|d| d.right.clone())
                    };
                    if let Some(a) = a {
                        self.dispatch(ev.control, a).await;
                    }
                }
            }
        }
    }

    fn tap_action(&self, c: Control) -> Option<Action> {
        let p = self.profile();
        match c {
            Control::Key(i) => p.pages.get(self.nav.page)?.keys.get(&i)?.tap.clone(),
            Control::Small(0) => p.small_buttons.left.clone(),
            Control::Small(_) => p.small_buttons.right.clone(),
            Control::Dial(i) => p.pages.get(self.nav.page)?.dials.get(i as usize)?.press.clone(),
            Control::Screen => match &p.screen {
                ScreenConfig::Custom { tap, .. } => tap.clone().or_else(|| self.screen.default_tap(p)),
                _ => None,
            },
            Control::Unknown(_) => None,
        }
    }

    fn hold_action(&self, c: Control) -> Option<Action> {
        let p = self.profile();
        match c {
            Control::Key(i) => p.pages.get(self.nav.page)?.keys.get(&i)?.hold.clone(),
            Control::Dial(i) => p.pages.get(self.nav.page)?.dials.get(i as usize)?.hold.clone(),
            Control::Screen => match &p.screen {
                ScreenConfig::Custom { hold, .. } => hold.clone(),
                _ => None,
            },
            Control::Small(_) | Control::Unknown(_) => None,
        }
    }

    async fn gesture(&mut self, c: Control, g: Gesture) {
        let a = match g {
            Gesture::Tap => self.tap_action(c),
            Gesture::Hold => self.hold_action(c),
        };
        let a = match a {
            Some(Action::Cycle { steps, .. }) => self.cycle_step(c, g, &steps).await,
            a => a,
        };
        if let Some(a) = a {
            self.dispatch(c, a).await;
        }
    }

    /// The step of a cycle to run now; the next press runs the one after, and the key shows it.
    async fn cycle_step(
        &mut self,
        c: Control,
        g: Gesture,
        steps: &[ulanzi_core::model::CycleStep],
    ) -> Option<Action> {
        if steps.is_empty() {
            return None;
        }
        let page = self.profile().pages.get(self.nav.page)?.id;
        let control = match g {
            Gesture::Tap => c.as_str(),
            Gesture::Hold => format!("{}:hold", c.as_str()),
        };
        let at = push::cycle_key(page, &control);
        let now = self.dynamic.cycles.get(&at).copied().unwrap_or(0) % steps.len();
        self.dynamic.cycles.insert(at, (now + 1) % steps.len());
        self.push_page(false).await;
        Some(steps[now].action.clone())
    }

    /// Deck-internal actions run here; everything else goes to the control's ordered queue.
    async fn dispatch(&mut self, c: Control, a: Action) {
        if self.internal(&a).await {
            return;
        }
        let tx = self.queues.entry(c).or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
            let runner = self.runner.clone();
            let events = self.events.clone();
            let levels = self.level_tx.clone();
            let name = c.as_str();
            tokio::spawn(async move {
                while let Some(a) = rx.recv().await {
                    let target = match &a {
                        Action::Volume { target, .. } => Some(target.clone()),
                        _ => None,
                    };
                    let result = runner.run(&a).await;
                    if result.is_ok()
                        && let Some(target) = target
                    {
                        levels.request(target);
                    }
                    if let Err(e) = result {
                        tracing::warn!("{name}: {e}");
                        let _ = events.send(DeckEvent::ActionFailed {
                            control: name.clone(),
                            message: e.to_string(),
                        });
                    }
                }
            });
            tx
        });
        let _ = tx.send(a);
    }

    /// Run navigation, brightness and sleep actions. Returns false for anything else.
    async fn internal(&mut self, a: &Action) -> bool {
        let pages = self.profile().pages.clone();
        let find = |id: &Uuid| pages.iter().position(|p| p.id == *id);
        match a {
            Action::NextPage => self.nav.next(&pages),
            Action::PrevPage => self.nav.prev(&pages),
            Action::Back => self.nav.back(),
            Action::GoToPage { page } => match find(page) {
                Some(i) => self.nav.goto(i, &pages),
                None => return self.missing("page"),
            },
            Action::OpenFolder { page } => match find(page) {
                Some(i) => self.nav.open_folder(i, &pages),
                None => return self.missing("folder"),
            },
            Action::SwitchProfile { profile } => {
                self.switch_profile(*profile).await;
                return true;
            }
            Action::AutoProfile => {
                // Turn switching by the app in front on or off.
                self.settings.auto_switch = !self.settings.auto_switch;
                if let Err(e) = self.store.save_settings(&self.settings) {
                    tracing::warn!("could not save settings: {e}");
                }
                self.emit(DeckEvent::SettingsChanged);
                tracing::info!(
                    "auto profile switching {}",
                    if self.settings.auto_switch { "on" } else { "off" }
                );
                self.auto_switch().await;
                return true;
            }
            Action::Brightness { op } => {
                let b = self.settings.brightness;
                self.settings.brightness = match op {
                    BrightnessOp::Set { percent } => (*percent).min(100),
                    BrightnessOp::Up { step } => b.saturating_add(*step).min(100),
                    BrightnessOp::Down { step } => b.saturating_sub(*step),
                };
                if let Err(e) = self.store.save_settings(&self.settings) {
                    tracing::warn!("could not save brightness: {e}");
                }
                self.emit(DeckEvent::SettingsChanged);
                self.send(&[brightness(self.settings.brightness)]).await;
                return true;
            }
            Action::ScreenNext => {
                self.screen.next(Instant::now());
                self.refresh_screen().await;
                return true;
            }
            Action::Timer { op } => {
                self.screen.timer(*op, Instant::now());
                self.refresh_screen().await;
                return true;
            }
            Action::SleepNow => {
                self.sleep.sleep();
                self.send(&[brightness(self.settings.sleep_brightness)]).await;
                return true;
            }
            _ => return false,
        }
        self.page_changed().await;
        true
    }

    fn missing(&self, what: &str) -> bool {
        self.emit(DeckEvent::ActionFailed {
            control: String::new(),
            message: format!("that {what} no longer exists in this profile"),
        });
        true
    }

    /// Switch to the profile that belongs to the window in front, if any profile claims apps.
    async fn auto_switch(&mut self) {
        use ulanzi_core::autoswitch::{Rule, aliases, pick_profile};
        let Some(w) = self.focused.clone() else { return };
        if !self.settings.auto_switch {
            return;
        }
        let mut order: Vec<(Uuid, ())> = self.profiles.iter().map(|p| (p.id, ())).collect();
        ulanzi_core::model::order_profiles(&mut order, &self.settings.profile_order);
        let rules: Vec<Rule> = order
            .iter()
            .filter_map(|(id, _)| self.profiles.iter().find(|p| p.id == *id))
            .filter(|p| !p.r#match.apps.is_empty())
            .map(|p| Rule {
                profile: p.id,
                apps: p
                    .r#match
                    .apps
                    .iter()
                    .map(|a| aliases(a, ulanzi_core::apps::lookup(a).as_ref()))
                    .collect(),
                priority: p.r#match.priority,
            })
            .collect();
        if rules.is_empty() {
            return;
        }
        let fallback = self
            .settings
            .default_profile
            .filter(|d| self.profiles.iter().any(|p| p.id == *d))
            .or_else(|| order.first().map(|(id, _)| *id));
        let Some(fallback) = fallback else { return };
        if let Some(target) = pick_profile(&rules, &w, fallback)
            && target != self.profile().id
        {
            tracing::info!(
                "{} is in front: switching profile",
                if w.class.is_empty() {
                    &w.desktop_id
                } else {
                    &w.class
                }
            );
            self.switch_profile(target).await;
        }
    }

    async fn switch_profile(&mut self, id: Uuid) {
        let Some(i) = self.profiles.iter().position(|p| p.id == id) else {
            self.missing("profile");
            return;
        };
        self.active = i;
        self.nav = NavState::default();
        self.send_window().await;
        self.page_changed().await;
    }

    async fn page_changed(&mut self) {
        self.update_screen();
        let profile = self.profile().id;
        self.emit(DeckEvent::Page {
            profile,
            page: self.nav.page,
        });
        self.push_page(false).await;
    }
}
