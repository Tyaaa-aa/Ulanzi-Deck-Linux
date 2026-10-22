//! The device drawing: 13 keys, the wide screen, two small buttons and three dials, each showing
//! exactly what the deck shows. Click to select (Ctrl/Shift for several keys).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
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

/// Where on a dial something was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialZone {
    Left,
    Press,
    Right,
}

/// Left third = turn left, right third = turn right, middle = press.
pub fn hit_zone(width: f64, x: f64) -> DialZone {
    if width <= 0.0 {
        DialZone::Press
    } else if x < width / 3.0 {
        DialZone::Left
    } else if x > width * 2.0 / 3.0 {
        DialZone::Right
    } else {
        DialZone::Press
    }
}

/// What is being dragged.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    Key(u8),
    Action(ulanzi_core::model::Action),
    /// An image file dropped from a file manager.
    File(std::path::PathBuf),
}

pub fn action_payload(a: &ulanzi_core::model::Action) -> String {
    format!("ulanzi-action:{}", serde_json::to_string(a).unwrap_or_default())
}

pub fn parse_payload(s: &str) -> Option<Payload> {
    if let Some(n) = s.strip_prefix("ulanzi-key:") {
        return n.parse().ok().map(Payload::Key);
    }
    let json = s.strip_prefix("ulanzi-action:")?;
    serde_json::from_str(json).ok().map(Payload::Action)
}

/// Accept dragged keys and actions on `w`; `on_drop` gets the payload and the drop x position.
fn accept_drops(w: &gtk::Button, on_drop: impl Fn(Payload, f64) + 'static) {
    let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY | gdk::DragAction::MOVE);
    target.set_types(&[glib::Type::STRING, gdk::FileList::static_type()]);
    {
        let w = w.clone();
        target.connect_enter(move |_, _, _| {
            w.add_css_class("drop-target");
            gdk::DragAction::COPY
        });
    }
    {
        let w = w.clone();
        target.connect_leave(move |_| w.remove_css_class("drop-target"));
    }
    {
        let w = w.clone();
        target.connect_drop(move |_, value, x, _| {
            w.remove_css_class("drop-target");
            let payload = match value.get::<gdk::FileList>() {
                Ok(files) => files.files().first().and_then(|f| f.path()).map(Payload::File),
                Err(_) => value.get::<String>().ok().and_then(|s| parse_payload(&s)),
            };
            match payload {
                Some(p) => {
                    on_drop(p, x);
                    true
                }
                None => false,
            }
        });
    }
    w.add_controller(target);
}

