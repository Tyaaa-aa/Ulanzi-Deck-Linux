//! The GTK user interface.

pub mod action_editor;
pub mod app_picker;
pub mod autoswitch;
pub mod canvas;
pub mod color_popover;
pub mod how_it_works;
pub mod icon_picker;
pub mod inspector;
pub mod library;
pub mod onboarding;
pub mod preferences;
pub mod shortcuts;
pub mod sidebar;
pub mod window;

use std::cell::RefCell;

use crate::state::State;

thread_local! {
    static TOASTS: RefCell<Option<adw::ToastOverlay>> = const { RefCell::new(None) };
}

/// Where toasts appear (set once by the window).
pub fn set_toast_overlay(o: &adw::ToastOverlay) {
    TOASTS.with(|t| *t.borrow_mut() = Some(o.clone()));
}

/// Show a short message at the bottom of the window.
pub fn toast(_state: &State, text: &str) {
    TOASTS.with(|t| {
        if let Some(o) = t.borrow().as_ref() {
            o.add_toast(adw::Toast::builder().title(text).timeout(4).build());
        }
    });
}
