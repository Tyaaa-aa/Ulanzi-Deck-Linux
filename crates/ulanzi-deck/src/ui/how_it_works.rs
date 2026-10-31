//! A small looping animation for the end of onboarding: drag an action onto a key, it appears on
//! the deck, press it and it runs. Drawn with Cairo every frame (no image or video files).

use std::f64::consts::PI;

use adw::prelude::*;
use gtk::cairo;

const PERIOD: f64 = 5.2;
const W: f64 = 440.0;
const H: f64 = 210.0;

// Deck geometry.
const DECK: (f64, f64, f64, f64) = (18.0, 16.0, 252.0, 178.0);
const KEY: f64 = 36.0;
const GAP: f64 = 8.0;
const KEYS_AT: (f64, f64) = (38.0, 36.0);
const TARGET: (usize, usize) = (0, 1);

// Library geometry.
const LIB: (f64, f64, f64, f64) = (296.0, 16.0, 128.0, 178.0);
const CARD_H: f64 = 34.0;

pub(crate) const BLUE: (f64, f64, f64) = (0.208, 0.518, 0.894);
pub(crate) const GREEN: (f64, f64, f64) = (0.2, 0.82, 0.478);
const ORANGE: (f64, f64, f64) = (1.0, 0.47, 0.0);

/// Where each step of the story starts, in seconds.
const T_GRAB: f64 = 0.4;
const T_DROP: f64 = 1.7;
const T_PRESS: f64 = 2.5;
const T_FADE: f64 = 4.6;

/// The caption for time `t`.
pub fn caption(t: f64) -> &'static str {
    if t < T_DROP {
        "Drag an action onto a key"
    } else if t < T_PRESS {
        "It shows on the deck right away"
    } else {
        "Press the key to run it"
    }
}

pub(crate) fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

pub(crate) fn phase(t: f64, from: f64, to: f64) -> f64 {
    ease((t - from) / (to - from))
}

pub(crate) fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

fn key_rect(row: usize, col: usize) -> (f64, f64) {
    (
        KEYS_AT.0 + col as f64 * (KEY + GAP),
        KEYS_AT.1 + row as f64 * (KEY + GAP),
    )
}

pub(crate) fn play_glyph(cr: &cairo::Context, cx: f64, cy: f64, s: f64) {
    cr.move_to(cx - 0.35 * s, cy - 0.42 * s);
    cr.line_to(cx + 0.45 * s, cy);
    cr.line_to(cx - 0.35 * s, cy + 0.42 * s);
    cr.close_path();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    let _ = cr.fill();
}

fn chip(cr: &cairo::Context, x: f64, y: f64, s: f64, rgb: (f64, f64, f64), glyph: bool) {
    rounded(cr, x, y, s, s, s * 0.22);
    cr.set_source_rgb(rgb.0, rgb.1, rgb.2);
    let _ = cr.fill();
    if glyph {
        play_glyph(cr, x + s / 2.0, y + s / 2.0, s * 0.45);
    }
}

/// An eighth note with its head at (x, y).
fn note(cr: &cairo::Context, x: f64, y: f64, k: f64) {
    cr.save().ok();
    cr.translate(x, y);
    cr.scale(k, k);
    cr.save().ok();
    cr.scale(1.25, 1.0);
    cr.arc(0.0, 0.0, 4.2, 0.0, 2.0 * PI);
    cr.restore().ok();
    let _ = cr.fill();
    cr.rectangle(3.6, -16.0, 2.0, 16.0);
    let _ = cr.fill();
    cr.move_to(5.6, -16.0);
    cr.curve_to(9.0, -13.0, 12.0, -11.0, 10.0, -6.0);
    cr.curve_to(10.0, -10.0, 8.0, -11.5, 5.6, -12.0);
    cr.close_path();
    let _ = cr.fill();
    cr.restore().ok();
}

pub(crate) fn cursor(cr: &cairo::Context, x: f64, y: f64, fg: (f64, f64, f64)) {
    cr.move_to(x, y);
    cr.line_to(x, y + 16.0);
    cr.line_to(x + 4.5, y + 12.0);
    cr.line_to(x + 8.0, y + 18.5);
    cr.line_to(x + 10.5, y + 17.2);
    cr.line_to(x + 7.2, y + 11.0);
    cr.line_to(x + 12.0, y + 11.0);
    cr.close_path();
    cr.set_source_rgb(1.0 - fg.0, 1.0 - fg.1, 1.0 - fg.2);
    cr.set_line_width(1.6);
    let _ = cr.stroke_preserve();
    cr.set_source_rgb(fg.0, fg.1, fg.2);
    let _ = cr.fill();
}

