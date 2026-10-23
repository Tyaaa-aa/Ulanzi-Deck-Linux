//! Everything the window shows, owned on the GTK thread. Widgets read it and subscribe to changes;
//! edits go through [`AppState::edit`] so every change is saved (debounced) and redrawn.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ulanzi_core::edit::{Editor, Selection, Slot};
use ulanzi_core::model::{Model, Profile, Settings};
use uuid::Uuid;

use crate::client::Client;

/// What changed, so listeners redraw only their part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The profile's content (keys, pages, ...).
    Profile,
    /// The current page index.
    Page,
    Selection,
    /// Device or service state.
    Status,
    /// The list of profiles.
    Profiles,
    Settings,
    /// A physical control was pressed.
    Pressed(Slot),
    /// A live preview of an appearance change on the selected keys started or ended.
    Preview,
    /// Dragging an action from the library started or ended.
    Drag,
    /// The OBS connection changed.
    Obs,
}

/// What the app knows about the OBS connection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObsInfo {
    /// `connected`, `not-running`, … (see the service's ObsStatus).
    pub state: String,
    pub title: String,
    pub detail: String,
    /// Scenes in OBS, while connected.
    pub scenes: Vec<String>,
}

impl ObsInfo {
    pub fn connected(&self) -> bool {
        self.state == "connected"
    }
}

/// Device / service status shown in the header pill and banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Connecting,
    ServiceDown,
    Device {
        state: String,
        detail: String,
        model: String,
    },
}

type Listener = Box<dyn Fn(Change)>;

pub struct AppState {
    pub editor: RefCell<Editor>,
    pub selection: RefCell<Selection>,
    pub settings: RefCell<Settings>,
    pub profiles: RefCell<Vec<(Uuid, String)>>,
    pub status: RefCell<Status>,
    pub client: RefCell<Option<Client>>,
    /// True while showing built-in demo data (no service).
    pub demo: Cell<bool>,
    /// True while the inspector applies its own edit (it then refreshes itself without a rebuild).
    pub inspector_edit: Cell<bool>,
    /// Each profile's auto-switch apps (for the sidebar), by profile.
    pub rules: RefCell<std::collections::HashMap<Uuid, ulanzi_core::model::MatchRule>>,
    /// An action from the library is being dragged (the dials show their drop zones).
    pub dragging_action: Cell<bool>,
    /// An appearance change shown on the selected keys before it is applied (hovering a colour).
    pub preview: RefCell<Option<ulanzi_core::edit::Field>>,
    /// Expandable rows the user opened, by path, so rebuilding the inspector keeps them open.
    pub expanded: RefCell<std::collections::HashSet<String>>,
    pub obs: RefCell<ObsInfo>,
    /// What the service sees playing (for the Now Playing preview); `None` when nothing plays.
    pub now_playing: RefCell<Option<ulanzi_core::render::screen::MediaData>>,
    listeners: RefCell<Vec<Listener>>,
}

pub type State = Rc<AppState>;

impl AppState {
    /// Keep `row` open or closed across rebuilds under the name `path`.
    pub fn remember_expanded(self: &Rc<Self>, row: &adw::ExpanderRow, path: &str) {
        use adw::prelude::*;
        row.set_expanded(self.expanded.borrow().contains(path));
        let (state, path) = (Rc::downgrade(self), path.to_string());
        row.connect_expanded_notify(move |r| {
            if let Some(s) = state.upgrade() {
                if r.is_expanded() {
                    s.expanded.borrow_mut().insert(path.clone());
                } else {
                    s.expanded.borrow_mut().remove(&path);
                }
            }
        });
    }

    pub fn new(profile: Profile) -> State {
        Rc::new(AppState {
            editor: RefCell::new(Editor::new(profile)),
            selection: RefCell::new(Selection::default()),
            settings: RefCell::new(Settings::default()),
            profiles: RefCell::new(Vec::new()),
            status: RefCell::new(Status::Connecting),
            client: RefCell::new(None),
            demo: Cell::new(false),
            inspector_edit: Cell::new(false),
            preview: RefCell::new(None),
            dragging_action: Cell::new(false),
            rules: RefCell::new(std::collections::HashMap::new()),
            expanded: RefCell::new(std::collections::HashSet::new()),
            obs: RefCell::default(),
            now_playing: RefCell::default(),
            listeners: RefCell::new(Vec::new()),
        })
    }

