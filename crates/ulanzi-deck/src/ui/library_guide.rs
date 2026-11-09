//! The Library's first-time guide: three short steps in the onboarding style, each with a looping
//! illustration drawn with Cairo (like How It Works). Shown the first time the tray opens, and again
//! from the tray's ? button or the end of onboarding.

use std::cell::Cell;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gtk::{cairo, glib};

use crate::state::State;
use crate::ui::how_it_works::{BLUE, GREEN, ORANGE, cursor, phase, rounded};

/// Each step's title and sentence.
pub const STEPS: [(&str, &str); 3] = [
    (
        "Drag Dials onto the Deck",
        "Drag a dial from the library onto a dial on the deck to use it on this page.",
    ),
    (
        "Swap and Pin",
        "Drag dials on the deck to swap them. Pin a dial to keep it on every page.",
    ),
    (
        "Copy and Paste",
        "Right-click a dial to copy it, then paste it onto another dial or page.",
    ),
];

/// Each illustration's loop length, in seconds.
const PERIODS: [f64; 3] = [4.6, 5.0, 6.0];
const W: f64 = 440.0;
const H: f64 = 220.0;
const YELLOW: (f64, f64, f64) = (0.96, 0.8, 0.16);

/// Set for screenshots: the guide never opens on its own.
static HELD_BACK: AtomicBool = AtomicBool::new(false);

/// Screenshots: keep the guide from opening on its own when the tray opens.
pub fn hold_back() {
    HELD_BACK.store(true, Ordering::Relaxed);
}

fn marker_in(state_home: &Path) -> PathBuf {
    state_home.join("ulanzi-deck").join("library-guide-shown")
}

fn should_show_in(state_home: &Path) -> bool {
    !marker_in(state_home).exists()
}

fn mark_shown_in(state_home: &Path) {
    let m = marker_in(state_home);
    let _ = m.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(m, b"1\n");
}

/// True until the guide has been shown once.
pub fn should_show() -> bool {
    !HELD_BACK.load(Ordering::Relaxed)
        && crate::ui::onboarding::state_home().is_some_and(|d| should_show_in(&d))
}

pub fn mark_shown() {
    if let Some(d) = crate::ui::onboarding::state_home() {
        mark_shown_in(&d);
    }
}

thread_local! {
    /// The guide on screen, so a second one never opens over it.
    static SHOWING: std::cell::RefCell<glib::WeakRef<adw::Dialog>> = Default::default();
}

/// The guide on screen, if any.
pub fn showing() -> Option<adw::Dialog> {
    SHOWING.with(|s| s.borrow().upgrade())
}

/// Open the guide over `parent` at its first step.
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>) {
    open_at(state, parent, 0);
}

