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
    listeners: RefCell<Vec<Listener>>,
}

pub type State = Rc<AppState>;

impl AppState {
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
            listeners: RefCell::new(Vec::new()),
        })
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
