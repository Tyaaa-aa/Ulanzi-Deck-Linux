//! The GTK user interface.

pub mod action_editor;
pub mod app_picker;
pub mod autoswitch;
pub mod bundle_dialogs;
pub mod canvas;
pub mod capture;
pub mod color_popover;
pub mod how_it_works;
pub mod icon_picker;
pub mod inspector;
pub mod integrations;
pub mod item_picker;
pub(crate) mod key_routing;
pub mod key_sets;
pub mod library;
pub mod macro_diagram;
pub mod macro_editor;
pub mod macro_steps;
pub mod onboarding;
pub mod pick_spot;
pub mod plugin_fields;
pub mod plugins_window;
pub mod preferences;
pub mod shortcuts;
pub mod sidebar;
pub mod templates;
pub mod updates;
pub mod widgets_editor;
pub mod window;
pub mod wrap_box;

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
    show_toast(text);
}

/// [`toast`], for code with no [`State`] handle.
pub fn show_toast(text: &str) {
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

/// Run `f` on the one thread that owns GTK in tests, after GTK was initialised there once; `f`
/// gets whether that worked (no display: it did not). GTK must be initialised once and used from
/// one thread: tests that each called `gtk::init()` on their own test thread crashed the test
/// binary ("cannot register existing type 'GtkWidget'"). Calls are serialised; a panic in `f` is
/// re-raised in the calling test.
#[cfg(test)]
pub(crate) fn on_gtk_thread(f: impl FnOnce(bool) + Send + 'static) {
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};
    type Job = Box<dyn FnOnce(bool) + Send>;
    static JOBS: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let jobs = JOBS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("gtk-tests".into())
            .spawn(move || {
                let ok = gtk::init().is_ok();
                if ok {
                    // Once, so every test sees the same libadwaita environment.
                    adw::init().ok();
                }
                for job in rx {
                    job(ok);
                }
            })
            .expect("gtk test thread");
        Mutex::new(tx)
    });
    let (done_tx, done_rx) = mpsc::channel();
    let job: Job = Box::new(move |ok| {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(ok)));
        let _ = done_tx.send(r);
    });
    jobs.lock()
        .unwrap_or_else(|e| e.into_inner())
        .send(job)
        .expect("gtk test thread gone");
    match done_rx.recv().expect("gtk test thread gone") {
        Ok(()) => {}
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Run a GTK test body on the GTK test thread; skipped (returns quietly) without a display.
#[cfg(test)]
pub(crate) fn with_gtk(f: impl FnOnce() + Send + 'static) {
    on_gtk_thread(move |ok| {
        if ok {
            f()
        }
    });
}

/// Assert that no item of a menu model (sections and submenus included) has a label
/// ending in an ellipsis; menu items never carry one. Needs no display.
#[cfg(test)]
pub(crate) fn assert_menu_labels(model: &impl gtk::gio::prelude::IsA<gtk::gio::MenuModel>) {
    use gtk::gio::prelude::*;
    let model = model.upcast_ref::<gtk::gio::MenuModel>();
    for i in 0..model.n_items() {
        if let Some(label) = model
            .item_attribute_value(i, "label", None)
            .and_then(|v| v.str().map(str::to_owned))
        {
            assert!(
                !label.ends_with('…') && !label.ends_with("..."),
                "menu item {label:?} ends in an ellipsis"
            );
        }
        for link in ["section", "submenu"] {
            if let Some(sub) = model.item_link(i, link) {
                assert_menu_labels(&sub);
            }
        }
    }
}

/// A plugin's full-colour logo, `size` px: its `icon` file from `assets/`, a theme icon it names,
/// else the add-on icon.
pub fn plugin_logo(state: &State, plugin: &str, size: i32) -> gtk::Image {
    let icon = state
        .plugins
        .borrow()
        .manifests
        .get(plugin)
        .and_then(|m| m.icon.clone());
    plugin_image(plugin, icon.as_deref(), size)
}

/// `icon` as a manifest names it for plugin `plugin`, `size` px (see [`plugin_logo`]).
pub fn plugin_image(plugin: &str, icon: Option<&str>, size: i32) -> gtk::Image {
    use gtk::prelude::*;
    let image = match icon {
        Some(i) if ulanzi_core::plugin::is_asset(i) => {
            // Drawn at twice the size, for sharp icons on high-density screens.
            let px = u32::try_from(size * 2).unwrap_or(64);
            ulanzi_core::render::plugin_icon_png(plugin, i, px, None)
                .and_then(|png| gtk::gdk::Texture::from_bytes(&glib::Bytes::from(&png)).ok())
                .map(|t| gtk::Image::from_paintable(Some(&t)))
        }
        _ => None,
    };
    let image = image.unwrap_or_else(|| {
        let name = icon
            .and_then(crate::plugins::icon_lookup)
            .filter(|n| {
                gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(n))
            })
            .unwrap_or("application-x-addon-symbolic");
        gtk::Image::from_icon_name(name)
    });
    image.set_pixel_size(size);
    image.set_size_request(size, size);
    image.set_valign(gtk::Align::Center);
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtk_tests_share_one_thread() {
        let mut seen = Vec::new();
        for _ in 0..3 {
            let (tx, rx) = std::sync::mpsc::channel();
            on_gtk_thread(move |_| {
                let _ = tx.send(std::thread::current().id());
            });
            seen.push(rx.recv().expect("ran"));
        }
        assert!(seen.iter().all(|t| *t == seen[0]));
        assert_ne!(seen[0], std::thread::current().id());
    }

    #[test]
    fn a_panic_on_the_gtk_thread_fails_the_test_and_keeps_the_thread() {
        let r = std::panic::catch_unwind(|| on_gtk_thread(|_| panic!("boom")));
        assert!(r.is_err());
        let (tx, rx) = std::sync::mpsc::channel();
        on_gtk_thread(move |_| {
            let _ = tx.send(());
        });
        assert!(rx.recv().is_ok());
    }

    #[test]
    fn repeats_show_a_counter() {
        assert_eq!(repeated_title("Oops", 1), "Oops");
        assert_eq!(repeated_title("Oops", 7), "Oops  ×7");
    }
}