/// Open the guide at step `step` (0-based; screenshots).
pub fn open_at(_state: &State, parent: &impl IsA<gtk::Widget>, step: usize) {
    if showing().is_some() {
        return;
    }
    if !HELD_BACK.load(Ordering::Relaxed) {
        mark_shown();
    }
    let dialog = adw::Dialog::builder()
        .title("Library Tour")
        .content_width(480)
        .content_height(520)
        .build();
    SHOWING.with(|s| s.borrow().set(Some(&dialog)));
    dialog.connect_closed(|_| SHOWING.with(|s| s.borrow().set(None)));
    let carousel = adw::Carousel::builder()
        .allow_long_swipes(false)
        .vexpand(true)
        .build();
    let current = Rc::new(Cell::new(0usize));
    let mut starts = Vec::new();
    for (i, (title, sentence)) in STEPS.iter().enumerate() {
        let (area, start) = illustration(i, current.clone());
        starts.push(start);
        let page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(24)
            .margin_end(24)
            .hexpand(true)
            .build();
        page.append(&area);
        page.append(
            &gtk::Label::builder()
                .label(*title)
                .css_classes(["title-1"])
                .margin_top(6)
                .wrap(true)
                .justify(gtk::Justification::Center)
                .build(),
        );
        page.append(
            &gtk::Label::builder()
                .label(*sentence)
                .wrap(true)
                .justify(gtk::Justification::Center)
                .max_width_chars(46)
                .build(),
        );
        carousel.append(&page);
    }

    let dots = gtk::Box::builder()
        .spacing(8)
        .valign(gtk::Align::Center)
        .css_classes(["guide-dots"])
        .build();
    let dot_list: Vec<gtk::Box> = (0..STEPS.len())
        .map(|_| {
            let d = gtk::Box::builder()
                .css_classes(["page-dot"])
                .valign(gtk::Align::Center)
                .build();
            dots.append(&d);
            d
        })
        .collect();
    let header = adw::HeaderBar::builder().title_widget(&dots).build();

    let next = gtk::Button::builder()
        .label("Next")
        .halign(gtk::Align::Center)
        .width_request(200)
        .css_classes(["pill", "suggested-action"])
        .build();
    let skip = gtk::Button::builder()
        .label("Skip the tour")
        .halign(gtk::Align::Center)
        .css_classes(["flat", "dim-label", "guide-skip"])
        .build();
    let bottom = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    bottom.append(&next);
    bottom.append(&skip);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&carousel));
    view.add_bottom_bar(&bottom);
    dialog.set_child(Some(&view));

    // Dots, buttons and the restarted loop follow the step in view.
    let show_step = {
        let (current, next, skip) = (current.clone(), next.clone(), skip.clone());
        move |i: usize| {
            current.set(i);
            for (n, d) in dot_list.iter().enumerate() {
                if n == i {
                    d.add_css_class("current");
                } else {
                    d.remove_css_class("current");
                }
            }
            let last = i + 1 == STEPS.len();
            next.set_label(if last { "Done" } else { "Next" });
            // Fades out instead of leaving a gap.
            if last {
                skip.add_css_class("gone");
            } else {
                skip.remove_css_class("gone");
            }
            skip.set_sensitive(!last);
            starts[i].set(None);
        }
    };
    show_step(0);
    {
        let current = current.clone();
        carousel.connect_position_notify(move |c| {
            let i = (c.position().round().max(0.0) as usize).min(STEPS.len() - 1);
            if i != current.get() {
                show_step(i);
            }
        });
    }
    {
        let (carousel, dialog, current) = (carousel.clone(), dialog.clone(), current.clone());
        next.connect_clicked(move |_| {
            let i = current.get();
            if i + 1 < STEPS.len() {
                carousel.scroll_to(&carousel.nth_page(i as u32 + 1), true);
            } else {
                dialog.close();
            }
        });
    }
    {
        let dialog = dialog.clone();
        skip.connect_clicked(move |_| {
            dialog.close();
        });
    }
    dialog.present(Some(parent));
    let step = step.min(STEPS.len() - 1);
    if step > 0 {
        // Once the carousel has its size.
        glib::idle_add_local_once(move || carousel.scroll_to(&carousel.nth_page(step as u32), false));
    }
}

/// Step `i`'s looping drawing; the loop restarts when its start is cleared.
fn illustration(i: usize, current: Rc<Cell<usize>>) -> (gtk::DrawingArea, Rc<Cell<Option<i64>>>) {
    let area = gtk::DrawingArea::builder()
        .content_width(W as i32)
        .content_height(H as i32)
        .halign(gtk::Align::Center)
        .margin_top(6)
        .build();
    let start = Rc::new(Cell::new(None::<i64>));
    let now = Rc::new(Cell::new(0.0f64));
    {
        let now = now.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let c = a.color();
            let fg = (c.red() as f64, c.green() as f64, c.blue() as f64);
            let k = (w as f64 / W).min(h as f64 / H);
            cr.translate((w as f64 - W * k) / 2.0, (h as f64 - H * k) / 2.0);
            cr.scale(k, k);
            draw(cr, i, now.get(), fg);
        });
    }
    {
        let start = start.clone();
        area.add_tick_callback(move |a, clock| {
            // Only the step in view moves.
            if current.get() != i {
                return glib::ControlFlow::Continue;
            }
            let us = clock.frame_time();
            let t0 = *start.get().get_or_insert(us);
            start.set(Some(t0));
            now.set(((us - t0) as f64 / 1e6) % PERIODS[i]);
            a.queue_draw();
            glib::ControlFlow::Continue
        });
    }
    (area, start)
}

