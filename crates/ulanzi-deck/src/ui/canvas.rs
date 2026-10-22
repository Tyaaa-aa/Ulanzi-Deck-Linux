//! The device drawing: 13 keys, the wide screen, two small buttons and three dials, each showing
//! exactly what the deck shows. Click to select (Ctrl/Shift for several keys).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::edit::Slot;
use ulanzi_core::model::{KeyLook, ScreenConfig, Widget};
use ulanzi_core::render::{
    Fonts, IconLibrary, NowPlaying, RenderCtx, look_hash, render_key, render_now_playing,
};
use ulanzi_core::store::Store;

use crate::state::{Change, State};

pub const KEY_PX: i32 = 88;
const GAP: i32 = 12;

/// How a click changes the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectOp {
    Only,
    Toggle,
    Range,
}

pub fn select_op(m: gdk::ModifierType) -> SelectOp {
    if m.contains(gdk::ModifierType::SHIFT_MASK) {
        SelectOp::Range
    } else if m.contains(gdk::ModifierType::CONTROL_MASK) {
        SelectOp::Toggle
    } else {
        SelectOp::Only
    }
}

fn modifiers(w: &impl IsA<gtk::Widget>) -> gdk::ModifierType {
    w.display()
        .default_seat()
        .and_then(|s| s.keyboard())
        .map(|k| k.modifier_state())
        .unwrap_or_else(gdk::ModifierType::empty)
}

thread_local! {
    static TEXTURES: RefCell<HashMap<u64, Option<gdk::Texture>>> = RefCell::new(HashMap::new());
}

fn texture(png: &[u8]) -> Option<gdk::Texture> {
    gdk::Texture::from_bytes(&glib::Bytes::from(png)).ok()
}

/// The texture for a key look (cached), or None for an empty key.
pub fn key_texture(look: &KeyLook, profile_dir: &std::path::Path) -> Option<gdk::Texture> {
    let h = look_hash(look, profile_dir);
    TEXTURES.with(|t| {
        t.borrow_mut()
            .entry(h)
            .or_insert_with(|| {
                let ctx = RenderCtx {
                    profile_dir,
                    icons: IconLibrary::bundled(),
                };
                render_key(look, &ctx).and_then(|r| texture(&r.png))
            })
            .clone()
    })
}

pub fn profile_dir(state: &State) -> std::path::PathBuf {
    Store::xdg()
        .map(|s| s.profile_dir(state.profile_id()))
        .unwrap_or_default()
}

struct KeyWidget {
    button: gtk::Button,
    picture: gtk::Picture,
    label: gtk::Label,
}

fn key_widget(width: i32) -> KeyWidget {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .build();
    picture.set_size_request(width - 12, KEY_PX - 12);
    let label = gtk::Label::builder()
        .css_classes(["deck-key-label"])
        .valign(gtk::Align::End)
        .halign(gtk::Align::Center)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(12)
        .build();
    let overlay = gtk::Overlay::builder().child(&picture).build();
    overlay.add_overlay(&label);
    let button = gtk::Button::builder()
        .child(&overlay)
        .css_classes(["deck-key"])
        .build();
    button.set_size_request(width, KEY_PX);
    KeyWidget {
        button,
        picture,
        label,
    }
}

fn flash(w: &impl IsA<gtk::Widget>) {
    let w = w.clone().upcast::<gtk::Widget>();
    w.add_css_class("flash");
    glib::timeout_add_local_once(std::time::Duration::from_millis(180), move || {
        w.remove_css_class("flash")
    });
}

fn screen_texture(state: &State) -> Option<gdk::Texture> {
    thread_local! {
        static FONTS: RefCell<Option<Fonts>> = const { RefCell::new(None) };
    }
    let screen = state.editor.borrow().profile().screen.clone();
    let png = match screen {
        ScreenConfig::Custom { widgets, .. } if widgets.first() == Some(&Widget::NowPlaying) => {
            FONTS.with(|f| {
                let mut f = f.borrow_mut();
                let fonts = f.get_or_insert_with(Fonts::system);
                let np = NowPlaying {
                    title: "Now Playing".into(),
                    artist: "Shows the current song".into(),
                };
                render_now_playing(&np, None, fonts, (458, 196))
            })
        }
        _ => return None,
    };
    texture(&png)
}