/// Make key `n` draggable (to swap it with another key).
fn drag_key(k: &KeyWidget, n: u8) {
    let source = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
    source.connect_prepare(move |_, _, _| {
        Some(gdk::ContentProvider::for_value(
            &format!("ulanzi-key:{n}").to_value(),
        ))
    });
    {
        let picture = k.picture.clone();
        source.connect_drag_begin(move |src, _| {
            if let Some(p) = picture.paintable() {
                src.set_icon(Some(&p), 30, 30);
            }
        });
    }
    k.button.add_controller(source);
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

/// `tex`, or a transparent stand-in of the same size, so empty keys keep their place in the grid.
fn or_blank(tex: Option<gdk::Texture>, w: i32, h: i32) -> gdk::Paintable {
    tex.map(|t| t.upcast())
        .unwrap_or_else(|| gdk::Paintable::new_empty(w, h))
}

/// Where the deck draws labels (`label_style.align` from Preferences).
fn label_valign(align: &str) -> gtk::Align {
    match align {
        "top" => gtk::Align::Start,
        "center" => gtk::Align::Center,
        _ => gtk::Align::End,
    }
}

/// The icons drawn on a dial.
struct DialFace {
    overlay: gtk::Overlay,
    press: gtk::Image,
    left: (gtk::Box, gtk::Image),
    right: (gtk::Box, gtk::Image),
}

fn dial_face(zones: &gtk::DrawingArea) -> DialFace {
    let overlay = gtk::Overlay::builder().child(zones).build();
    let press = gtk::Image::builder()
        .pixel_size(22)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .can_target(false)
        .build();
    overlay.add_overlay(&press);
    let badge = |h: gtk::Align| {
        let img = gtk::Image::builder().pixel_size(12).build();
        let b = gtk::Box::builder()
            .css_classes(["dial-badge"])
            .halign(h)
            .valign(gtk::Align::End)
            .can_target(false)
            .build();
        b.append(&img);
        overlay.add_overlay(&b);
        (b, img)
    };
    let left = badge(gtk::Align::Start);
    let right = badge(gtk::Align::End);
    DialFace {
        overlay,
        press,
        left,
        right,
    }
}

impl DialFace {
    fn show(&self, d: &ulanzi_core::model::Dial) {
        let set = |img: &gtk::Image, a: &Option<ulanzi_core::model::Action>, px: i32| match a
            .as_ref()
            .and_then(|a| action_icon(a, px))
        {
            Some(t) => {
                img.set_paintable(Some(&t));
                true
            }
            None => {
                img.set_paintable(None::<&gdk::Paintable>);
                false
            }
        };
        set(&self.press, &d.press, 22);
        self.left.0.set_visible(set(&self.left.1, &d.left, 12));
        self.right.0.set_visible(set(&self.right.1, &d.right, 12));
    }

    fn set_icons_visible(&self, v: bool) {
        let o = if v { 1.0 } else { 0.0 };
        self.press.set_opacity(o);
        self.left.0.set_opacity(o);
        self.right.0.set_opacity(o);
    }
}

type IconKey = (String, i32);

thread_local! {
    static ACTION_ICONS: RefCell<HashMap<IconKey, Option<gdk::Texture>>> = RefCell::new(HashMap::new());
}

/// An action's icon in its category colour, for small places like the dials.
pub fn action_icon(a: &ulanzi_core::model::Action, px: i32) -> Option<gdk::Texture> {
    let spec = ulanzi_core::catalog::spec_for(a)?;
    ACTION_ICONS.with(|m| {
        m.borrow_mut()
            .entry((spec.icon.to_string(), px))
            .or_insert_with(|| {
                ulanzi_core::render::IconLibrary::bundled()
                    .icon_png(
                        ulanzi_core::model::IconSet::Lucide,
                        spec.icon,
                        (px * 2) as u32,
                        spec.category.color(),
                    )
                    .and_then(|png| texture(&png))
            })
            .clone()
    })
}

/// "Turn left: …" lines for a dial's tooltip.
fn dial_tooltip(n: usize, d: &ulanzi_core::model::Dial) -> String {
    let line = |what: &str, a: &Option<ulanzi_core::model::Action>| {
        let text = a
            .as_ref()
            .map(ulanzi_core::catalog::describe)
            .unwrap_or_else(|| "Nothing".into());
        format!("{what}: {}", glib::markup_escape_text(&text))
    };
    format!(
        "<b>Dial {}</b>\n{}\n{}\n{}\n{}",
        n + 1,
        line("Turn left", &d.left),
        line("Press", &d.press),
        line("Turn right", &d.right),
        line("Hold", &d.hold),
    )
}

const HINT: &str = "Right-click a key to copy or paste it. The ‹ › buttons flip pages on the deck.";

/// The three drop zones of a dial: left third, middle, right third (as [`hit_zone`]).
fn draw_dial_zones(cr: &gtk::cairo::Context, w: f64, h: f64, hover: Option<DialZone>) {
    use std::f64::consts::PI;
    let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0 - 1.0);
    let accent = adw::StyleManager::default().accent_color_rgba();
    cr.save().ok();
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    cr.clip();
    for (i, z) in [DialZone::Left, DialZone::Press, DialZone::Right]
        .iter()
        .enumerate()
    {
        let x0 = cx - r + i as f64 * (2.0 * r / 3.0);
        cr.rectangle(x0, cy - r, 2.0 * r / 3.0, 2.0 * r);
        if hover == Some(*z) {
            cr.set_source_rgba(
                accent.red() as f64,
                accent.green() as f64,
                accent.blue() as f64,
                0.75,
            );
        } else {
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.07);
        }
        let _ = cr.fill();
    }
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
    cr.set_line_width(1.0);
    for i in 1..3 {
        let x = cx - r + i as f64 * (2.0 * r / 3.0);
        cr.move_to(x, cy - r);
        cr.line_to(x, cy + r);
    }
    let _ = cr.stroke();
    cr.restore().ok();
    // Glyphs: ‹ turn left, ● press, › turn right.
    let s = r * 0.18;
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
    cr.set_line_width(2.0);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    let lx = cx - r * 2.0 / 3.0;
    cr.move_to(lx + s * 0.5, cy - s);
    cr.line_to(lx - s * 0.5, cy);
    cr.line_to(lx + s * 0.5, cy + s);
    let rx = cx + r * 2.0 / 3.0;
    cr.move_to(rx - s * 0.5, cy - s);
    cr.line_to(rx + s * 0.5, cy);
    cr.line_to(rx - s * 0.5, cy + s);
    let _ = cr.stroke();
    cr.arc(cx, cy, s * 0.6, 0.0, 2.0 * PI);
    let _ = cr.fill();
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
        ScreenConfig::Custom { widgets, .. }
            if matches!(widgets.first(), Some(Widget::NowPlaying { .. })) =>
        {
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

/// The canvas widget and a switch for its compact size (narrow windows).
pub struct Canvas {
    pub widget: gtk::Widget,
    pub set_compact: Rc<dyn Fn(bool)>,
}

pub const COMPACT_KEY_PX: i32 = 62;

pub fn build(state: &State) -> Canvas {
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
    // Each dial draws its three drop zones (turn left, press, turn right) while an action is dragged.
    let zones: Rc<Vec<Rc<std::cell::Cell<Option<DialZone>>>>> =
        Rc::new((0..3).map(|_| Rc::new(std::cell::Cell::new(None))).collect());
    let zone_areas: Rc<Vec<gtk::DrawingArea>> = Rc::new(
        (0..3)
            .map(|n| {
                let area = gtk::DrawingArea::new();
                let (hover, state) = (zones[n].clone(), state.clone());
                area.set_draw_func(move |_, cr, w, h| {
                    if state.dragging_action.get() {
                        draw_dial_zones(cr, w as f64, h as f64, hover.get());
                    }
                });
                area
            })
            .collect(),
    );
    // At a glance: the press action in the middle, turn left / right as badges on the rim.
    let faces: Rc<Vec<DialFace>> = Rc::new((0..3).map(|n| dial_face(&zone_areas[n])).collect());
    let dials: Rc<Vec<gtk::Button>> = Rc::new(
        (0..3)
            .map(|n| {
                gtk::Button::builder()
                    .css_classes(["deck-dial"])
                    .tooltip_text(format!("Dial {}", n + 1))
                    .valign(gtk::Align::Center)
                    .child(&faces[n].overlay)
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
        .label(HINT)
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
    // Drag and drop.
    for (n, k) in keys.iter().enumerate() {
        let n = n as u8;
        drag_key(k, n);
        let state = state.clone();
        accept_drops(&k.button, move |p, _| match p {
            Payload::Key(from) if from != n => {
                state.edit(|e| e.swap(from, n));
                state.select(|s| s.click(Slot::Key(n)));
            }
            Payload::Key(_) => {}
            Payload::Action(a) => {
                state.edit(|e| e.drop_action(Slot::Key(n), a));
                state.select(|s| s.click(Slot::Key(n)));
            }
            Payload::File(path) if path.extension().is_some_and(|e| e == "ulanzideck") => {
                if let Some(root) = state.toast_root() {
                    crate::ui::bundle_dialogs::open_import(&state, &root, &path);
                }
            }
            Payload::File(path) => {
                // Dropping on a selected key sets the image on the whole selection.
                let sel = state.selection.borrow().keys();
                let keys = if sel.contains(&n) { sel } else { vec![n] };
                crate::ui::icon_picker::import_into(&state, &path, &keys, |_| {});
            }
        });
    }
    {
        let state = state.clone();
        accept_drops(&screen.0.button, move |p, _| {
            if let Payload::Action(a) = p {
                state.edit(|e| e.drop_action(Slot::Screen, a));
                state.select(|s| s.click(Slot::Screen));
            }
        });
    }
    for (n, b) in small.iter().enumerate() {
        let state = state.clone();
        accept_drops(b, move |p, _| {
            if let Payload::Action(a) = p {
                state.edit(|e| e.drop_action(Slot::Small(n as u8), a));
                state.select(|s| s.click(Slot::Small(n as u8)));
            }
        });
    }
    for (n, d) in dials.iter().enumerate() {
        // Show which zone the pointer is over while dragging across the dial.
        let motion = gtk::DropControllerMotion::new();
        {
            let (zone, area, dial, hint) = (zones[n].clone(), zone_areas[n].clone(), d.clone(), hint.clone());
            motion.connect_motion(move |_, x, _| {
                let z = hit_zone(dial.width() as f64, x);
                if zone.get() != Some(z) {
                    zone.set(Some(z));
                    area.queue_draw();
                    hint.set_label(match z {
                        DialZone::Left => "Drop here: runs when the dial turns left",
                        DialZone::Press => "Drop here: runs when the dial is pressed",
                        DialZone::Right => "Drop here: runs when the dial turns right",
                    });
                }
            });
        }
        {
            let (zone, area) = (zones[n].clone(), zone_areas[n].clone());
            motion.connect_leave(move |_| {
                zone.set(None);
                area.queue_draw();
            });
        }
        d.add_controller(motion);
        let state = state.clone();
        let dial = d.clone();
        accept_drops(d, move |p, x| {
            if let Payload::Action(a) = p {
                let zone = hit_zone(dial.width() as f64, x);
                state.edit(|e| {
                    e.set_dial(n as u8, |d| match zone {
                        DialZone::Left => d.left = Some(a),
                        DialZone::Right => d.right = Some(a),
                        DialZone::Press => d.press = Some(a),
                    })
                });
                state.select(|s| s.click(Slot::Dial(n as u8)));
            }
        });
    }

    // Right-click a key: copy, paste and clear, with their shortcuts shown.
    let key_menu = {
        let m = gio::Menu::new();
        let keys_section = gio::Menu::new();
        keys_section.append(Some("Copy Key"), Some("win.copy"));
        keys_section.append(Some("Paste Key"), Some("win.paste"));
        m.append_section(None, &keys_section);
        let style = gio::Menu::new();
        style.append(Some("Copy Style"), Some("win.copy-style"));
        style.append(Some("Paste Style"), Some("win.paste-style"));
        m.append_section(Some("Colours, image and label only"), &style);
        let clear = gio::Menu::new();
        clear.append(Some("Clear Key"), Some("win.clear"));
        m.append_section(None, &clear);
        m
    };
    for (n, k) in keys.iter().enumerate() {
        let pop = gtk::PopoverMenu::from_model(Some(&key_menu));
        pop.set_has_arrow(true);
        pop.set_parent(&k.button);
        let right = gtk::GestureClick::builder().button(3).build();
        let state = state.clone();
        right.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            // Act on the key under the pointer (keep a multi-selection that includes it).
            let slot = Slot::Key(n as u8);
            if !state.selection.borrow().contains(slot) {
                state.select(|s| s.click(slot));
            }
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        });
        k.button.add_controller(right);
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
        let (state, keys, screen, small, dials, dots, faces) = (
            state.clone(),
            keys.clone(),
            screen.clone(),
            small.clone(),
            dials.clone(),
            dots.clone(),
            faces.clone(),
        );
        move |c: Change| match c {
            Change::Profile | Change::Page | Change::Settings | Change::Preview => {
                let dir = profile_dir(&state);
                let hide = state.settings.borrow().hide_labels;
                let align = state.settings.borrow().label_style.align.clone();
                let preview = state.preview.borrow().clone();
                let sel = state.selection.borrow().clone();
                let e = state.editor.borrow();
                let page = &e.profile().pages[e.page()];
                for (i, k) in keys.iter().enumerate() {
                    // A hovered colour shows on the selected keys before it is applied.
                    let previewed = preview.as_ref().filter(|_| sel.contains(Slot::Key(i as u8)));
                    let owned;
                    let key = match (page.keys.get(&(i as u8)), previewed) {
                        (k, Some(f)) => {
                            let mut key = k.cloned().unwrap_or_default();
                            ulanzi_core::edit::apply_field(&mut key.look, f);
                            owned = key;
                            Some(&owned)
                        }
                        (k, None) => k,
                    };
                    match key {
                        Some(key) => {
                            k.picture.set_paintable(Some(&or_blank(
                                key_texture(&key.look, &dir),
                                ulanzi_core::render::KEY_PX as i32,
                                ulanzi_core::render::KEY_PX as i32,
                            )));
                            let text = &key.look.label.text;
                            k.label.set_label(text);
                            // A label with its own position is part of the key image.
                            let own = key.look.label.position.is_some();
                            k.label.set_valign(label_valign(&align));
                            k.label
                                .set_visible(!hide && !own && key.look.label.visible && !text.is_empty());
                            k.button.set_tooltip_text(
                                key.tap
                                    .as_ref()
                                    .map(ulanzi_core::catalog::describe)
                                    .as_deref()
                                    .or(Some("No action")),
                            );
                        }
                        None => {
                            k.picture.set_paintable(Some(&or_blank(
                                None,
                                ulanzi_core::render::KEY_PX as i32,
                                ulanzi_core::render::KEY_PX as i32,
                            )));
                            k.label.set_visible(false);
                            k.button
                                .set_tooltip_text(Some("Empty key: drop an action or an image here"));
                        }
                    }
                }
                // Dials of this page.
                for (n, (face, button)) in faces.iter().zip(dials.iter()).enumerate() {
                    let d = &e.dials()[n];
                    face.show(d);
                    button.set_tooltip_markup(Some(&dial_tooltip(n, d)));
                }
                drop(e);
                screen
                    .0
                    .picture
                    .set_paintable(Some(&or_blank(screen_texture(&state), 458, 196)));
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
            Change::Drag => {
                for a in zone_areas.iter() {
                    a.queue_draw();
                }
                // The zones replace the icons while dragging.
                for f in faces.iter() {
                    f.set_icons_visible(!state.dragging_action.get());
                }
                hint.set_label(if state.dragging_action.get() {
                    "Drop on a key, or on a dial: left side = turn left, middle = press, right side = turn right"
                } else {
                    HINT
                });
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
    let set_compact: Rc<dyn Fn(bool)> = {
        let (keys, screen, body, grid) = (keys.clone(), screen.clone(), body.clone(), grid.clone());
        Rc::new(move |compact: bool| {
            let (px, gap) = if compact {
                (COMPACT_KEY_PX, 8)
            } else {
                (KEY_PX, GAP)
            };
            for k in keys.iter() {
                k.button.set_size_request(px, px);
                k.picture.set_size_request(px - 12, px - 12);
            }
            screen.0.button.set_size_request(px * 2 + gap, px);
            screen.0.picture.set_size_request(px * 2 + gap - 12, px - 12);
            grid.set_row_spacing(gap as u32);
            grid.set_column_spacing(gap as u32);
            set_class(&body, "compact", compact);
        })
    };
    Canvas {
        widget: scroller.upcast(),
        set_compact,
    }
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
    fn dial_hit_zones() {
        assert_eq!(hit_zone(76.0, 5.0), DialZone::Left);
        assert_eq!(hit_zone(76.0, 38.0), DialZone::Press);
        assert_eq!(hit_zone(76.0, 70.0), DialZone::Right);
        assert_eq!(hit_zone(0.0, 0.0), DialZone::Press);
    }

    #[test]
    fn drag_payloads_roundtrip() {
        assert_eq!(parse_payload("ulanzi-key:7"), Some(Payload::Key(7)));
        let a = ulanzi_core::model::Action::NextPage;
        assert_eq!(parse_payload(&action_payload(&a)), Some(Payload::Action(a)));
        assert_eq!(parse_payload("hello"), None);
    }

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
