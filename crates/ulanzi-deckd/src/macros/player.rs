//! Plays one macro per control, as a task with a stop signal. Every key and button a macro holds
//! is tracked, so a stop for any reason lets go of them.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use ulanzi_core::model::{
    Action, Macro, MacroEvent, MacroLibrary, MacroStep, MouseButton, PressAgain, Release, SequencePart,
    Trigger,
};
use ulanzi_proto::Control;
use uuid::Uuid;

use crate::actions::mouse::InputSink;
use crate::actions::{ActionError, ActionRunner};

/// A loop takes at least this long per pass, so an empty one cannot flood the desktop.
const MIN_PASS: Duration = Duration::from_millis(10);
/// How deep Run Macro goes.
const MAX_DEPTH: usize = 8;

const DELETED: &str = "This macro no longer exists";
const TOO_DEEP: &str = "Macros are nested too deeply";

/// What the deck hears about a playing macro.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    /// Step `index` of the macro's `total` is starting.
    Step {
        control: Control,
        run: u64,
        index: usize,
        total: usize,
        name: String,
    },
    Ended {
        control: Control,
        run: u64,
    },
    Failed {
        control: Control,
        message: String,
    },
    /// Time to take a finished macro's look off its key. Sent by the deck to itself.
    Clear {
        control: Control,
        run: u64,
    },
    /// A step the deck runs itself, such as changing page.
    DeckAction {
        control: Control,
        action: Action,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Held {
    Key(String),
    Button(MouseButton),
}

/// Why a run ended early.
enum Halt {
    Stopped,
    Failed(String),
}

impl From<ActionError> for Halt {
    fn from(e: ActionError) -> Self {
        Halt::Failed(e.to_string())
    }
}

struct Run {
    id: u64,
    stop: watch::Sender<bool>,
    released: watch::Sender<bool>,
    queued: Arc<AtomicU32>,
    trigger: Trigger,
    task: JoinHandle<()>,
}

struct Inner {
    sink: Arc<Mutex<dyn InputSink>>,
    runner: Arc<dyn ActionRunner>,
    library: Arc<RwLock<MacroLibrary>>,
    events: mpsc::UnboundedSender<PlayerEvent>,
    /// By control and macro: two macros on one control (on different pages) play side by side.
    runs: Mutex<HashMap<(Control, Uuid), Run>>,
    next_run: AtomicU64,
    /// Everything down, and which run pressed it.
    held: Mutex<Vec<(Held, u64)>>,
}

/// The signals one run listens to.
#[derive(Clone)]
struct Signals {
    control: Control,
    run: u64,
    stop: watch::Receiver<bool>,
    /// Also ends the run early: the key let go of a Repeat While Holding that stops at once.
    cut: Option<watch::Receiver<bool>>,
}

impl Signals {
    fn halted(&self) -> bool {
        *self.stop.borrow() || self.cut.as_ref().is_some_and(|c| *c.borrow())
    }

    async fn pause(&mut self, ms: u64) -> Result<(), Halt> {
        if self.halted() {
            return Err(Halt::Stopped);
        }
        let mut cut = self.cut.clone();
        let cut = async move {
            match &mut cut {
                Some(c) => {
                    let _ = c.wait_for(|v| *v).await;
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(ms)) => Ok(()),
            _ = self.stop.wait_for(|v| *v) => Err(Halt::Stopped),
            _ = cut => Err(Halt::Stopped),
        }
    }
}

/// Plays macros. Dropping it stops them and lets go of everything.
pub struct Player {
    inner: Arc<Inner>,
}

impl Player {
    pub fn new(
        sink: Arc<Mutex<dyn InputSink>>,
        runner: Arc<dyn ActionRunner>,
        library: Arc<RwLock<MacroLibrary>>,
        events: mpsc::UnboundedSender<PlayerEvent>,
    ) -> Player {
        Player {
            inner: Arc::new(Inner {
                sink,
                runner,
                library,
                events,
                runs: Mutex::default(),
                next_run: AtomicU64::new(1),
                held: Mutex::default(),
            }),
        }
    }

    /// The control running `m` was pressed. Takes a copy: editing the macro leaves this run alone.
    /// Returns the new run's number, or `None` when the press went to a run already playing.
    pub fn press(&self, control: Control, m: Macro) -> Option<u64> {
        let mut runs = self.inner.runs.lock().ok()?;
        let key = (control, m.id);
        let mut before = None;
        if let Some(run) = runs.get(&key)
            && !run.task.is_finished()
        {
            match run.trigger {
                Trigger::Once {
                    again: PressAgain::Stop,
                }
                | Trigger::Toggle => {
                    let _ = run.stop.send(true);
                    return None;
                }
                Trigger::Once {
                    again: PressAgain::Queue,
                } => {
                    run.queued.fetch_add(1, Ordering::SeqCst);
                    return None;
                }
                Trigger::Once {
                    again: PressAgain::Restart,
                } => {
                    let _ = run.stop.send(true);
                    before = runs.remove(&key).map(|r| r.task);
                }
                Trigger::Once {
                    again: PressAgain::Ignore,
                }
                | Trigger::WhileHeld { .. }
                | Trigger::Sequence => return None,
            }
        }
        let (stop, stop_rx) = watch::channel(false);
        let (released, released_rx) = watch::channel(false);
        let queued = Arc::new(AtomicU32::new(0));
        let trigger = m.trigger;
        let id = self.inner.next_run.fetch_add(1, Ordering::SeqCst);
        let inner = self.inner.clone();
        let count = queued.clone();
        let task = tokio::spawn(async move {
            if let Some(task) = before {
                let _ = task.await;
            }
            inner.play(control, id, m, stop_rx, released_rx, count).await;
        });
        runs.insert(
            key,
            Run {
                id,
                stop,
                released,
                queued,
                trigger,
                task,
            },
        );
        Some(id)
    }

    /// The control was let go.
    pub fn release(&self, control: Control) {
        if let Ok(runs) = self.inner.runs.lock() {
            for ((c, _), run) in runs.iter() {
                if *c == control {
                    let _ = run.released.send(true);
                }
            }
        }
    }

    /// Stop every macro and let go of everything, including keys left down on purpose.
    pub fn stop_all(&self) {
        let mut running = Vec::new();
        if let Ok(runs) = self.inner.runs.lock() {
            for run in runs.values() {
                if !run.task.is_finished() {
                    let _ = run.stop.send(true);
                    running.push(run.id);
                }
            }
        }
        self.inner.let_go(|owner| !running.contains(&owner));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Ok(runs) = self.inner.runs.lock() {
            runs.values().for_each(|run| run.task.abort());
        }
        self.inner.let_go(|_| true);
    }
}

impl Inner {
    fn input<T>(&self, f: impl FnOnce(&mut dyn InputSink) -> Result<T, ActionError>) -> Result<T, Halt> {
        let mut sink = self
            .sink
            .lock()
            .map_err(|_| Halt::Failed("the input device is unavailable".into()))?;
        Ok(f(&mut *sink)?)
    }

    fn press_down(&self, run: u64, what: Held) {
        if let Ok(mut held) = self.held.lock()
            && !held.iter().any(|(h, _)| *h == what)
        {
            held.push((what, run));
        }
    }

    fn press_up(&self, what: &Held) {
        if let Ok(mut held) = self.held.lock() {
            held.retain(|(h, _)| h != what);
        }
    }

    /// Release what `owned` selects, last pressed first.
    fn let_go(&self, owned: impl Fn(u64) -> bool) {
        let Ok(mut held) = self.held.lock() else {
            return;
        };
        let (gone, kept) = std::mem::take(&mut *held)
            .into_iter()
            .partition::<Vec<_>, _>(|(_, c)| owned(*c));
        *held = kept;
        drop(held);
        let Ok(mut sink) = self.sink.lock() else {
            return;
        };
        for (what, _) in gone.into_iter().rev() {
            let done = match what {
                Held::Key(k) => sink.key(&k, false),
                Held::Button(b) => sink.button(b, false),
            };
            if let Err(e) = done {
                tracing::warn!("could not release input: {e}");
            }
        }
    }

    async fn play(
        self: Arc<Self>,
        control: Control,
        run: u64,
        m: Macro,
        stop: watch::Receiver<bool>,
        released: watch::Receiver<bool>,
        queued: Arc<AtomicU32>,
    ) {
        let mut signals = Signals {
            control,
            run,
            stop,
            cut: None,
        };
        let outcome = self.drive(&m, &mut signals, released, queued).await;
        // A stop that arrived as the run ended still lets go of what it left down.
        if *signals.stop.borrow() || outcome.is_err() {
            self.let_go(|owner| owner == run);
        }
        if let Err(Halt::Failed(message)) = outcome {
            let _ = self.events.send(PlayerEvent::Failed { control, message });
        }
        let _ = self.events.send(PlayerEvent::Ended { control, run });
    }

    async fn drive(
        &self,
        m: &Macro,
        signals: &mut Signals,
        released: watch::Receiver<bool>,
        queued: Arc<AtomicU32>,
    ) -> Result<(), Halt> {
        let all = |_: &MacroStep| true;
        match m.trigger {
            Trigger::Once { .. } => loop {
                self.steps(m, &all, signals, 0).await?;
                if queued
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .is_err()
                {
                    return Ok(());
                }
            },
            Trigger::WhileHeld { on_release } => {
                if on_release == Release::StopNow {
                    signals.cut = Some(released.clone());
                }
                while !*released.borrow() {
                    self.pass(m, &all, signals).await?;
                }
                Ok(())
            }
            Trigger::Toggle => loop {
                self.pass(m, &all, signals).await?;
            },
            Trigger::Sequence => {
                let part = |p: SequencePart| move |s: &MacroStep| s.part == p;
                self.steps(m, &part(SequencePart::Press), signals, 0).await?;
                let repeating = m.steps.iter().any(part(SequencePart::Held));
                while repeating && !*released.borrow() {
                    self.pass(m, &part(SequencePart::Held), signals).await?;
                }
                let mut released = released;
                tokio::select! {
                    _ = released.wait_for(|v| *v) => {}
                    _ = signals.stop.wait_for(|v| *v) => return Err(Halt::Stopped),
                }
                self.steps(m, &part(SequencePart::ReleaseKey), signals, 0).await
            }
        }
    }

    /// One pass of a loop, taking at least [`MIN_PASS`].
    async fn pass(
        &self,
        m: &Macro,
        keep: &(dyn Fn(&MacroStep) -> bool + Sync),
        signals: &mut Signals,
    ) -> Result<(), Halt> {
        let start = Instant::now();
        self.steps(m, keep, signals, 0).await?;
        let left = MIN_PASS.saturating_sub(start.elapsed());
        signals.pause(left.as_millis() as u64).await
    }

    fn steps<'a>(
        &'a self,
        m: &'a Macro,
        keep: &'a (dyn Fn(&MacroStep) -> bool + Sync),
        signals: &'a mut Signals,
        depth: usize,
    ) -> Pin<Box<dyn Future<Output = Result<(), Halt>> + Send + 'a>> {
        Box::pin(async move {
            for (index, step) in m.steps.iter().enumerate().filter(|(_, s)| keep(s)) {
                if signals.halted() {
                    return Err(Halt::Stopped);
                }
                if depth == 0 {
                    let _ = self.events.send(PlayerEvent::Step {
                        control: signals.control,
                        run: signals.run,
                        index,
                        total: m.steps.len(),
                        name: m.name.clone(),
                    });
                }
                self.step(&step.event, signals, depth).await?;
                if !matches!(step.event, MacroEvent::Wait { .. }) {
                    let delay = m.effective_delay(step);
                    if delay > 0 {
                        signals.pause(delay as u64).await?;
                    }
                }
            }
            Ok(())
        })
    }

    async fn step(&self, event: &MacroEvent, signals: &mut Signals, depth: usize) -> Result<(), Halt> {
        let (control, run) = (signals.control, signals.run);
        match event {
            MacroEvent::KeyDown { key } => {
                self.input(|s| s.key(key, true))?;
                self.press_down(run, Held::Key(key.clone()));
            }
            MacroEvent::KeyUp { key } => {
                self.input(|s| s.key(key, false))?;
                self.press_up(&Held::Key(key.clone()));
            }
            MacroEvent::MouseDown { button } => {
                self.input(|s| s.button(*button, true))?;
                self.press_down(run, Held::Button(*button));
            }
            MacroEvent::MouseUp { button } => {
                self.input(|s| s.button(*button, false))?;
                self.press_up(&Held::Button(*button));
            }
            MacroEvent::Scroll { dx, dy } => self.input(|s| s.scroll(*dx, *dy))?,
            MacroEvent::MoveBy { dx, dy } => self.input(|s| s.move_by(*dx, *dy))?,
            MacroEvent::MoveTo { x, y } => self.input(|s| s.move_to(*x, *y))?,
            MacroEvent::Wait { ms } => signals.pause(*ms as u64).await?,
            MacroEvent::TypeText { text } => {
                let action = Action::TypeText { text: text.clone() };
                self.runner.run(&action).await?;
            }
            MacroEvent::RunMacro { id } => self.nested(*id, None, signals, depth).await?,
            MacroEvent::Action { action } => match action {
                Action::Macro { id, own } => {
                    self.nested(*id, own.as_deref(), signals, depth).await?;
                }
                a if deck_runs(a) => {
                    let _ = self.events.send(PlayerEvent::DeckAction {
                        control,
                        action: a.clone(),
                    });
                }
                a => self.runner.run(a).await?,
            },
        }
        Ok(())
    }

    async fn nested(
        &self,
        id: uuid::Uuid,
        own: Option<&Macro>,
        signals: &mut Signals,
        depth: usize,
    ) -> Result<(), Halt> {
        if depth >= MAX_DEPTH {
            return Err(Halt::Failed(TOO_DEEP.into()));
        }
        let m = match own {
            Some(m) => m.clone(),
            None => self
                .library
                .read()
                .ok()
                .and_then(|l| l.get(id).cloned())
                .ok_or_else(|| Halt::Failed(DELETED.into()))?,
        };
        self.steps(&m, &|_: &MacroStep| true, signals, depth + 1).await
    }
}

/// Actions the deck itself carries out.
fn deck_runs(a: &Action) -> bool {
    matches!(
        a,
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
            | Action::Timer { .. }
            | Action::StopMacros
    )
}