pub fn build(state: &State) -> gtk::Widget {
    let keys: Rc<Vec<KeyWidget>> = Rc::new((0..13).map(|_| key_widget(KEY_PX)).collect());
    let screen = key_widget(KEY_PX * 2 + GAP);
    screen.button.remove_css_class("deck-key");
    screen.button.add_css_class("deck-screen");
    let screen_clock = gtk::Label::builder().css_classes(["title-1"]).build();
    screen_clock.add_css_class("deck-key-label");
    let screen = Rc::new((screen, screen_clock));

    let grid = gtk::Grid::builder().row_spacing(GAP).column_spacing(GAP).build();
    let mut i = 0;
    for row in 0..3 {
        for col in 0..5 {
            if row == 2 && col == 3 {
                grid.attach(&screen.0.button, 3, 2, 2, 1);
                break;
            }
            grid.attach(&keys[i].button, col, row, 1, 1);
            i += 1;
        }
    }

    let small: Rc<[gtk::Button; 2]> = Rc::new([
        gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .css_classes(["deck-small"])
            .tooltip_text("Left small button")
            .valign(gtk::Align::Center)
            .build(),
        gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .css_classes(["deck-small"])
            .tooltip_text("Right small button")
            .valign(gtk::Align::Center)
            .build(),
    ]);
    let dials: Rc<Vec<gtk::Button>> = Rc::new(
        (0..3)
            .map(|n| {
                gtk::Button::builder()
                    .css_classes(["deck-dial"])
                    .tooltip_text(format!("Dial {}", n + 1))
                    .valign(gtk::Align::Center)
                    .build()
            })
            .collect(),
    );
    let bottom = gtk::Box::builder().spacing(GAP).margin_top(26).build();
    bottom.append(&small[0]);
    bottom.append(&small[1]);
    let spacer = gtk::Box::builder().hexpand(true).build();
    bottom.append(&spacer);
    for d in dials.iter() {
        bottom.append(d);
    }
    let dial_box = gtk::Box::builder().spacing(36).build();
    for d in dials.iter() {
        bottom.remove(d);
        dial_box.append(d);
    }
    bottom.append(&dial_box);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["deck-body"])
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    body.append(&grid);
    body.append(&bottom);

    let dots = gtk::Box::builder()
        .spacing(9)
        .halign(gtk::Align::Center)
        .margin_top(28)
        .build();
    let hint = gtk::Label::builder()
        .label("The ‹ › buttons flip pages on the deck. Drop an image on a key to use it.")
        .css_classes(["hint"])
        .margin_top(14)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();

    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    column.append(&body);
    column.append(&dots);
    column.append(&hint);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&column)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroller.add_css_class("canvas-area");

    // Clicks.
    let click = |state: &State, slot: Slot, w: &gtk::Button| {
        let op = select_op(modifiers(w));
        state.select(|s| match op {
            SelectOp::Only => s.click(slot),
            SelectOp::Toggle => s.toggle(slot),
            SelectOp::Range => s.range_to(slot),
        });
    };
    for (n, k) in keys.iter().enumerate() {
        let state = state.clone();
        k.button
            .connect_clicked(move |b| click(&state, Slot::Key(n as u8), b));
    }
    {
        let state = state.clone();
        screen
            .0
            .button
            .connect_clicked(move |b| click(&state, Slot::Screen, b));
    }
    for (n, b) in small.iter().enumerate() {
        let state = state.clone();
        b.connect_clicked(move |b| click(&state, Slot::Small(n as u8), b));
    }
    for (n, d) in dials.iter().enumerate() {
        let state = state.clone();
        d.connect_clicked(move |b| click(&state, Slot::Dial(n as u8), b));
    }
    // Clicking the background clears the selection.
    {
        let state = state.clone();
        let g = gtk::GestureClick::new();
        g.connect_released(move |g, _, _, _| {
            if g.widget().is_some() {
                state.select(|s| s.clear());
            }
        });
        g.set_propagation_phase(gtk::PropagationPhase::Bubble);
        scroller.add_controller(g);
    }

    // Redraw.
    let redraw = {
        let (state, keys, screen, small, dials, dots) = (
            state.clone(),
            keys.clone(),
            screen.clone(),
            small.clone(),
            dials.clone(),
            dots.clone(),
        );
        move |c: Change| match c {
            Change::Profile | Change::Page | Change::Settings => {
                let dir = profile_dir(&state);
                let hide = state.settings.borrow().hide_labels;
                let e = state.editor.borrow();
                let page = &e.profile().pages[e.page()];
                for (i, k) in keys.iter().enumerate() {
                    match page.keys.get(&(i as u8)) {
                        Some(key) => {
                            k.picture.set_paintable(key_texture(&key.look, &dir).as_ref());
                            let text = &key.look.label.text;
                            k.label.set_label(text);
                            k.label
                                .set_visible(!hide && key.look.label.visible && !text.is_empty());
                            k.button.set_tooltip_text(
                                key.tap
                                    .as_ref()
                                    .map(ulanzi_core::catalog::describe)
                                    .as_deref()
                                    .or(Some("No action")),
                            );
                        }
                        None => {
                            k.picture.set_paintable(None::<&gdk::Paintable>);
                            k.label.set_visible(false);
                            k.button
                                .set_tooltip_text(Some("Empty key: drop an action or an image here"));
                        }
                    }
                }
                drop(e);
                screen.0.picture.set_paintable(screen_texture(&state).as_ref());
                screen.0.label.set_visible(false);
                // Page dots.
                while let Some(c) = dots.first_child() {
                    dots.remove(&c);
                }
                let e = state.editor.borrow();
                for (i, p) in e.profile().pages.iter().enumerate() {
                    if p.kind != ulanzi_core::model::PageKind::Page && i != e.page() {
                        continue;
                    }
                    let dot = gtk::Box::builder()
                        .css_classes(["page-dot"])
                        .valign(gtk::Align::Center)
                        .build();
                    if i == e.page() {
                        dot.add_css_class("current");
                    }
                    dots.append(&dot);
                }
            }
            Change::Selection => {
                let sel = state.selection.borrow();
                for (i, k) in keys.iter().enumerate() {
                    set_class(&k.button, "selected", sel.contains(Slot::Key(i as u8)));
                }
                set_class(&screen.0.button, "selected", sel.contains(Slot::Screen));
                for (i, b) in small.iter().enumerate() {
                    set_class(b, "selected", sel.contains(Slot::Small(i as u8)));
                }
                for (i, d) in dials.iter().enumerate() {
                    set_class(d, "selected", sel.contains(Slot::Dial(i as u8)));
                }
            }
            Change::Pressed(slot) => match slot {
                Slot::Key(i) => {
                    if let Some(k) = keys.get(i as usize) {
                        flash(&k.button);
                    }
                }
                Slot::Screen => flash(&screen.0.button),
                Slot::Small(i) => {
                    if let Some(b) = small.get(i as usize) {
                        flash(b);
                    }
                }
                Slot::Dial(i) => {
                    if let Some(d) = dials.get(i as usize) {
                        flash(d);
                    }
                }
            },
            _ => {}
        }
    };
    redraw(Change::Profile);
    redraw(Change::Selection);
    state.on(redraw);
    scroller.upcast()
}

fn set_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_to_selection_op() {
        assert_eq!(select_op(gdk::ModifierType::empty()), SelectOp::Only);
        assert_eq!(select_op(gdk::ModifierType::CONTROL_MASK), SelectOp::Toggle);
        assert_eq!(
            select_op(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK),
            SelectOp::Range
        );
    }
}
