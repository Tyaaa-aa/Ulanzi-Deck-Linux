//! The deck actor: owns the connected transport and the active profile, turns input into actions,
//! and keeps the device showing the right page.

mod nav;
mod press;
mod push;
mod sleep;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::{Instant, MissedTickBehavior};
use ulanzi_core::model::{Action, BrightnessOp, Model, Profile, ScreenConfig, Settings};
use ulanzi_core::render::{IconLibrary, RenderCtx};
use ulanzi_core::store::{ProfileLoad, Store};
use ulanzi_proto::{
    ButtonEntry, ButtonImage, Command, Control, DeviceReport, InputEvent, InputKind, ScreenMode, SmallWindow,
    brightness, button_zip, file_packets, label_style, parse_report, request_device_info, sleep_time,
    small_window, unlock,
};
use uuid::Uuid;

pub use nav::NavState;
pub use press::{Gesture, PressTracker};
pub use push::{BuiltPage, DynamicState, RenderCache, SCREEN_INDEX, build_page};
pub use sleep::SleepTimer;

use crate::actions::ActionRunner;
use crate::transport::{DeviceState, Transport};

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
    SetPage(usize),
    NextPage,
    PrevPage,
    Brightness(BrightnessOp),
    Dynamic(DynamicState),
    Query(oneshot::Sender<DeckSnapshot>),
    Shutdown,
}

/// Things that happened, for D-Bus signals and the GUI.
#[derive(Debug, Clone, PartialEq)]
pub enum DeckEvent {
    State(DeviceState),
    Page { profile: Uuid, page: usize },
    Input(InputEvent),
    ActionFailed { control: String, message: String },
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
    let (events, events_rx) = broadcast::channel(256);
    let deck = Deck::new(store, settings, runner, events);
    tokio::spawn(deck.run(rx));
    (tx, events_rx)
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
                }
            }
            DeckMsg::SwitchProfile(id) => self.switch_profile(id).await,
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
                if d != self.dynamic {
                    self.dynamic = d;
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
                    profiles: self.profiles.iter().map(|p| (p.id, p.name.clone())).collect(),
                    firmware: self.firmware.clone(),
                });
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
        let mode = match self.profile().screen {
            ScreenConfig::FirmwareClock => ScreenMode::Clock,
            ScreenConfig::FirmwareStats => ScreenMode::Stats,
            ScreenConfig::Custom { .. } => ScreenMode::Image,
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
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            gpu: pct("gpu"),
            clock_24h: true,
            date: String::new(),
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
        let profile = &self.profiles[self.active];
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
                let names: Vec<&str> = changed.values().filter_map(|e| e.icon.as_deref()).collect();
                let images: Vec<ButtonImage> = built
                    .images
                    .iter()
                    .filter(|img| names.iter().any(|n| n.ends_with(&img.name)))
                    .cloned()
                    .collect();
                tracing::info!("partial upload: keys {:?}", changed.keys().collect::<Vec<_>>());
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
            tracing::info!(
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
        {
            self.send_window().await;
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
                    let dial = self.profile().dials.get(i as usize);
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
            Control::Dial(i) => p.dials.get(i as usize)?.press.clone(),
            Control::Screen => match &p.screen {
                ScreenConfig::Custom { tap, .. } => tap.clone(),
                _ => None,
            },
            Control::Unknown(_) => None,
        }
    }

    fn hold_action(&self, c: Control) -> Option<Action> {
        let p = self.profile();
        match c {
            Control::Key(i) => p.pages.get(self.nav.page)?.keys.get(&i)?.hold.clone(),
            Control::Dial(i) => p.dials.get(i as usize)?.hold.clone(),
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
        if let Some(a) = a {
            self.dispatch(c, a).await;
        }
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
            let name = c.as_str();
            tokio::spawn(async move {
                while let Some(a) = rx.recv().await {
                    if let Err(e) = runner.run(&a).await {
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
                if let Some(id) = self.settings.default_profile {
                    self.switch_profile(id).await;
                }
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
                self.send(&[brightness(self.settings.brightness)]).await;
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
        let profile = self.profile().id;
        self.emit(DeckEvent::Page {
            profile,
            page: self.nav.page,
        });
        self.push_page(false).await;
    }
}
