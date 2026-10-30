//! The macro player against a fake input device.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{advance, sleep};
use ulanzi_core::model::*;
use ulanzi_deckd::actions::mouse::InputSink;
use ulanzi_deckd::actions::{ActionError, ActionRunner};
use ulanzi_deckd::macros::{Player, PlayerEvent};
use ulanzi_proto::Control;

#[derive(Default)]
struct Sink {
    log: Vec<String>,
    held: Vec<String>,
    fail_on: Option<String>,
}

impl Sink {
    fn set(&mut self, name: String, down: bool) -> Result<(), ActionError> {
        if self.fail_on.as_deref() == Some(name.as_str()) {
            return Err(ActionError::Failed("boom".into()));
        }
        self.log.push(format!("{}{name}", if down { "+" } else { "-" }));
        self.held.retain(|h| *h != name);
        if down {
            self.held.push(name);
        }
        Ok(())
    }
}

impl InputSink for Sink {
    fn key(&mut self, key: &str, down: bool) -> Result<(), ActionError> {
        self.set(key.to_string(), down)
    }
    fn button(&mut self, b: MouseButton, down: bool) -> Result<(), ActionError> {
        self.set(format!("{b:?}"), down)
    }
    fn scroll(&mut self, dx: i32, dy: i32) -> Result<(), ActionError> {
        self.log.push(format!("scroll {dx} {dy}"));
        Ok(())
    }
    fn move_by(&mut self, dx: i32, dy: i32) -> Result<(), ActionError> {
        self.log.push(format!("by {dx} {dy}"));
        Ok(())
    }
    fn move_to(&mut self, x: f32, y: f32) -> Result<(), ActionError> {
        self.log.push(format!("to {x} {y}"));
        Ok(())
    }
}

#[derive(Default)]
struct Runner(Mutex<Vec<Action>>);

#[async_trait::async_trait]
impl ActionRunner for Runner {
    async fn run(&self, action: &Action) -> Result<(), ActionError> {
        self.0.lock().expect("runner").push(action.clone());
        Ok(())
    }
}

struct Rig {
    player: Player,
    sink: Arc<Mutex<Sink>>,
    runner: Arc<Runner>,
    library: Arc<RwLock<MacroLibrary>>,
    events: mpsc::UnboundedReceiver<PlayerEvent>,
}

fn rig() -> Rig {
    let sink = Arc::new(Mutex::new(Sink::default()));
    let runner = Arc::new(Runner::default());
    let library = Arc::new(RwLock::new(MacroLibrary::default()));
    let (tx, events) = mpsc::unbounded_channel();
    let player = Player::new(sink.clone(), runner.clone(), library.clone(), tx);
    Rig {
        player,
        sink,
        runner,
        library,
        events,
    }
}

impl Rig {
    fn log(&self) -> Vec<String> {
        self.sink.lock().expect("sink").log.clone()
    }
    fn held(&self) -> Vec<String> {
        self.sink.lock().expect("sink").held.clone()
    }
    fn clear(&self) {
        self.sink.lock().expect("sink").log.clear();
    }
}

fn step(event: MacroEvent, delay_ms: u32) -> MacroStep {
    MacroStep {
        event,
        delay_ms,
        look: LookOverride::default(),
        part: SequencePart::Press,
    }
}

fn down(k: &str, ms: u32) -> MacroStep {
    step(MacroEvent::KeyDown { key: k.into() }, ms)
}

fn up(k: &str, ms: u32) -> MacroStep {
    step(MacroEvent::KeyUp { key: k.into() }, ms)
}

fn in_part(mut s: MacroStep, part: SequencePart) -> MacroStep {
    s.part = part;
    s
}

fn macro_of(trigger: Trigger, steps: Vec<MacroStep>) -> Macro {
    let mut m = Macro::new("M");
    m.trigger = trigger;
    m.steps = steps;
    m
}

fn once(again: PressAgain, steps: Vec<MacroStep>) -> Macro {
    macro_of(Trigger::Once { again }, steps)
}

const KEY: Control = Control::Key(0);

async fn wait(ms: u64) {
    sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(start_paused = true)]
