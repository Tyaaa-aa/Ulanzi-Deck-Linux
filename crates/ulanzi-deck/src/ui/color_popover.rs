//! A colour button whose popover previews colours live: hovering a swatch shows it on the selected
//! keys, clicking applies it and keeps the popover open so you can compare.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio};
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
    let more = gtk::Button::builder()
        .label("More…")
        .tooltip_text("The full colour picker")
        .build();
    let row = gtk::Box::builder().spacing(6).build();
    row.append(&hex);
    row.append(&more);
    column.append(&row);

    let popover = gtk::Popover::builder().child(&column).build();
    {
        let preview = preview.clone();
        popover.connect_closed(move |_| preview(None));
    }
    {
        let (apply, face_value, popover) = (apply.clone(), face_value.clone(), popover.clone());
        more.connect_clicked(move |b| {
            popover.popdown();
            let dialog = gtk::ColorDialog::builder().with_alpha(false).build();
            let window = b.root().and_downcast::<gtk::Window>();
            let initial = face_value.get().map(rgba);
            let apply = apply.clone();
            dialog.choose_rgba(
                window.as_ref(),
                initial.as_ref(),
                None::<&gio::Cancellable>,
                move |r| {
                    if let Ok(c) = r {
                        apply(color(&c));
                    }
                },
            );
        });
    }
    button.set_popover(Some(&popover));
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_convert() {
        let c = Color::rgb(0x35, 0x84, 0xe4);
        assert_eq!(color(&rgba(c)), c);
        assert!(PALETTE.iter().all(|h| h.parse::<Color>().is_ok()));
    }
}