/// What a dial shows.
#[derive(Clone, Copy)]
enum Glyph {
    Play,
    Volume,
    Sun,
    Arrows,
    Bars,
    Search,
}

fn lerp(a: f64, b: f64, p: f64) -> f64 {
    a + (b - a) * p
}

fn lerp2(a: (f64, f64), b: (f64, f64), p: f64) -> (f64, f64) {
    (lerp(a.0, b.0, p), lerp(a.1, b.1, p))
}

fn rgba(cr: &cairo::Context, c: (f64, f64, f64), a: f64) {
    cr.set_source_rgba(c.0, c.1, c.2, a);
}

/// Draw `f` at opacity `a`.
fn faded(cr: &cairo::Context, a: f64, f: impl FnOnce()) {
    if a <= 0.0 {
        return;
    }
    if a >= 1.0 {
        f();
        return;
    }
    cr.push_group();
    f();
    let _ = cr.pop_group_to_source();
    let _ = cr.paint_with_alpha(a);
}

fn text(cr: &cairo::Context, x: f64, y: f64, size: f64, s: &str) {
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(size);
    cr.move_to(x, y);
    let _ = cr.show_text(s);
    // Text leaves a current point that the next arc would join with a line.
    cr.new_path();
}

fn text_width(cr: &cairo::Context, size: f64, s: &str) -> f64 {
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
    cr.set_font_size(size);
    cr.text_extents(s).map(|e| e.x_advance()).unwrap_or(0.0)
}

/// A deck page: physically dark in both themes.
fn deck_panel(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, label: &str) {
    rounded(cr, x, y, w, h, 14.0);
    let grad = cairo::LinearGradient::new(0.0, y, 0.0, y + h);
    grad.add_color_stop_rgb(0.0, 0.17, 0.17, 0.19);
    grad.add_color_stop_rgb(1.0, 0.1, 0.1, 0.115);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.55);
    text(cr, x + 14.0, y + 18.0, 10.0, label);
}