async fn steps_run_in_order_with_their_delays() {
    let r = rig();
    let m = once(
        PressAgain::Stop,
        vec![down("a", 100), up("a", 0), down("b", 0), up("b", 0)],
    );
    r.player.press(KEY, m);
    wait(50).await;
    assert_eq!(r.log(), ["+a"]);
    wait(100).await;
    assert_eq!(r.log(), ["+a", "-a", "+b", "-b"]);
}

#[tokio::test(start_paused = true)]
async fn standard_delays_are_used_when_set() {
    let r = rig();
    let mut m = once(PressAgain::Stop, vec![down("a", 900), up("a", 0)]);
    m.standard_delay_ms = Some(50);
    r.player.press(KEY, m);
    wait(60).await;
    assert_eq!(r.log(), ["+a", "-a"]);
}

#[tokio::test(start_paused = true)]
async fn every_kind_of_step_reaches_its_device() {
    let r = rig();
    let m = once(
        PressAgain::Stop,
        vec![
            step(
                MacroEvent::MouseDown {
                    button: MouseButton::Left,
                },
                0,
            ),
            step(
                MacroEvent::MouseUp {
                    button: MouseButton::Left,
                },
                0,
            ),
            step(MacroEvent::Scroll { dx: 1, dy: -2 }, 0),
            step(MacroEvent::MoveBy { dx: 3, dy: 4 }, 0),
            step(MacroEvent::MoveTo { x: 0.5, y: 0.25 }, 0),
            step(MacroEvent::Wait { ms: 10 }, 0),
            step(MacroEvent::TypeText { text: "hi".into() }, 0),
            step(
                MacroEvent::Action {
                    action: Action::Timer { op: TimerOp::Reset },
                },
                0,
            ),
        ],
    );
    r.player.press(KEY, m);
    wait(50).await;
    assert_eq!(
        r.log(),
        ["+Left", "-Left", "scroll 1 -2", "by 3 4", "to 0.5 0.25"]
    );
    let ran = r.runner.0.lock().expect("runner").clone();
    assert_eq!(ran[0], Action::TypeText { text: "hi".into() });
}

#[tokio::test(start_paused = true)]
async fn pressing_again_can_stop_it() {
    let r = rig();
    let m = once(
        PressAgain::Stop,
        vec![down("a", 100), up("a", 100), down("b", 0), up("b", 0)],
    );
    r.player.press(KEY, m.clone());
    wait(50).await;
    r.player.press(KEY, m);
    wait(300).await;
    assert_eq!(r.log(), ["+a", "-a"], "released, and b never ran");
}

#[tokio::test(start_paused = true)]
async fn pressing_again_can_restart_it() {
    let r = rig();
    let m = once(PressAgain::Restart, vec![down("a", 100), up("a", 0)]);
    r.player.press(KEY, m.clone());
    wait(50).await;
    r.player.press(KEY, m);
    wait(300).await;
    assert_eq!(r.log(), ["+a", "-a", "+a", "-a"]);
}

#[tokio::test(start_paused = true)]
async fn pressing_again_can_be_ignored() {
    let r = rig();
    let m = once(PressAgain::Ignore, vec![down("a", 100), up("a", 0)]);
    r.player.press(KEY, m.clone());
    wait(50).await;
    r.player.press(KEY, m);
    wait(300).await;
    assert_eq!(r.log(), ["+a", "-a"]);
}

#[tokio::test(start_paused = true)]
async fn pressing_again_can_queue_another_run() {
    let r = rig();
    let m = once(PressAgain::Queue, vec![down("a", 100), up("a", 0)]);
    r.player.press(KEY, m.clone());
    wait(50).await;
    r.player.press(KEY, m);
    wait(300).await;
    assert_eq!(r.log(), ["+a", "-a", "+a", "-a"]);
}

#[tokio::test(start_paused = true)]
async fn while_held_repeats_until_release_and_finishes_the_pass() {
    let r = rig();
    let m = macro_of(
        Trigger::WhileHeld {
            on_release: Release::FinishPass,
        },
        vec![down("a", 100), up("a", 100)],
    );
    r.player.press(KEY, m);
    wait(250).await;
    r.player.release(KEY);
    wait(500).await;
    assert_eq!(r.log(), ["+a", "-a", "+a", "-a"]);
}

