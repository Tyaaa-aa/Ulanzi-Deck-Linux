//! A colour button whose popover previews colours live: hovering a swatch shows it on the selected
//! keys, clicking applies it and keeps the popover open so you can compare.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use ulanzi_core::Color;

/// Swatches in rows of eight: neutrals, then GNOME palette colours light to dark.
const PALETTE: [&str; 32] = [
    "#000000", "#1e1e1e", "#3d3846", "#5e5c64", "#9a9996", "#c0bfbc", "#deddda", "#ffffff", "#99c1f1",
    "#8ff0a4", "#f9f06b", "#ffbe6f", "#f66151", "#dc8add", "#cdab8f", "#62a0ea", "#3584e4", "#33d17a",
    "#f6d32d", "#ff7800", "#e01b24", "#9141ac", "#986a44", "#2ec27e", "#1a5fb4", "#26a269", "#e5a50a",
    "#c64600", "#a51d2d", "#613583", "#63452c", "#241f31",
];

pub fn rgba(c: Color) -> gdk::RGBA {
    let [r, g, b, a] = c.0;
    gdk::RGBA::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    )
}

pub fn color(c: &gdk::RGBA) -> Color {
    let v = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color([v(c.red()), v(c.green()), v(c.blue()), 255])
}

/// Hue in degrees, saturation and value in 0..1.
pub fn to_hsv(c: Color) -> (f64, f64, f64) {
    let [r, g, b, _] = c.0.map(|v| v as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

pub fn from_hsv(h: f64, s: f64, v: f64) -> Color {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |f: f64| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::rgb(q(r), q(g), q(b))
}

/// A small square that paints one colour (striped when `None`, meaning mixed).
fn swatch(size: i32) -> (gtk::DrawingArea, Rc<Cell<Option<Color>>>) {
    let value: Rc<Cell<Option<Color>>> = Rc::new(Cell::new(None));
    let area = gtk::DrawingArea::builder()
        .content_width(size)
        .content_height(size)
        .css_classes(["color-swatch"])
        .build();
    {
        let value = value.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let r = 6.0_f64.min(w / 4.0);
            cr.new_sub_path();
            cr.arc(w - r, r, r, -90f64.to_radians(), 0.0);
            cr.arc(w - r, h - r, r, 0.0, 90f64.to_radians());
            cr.arc(r, h - r, r, 90f64.to_radians(), 180f64.to_radians());
            cr.arc(r, r, r, 180f64.to_radians(), 270f64.to_radians());
            cr.close_path();
            cr.save().ok();
            cr.clip_preserve();
            match value.get() {
                Some(c) => {
                    let [r8, g8, b8, _] = c.0;
                    cr.set_source_rgb(r8 as f64 / 255.0, g8 as f64 / 255.0, b8 as f64 / 255.0);
                    cr.fill_preserve().ok();
                }
                None => {
                    cr.set_source_rgb(0.55, 0.55, 0.55);
                    cr.fill_preserve().ok();
                    cr.set_source_rgb(0.75, 0.75, 0.75);
                    cr.set_line_width(3.0);
                    let mut x = -h;
                    while x < w {
                        cr.move_to(x, h);
                        cr.line_to(x + h, 0.0);
                        x += 8.0;
                    }
                    cr.stroke().ok();
                }
            }
            cr.restore().ok();
            cr.set_source_rgba(0.5, 0.5, 0.5, 0.45);
            cr.set_line_width(1.0);
            cr.stroke().ok();
        });
    }
    (area, value)
}