fn glyph(cr: &cairo::Context, g: Glyph, cx: f64, cy: f64, r: f64) {
    let s = r * 0.5;
    let c = match g {
        Glyph::Play | Glyph::Volume | Glyph::Bars => ORANGE,
        Glyph::Sun => YELLOW,
        Glyph::Arrows => BLUE,
        Glyph::Search => GREEN,
    };
    rgba(cr, c, 1.0);
    cr.set_line_width((r * 0.09).max(1.5));
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);
    match g {
        Glyph::Play => {
            cr.move_to(cx - 0.35 * s, cy - 0.45 * s);
            cr.line_to(cx + 0.5 * s, cy);
            cr.line_to(cx - 0.35 * s, cy + 0.45 * s);
            cr.close_path();
            let _ = cr.stroke();
        }
        Glyph::Volume => {
            cr.move_to(cx - 0.6 * s, cy - 0.2 * s);
            cr.line_to(cx - 0.3 * s, cy - 0.2 * s);
            cr.line_to(cx + 0.05 * s, cy - 0.5 * s);
            cr.line_to(cx + 0.05 * s, cy + 0.5 * s);
            cr.line_to(cx - 0.3 * s, cy + 0.2 * s);
            cr.line_to(cx - 0.6 * s, cy + 0.2 * s);
            cr.close_path();
            let _ = cr.fill();
            cr.new_sub_path();
            cr.arc(cx + 0.1 * s, cy, 0.42 * s, -0.8, 0.8);
            let _ = cr.stroke();
        }
        Glyph::Sun => {
            cr.arc(cx, cy, 0.22 * s, 0.0, 2.0 * PI);
            let _ = cr.fill();
            for i in 0..8 {
                let a = i as f64 * PI / 4.0;
                cr.move_to(cx + 0.4 * s * a.cos(), cy + 0.4 * s * a.sin());
                cr.line_to(cx + 0.6 * s * a.cos(), cy + 0.6 * s * a.sin());
            }
            let _ = cr.stroke();
        }
        Glyph::Arrows => {
            cr.move_to(cx - 0.3 * s, cy - 0.1 * s);
            cr.line_to(cx, cy - 0.42 * s);
            cr.line_to(cx + 0.3 * s, cy - 0.1 * s);
            cr.move_to(cx - 0.3 * s, cy + 0.1 * s);
            cr.line_to(cx, cy + 0.42 * s);
            cr.line_to(cx + 0.3 * s, cy + 0.1 * s);
            let _ = cr.stroke();
        }
        Glyph::Bars => {
            for (i, h) in [0.35, 0.6, 0.85].iter().enumerate() {
                let x = cx - 0.42 * s + i as f64 * 0.3 * s;
                cr.rectangle(x, cy + 0.42 * s - h * s, 0.2 * s, h * s);
            }
            let _ = cr.fill();
        }
        Glyph::Search => {
            cr.new_sub_path();
            cr.arc(cx - 0.1 * s, cy - 0.1 * s, 0.3 * s, 0.0, 2.0 * PI);
            cr.move_to(cx + 0.12 * s, cy + 0.12 * s);
            cr.line_to(cx + 0.45 * s, cy + 0.45 * s);
            let _ = cr.stroke();
        }
    }
}

/// A dial: a dark knob with its glyph; `lift` adds a shadow as if picked up.
fn dial(cr: &cairo::Context, g: Glyph, cx: f64, cy: f64, r: f64, lift: f64) {
    if lift > 0.0 {
        cr.arc(cx + 2.0, cy + 5.0 * lift, r, 0.0, 2.0 * PI);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.3 * lift);
        let _ = cr.fill();
    }
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    let grad = cairo::RadialGradient::new(cx - r * 0.3, cy - r * 0.4, 0.0, cx, cy, r);
    grad.add_color_stop_rgb(0.0, 0.11, 0.11, 0.125);
    grad.add_color_stop_rgb(1.0, 0.045, 0.045, 0.05);
    let _ = cr.set_source(&grad);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
    cr.set_line_width(1.2);
    let _ = cr.stroke();
    glyph(cr, g, cx, cy, r);
}

/// An empty dial spot, outlined with dashes.
fn spot(cr: &cairo::Context, cx: f64, cy: f64, r: f64, a: f64) {
    if a <= 0.0 {
        return;
    }
    cr.set_dash(&[4.0, 4.0], 0.0);
    cr.set_line_width(2.0);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    rgba(cr, BLUE, a);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
}

fn ring(cr: &cairo::Context, cx: f64, cy: f64, r: f64, a: f64) {
    if a <= 0.0 {
        return;
    }
    cr.set_line_width(2.5);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    rgba(cr, BLUE, a);
    let _ = cr.stroke();
}

fn pointer(cr: &cairo::Context, at: (f64, f64), a: f64, fg: (f64, f64, f64)) {
    faded(cr, a, || cursor(cr, at.0, at.1, fg));
}

fn draw(cr: &cairo::Context, step: usize, t: f64, fg: (f64, f64, f64)) {
    match step {
        0 => draw_drag(cr, t, fg),
        1 => draw_swap(cr, t, fg),
        _ => draw_copy(cr, t, fg),
    }
}