#[tokio::test(start_paused = true)]
async fn while_held_can_stop_at_once() {
    let r = rig();
    let m = macro_of(
        Trigger::WhileHeld {
            on_release: Release::StopNow,
        },
        vec![down("a", 100), up("a", 100)],
    );
    r.player.press(KEY, m);
    wait(50).await;
    r.player.release(KEY);
    wait(500).await;
    assert_eq!(r.log(), ["+a", "-a"]);
}

#[tokio::test(start_paused = true)]
async fn toggle_repeats_until_pressed_again() {
    let r = rig();
    let m = macro_of(Trigger::Toggle, vec![down("a", 50), up("a", 50)]);
    r.player.press(KEY, m.clone());
    wait(250).await;
    r.player.press(KEY, m);
    wait(10).await;
    let passes = r.log().iter().filter(|l| *l == "+a").count();
    assert_eq!(passes, 3);
    r.clear();
    wait(300).await;
    assert!(r.log().is_empty(), "{:?}", r.log());
    assert!(r.held().is_empty());
}

#[tokio::test(start_paused = true)]
async fn sequence_runs_its_three_parts() {
    let r = rig();
    let m = macro_of(
        Trigger::Sequence,
        vec![
            in_part(down("p", 0), SequencePart::Press),
            in_part(up("p", 0), SequencePart::Press),
            in_part(down("h", 50), SequencePart::Held),
            in_part(up("h", 50), SequencePart::Held),
            in_part(down("r", 0), SequencePart::ReleaseKey),
            in_part(up("r", 0), SequencePart::ReleaseKey),
        ],
    );
    r.player.press(KEY, m);
    wait(150).await;
    r.player.release(KEY);
    wait(200).await;
    assert_eq!(r.log(), ["+p", "-p", "+h", "-h", "+h", "-h", "+r", "-r"]);
}

