use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use ulanzi_core::model::*;
use ulanzi_core::store::Store;
use ulanzi_deckd::actions::{ActionError, ActionRunner};
use ulanzi_deckd::deck::{DeckEvent, DeckMsg, DynamicState, spawn_deck};
use ulanzi_deckd::transport::{DeviceState, MockHandle, MockTransport};

const BRIGHTNESS: u16 = 0x000a;
const LABEL: u16 = 0x000b;
const WINDOW: u16 = 0x0006;
const BUTTONS: u16 = 0x0001;

#[derive(Default, Clone)]
struct FakeRunner {
    ran: Arc<Mutex<Vec<Action>>>,
}

#[async_trait::async_trait]
impl ActionRunner for FakeRunner {
    async fn run(&self, action: &Action) -> Result<(), ActionError> {
        self.ran.lock().expect("lock").push(action.clone());
        match action {
            Action::Command { cmd, .. } if cmd == "fail" => Err(ActionError::Failed("boom".into())),
            _ => Ok(()),
        }
    }
}

impl FakeRunner {
    fn ran(&self) -> Vec<Action> {
        self.ran.lock().expect("lock").clone()
    }
}

fn cmd(c: &str) -> Action {
    Action::Command {
        cmd: c.into(),
        terminal: false,
    }
}

fn key(tap: Option<Action>, hold: Option<Action>) -> Key {
    let look = KeyLook {
        source: ImageSource::Icon {
            set: IconSet::Lucide,
            name: "play".into(),
        },
        ..KeyLook::default()
    };
    Key { look, tap, hold }
}

/// Pages: 0 "Main", 1 "Two", 2 folder "Stuff".
fn profile() -> Profile {
    let mut p = Profile::new_default("Default", Model::D200X);
    p.pages.push(Page::new("Two"));
    let mut folder = Page::new("Stuff");
    folder.kind = PageKind::Folder;
    folder.keys.insert(4, key(Some(Action::Back), None));
    let folder_id = folder.id;
    p.pages.push(folder);
    let keys = &mut p.pages[0].keys;
    keys.insert(0, key(Some(Action::OpenFolder { page: folder_id }), None));
    keys.insert(1, key(Some(cmd("tap")), Some(cmd("hold"))));
    keys.insert(2, key(Some(cmd("only-tap")), None));
    keys.insert(3, key(Some(cmd("fail")), None));
    p.dials[0].right = Some(Action::Volume {
        target: AudioTarget::Default,
        op: VolumeOp::Up,
        step: 2,
        limit_100: true,
    });
    p
}

struct Rig {
    _dir: tempfile::TempDir,
    tx: mpsc::Sender<DeckMsg>,
    events: broadcast::Receiver<DeckEvent>,
    mock: MockHandle,
    runner: FakeRunner,
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(20)).await;
}

async fn rig_with(settings: Settings) -> Rig {
    let dir = tempfile::tempdir().expect("tmp");
    let store = Store::at(dir.path().join("c"), dir.path().join("s"), dir.path().join("k"));
    let p = profile();
    store.save_profile(&p).expect("save");
    let settings = Settings {
        default_profile: Some(p.id),
        ..settings
    };
    let runner = FakeRunner::default();
    let (tx, events) = spawn_deck(store, settings, Arc::new(runner.clone()));
    let (t, mock) = MockTransport::new();
    tx.send(DeckMsg::DeviceUp(Box::new(t), "/dev/mock".into()))
        .await
        .expect("send");
    settle().await;
    Rig {
        _dir: dir,
        tx,
        events,
        mock,
        runner,
    }
}

async fn rig() -> Rig {
    rig_with(Settings::default()).await
}

fn report(index: u8, b2: u8, b3: u8) -> Vec<u8> {
    let mut v = vec![0x7c, 0x7c, 0x01, 0x01, 0, 0, 0, 0, 0, index, b2, b3];
    v.resize(1024, 0);
    v
}

impl Rig {
    async fn press(&self, index: u8) {
        self.mock
            .inject
            .send(Ok(report(index, 0, 1)))
            .await
            .expect("inject");
        settle().await;
    }
    async fn release(&self, index: u8) {
        self.mock
            .inject
            .send(Ok(report(index, 0, 0)))
            .await
            .expect("inject");
        settle().await;
    }
    async fn click(&self, index: u8) {
        self.press(index).await;
        self.release(index).await;
    }
    fn pages_seen(&mut self) -> Vec<usize> {
        let mut out = Vec::new();
        while let Ok(e) = self.events.try_recv() {
            if let DeckEvent::Page { page, .. } = e {
                out.push(page);
            }
        }
        out
    }
    fn brightness_values(&self) -> Vec<String> {
        self.mock
            .sent()
            .iter()
            .filter(|p| p[0] == 0x7c && u16::from_be_bytes([p[2], p[3]]) == BRIGHTNESS)
            .map(|p| {
                let n = u32::from_le_bytes([p[4], p[5], p[6], p[7]]) as usize;
                String::from_utf8_lossy(&p[8..8 + n]).into_owned()
            })
            .collect()
    }
}

#[tokio::test(start_paused = true)]
async fn connect_sends_brightness_label_window_page() {
    let r = rig().await;
    assert_eq!(r.mock.commands(), vec![BRIGHTNESS, LABEL, WINDOW, BUTTONS]);
}