/// Draw the scene at time `t` (seconds into the loop). `fg` is the theme's text colour.
fn draw(cr: &cairo::Context, t: f64, fg: (f64, f64, f64)) {
    // Deck body (physically dark in both themes).
    let (dx, dy, dw, dh) = DECK;
    rounded(cr, dx, dy, dw, dh, 22.0);
    let grad = cairo::LinearGradient::new(0.0, dy, 0.0, dy + dh);
    grad.add_color_stop_rgb(0.0, 0.17, 0.17, 0.19);
    grad.add_color_stop_rgb(1.0, 0.086, 0.086, 0.1);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();

    let fade_out = 1.0 - phase(t, T_FADE, PERIOD - 0.15);
    let dropped = phase(t, T_DROP, T_DROP + 0.25) * fade_out;
    let press = if t > T_PRESS && t < T_PRESS + 0.3 {
        (((t - T_PRESS) / 0.3) * PI).sin()
    } else {
        0.0
    };

    // Keys: 13 plus the wide screen.
    for row in 0..3 {
        for col in 0..5 {
            if row == 2 && col >= 3 {
                continue;
            }
            let (x, y) = key_rect(row, col);
            let target = (row, col) == TARGET;
            let s = if target { 1.0 - 0.1 * press } else { 1.0 };
            let (cx, cy) = (x + KEY / 2.0, y + KEY / 2.0);
            let k = KEY * s;
            rounded(cr, cx - k / 2.0, cy - k / 2.0, k, k, 7.0);
            cr.set_source_rgb(0.047, 0.047, 0.055);
            let _ = cr.fill();
            // A few keys already set up, so the deck looks used.
            let preset = match (row, col) {
                (0, 3) => Some(GREEN),
                (1, 0) => Some(ORANGE),
                _ => None,
            };
            if let Some(c) = preset {
                rounded(cr, cx - k / 2.0 + 3.0, cy - k / 2.0 + 3.0, k - 6.0, k - 6.0, 5.0);
                cr.set_source_rgba(c.0, c.1, c.2, 0.85);
                let _ = cr.fill();
            }
            if target && dropped > 0.0 {
                let inner = (k - 6.0) * (0.6 + 0.4 * dropped);
                rounded(cr, cx - inner / 2.0, cy - inner / 2.0, inner, inner, 5.0);
                cr.set_source_rgba(BLUE.0, BLUE.1, BLUE.2, dropped);
                let _ = cr.fill();
                cr.push_group();
                play_glyph(cr, cx, cy, inner * 0.45);
                let _ = cr.pop_group_to_source();
                let _ = cr.paint_with_alpha(dropped);
            }
        }
    }
    let (sx, sy) = key_rect(2, 3);
    rounded(cr, sx, sy, KEY * 2.0 + GAP, KEY, 7.0);
    cr.set_source_rgb(0.08, 0.13, 0.25);
    let _ = cr.fill();
    // Small buttons and dials.
    for i in 0..2 {
        rounded(cr, 38.0 + i as f64 * 24.0, 168.0, 18.0, 14.0, 4.0);
        cr.set_source_rgb(0.06, 0.06, 0.07);
        let _ = cr.fill();
    }
    for i in 0..3 {
        cr.arc(170.0 + i as f64 * 30.0, 175.0, 11.0, 0.0, 2.0 * PI);
        cr.set_source_rgb(0.06, 0.06, 0.07);
        let _ = cr.fill();
    }

    // Press ripple and a note: the action ran.
    let (tx, ty) = key_rect(TARGET.0, TARGET.1);
    let (tcx, tcy) = (tx + KEY / 2.0, ty + KEY / 2.0);
    if t > T_PRESS && t < T_PRESS + 0.9 {
        let p = (t - T_PRESS) / 0.9;
        cr.arc(tcx, tcy, KEY * 0.5 + 26.0 * p, 0.0, 2.0 * PI);
        cr.set_source_rgba(BLUE.0, BLUE.1, BLUE.2, 0.7 * (1.0 - p));
        cr.set_line_width(3.0);
        let _ = cr.stroke();
    }
    if t > T_PRESS + 0.2 && t < T_FADE {
        let p = (t - T_PRESS - 0.2) / (T_FADE - T_PRESS - 0.2);
        let a = (1.0 - p).min(p * 6.0).clamp(0.0, 1.0);
        cr.set_source_rgba(BLUE.0, BLUE.1, BLUE.2, a);
        note(cr, tcx + 26.0, ty + 4.0 - 14.0 * p, 1.0);
        note(cr, tcx + 44.0, ty + 16.0 - 20.0 * p, 0.75);
    }

    // Library panel.
    let (lx, ly, lw, lh) = LIB;
    rounded(cr, lx, ly, lw, lh, 14.0);
    cr.set_source_rgba(fg.0, fg.1, fg.2, 0.07);
    let _ = cr.fill();
    let cards = [(BLUE, 0.62), (GREEN, 0.48), (ORANGE, 0.55)];
    for (i, (c, len)) in cards.iter().enumerate() {
        let y = ly + 14.0 + i as f64 * (CARD_H + 10.0);
        rounded(cr, lx + 10.0, y, lw - 20.0, CARD_H, 9.0);
        cr.set_source_rgba(fg.0, fg.1, fg.2, 0.08);
        let _ = cr.fill();
        chip(cr, lx + 17.0, y + 7.0, 20.0, *c, i == 0);
        rounded(cr, lx + 44.0, y + 13.5, (lw - 64.0) * len, 7.0, 3.5);
        cr.set_source_rgba(fg.0, fg.1, fg.2, 0.35);
        let _ = cr.fill();
    }

    // The dragged card and the pointer.
    let card = (lx + 27.0, ly + 14.0 + CARD_H / 2.0);
    let start_ptr = (lx + 70.0, ly + 150.0);
    if t < T_GRAB {
        let p = phase(t, 0.0, T_GRAB);
        let (x, y) = (
            start_ptr.0 + (card.0 - start_ptr.0) * p,
            start_ptr.1 + (card.1 - start_ptr.1) * p,
        );
        cursor(cr, x, y, fg);
    } else if t < T_DROP {
        let p = phase(t, T_GRAB + 0.1, T_DROP);
        // A gentle arc from the library to the key.
        let (x0, y0) = card;
        let (x1, y1) = (tcx, tcy);
        let (mx, my) = ((x0 + x1) / 2.0, y0.min(y1) - 14.0);
        let x = (1.0 - p).powi(2) * x0 + 2.0 * (1.0 - p) * p * mx + p * p * x1;
        let y = (1.0 - p).powi(2) * y0 + 2.0 * (1.0 - p) * p * my + p * p * y1;
        let s = 24.0 + 8.0 * (p * PI).sin();
        cr.save().ok();
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
        rounded(cr, x - s / 2.0 + 2.0, y - s / 2.0 + 4.0, s, s, s * 0.22);
        let _ = cr.fill();
        cr.restore().ok();
        chip(cr, x - s / 2.0, y - s / 2.0, s, BLUE, true);
        cursor(cr, x + 4.0, y + 4.0, fg);
    } else if t < T_PRESS + 0.35 {
        let p = phase(t, T_DROP, T_PRESS);
        let (x, y) = (tcx + 4.0 + 6.0 * (1.0 - p), tcy + 4.0);
        cursor(cr, x, y, fg);
    }
}