/// A colour button. `preview(Some(c))` while a swatch is hovered, `preview(None)` when it is left;
/// `pick(c)` when one is chosen.
pub fn color_button(
    current: Option<Color>,
    preview: impl Fn(Option<Color>) + 'static,
    pick: impl Fn(Color) + 'static,
) -> gtk::MenuButton {
    let (face, face_value) = swatch(22);
    face_value.set(current);
    let button = gtk::MenuButton::builder()
        .child(&face)
        .valign(gtk::Align::Center)
        .tooltip_text("Choose a colour")
        .build();
    let preview: Rc<dyn Fn(Option<Color>)> = Rc::new(preview);
    let pick: Rc<dyn Fn(Color)> = Rc::new(pick);
    // Applying also updates the button's own face.
    let apply: Rc<dyn Fn(Color)> = {
        let (face, face_value, pick) = (face.clone(), face_value.clone(), pick.clone());
        Rc::new(move |c| {
            face_value.set(Some(c));
            face.queue_draw();
            pick(c);
        })
    };

    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);
    let grid = gtk::Grid::builder().row_spacing(6).column_spacing(6).build();
    for (i, hex) in PALETTE.iter().enumerate() {
        let Ok(c) = hex.parse::<Color>() else { continue };
        let (area, v) = swatch(26);
        v.set(Some(c));
        let b = gtk::Button::builder()
            .child(&area)
            .css_classes(["flat", "swatch-button"])
            .tooltip_text(*hex)
            .build();
        let motion = gtk::EventControllerMotion::new();
        {
            let preview = preview.clone();
            motion.connect_enter(move |_, _, _| preview(Some(c)));
        }
        {
            let preview = preview.clone();
            motion.connect_leave(move |_| preview(None));
        }
        b.add_controller(motion);
        {
            let apply = apply.clone();
            b.connect_clicked(move |_| apply(c));
        }
        grid.attach(&b, (i % 8) as i32, (i / 8) as i32, 1, 1);
    }
    column.append(&grid);

    let hex = gtk::Entry::builder()
        .placeholder_text("#rrggbb")
        .max_width_chars(9)
        .hexpand(true)
        .text(current.map(|c| c.to_string()).unwrap_or_default())
        .build();
    {
        let (apply, preview) = (apply.clone(), preview.clone());
        hex.connect_changed(move |e| {
            if let Ok(c) = e.text().trim().parse::<Color>() {
                preview(Some(c));
                apply(c);
            }
        });
    }
    // Any colour: a saturation/brightness square and a hue strip, previewed while dragging.
    let custom = gtk::ToggleButton::builder()
        .label("Custom")
        .tooltip_text("Pick any colour")
        .build();
    let row = gtk::Box::builder().spacing(6).build();
    row.append(&hex);
    row.append(&custom);
    column.append(&row);

    let hsv = Rc::new(Cell::new(current.map(to_hsv).unwrap_or((0.0, 0.0, 0.5))));
    let sv = gtk::DrawingArea::builder()
        .content_width(244)
        .content_height(140)
        .css_classes(["color-plane"])
        .build();
    let hue = gtk::DrawingArea::builder()
        .content_width(244)
        .content_height(16)
        .build();
    {
        let hsv = hsv.clone();
        sv.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let (hh, s, v) = hsv.get();
            let base = from_hsv(hh, 1.0, 1.0).0;
            cr.rectangle(0.0, 0.0, w, h);
            cr.set_source_rgb(
                base[0] as f64 / 255.0,
                base[1] as f64 / 255.0,
                base[2] as f64 / 255.0,
            );
            let _ = cr.fill_preserve();
            let white = gtk::cairo::LinearGradient::new(0.0, 0.0, w, 0.0);
            white.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 1.0);
            white.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
            let _ = cr.set_source(&white);
            let _ = cr.fill_preserve();
            let black = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
            black.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
            black.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 1.0);
            let _ = cr.set_source(&black);
            let _ = cr.fill();
            let (x, y) = (s * w, (1.0 - v) * h);
            cr.arc(x, y, 6.0, 0.0, std::f64::consts::TAU);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.5);
            let _ = cr.stroke_preserve();
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        });
    }
    {
        let hsv = hsv.clone();
        hue.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, 0.0);
            for k in 0..=6 {
                let c = from_hsv(k as f64 * 60.0, 1.0, 1.0).0;
                g.add_color_stop_rgb(
                    k as f64 / 6.0,
                    c[0] as f64 / 255.0,
                    c[1] as f64 / 255.0,
                    c[2] as f64 / 255.0,
                );
            }
            cr.rectangle(0.0, 0.0, w, h);
            let _ = cr.set_source(&g);
            let _ = cr.fill();
            let x = hsv.get().0 / 360.0 * w;
            cr.rectangle(x - 3.0, 0.0, 6.0, h);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.0);
            let _ = cr.stroke();
        });
    }
    // Dragging previews; letting go applies.
    let drag_on = |area: &gtk::DrawingArea, is_plane: bool| {
        let drag = gtk::GestureDrag::new();
        let update = {
            let (hsv, sv, hue, preview, area) = (
                hsv.clone(),
                sv.clone(),
                hue.clone(),
                preview.clone(),
                area.clone(),
            );
            move |x: f64, y: f64| {
                let (w, h) = (area.width().max(1) as f64, area.height().max(1) as f64);
                let (mut hh, mut s, mut v) = hsv.get();
                if is_plane {
                    s = (x / w).clamp(0.0, 1.0);
                    v = 1.0 - (y / h).clamp(0.0, 1.0);
                } else {
                    hh = (x / w).clamp(0.0, 1.0) * 359.9;
                }
                hsv.set((hh, s, v));
                sv.queue_draw();
                hue.queue_draw();
                preview(Some(from_hsv(hh, s, v)));
            }
        };
        let start = Rc::new(Cell::new((0.0, 0.0)));
        {
            let (update, start) = (update.clone(), start.clone());
            drag.connect_drag_begin(move |_, x, y| {
                start.set((x, y));
                update(x, y);
            });
        }
        {
            let (update, start) = (update.clone(), start.clone());
            drag.connect_drag_update(move |_, dx, dy| {
                let (x, y) = start.get();
                update(x + dx, y + dy);
            });
        }
        {
            let (hsv, apply, hex) = (hsv.clone(), apply.clone(), hex.clone());
            drag.connect_drag_end(move |_, _, _| {
                let (hh, s, v) = hsv.get();
                let c = from_hsv(hh, s, v);
                hex.set_text(&c.to_string());
                apply(c);
            });
        }
        area.add_controller(drag);
    };
    drag_on(&sv, true);
    drag_on(&hue, false);
    let picker = gtk::Box::new(gtk::Orientation::Vertical, 8);
    picker.append(&sv);
    picker.append(&hue);
    let reveal = gtk::Revealer::builder()
        .child(&picker)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .build();
    custom
        .bind_property("active", &reveal, "reveal-child")
        .sync_create()
        .build();
    column.append(&reveal);

    let popover = gtk::Popover::builder().child(&column).build();
    {
        let preview = preview.clone();
        popover.connect_closed(move |_| preview(None));
    }
    button.set_popover(Some(&popover));
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for hex in ["#e8605a", "#3584e4", "#000000", "#ffffff", "#808080", "#33d17a"] {
            let c: Color = hex.parse().expect("hex");
            let (h, sat, v) = to_hsv(c);
            assert_eq!(from_hsv(h, sat, v), c, "{hex}");
        }
        assert_eq!(from_hsv(0.0, 1.0, 1.0), Color::rgb(255, 0, 0));
        assert_eq!(from_hsv(120.0, 1.0, 1.0), Color::rgb(0, 255, 0));
        assert_eq!(from_hsv(240.0, 1.0, 0.5), Color::rgb(0, 0, 128));
    }

    #[test]
    fn colors_convert() {
        let c = Color::rgb(0x35, 0x84, 0xe4);
        assert_eq!(color(&rgba(c)), c);
        assert!(PALETTE.iter().all(|h| h.parse::<Color>().is_ok()));
    }
}
