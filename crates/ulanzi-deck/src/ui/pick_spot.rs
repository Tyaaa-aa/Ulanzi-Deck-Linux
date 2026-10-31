//! Pick a Spot: a click on a full-screen overlay gives a macro a place on the desktop.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::state::State;

/// Seconds before the overlay gives up by itself, so nothing can leave a screen covered for good.
const GIVE_UP_AFTER: u32 = 30;

fn hint_text(seconds_left: u32) -> String {
    format!(
        "Click the spot the pointer should go to. Press Escape to cancel.\nCancels by itself in {seconds_left} s."
    )
}

/// A monitor's place on the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// (`x`, `y`) on monitor `monitor` as shares of the box around every monitor.
pub fn desktop_share(monitors: &[Rect], monitor: usize, x: f32, y: f32) -> (f32, f32) {
    let edge = |f: fn(&Rect) -> i32| monitors.iter().map(f);
    let left = edge(|m| m.x).min().unwrap_or(0);
    let top = edge(|m| m.y).min().unwrap_or(0);
    let right = edge(|m| m.x + m.width).max().unwrap_or(1);
    let bottom = edge(|m| m.y + m.height).max().unwrap_or(1);
    let Some(m) = monitors.get(monitor) else {
        return (0.0, 0.0);
    };
    let share = |at: f32, start: i32, offset: i32, end: i32| {
        ((at + (offset - start) as f32) / (end - start).max(1) as f32).clamp(0.0, 1.0)
    };
    (share(x, left, m.x, right), share(y, top, m.y, bottom))
}

/// Cover every monitor and hand the clicked spot to `done`. Escape cancels.
pub fn open(_state: &State, done: impl Fn(f32, f32) + 'static) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let list = display.monitors();
    let monitors: Vec<gdk::Monitor> = (0..list.n_items())
        .filter_map(|i| list.item(i).and_downcast::<gdk::Monitor>())
        .collect();
    let rects: Rc<Vec<Rect>> = Rc::new(
        monitors
            .iter()
            .map(|m| {
                let g = m.geometry();
                Rect {
                    x: g.x(),
                    y: g.y(),
                    width: g.width(),
                    height: g.height(),
                }
            })
            .collect(),
    );
    let windows: Rc<RefCell<Vec<gtk::Window>>> = Rc::default();
    let done = Rc::new(done);
    let hints: Rc<RefCell<Vec<gtk::Label>>> = Rc::default();
    // The macro editor is modal, and a modal window takes the input of its whole window group. In
    // a group of their own, these windows hear the click and Escape.
    let group = gtk::WindowGroup::new();
    let close_all = {
        let windows = windows.clone();
        move || {
            for w in windows.borrow_mut().drain(..) {
                w.close();
            }
        }
    };
    for (i, monitor) in monitors.iter().enumerate() {
        let hint = gtk::Label::builder()
            .label(hint_text(GIVE_UP_AFTER))
            .justify(gtk::Justification::Center)
            .css_classes(["title-2"])
            .build();
        hints.borrow_mut().push(hint.clone());
        let window = gtk::Window::builder()
            .decorated(false)
            .child(&hint)
            .css_classes(["pick-spot"])
            .build();
        let click = gtk::GestureClick::new();
        {
            let (rects, done, close_all) = (rects.clone(), done.clone(), close_all.clone());
            click.connect_pressed(move |_, _, x, y| {
                let (sx, sy) = desktop_share(&rects, i, x as f32, y as f32);
                close_all();
                done(sx, sy);
            });
        }
        window.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        {
            let close_all = close_all.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    close_all();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        window.add_controller(keys);
        group.add_window(&window);
        window.fullscreen_on_monitor(monitor);
        window.present();
        windows.borrow_mut().push(window);
    }
    let left = Cell::new(GIVE_UP_AFTER);
    glib::timeout_add_seconds_local(1, move || {
        if windows.borrow().is_empty() {
            return glib::ControlFlow::Break;
        }
        left.set(left.get().saturating_sub(1));
        if left.get() == 0 {
            close_all();
            return glib::ControlFlow::Break;
        }
        for hint in hints.borrow().iter() {
            hint.set_label(&hint_text(left.get()));
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: Rect = Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    const RIGHT: Rect = Rect {
        x: 1920,
        y: 180,
        width: 1280,
        height: 720,
    };

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.001 && (a.1 - b.1).abs() < 0.001
    }

    #[test]
    fn a_modal_window_cannot_take_the_overlays_input() {
        crate::ui::with_gtk(|| {
            let state = crate::state::AppState::new(ulanzi_core::model::Profile::new_default(
                "Default",
                ulanzi_core::model::Model::D200X,
            ));
            let modal = gtk::Window::builder().modal(true).build();
            modal.present();
            open(&state, |_, _| {});
            let overlays: Vec<gtk::Window> = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Window>().ok())
                .filter(|w| w.has_css_class("pick-spot"))
                .collect();
            assert!(!overlays.is_empty());
            for overlay in &overlays {
                assert_ne!(overlay.group(), modal.group());
                overlay.destroy();
            }
            modal.destroy();
        });
    }

    #[test]
    fn a_spot_is_a_share_of_the_whole_desktop() {
        let all = [LEFT, RIGHT];
        assert!(close(desktop_share(&all, 0, 0.0, 0.0), (0.0, 0.0)));
        assert!(close(desktop_share(&all, 0, 1600.0, 540.0), (0.5, 0.5)));
        let (x, y) = desktop_share(&all, 1, 1280.0, 720.0);
        assert!(close((x, y), (1.0, 900.0 / 1080.0)));
    }

    #[test]
    fn monitors_with_negative_positions_work() {
        let left = Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let main = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let all = [left, main];
        assert!(close(desktop_share(&all, 1, 0.0, 0.0), (0.5, 0.0)));
        assert!(close(desktop_share(&all, 0, 0.0, 0.0), (0.0, 0.0)));
    }

    #[test]
    fn a_spot_stays_on_the_desktop() {
        assert!(close(desktop_share(&[LEFT], 0, 5000.0, -5.0), (1.0, 0.0)));
    }
}