/// The animation widget and a caption that follows it.
pub fn build() -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let area = gtk::DrawingArea::builder()
        .content_width(W as i32)
        .content_height(H as i32)
        .halign(gtk::Align::Center)
        .build();
    let label = gtk::Label::builder()
        .label(caption(0.0))
        .css_classes(["title-4"])
        .build();
    let start = std::rc::Rc::new(std::cell::Cell::new(None::<i64>));
    let now = std::rc::Rc::new(std::cell::Cell::new(0.0f64));
    {
        let now = now.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let c = a.color();
            let fg = (c.red() as f64, c.green() as f64, c.blue() as f64);
            // Scale to the space given, keeping the aspect ratio.
            let k = (w as f64 / W).min(h as f64 / H);
            cr.translate((w as f64 - W * k) / 2.0, (h as f64 - H * k) / 2.0);
            cr.scale(k, k);
            draw(cr, now.get(), fg);
        });
    }
    {
        let label = label.clone();
        area.add_tick_callback(move |a, clock| {
            let us = clock.frame_time();
            let t0 = *start.get().get_or_insert(us);
            start.set(Some(t0));
            let t = ((us - t0) as f64 / 1e6) % PERIOD;
            now.set(t);
            let text = caption(t);
            if label.label() != text {
                label.set_label(text);
            }
            a.queue_draw();
            glib::ControlFlow::Continue
        });
    }
    column.append(&area);
    column.append(&label);
    column
}

use gtk::glib;

#[cfg(test)]
mod tests {
    use super::*;

    /// Render frames to PNG for a look at the animation: `HOW_FRAMES=dir cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn frames_to_png() {
        let Some(dir) = std::env::var_os("HOW_FRAMES") else {
            return;
        };
        for (i, t) in [0.2, 1.1, 1.9, 2.6, 3.4, 5.0].iter().enumerate() {
            let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, W as i32 * 2, H as i32 * 2)
                .expect("surface");
            let cr = cairo::Context::new(&surf).expect("context");
            cr.set_source_rgb(0.14, 0.14, 0.15);
            cr.paint().expect("bg");
            cr.scale(2.0, 2.0);
            draw(&cr, *t, (1.0, 1.0, 1.0));
            drop(cr);
            let (w, h, stride) = (surf.width() as u32, surf.height() as u32, surf.stride() as usize);
            let data = surf.take_data().expect("data");
            let img = image::RgbaImage::from_fn(w, h, |x, y| {
                let o = y as usize * stride + x as usize * 4;
                image::Rgba([data[o + 2], data[o + 1], data[o], 255])
            });
            img.save(std::path::Path::new(&dir).join(format!("f{i}.png")))
                .expect("png");
        }
    }

    #[test]
    fn captions_follow_the_story() {
        assert_eq!(caption(0.1), "Drag an action onto a key");
        assert_eq!(caption(T_DROP + 0.1), "It shows on the deck right away");
        assert_eq!(caption(T_PRESS + 0.1), "Press the key to run it");
    }
}
