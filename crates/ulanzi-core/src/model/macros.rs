use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Action, LookOverride};

/// A named list of inputs that plays back in whatever app is in front.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    pub id: Uuid,
    pub name: String,
    pub trigger: Trigger,
    /// `Some(ms)`: every step waits this long, and recorded delays are kept but not used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard_delay_ms: Option<u32>,
    #[serde(default)]
    pub steps: Vec<MacroStep>,
    #[serde(default)]
    pub feedback: Feedback,
    /// The version before the last change, so an overwrite can be recovered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Box<Macro>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroStep {
    pub event: MacroEvent,
    /// Wait after this step.
    #[serde(default)]
    pub delay_ms: u32,
    /// What the key looks like during this step, with [`Feedback::Custom`].
    #[serde(default, skip_serializing_if = "LookOverride::is_empty")]
    pub look: LookOverride,
    /// Which part of a [`Trigger::Sequence`] this step belongs to.
    #[serde(default)]
    pub part: SequencePart,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MacroEvent {
    KeyDown {
        key: String,
    },
    KeyUp {
        key: String,
    },
    MouseDown {
        button: MouseButton,
    },
    MouseUp {
        button: MouseButton,
    },
    TypeText {
        text: String,
    },
    Scroll {
        dx: i32,
        dy: i32,
    },
    MoveBy {
        dx: i32,
        dy: i32,
    },
    /// Shares of the whole desktop, 0 to 1.
    MoveTo {
        x: f32,
        y: f32,
    },
    /// A pause of its own, for macros with standard delays.
    Wait {
        ms: u32,
    },
    RunMacro {
        id: Uuid,
    },
    Action {
        action: Action,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Plays once. `again` is what another press does while it plays.
    Once { again: PressAgain },
    /// Repeats while the key is down.
    WhileHeld { on_release: Release },
    /// Repeats until the key is pressed again.
    Toggle,
    /// Steps marked Press run on press, While Held repeat, Release run on release.
    Sequence,
}

impl Default for Trigger {
    fn default() -> Self {
        Trigger::Once {
            again: PressAgain::Stop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressAgain {
    Stop,
    Restart,
    Ignore,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Release {
    FinishPass,
    StopNow,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequencePart {
    #[default]
    Press,
    Held,
    ReleaseKey,
}

/// What the key shows while its macro plays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Feedback {
    #[default]
    Off,
    Progress,
    Count {
        show_total: bool,
    },
    Custom,
}

impl Macro {
    pub fn new(name: &str) -> Macro {
        Macro {
            id: Uuid::new_v4(),
            name: name.to_string(),
            trigger: Trigger::default(),
            standard_delay_ms: None,
            steps: Vec::new(),
            feedback: Feedback::default(),
            previous: None,
        }
    }

    /// How long to wait after `step`.
    pub fn effective_delay(&self, step: &MacroStep) -> u32 {
        self.standard_delay_ms.unwrap_or(step.delay_ms)
    }

    /// The keys and buttons still down after the last step.
    pub fn end_state(&self) -> Vec<String> {
        let mut held: Vec<String> = Vec::new();
        for s in &self.steps {
            let (name, down) = match &s.event {
                MacroEvent::KeyDown { key } => (key.clone(), true),
                MacroEvent::KeyUp { key } => (key.clone(), false),
                MacroEvent::MouseDown { button } => (format!("{button:?}"), true),
                MacroEvent::MouseUp { button } => (format!("{button:?}"), false),
                _ => continue,
            };
            held.retain(|h| *h != name);
            if down {
                held.push(name);
            }
        }
        held
    }

    /// The actions this macro runs itself.
    pub fn actions(&self) -> impl Iterator<Item = &Action> {
        self.steps.iter().filter_map(|s| match &s.event {
            MacroEvent::Action { action } => Some(action),
            _ => None,
        })
    }

    /// The macros this one runs directly.
    pub fn runs(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.steps.iter().filter_map(|s| match s.event {
            MacroEvent::RunMacro { id } => Some(id),
            _ => None,
        })
    }

    /// Whether two macros do the same, whatever their histories.
    pub fn same_as(&self, other: &Macro) -> bool {
        Macro {
            previous: None,
            ..self.clone()
        } == Macro {
            previous: None,
            ..other.clone()
        }
    }
}

/// Every saved macro (`macros.toml`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MacroLibrary {
    #[serde(default)]
    pub macros: Vec<Macro>,
}

impl MacroLibrary {
    pub fn get(&self, id: Uuid) -> Option<&Macro> {
        self.macros.iter().find(|m| m.id == id)
    }

    /// Whether saving `m` would make a macro run itself.
    pub fn would_loop(&self, m: &Macro) -> bool {
        let others = self.closure(m.runs().filter(|id| *id != m.id));
        m.runs().any(|id| id == m.id) || others.macros.iter().any(|o| o.runs().any(|id| id == m.id))
    }

    /// `ids` and every macro they run, however deep.
    pub fn closure(&self, ids: impl IntoIterator<Item = Uuid>) -> MacroLibrary {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<Uuid> = ids.into_iter().collect();
        while let Some(id) = todo.pop() {
            if seen.insert(id)
                && let Some(m) = self.get(id)
            {
                todo.extend(m.runs());
            }
        }
        MacroLibrary {
            macros: self
                .macros
                .iter()
                .filter(|m| seen.contains(&m.id))
                .cloned()
                .collect(),
        }
    }
}