#[tokio::test(start_paused = true)]
async fn stopping_releases_everything_at_every_step() {
    let steps = || {
        vec![
            down("ctrl", 10),
            step(
                MacroEvent::MouseDown {
                    button: MouseButton::Left,
                },
                10,
            ),
            down("a", 10),
            up("a", 10),
            step(
                MacroEvent::MouseUp {
                    button: MouseButton::Left,
                },
                10,
            ),
            up("ctrl", 10),
        ]
    };
    for at in [1, 11, 21, 31, 41, 51] {
        let r = rig();
        r.player.press(KEY, once(PressAgain::Stop, steps()));
        wait(at).await;
        r.player.stop_all();
        wait(20).await;
        assert!(r.held().is_empty(), "after {at} ms: {:?}", r.held());
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_step_stops_and_releases() {
    let mut r = rig();
    r.sink.lock().expect("sink").fail_on = Some("b".into());
    r.player.press(
        KEY,
        once(PressAgain::Stop, vec![down("a", 0), down("b", 0), up("a", 0)]),
    );
    wait(10).await;
    assert!(r.held().is_empty());
    let mut failed = None;
    while let Ok(e) = r.events.try_recv() {
        if let PlayerEvent::Failed { message, .. } = e {
            failed = Some(message);
        }
    }
    assert_eq!(failed.as_deref(), Some("boom"));
}

#[tokio::test(start_paused = true)]
async fn a_key_left_down_on_purpose_stays_down_until_stop_all() {
    let r = rig();
    r.player.press(KEY, once(PressAgain::Stop, vec![down("ctrl", 0)]));
    wait(10).await;
    assert_eq!(r.held(), ["ctrl"]);
    r.player.stop_all();
    wait(10).await;
    assert!(r.held().is_empty());
}

#[tokio::test(start_paused = true)]
async fn dropping_the_player_releases_everything() {
    let r = rig();
    r.player.press(
        Control::Key(1),
        once(PressAgain::Stop, vec![down("ctrl", 1000), up("ctrl", 0)]),
    );
    wait(10).await;
    let Rig { player, sink, .. } = r;
    drop(player);
    wait(10).await;
    assert!(sink.lock().expect("sink").held.is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_empty_loop_waits_ten_ms_a_pass() {
    let r = rig();
    let m = macro_of(Trigger::Toggle, vec![down("a", 0), up("a", 0)]);
    r.player.press(KEY, m.clone());
    wait(105).await;
    r.player.press(KEY, m);
    wait(1).await;
    let passes = r.log().iter().filter(|l| *l == "+a").count();
    assert!((10..=11).contains(&passes), "{passes}");
}

fn chain(r: &Rig, length: usize) -> Macro {
    let mut next = None;
    let mut library = Vec::new();
    for i in 0..length {
        let mut m = Macro::new(&format!("level {i}"));
        m.steps = match next {
            Some(id) => vec![step(MacroEvent::RunMacro { id }, 0)],
            None => vec![down("bottom", 0), up("bottom", 0)],
        };
        next = Some(m.id);
        library.push(m);
    }
    let top = library.last().cloned().expect("top");
    *r.library.write().expect("library") = MacroLibrary { macros: library };
    top
}

#[tokio::test(start_paused = true)]
async fn macros_nest_eight_levels_and_no_further() {
    let r = rig();
    let top = chain(&r, 9);
    r.player.press(KEY, top);
    wait(10).await;
    assert_eq!(r.log(), ["+bottom", "-bottom"]);
    let mut r = rig();
    let top = chain(&r, 10);
    r.player.press(KEY, top);
    wait(10).await;
    assert!(r.log().is_empty());
    let mut failed = false;
    while let Ok(e) = r.events.try_recv() {
        failed |= matches!(e, PlayerEvent::Failed { .. });
    }
    assert!(failed);
}

#[tokio::test(start_paused = true)]
async fn a_deleted_macro_in_a_step_stops_the_run() {
    let mut r = rig();
    let m = once(
        PressAgain::Stop,
        vec![
            step(
                MacroEvent::RunMacro {
                    id: uuid::Uuid::new_v4(),
                },
                0,
            ),
            down("never", 0),
        ],
    );
    r.player.press(KEY, m);
    wait(10).await;
    assert!(r.log().is_empty());
    let mut failed = None;
    while let Ok(e) = r.events.try_recv() {
        if let PlayerEvent::Failed { message, .. } = e {
            failed = Some(message);
        }
    }
    assert_eq!(failed.as_deref(), Some("This macro no longer exists"));
}

#[tokio::test(start_paused = true)]
async fn editing_a_playing_macro_leaves_the_running_copy_alone() {
    let r = rig();
    let mut m = once(PressAgain::Ignore, vec![down("a", 100), up("a", 0)]);
    r.player.press(KEY, m.clone());
    m.steps = vec![down("changed", 0), up("changed", 0)];
    wait(200).await;
    assert_eq!(r.log(), ["+a", "-a"]);
    r.player.press(KEY, m);
    wait(10).await;
    assert_eq!(r.log(), ["+a", "-a", "+changed", "-changed"]);
}

#[tokio::test(start_paused = true)]
async fn each_step_reports_its_number() {
    let mut r = rig();
    r.player
        .press(KEY, once(PressAgain::Stop, vec![down("a", 0), up("a", 0)]));
    wait(10).await;
    let mut seen = Vec::new();
    while let Ok(e) = r.events.try_recv() {
        match e {
            PlayerEvent::Step { index, total, .. } => seen.push((index, total)),
            PlayerEvent::Ended { .. } => seen.push((99, 99)),
            _ => {}
        }
    }
    assert_eq!(seen, [(0, 2), (1, 2), (99, 99)]);
}

#[tokio::test(start_paused = true)]
async fn deck_actions_go_back_to_the_deck() {
    let mut r = rig();
    let m = once(
        PressAgain::Stop,
        vec![step(
            MacroEvent::Action {
                action: Action::NextPage,
            },
            0,
        )],
    );
    r.player.press(KEY, m);
    wait(10).await;
    let mut asked = false;
    while let Ok(e) = r.events.try_recv() {
        asked |= matches!(
            e,
            PlayerEvent::DeckAction {
                action: Action::NextPage,
                ..
            }
        );
    }
    assert!(asked);
    assert!(r.runner.0.lock().expect("runner").is_empty());
    advance(Duration::from_millis(1)).await;
}