/// Step 1: a library dial moves onto an empty dial spot on the deck.
fn draw_drag(cr: &cairo::Context, t: f64, fg: (f64, f64, f64)) {
    deck_panel(cr, 40.0, 10.0, 360.0, 96.0, "Deck");
    let deck_y = 62.0;
    let target = (290.0, deck_y);
    dial(cr, Glyph::Volume, 150.0, deck_y, 22.0, 0.0);
    dial(cr, Glyph::Play, 220.0, deck_y, 22.0, 0.0);

    // The library under the deck.
    let (lx, ly, lw, lh) = (40.0, 118.0, 360.0, 92.0);
    rounded(cr, lx, ly, lw, lh, 14.0);
    rgba(cr, fg, 0.07);
    let _ = cr.fill();
    rgba(cr, fg, 0.55);
    text(cr, lx + 14.0, ly + 18.0, 10.0, "Library");
    let lib_y = 170.0;
    let from = (252.0, lib_y);

    let carry = phase(t, 0.75, 1.85);
    let lift = phase(t, 0.6, 0.75) - phase(t, 1.85, 2.1);
    let gone = phase(t, 3.9, 4.4);
    let filled = phase(t, 1.85, 2.1) * (1.0 - gone);
    // Dimmed in the library while it is carried; it stays there too.
    let lib_a = 1.0 - 0.65 * phase(t, 0.75, 0.95) + 0.65 * phase(t, 1.85, 2.2);
    for (i, g) in [Glyph::Sun, Glyph::Arrows, Glyph::Bars, Glyph::Search]
        .iter()
        .enumerate()
    {
        let x = 124.0 + i as f64 * 64.0;
        let a = if i == 2 { lib_a } else { 1.0 };
        faded(cr, a, || dial(cr, *g, x, lib_y, 18.0, 0.0));
    }

    // The empty spot brightens as the dial nears, then the dial covers it.
    let glow = carry * (1.0 - phase(t, 1.85, 2.1));
    spot(cr, target.0, target.1, 24.0, (0.5 + 0.5 * glow) * (1.0 - filled));

    // A gentle arc from the library up to the spot.
    let ctrl = (340.0, 120.0);
    let at = (
        (1.0 - carry).powi(2) * from.0 + 2.0 * (1.0 - carry) * carry * ctrl.0 + carry * carry * target.0,
        (1.0 - carry).powi(2) * from.1 + 2.0 * (1.0 - carry) * carry * ctrl.1 + carry * carry * target.1,
    );
    if t >= 0.6 {
        let r = lerp(18.0, 22.0, carry) * (1.0 + 0.1 * lift);
        faded(cr, 1.0 - gone, || dial(cr, Glyph::Bars, at.0, at.1, r, lift));
    }

    let rest = (384.0, 196.0);
    let ptr = if t < 0.6 {
        lerp2(rest, (from.0 + 4.0, from.1 + 4.0), phase(t, 0.0, 0.6))
    } else if t < 2.1 {
        (at.0 + 4.0, at.1 + 4.0)
    } else {
        lerp2((target.0 + 4.0, target.1 + 4.0), rest, phase(t, 2.1, 2.9))
    };
    pointer(cr, ptr, phase(t, 0.0, 0.3) * (1.0 - phase(t, 2.5, 2.9)), fg);
}

/// A pushpin badge.
fn pin(cr: &cairo::Context, cx: f64, cy: f64) {
    cr.arc(cx, cy, 9.0, 0.0, 2.0 * PI);
    rgba(cr, BLUE, 1.0);
    let _ = cr.fill();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    rounded(cr, cx - 3.0, cy - 5.0, 6.0, 5.0, 1.5);
    let _ = cr.fill();
    cr.rectangle(cx - 4.5, cy - 0.5, 9.0, 1.6);
    let _ = cr.fill();
    cr.set_line_width(1.4);
    cr.move_to(cx, cy + 1.0);
    cr.line_to(cx, cy + 5.5);
    let _ = cr.stroke();
}