    /// The main window, for dialogs opened from places without a widget at hand.
    pub fn toast_root(&self) -> Option<gtk::Widget> {
        use adw::prelude::*;
        gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|w| w.downcast::<adw::ApplicationWindow>().ok())
            .map(|w| w.upcast())
    }

    pub fn on(&self, f: impl Fn(Change) + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    pub fn notify(&self, c: Change) {
        // Listeners may read state but must not register new listeners while notified.
        let listeners = self.listeners.borrow();
        for l in listeners.iter() {
            l(c);
        }
    }

    pub fn profile_id(&self) -> Uuid {
        self.editor.borrow().profile().id
    }

    pub fn model(&self) -> Model {
        self.editor.borrow().profile().model
    }

    /// Apply an edit, save it (debounced) and redraw.
    pub fn edit(&self, f: impl FnOnce(&mut Editor)) {
        {
            let mut e = self.editor.borrow_mut();
            e.set_merge(None);
            f(&mut e);
        }
        self.after_edit();
    }

    /// Like [`AppState::edit`], but consecutive edits with the same `key` share one undo step.
    pub fn edit_merged(&self, key: &'static str, f: impl FnOnce(&mut Editor)) {
        {
            let mut e = self.editor.borrow_mut();
            e.set_merge(Some(key));
            f(&mut e);
        }
        self.after_edit();
    }

    fn after_edit(&self) {
        let profile = {
            let mut e = self.editor.borrow_mut();
            if !e.is_dirty() {
                return;
            }
            e.mark_clean();
            e.profile().clone()
        };
        if let Some(c) = self.client.borrow().as_ref() {
            c.apply(profile);
        }
        self.notify(Change::Profile);
    }

    pub fn undo(&self) {
        if self.editor.borrow_mut().undo() {
            self.after_edit();
        }
    }

    pub fn redo(&self) {
        if self.editor.borrow_mut().redo() {
            self.after_edit();
        }
    }

    pub fn set_page(&self, page: usize) {
        self.editor.borrow_mut().set_page(page);
        self.selection.borrow_mut().clear();
        self.notify(Change::Page);
        self.notify(Change::Selection);
    }

    pub fn select(&self, f: impl FnOnce(&mut Selection)) {
        f(&mut self.selection.borrow_mut());
        self.notify(Change::Selection);
    }

    pub fn load_profile(&self, profile: Profile) {
        let page = self.editor.borrow().page();
        let mut editor = Editor::new(profile);
        editor.set_page(page);
        *self.editor.borrow_mut() = editor;
        self.selection.borrow_mut().clear();
        self.notify(Change::Profile);
        self.notify(Change::Page);
        self.notify(Change::Selection);
    }
}

/// Header pill text for a status.
pub fn status_text(s: &Status) -> (String, String) {
    match s {
        Status::Connecting => ("Connecting".into(), String::new()),
        Status::ServiceDown => ("Service stopped".into(), String::new()),
        Status::Device { state, model, .. } => {
            let what = match state.as_str() {
                "ready" => "Connected",
                "not_found" => "Not connected",
                "no_permission" => "Needs permission",
                "busy" => "In use",
                _ => "Problem",
            };
            (
                if model.is_empty() {
                    "Deck".into()
                } else {
                    model.clone()
                },
                what.into(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_texts() {
        let ready = Status::Device {
            state: "ready".into(),
            detail: String::new(),
            model: "D200X".into(),
        };
        assert_eq!(
            status_text(&ready),
            ("D200X".to_string(), "Connected".to_string())
        );
        let perm = Status::Device {
            state: "no_permission".into(),
            detail: String::new(),
            model: String::new(),
        };
        assert_eq!(status_text(&perm).1, "Needs permission");
        assert_eq!(status_text(&Status::ServiceDown).0, "Service stopped");
    }
}
