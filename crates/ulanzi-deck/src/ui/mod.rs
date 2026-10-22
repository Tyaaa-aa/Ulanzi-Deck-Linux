//! The GTK user interface.

pub mod action_editor;
pub mod app_picker;
pub mod autoswitch;
pub mod bundle_dialogs;
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
pub mod templates;
pub mod window;

use std::cell::RefCell;

use gtk::glib;

use crate::state::State;

thread_local! {
    static TOASTS: RefCell<Option<adw::ToastOverlay>> = const { RefCell::new(None) };
}

/// Where toasts appear (set once by the window).
pub fn set_toast_overlay(o: &adw::ToastOverlay) {
    TOASTS.with(|t| *t.borrow_mut() = Some(o.clone()));
}

/// Show a ready-made toast (e.g. one with an Undo button).
pub fn add_toast(t: adw::Toast) {
    TOASTS.with(|o| {
        if let Some(o) = o.borrow().as_ref() {
            o.add_toast(t);
        }
    });
}

/// The text shown for a message seen `count` times in a row.
pub fn repeated_title(text: &str, count: u32) -> String {
    if count <= 1 {
        text.to_string()
    } else {
        format!("{text}  ×{count}")
    }
}

/// The message on screen now, so repeats update it instead of piling up.
struct Showing {
    text: String,
    toast: adw::Toast,
    count: u32,
    timer: Option<glib::SourceId>,
}

thread_local! {
    static SHOWING: RefCell<Option<Showing>> = const { RefCell::new(None) };
}

const TOAST_SECS: u64 = 4;

/// Hide the toast `TOAST_SECS` after the last repeat.
fn arm_timer(toast: &adw::Toast) -> glib::SourceId {
    let toast = toast.clone();
    glib::timeout_add_local_once(std::time::Duration::from_secs(TOAST_SECS), move || {
        SHOWING.with(|s| {
            if let Some(sh) = s.borrow_mut().as_mut().filter(|sh| sh.toast == toast) {
                sh.timer = None;
            }
        });
        toast.dismiss();
    })
}

/// Show a short message at the bottom of the window. The same message again while it is still
/// shown bumps a counter on it instead of queueing another (dials can fail many times a second).
pub fn toast(_state: &State, text: &str) {
    let repeated = SHOWING.with(|s| {
        let mut s = s.borrow_mut();
        let Some(sh) = s.as_mut().filter(|sh| sh.text == text) else {
            return false;
        };
        sh.count += 1;
        sh.toast.set_title(&repeated_title(text, sh.count));
        if let Some(t) = sh.timer.take() {
            t.remove();
        }
        sh.timer = Some(arm_timer(&sh.toast));
        true
    });
    if repeated {
        return;
    }
    // Our own timer, so repeats can keep the toast up.
    let t = adw::Toast::builder().title(text).timeout(0).build();
    {
        let text = text.to_string();
        t.connect_dismissed(move |_| {
            SHOWING.with(|s| {
                let mut s = s.borrow_mut();
                if s.as_ref().is_some_and(|sh| sh.text == text) {
                    if let Some(timer) = s.as_mut().and_then(|sh| sh.timer.take()) {
                        timer.remove();
                    }
                    *s = None;
                }
            });
        });
    }
    let timer = arm_timer(&t);
    SHOWING.with(|s| {
        *s.borrow_mut() = Some(Showing {
            text: text.to_string(),
            toast: t.clone(),
            count: 1,
            timer: Some(timer),
        })
    });
    add_toast(t);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_show_a_counter() {
        assert_eq!(repeated_title("Oops", 1), "Oops");
        assert_eq!(repeated_title("Oops", 7), "Oops  ×7");
    }
}