/// Step 2: two dials swap places, then a pin drops onto a third.
fn draw_swap(cr: &cairo::Context, t: f64, fg: (f64, f64, f64)) {
    deck_panel(cr, 40.0, 24.0, 360.0, 168.0, "Deck");
    let y = 100.0;
    let (a0, b0, c) = (130.0, 200.0, 320.0);
    let r = 26.0;

    // There and back, so the loop starts where it ends.
    let s = phase(t, 0.75, 1.6) - phase(t, 4.2, 4.9);
    let lift = phase(t, 0.6, 0.75) - phase(t, 1.6, 1.8);
    let ax = lerp(a0, b0, s);
    let ay = y - 18.0 * (s * PI).sin();
    let bx = lerp(b0, a0, s);
    ring(
        cr,
        b0,
        y,
        r + 4.0,
        phase(t, 0.75, 0.95) * (1.0 - phase(t, 1.6, 1.85)),
    );
    dial(cr, Glyph::Volume, bx, y, r, 0.0);
    dial(cr, Glyph::Sun, c, y, r, 0.0);
    dial(cr, Glyph::Play, ax, ay, r * (1.0 + 0.08 * lift), lift);

    // The pin drops onto the third dial.
    let away = phase(t, 4.2, 4.6);
    let drop = phase(t, 2.6, 3.0);
    faded(cr, drop * (1.0 - away), || {
        pin(cr, c + 20.0, y - 20.0 - 26.0 * (1.0 - drop))
    });
    let note = "Pinned: on every page";
    let nw = text_width(cr, 10.0, note);
    faded(cr, phase(t, 2.9, 3.2) * (1.0 - away), || {
        rgba(cr, BLUE, 1.0);
        text(cr, c - nw / 2.0, y + r + 24.0, 10.0, note);
    });

    let rest = (386.0, 178.0);
    let ptr = if t < 0.6 {
        lerp2(rest, (a0 + 4.0, y + 4.0), phase(t, 0.0, 0.6))
    } else if t < 1.8 {
        (ax + 4.0, ay + 4.0)
    } else if t < 3.0 {
        lerp2((b0 + 4.0, y + 4.0), (c + 4.0, y + 4.0), phase(t, 1.8, 2.4))
    } else {
        lerp2((c + 4.0, y + 4.0), rest, phase(t, 3.0, 3.6))
    };
    pointer(cr, ptr, phase(t, 0.0, 0.3) * (1.0 - phase(t, 3.1, 3.5)), fg);
}

/// A small right-click menu at (x, y); `hi` highlights Copy (0) or Paste (1).
fn menu(cr: &cairo::Context, x: f64, y: f64, hi: [f64; 2]) {
    let (w, h) = (116.0, 54.0);
    rounded(cr, x + 1.0, y + 3.0, w, h, 8.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.3);
    let _ = cr.fill();
    rounded(cr, x, y, w, h, 8.0);
    cr.set_source_rgb(0.98, 0.98, 0.985);
    let _ = cr.fill();
    for (i, (label, keys)) in [("Copy", "Ctrl+C"), ("Paste", "Ctrl+V")].iter().enumerate() {
        let ry = y + 4.0 + i as f64 * 23.0;
        if hi[i] > 0.0 {
            rounded(cr, x + 4.0, ry, w - 8.0, 23.0, 5.0);
            rgba(cr, BLUE, 0.2 * hi[i]);
            let _ = cr.fill();
        }
        cr.set_source_rgb(0.13, 0.13, 0.15);
        text(cr, x + 12.0, ry + 15.5, 11.0, label);
        cr.set_source_rgb(0.52, 0.52, 0.56);
        let kw = text_width(cr, 9.0, keys);
        text(cr, x + w - 12.0 - kw, ry + 15.0, 9.0, keys);
    }
}

