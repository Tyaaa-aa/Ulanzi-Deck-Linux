//! Taking the keyboard for a recorder: the app's own shortcuts step aside and the desktop is asked
//! to let its shortcuts through, for as long as a recorder listens. Another app taking focus ends it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

/// What a recorder is holding, given back by [`Capture::end`] or on drop.
#[derive(Default)]
pub struct Capture {
    /// The toplevel whose system shortcuts are inhibited, restored when recording ends.
    inhibited: RefCell<Option<glib::WeakRef<gdk::Toplevel>>>,
    /// The app's accelerators, off while recording so any combination can be recorded.
    accels: RefCell<Option<crate::ui::key_routing::AccelGuard>>,
    inhibit: Cell<InhibitState>,
    /// Handlers on the window and its surface while recording; disconnected when it ends.
    handlers: RefCell<Vec<(glib::WeakRef<glib::Object>, glib::SignalHandlerId)>>,
}

impl Capture {
    /// Start capturing for `w`. `on_lost` runs when another application takes focus (unless it is
    /// the desktop's prompt for the shortcut inhibit).
    pub fn begin(self: &Rc<Self>, w: &gtk::Widget, on_lost: impl Fn() + 'static) {
        if let Some(win) = w.root().and_downcast::<gtk::Window>() {
            let cap = Rc::downgrade(self);
            let id = win.connect_is_active_notify(move |w| {
                let Some(cap) = cap.upgrade() else {
                    return;
                };
                let (state, stop) = on_window_active(cap.inhibit.get(), w.is_active());
                cap.inhibit.set(state);
                if stop {
                    on_lost();
                }
            });
            self.handlers
                .borrow_mut()
                .push((win.upcast_ref::<glib::Object>().downgrade(), id));
        }
        // Ctrl+L, Ctrl+O, Ctrl+comma and every other app shortcut: off until recording ends.
        if let Some(app) = w
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|w| w.application())
        {
            *self.accels.borrow_mut() = Some(crate::ui::key_routing::AccelGuard::park(&app));
        }
        // Let Super, Alt+Tab and friends reach the recorder instead of the desktop.
        if let Some(top) = w
            .native()
            .and_then(|n| n.surface())
            .and_downcast::<gdk::Toplevel>()
        {
            let cap = Rc::downgrade(self);
            let id = top.connect_shortcuts_inhibited_notify(move |_| {
                if let Some(cap) = cap.upgrade()
                    && cap.inhibit.get() == InhibitState::Waiting
                {
                    cap.inhibit.set(InhibitState::Answered);
                }
            });
            self.handlers
                .borrow_mut()
                .push((top.upcast_ref::<glib::Object>().downgrade(), id));
            self.inhibit.set(InhibitState::Waiting);
            top.inhibit_system_shortcuts(None::<&gdk::Event>);
            // X11 grabs the keyboard at once.
            if top.is_shortcuts_inhibited() {
                self.inhibit.set(InhibitState::Answered);
            }
            *self.inhibited.borrow_mut() = Some(top.downgrade());
        }
    }

    /// Disconnects the handlers and restores the desktop's shortcuts.
    pub fn end(&self) {
        for (obj, id) in self.handlers.take() {
            if let Some(obj) = obj.upgrade() {
                obj.disconnect(id);
            }
        }
        self.inhibit.set(InhibitState::Off);
        self.accels.take();
        if let Some(top) = self.inhibited.take().and_then(|w| w.upgrade()) {
            top.restore_system_shortcuts();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.end();
    }
}

/// Where the request to pause the desktop's shortcuts stands while recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InhibitState {
    /// Not requested (no toplevel surface).
    #[default]
    Off,
    /// Requested, no answer yet: the compositor may be asking the user (GNOME's prompt).
    Waiting,
    /// Granted or denied.
    Answered,
}

/// What the window's activation changing means for recording: the new inhibit state, and
/// whether to stop. Losing activation stops recording unless the inhibit request is still
/// waiting (its prompt takes focus); coming back while waiting means the prompt was answered.
pub fn on_window_active(state: InhibitState, active: bool) -> (InhibitState, bool) {
    match (state, active) {
        (InhibitState::Waiting, true) => (InhibitState::Answered, false),
        (InhibitState::Waiting, false) => (state, false),
        (_, active) => (state, !active),
    }
}