#[tokio::test(start_paused = true)]
async fn next_page_wraps_and_repushes() {
    let mut r = rig().await;
    r.pages_seen();
    r.mock.clear();
    r.click(16).await; // small right = next page
    assert_eq!(r.mock.commands(), vec![BUTTONS]);
    r.click(16).await; // folder page is skipped, so it wraps to 0
    assert_eq!(r.pages_seen(), vec![1, 0]);
}

#[tokio::test(start_paused = true)]
async fn small_buttons_default_to_prev_next() {
    let mut r = rig().await;
    r.pages_seen();
    r.click(15).await;
    assert_eq!(r.pages_seen(), vec![1]);
}

#[tokio::test(start_paused = true)]
async fn folder_open_and_back() {
    let mut r = rig().await;
    r.pages_seen();
    r.click(0).await;
    r.click(4).await;
    assert_eq!(r.pages_seen(), vec![2, 0]);
    assert!(r.runner.ran().is_empty(), "navigation never reaches the runner");
}

#[tokio::test(start_paused = true)]
async fn tap_vs_hold() {
    let r = rig().await;
    r.press(1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    r.release(1).await;
    assert_eq!(r.runner.ran(), vec![cmd("tap")]);
    r.press(1).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        r.runner.ran(),
        vec![cmd("tap"), cmd("hold")],
        "hold fires while still held"
    );
    r.release(1).await;
    assert_eq!(r.runner.ran(), vec![cmd("tap"), cmd("hold")]);
}

#[tokio::test(start_paused = true)]
async fn no_hold_action_means_tap_on_long_press() {
    let r = rig().await;
    r.press(2).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    r.release(2).await;
    assert_eq!(r.runner.ran(), vec![cmd("only-tap")]);
}

#[tokio::test(start_paused = true)]
async fn wake_press_only_wakes() {
    let r = rig_with(Settings {
        sleep_after_min: 1,
        ..Settings::default()
    })
    .await;
    tokio::time::sleep(Duration::from_secs(63)).await;
    assert_eq!(r.brightness_values().last().map(String::as_str), Some("0"));
    r.click(2).await;
    assert_eq!(r.brightness_values().last().map(String::as_str), Some("100"));
    assert!(
        r.runner.ran().is_empty(),
        "the waking press must not run the action"
    );
    r.click(2).await;
    assert_eq!(r.runner.ran(), vec![cmd("only-tap")]);
}

#[tokio::test(start_paused = true)]
async fn sleep_after_timeout() {
    let r = rig().await;
    tokio::time::sleep(Duration::from_secs(9 * 60)).await;
    assert_eq!(r.brightness_values(), vec!["100"]);
    tokio::time::sleep(Duration::from_secs(63)).await;
    assert_eq!(r.brightness_values(), vec!["100", "0"]);
}

#[tokio::test(start_paused = true)]
async fn keepalive_when_never_sleep() {
    let r = rig_with(Settings {
        sleep_after_min: 0,
        ..Settings::default()
    })
    .await;
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(r.brightness_values().len() >= 2, "{:?}", r.brightness_values());
}

#[tokio::test(start_paused = true)]
async fn unchanged_page_not_resent() {
    let r = rig().await;
    r.mock.clear();
    r.tx.send(DeckMsg::Dynamic(DynamicState::default()))
        .await
        .expect("send");
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!r.mock.commands().contains(&BUTTONS), "{:?}", r.mock.commands());
}

#[tokio::test(start_paused = true)]
async fn unplug_mid_upload_repushes_on_reconnect() {
    let mut r = rig().await;
    while r.events.try_recv().is_ok() {}
    let already = r.mock.sent().len();
    r.mock.fail_after.store(already + 1, Ordering::SeqCst);
    r.click(0).await; // opens the folder page, whose upload spans several packets and fails part-way
    let mut saw_down = false;
    while let Ok(e) = r.events.try_recv() {
        saw_down |= matches!(e, DeckEvent::State(DeviceState::NotFound));
    }
    assert!(saw_down);
    let (t2, mock2) = MockTransport::new();
    r.tx.send(DeckMsg::DeviceUp(Box::new(t2), "/dev/mock".into()))
        .await
        .expect("send");
    settle().await;
    assert_eq!(mock2.commands(), vec![BRIGHTNESS, LABEL, WINDOW, BUTTONS]);
}

#[tokio::test(start_paused = true)]
async fn dial_burst_runs_in_order() {
    let r = rig().await;
    for _ in 0..50 {
        r.mock.inject.send(Ok(report(17, 2, 3))).await.expect("inject");
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let ran = r.runner.ran();
    assert_eq!(ran.len(), 50);
    assert!(
        ran.iter()
            .all(|a| matches!(a, Action::Volume { op: VolumeOp::Up, .. }))
    );
}

#[tokio::test(start_paused = true)]
async fn action_error_emits_event_and_continues() {
    let mut r = rig().await;
    while r.events.try_recv().is_ok() {}
    r.click(3).await;
    let failed = std::iter::from_fn(|| r.events.try_recv().ok())
        .find_map(|e| match e {
            DeckEvent::ActionFailed { control, message } => Some((control, message)),
            _ => None,
        })
        .expect("ActionFailed event");
    assert_eq!(failed.0, "key:3");
    assert!(failed.1.contains("boom"));
    r.click(2).await;
    assert_eq!(r.runner.ran(), vec![cmd("fail"), cmd("only-tap")]);
}