/// A menu popping open at its top-left corner.
fn popped(cr: &cairo::Context, x: f64, y: f64, p: f64, hi: [f64; 2]) {
    faded(cr, p, || {
        cr.save().ok();
        cr.translate(x, y);
        let k = 0.9 + 0.1 * p;
        cr.scale(k, k);
        menu(cr, 0.0, 0.0, hi);
        cr.restore().ok();
    });
}

/// Step 3: right-click a dial, Copy, then right-click a spot on another page, Paste.
fn draw_copy(cr: &cairo::Context, t: f64, fg: (f64, f64, f64)) {
    deck_panel(cr, 20.0, 16.0, 196.0, 104.0, "Page 1");
    deck_panel(cr, 224.0, 72.0, 196.0, 104.0, "Page 2");
    let (y1, y2, r) = (76.0, 132.0, 18.0);
    let src = (118.0, y1);
    let dst = (322.0, y2);
    dial(cr, Glyph::Volume, 62.0, y1, r, 0.0);
    dial(cr, Glyph::Play, src.0, src.1, r, 0.0);
    dial(cr, Glyph::Sun, 174.0, y1, r, 0.0);
    dial(cr, Glyph::Arrows, 266.0, y2, r, 0.0);
    dial(cr, Glyph::Search, 378.0, y2, r, 0.0);

    // The copied dial stays outlined until it is pasted.
    ring(
        cr,
        src.0,
        src.1,
        r + 4.0,
        phase(t, 0.8, 1.0) * (1.0 - phase(t, 3.9, 4.2)),
    );

    let gone = phase(t, 5.2, 5.8);
    let pasted = phase(t, 3.6, 3.9) * (1.0 - gone);
    let near = phase(t, 2.2, 2.8) * (1.0 - phase(t, 3.6, 3.9));
    spot(cr, dst.0, dst.1, r + 2.0, (0.5 + 0.5 * near) * (1.0 - pasted));
    faded(cr, pasted, || {
        let k = 0.7 + 0.3 * phase(t, 3.6, 3.9);
        dial(cr, Glyph::Play, dst.0, dst.1, r * k, 0.0);
    });

    // Copy, then Paste.
    let m1 = (src.0 + 10.0, src.1 + 10.0);
    let m2 = (dst.0 - 90.0, dst.1 + 10.0);
    let copy_row = (m1.0 + 34.0, m1.1 + 14.0);
    let paste_row = (m2.0 + 34.0, m2.1 + 37.0);
    popped(
        cr,
        m1.0,
        m1.1,
        phase(t, 0.9, 1.1) * (1.0 - phase(t, 1.7, 1.9)),
        [phase(t, 1.3, 1.45), 0.0],
    );
    popped(
        cr,
        m2.0,
        m2.1,
        phase(t, 2.9, 3.1) * (1.0 - phase(t, 3.6, 3.8)),
        [0.0, phase(t, 3.35, 3.5)],
    );

    let rest = (420.0, 40.0);
    let on_src = (src.0 + 4.0, src.1 + 4.0);
    let on_dst = (dst.0 + 4.0, dst.1 + 4.0);
    let ptr = if t < 1.1 {
        lerp2(rest, on_src, phase(t, 0.0, 0.7))
    } else if t < 1.7 {
        lerp2(on_src, copy_row, phase(t, 1.1, 1.35))
    } else if t < 3.1 {
        lerp2(copy_row, on_dst, phase(t, 1.9, 2.8))
    } else if t < 4.0 {
        lerp2(on_dst, paste_row, phase(t, 3.1, 3.35))
    } else {
        lerp2(paste_row, rest, phase(t, 4.0, 4.6))
    };
    // A small copy rides along with the pointer to the other page.
    let carried = phase(t, 1.8, 2.0) * (1.0 - phase(t, 2.8, 3.0));
    faded(cr, 0.75 * carried, || {
        dial(cr, Glyph::Play, ptr.0 + 18.0, ptr.1 + 20.0, 11.0, 0.0)
    });
    pointer(cr, ptr, phase(t, 0.0, 0.3) * (1.0 - phase(t, 4.2, 4.6)), fg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_read_as_specified() {
        let titles: Vec<_> = STEPS.iter().map(|s| s.0).collect();
        assert_eq!(
            titles,
            ["Drag Dials onto the Deck", "Swap and Pin", "Copy and Paste"]
        );
        for (_, sentence) in STEPS {
            assert!(sentence.ends_with('.'), "{sentence:?}");
        }
    }

    #[test]
    fn only_one_guide_opens_at_a_time() {
        crate::ui::with_gtk(|| {
            let state = crate::state::AppState::new(ulanzi_core::model::Profile::new_default(
                "Default",
                ulanzi_core::model::Model::D200X,
            ));
            hold_back();
            let win = adw::Window::new();
            win.present();
            assert!(showing().is_none());
            open(&state, &win);
            let first = showing().expect("open");
            open(&state, &win);
            assert_eq!(showing(), Some(first.clone()));
            first.force_close();
            let main = glib::MainContext::default();
            let start = std::time::Instant::now();
            while showing().is_some() && start.elapsed() < std::time::Duration::from_secs(3) {
                main.iteration(false);
            }
            assert!(showing().is_none());
            win.destroy();
        });
    }

    #[test]
    fn the_guide_shows_once() {
        // A temporary state directory, never the real one.
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(should_show_in(dir.path()));
        mark_shown_in(dir.path());
        assert!(!should_show_in(dir.path()));
        assert!(marker_in(dir.path()).ends_with("ulanzi-deck/library-guide-shown"));
    }

    /// Every loop ends where it starts, so nothing jumps when it wraps.
    #[test]
    fn loops_end_where_they_start() {
        let render = |step: usize, t: f64| {
            let surf =
                cairo::ImageSurface::create(cairo::Format::ARgb32, W as i32, H as i32).expect("surface");
            let cr = cairo::Context::new(&surf).expect("context");
            draw(&cr, step, t, (1.0, 1.0, 1.0));
            drop(cr);
            surf.take_data().expect("data").to_vec()
        };
        for (step, period) in PERIODS.iter().enumerate() {
            let (a, b) = (render(step, 0.0), render(step, period - 1e-6));
            let diff = a.iter().zip(&b).filter(|(x, y)| x.abs_diff(**y) > 8).count();
            assert!(diff < 40, "step {step}: {diff} bytes differ");
        }
    }

    /// Render frames to PNG for a look: `GUIDE_FRAMES=dir cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn frames_to_png() {
        let Some(dir) = std::env::var_os("GUIDE_FRAMES") else {
            return;
        };
        for (step, period) in PERIODS.iter().enumerate() {
            for i in 0..8 {
                let t = period * i as f64 / 8.0;
                let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, W as i32 * 2, H as i32 * 2)
                    .expect("surface");
                let cr = cairo::Context::new(&surf).expect("context");
                cr.set_source_rgb(0.14, 0.14, 0.15);
                cr.paint().expect("bg");
                cr.scale(2.0, 2.0);
                draw(&cr, step, t, (1.0, 1.0, 1.0));
                drop(cr);
                let (w, h, stride) = (surf.width() as u32, surf.height() as u32, surf.stride() as usize);
                let data = surf.take_data().expect("data");
                let img = image::RgbaImage::from_fn(w, h, |x, y| {
                    let o = y as usize * stride + x as usize * 4;
                    image::Rgba([data[o + 2], data[o + 1], data[o], 255])
                });
                img.save(std::path::Path::new(&dir).join(format!("s{step}-{i}.png")))
                    .expect("png");
            }
        }
    }
}
