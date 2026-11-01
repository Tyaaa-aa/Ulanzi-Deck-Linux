//! CPU use of the deck loop. It reads the whole process's time, so it lives alone in this file:
//! test files run one at a time, and any test beside it would be counted too.

use std::sync::Arc;
use std::time::Duration;

use ulanzi_core::model::*;
use ulanzi_core::store::Store;
use ulanzi_deckd::actions::{ActionError, ActionRunner};
use ulanzi_deckd::deck::{DeckMsg, DynamicState, spawn_deck};
use ulanzi_deckd::transport::{MockHandle, MockTransport};

const INFO: u16 = 0x0003;
const UNLOCK: u16 = 0x0005;

struct NoRunner;

#[async_trait::async_trait]
impl ActionRunner for NoRunner {
    async fn run(&self, _action: &Action) -> Result<(), ActionError> {
        Ok(())
    }
}

fn info_frame() -> Vec<u8> {
    let json = br#"{ "SerialNumber": "X", "Dversion": "5.2.3", "DeviceType": "D200" }"#;
    let mut v = vec![0x7c, 0x7c, 0x03, 0x03];
    v.extend_from_slice(&(json.len() as u32).to_le_bytes());
    v.extend_from_slice(json);
    v.resize(1024, 0);
    v
}

/// Answer like the real deck: an info frame for every info request and unlock.
fn answer_info_requests(mock: MockHandle) {
    tokio::spawn(async move {
        let mut answered = 0;
        loop {
            let asked = mock
                .commands()
                .iter()
                .filter(|c| [INFO, UNLOCK].contains(c))
                .count();
            while answered < asked {
                answered += 1;
                if mock.inject.send(Ok(info_frame())).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
}

fn cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("stat");
    let after = &stat[stat.rfind(')').expect("comm") + 2..];
    let f: Vec<&str> = after.split_whitespace().collect();
    f[11].parse::<u64>().expect("utime") + f[12].parse::<u64>().expect("stime")
}

#[tokio::test(flavor = "multi_thread")]
async fn no_busy_loop_when_the_deck_goes_away_with_an_upload_pending() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = Store::at(dir.path().join("c"), dir.path().join("s"), dir.path().join("k"));
    let mut p = Profile::new_default("Default", Model::D200X);
    let tap = Some(Action::Media {
        control: MediaControl::PlayPause,
        player: None,
    });
    p.pages[0].keys.insert(
        5,
        Key {
            tap,
            ..Key::default()
        },
    );
    store.save_profile(&p).expect("save");
    let settings = Settings {
        default_profile: Some(p.id),
        key_effects: false,
        ..Settings::default()
    };
    let (tx, _events) = spawn_deck(store, settings, Arc::new(NoRunner));
    let (t, mock) = MockTransport::new();
    answer_info_requests(mock.clone());
    tx.send(DeckMsg::DeviceUp(Box::new(t), "/dev/mock".into()))
        .await
        .expect("up");
    tokio::time::sleep(Duration::from_millis(200)).await;
    // A live change inside the upload cooldown leaves an upload pending...
    let playing = DynamicState {
        playing: Some(true),
        ..DynamicState::default()
    };
    tx.send(DeckMsg::Dynamic(Box::new(playing)))
        .await
        .expect("dynamic");
    tokio::time::sleep(Duration::from_millis(100)).await;
    // ...then the deck disappears.
    mock.inject
        .send(Err(std::io::Error::from_raw_os_error(19)))
        .await
        .expect("unplug");
    tokio::time::sleep(Duration::from_millis(3000)).await;
    let before = cpu_ticks();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let used = cpu_ticks() - before;
    assert!(
        used < 30,
        "the deck loop used {used} ticks of CPU in 1 s while unplugged"
    );
}
