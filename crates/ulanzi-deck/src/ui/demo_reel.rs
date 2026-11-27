//! A scripted tour of the app, recorded to video for the project's demo. Run in a sandbox with a
//! pretend deck (`scripts/demo-reel.sh` sets one up):
//!
//! ```text
//! scripts/sandbox.sh --demo --dir BOX -- --screenshot /dev/null --size 1720x968 \
//!     --demo-reel OUT.mp4 --scene keys
//! ```
//!
//! The window draws itself into frames ([`gtk::WidgetPaintable`]), so nothing else on the screen
//! is in the video. A pointer, click rings and the carried item are drawn on top, where the script
//! moves them. The script takes the same steps the app's handlers take (select, drop, carry,
//! hover); GTK's own event delivery is not part of it. Frames go to `ffmpeg` at 30 per second of
//! real time: a slow frame is repeated, so animations keep their speed.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib, graphene, gsk};
use ulanzi_core::edit::Slot;
use uuid::Uuid;

use crate::state::{Change, State};
use crate::ui::canvas::{Payload, parse_payload};
use crate::ui::library_tray::{Tab, Tray};

const FPS: f64 = 30.0;
const OUT: (i32, i32) = (1920, 1080);

type Frame = (Arc<Vec<u8>>, u64);

/// The recorder: frames of the window, with the pretend pointer on top.
pub struct Rec {
    window: gtk::Window,
    paint: gtk::WidgetPaintable,
    scale: f32,
    tx: RefCell<Option<mpsc::SyncSender<Frame>>>,
    writer: RefCell<Option<std::thread::JoinHandle<bool>>>,
    start: Cell<Option<Instant>>,
    written: Cell<u64>,
    cursor: Cell<(f64, f64)>,
    cursor_on: Cell<bool>,
    pressed: Cell<bool>,
    ripple: Cell<Option<((f64, f64), Instant)>>,
    /// What is carried under the pointer, and its size.
    carried: RefCell<Option<(gdk::Paintable, f64, f64)>>,
    /// A line of text at the top, for what happens outside the app (the app in front).
    banner: RefCell<Option<(String, Instant)>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
}

/// How fast the script goes: `ULANZI_REEL_TEMPO=0.8` shortens its pauses and pointer moves to
/// 80% (the app's own animations keep their speed).
fn tempo() -> f64 {
    std::env::var("ULANZI_REEL_TEMPO")
        .ok()
        .and_then(|t| t.parse::<f64>().ok())
        .filter(|t| *t > 0.1 && *t <= 2.0)
        .unwrap_or(1.0)
}

fn paced(ms: u64) -> u64 {
    (ms as f64 * tempo()).round() as u64
}

pub async fn wait(ms: u64) {
    glib::timeout_future(Duration::from_millis(paced(ms))).await;
}

fn ease(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

impl Rec {
    pub fn new(window: &gtk::Window) -> Rc<Rec> {
        let w = window.width().max(1) as f32;
        Rc::new(Rec {
            window: window.clone(),
            paint: gtk::WidgetPaintable::new(Some(window)),
            scale: OUT.0 as f32 / w,
            tx: RefCell::default(),
            writer: RefCell::default(),
            start: Cell::default(),
            written: Cell::new(0),
            cursor: Cell::new((window.width() as f64 * 0.55, window.height() as f64 * 0.6)),
            cursor_on: Cell::new(true),
            pressed: Cell::new(false),
            ripple: Cell::default(),
            carried: RefCell::default(),
            banner: RefCell::default(),
            tick: RefCell::default(),
        })
    }

    /// Start writing frames to `path` (an `.mp4`).
    pub fn start(self: &Rc<Self>, path: &Path) -> std::io::Result<()> {
        let mut child = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-s",
            ])
            .arg(format!("{}x{}", OUT.0, OUT.1))
            .args([
                "-r", "30", "-i", "-", "-c:v", "libx264", "-preset", "fast", "-crf", "14",
            ])
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(path)
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let (tx, rx) = mpsc::sync_channel::<Frame>(8);
        let writer = std::thread::spawn(move || {
            let mut ok = true;
            for (frame, n) in rx {
                for _ in 0..n {
                    if stdin.write_all(&frame).is_err() {
                        ok = false;
                    }
                }
            }
            drop(stdin);
            child.wait().is_ok_and(|s| s.success()) && ok
        });
        *self.tx.borrow_mut() = Some(tx);
        *self.writer.borrow_mut() = Some(writer);
        self.written.set(0);
        self.start.set(Some(Instant::now()));
        let me = Rc::downgrade(self);
        let id = self.window.add_tick_callback(move |_, _| {
            if let Some(r) = me.upgrade() {
                r.on_tick();
            }
            glib::ControlFlow::Continue
        });
        *self.tick.borrow_mut() = Some(id);
        Ok(())
    }

    /// Stop and finish the file; true when ffmpeg was happy.
    pub fn stop(&self) -> bool {
        if let Some(id) = self.tick.take() {
            id.remove();
        }
        self.start.set(None);
        self.tx.take();
        self.writer.take().is_some_and(|w| w.join().unwrap_or(false))
    }

    fn on_tick(&self) {
        let Some(start) = self.start.get() else { return };
        let due = (start.elapsed().as_secs_f64() * FPS).floor() as u64 + 1;
        let n = due.saturating_sub(self.written.get());
        if n == 0 {
            return;
        }
        let Some(frame) = self.grab() else { return };
        if let Some(tx) = self.tx.borrow().as_ref() {
            tx.send((Arc::new(frame), n.min(30))).ok();
        }
        self.written.set(due);
    }

    /// Where `w` is drawn in the frame's window coordinates, also inside a popover.
    pub fn bounds(&self, w: &impl IsA<gtk::Widget>) -> Option<graphene::Rect> {
        let w = w.as_ref();
        if let Some(p) = w
            .ancestor(gtk::Popover::static_type())
            .and_downcast::<gtk::Popover>()
        {
            let b = w.compute_bounds(&p)?;
            let (x, y) = self.popover_at(&p)?;
            return Some(graphene::Rect::new(b.x() + x, b.y() + y, b.width(), b.height()));
        }
        w.compute_bounds(&self.window)
    }

    pub fn center(&self, w: &impl IsA<gtk::Widget>) -> (f64, f64) {
        self.bounds(w).map_or(self.cursor.get(), |b| {
            (
                (b.x() + b.width() / 2.0) as f64,
                (b.y() + b.height() / 2.0) as f64,
            )
        })
    }

    /// Where popover `p` sits over the window, as the compositor put it.
    fn popover_at(&self, p: &gtk::Popover) -> Option<(f32, f32)> {
        let popup = p.surface().and_downcast::<gdk::Popup>()?;
        let (px, py) = p.surface_transform();
        let (wx, wy) = self.window.surface_transform();
        Some((
            (popup.position_x() as f64 + px - wx) as f32,
            (popup.position_y() as f64 + py - wy) as f32,
        ))
    }

    fn grab(&self) -> Option<Vec<u8>> {
        let (w, h) = (self.window.width() as f64, self.window.height() as f64);
        let snap = gtk::Snapshot::new();
        snap.scale(self.scale, self.scale);
        snap.append_color(
            &gdk::RGBA::BLACK,
            &graphene::Rect::new(0.0, 0.0, w as f32, OUT.1 as f32 / self.scale),
        );
        self.paint.snapshot(&snap, w, h);
        self.draw_popovers(&snap, self.window.upcast_ref());
        self.draw_banner(&snap, w);
        let (cx, cy) = self.cursor.get();
        if let Some((p, pw, ph)) = self.carried.borrow().as_ref() {
            snap.save();
            snap.translate(&graphene::Point::new(
                (cx - pw / 2.0) as f32,
                (cy - ph / 2.0) as f32,
            ));
            snap.push_opacity(0.9);
            p.snapshot(&snap, *pw, *ph);
            snap.pop();
            snap.restore();
        }
        if let Some(((rx, ry), at)) = self.ripple.get() {
            let t = at.elapsed().as_secs_f64() / 0.45;
            if t < 1.0 {
                let e = 1.0 - (1.0 - t).powi(3);
                let b = gsk::PathBuilder::new();
                b.add_circle(
                    &graphene::Point::new(rx as f32, ry as f32),
                    (8.0 + 22.0 * e) as f32,
                );
                let path = b.to_path();
                let a = (0.7 * (1.0 - t)) as f32;
                snap.append_stroke(&path, &gsk::Stroke::new(3.0), &gdk::RGBA::new(1.0, 0.47, 0.0, a));
            } else {
                self.ripple.set(None);
            }
        }
        if self.cursor_on.get() {
            draw_pointer(&snap, cx, cy, if self.pressed.get() { 1.15 } else { 1.3 });
        }
        let node = snap.to_node()?;
        let renderer = self.window.renderer()?;
        let tex = renderer.render_texture(
            &node,
            Some(&graphene::Rect::new(0.0, 0.0, OUT.0 as f32, OUT.1 as f32)),
        );
        let mut d = gdk::TextureDownloader::new(&tex);
        d.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = d.download_bytes();
        let (tw, th) = (tex.width() as usize, tex.height() as usize);
        let row = OUT.0 as usize * 4;
        let mut out = vec![0u8; row * OUT.1 as usize];
        for y in 0..th.min(OUT.1 as usize) {
            let n = (tw * 4).min(row);
            out[y * row..y * row + n].copy_from_slice(&bytes[y * stride..y * stride + n]);
        }
        Some(out)
    }

    fn draw_popovers(&self, snap: &gtk::Snapshot, root: &gtk::Widget) {
        let mut popovers = Vec::new();
        open_popovers(root, &mut popovers);
        for p in popovers {
            let Some((x, y)) = self.popover_at(&p) else {
                continue;
            };
            snap.save();
            snap.translate(&graphene::Point::new(x, y));
            gtk::WidgetPaintable::new(Some(&p)).snapshot(snap, p.width() as f64, p.height() as f64);
            snap.restore();
        }
    }

    fn draw_banner(&self, snap: &gtk::Snapshot, w: f64) {
        let Some((text, at)) = self.banner.borrow().clone() else {
            return;
        };
        let t = (at.elapsed().as_secs_f64() / 0.25).min(1.0);
        let layout = self.window.create_pango_layout(Some(&text));
        let mut font = gtk::pango::FontDescription::from_string("Sans Bold 15");
        font.set_weight(gtk::pango::Weight::Bold);
        layout.set_font_description(Some(&font));
        let (lw, lh) = layout.pixel_size();
        let (pad_x, pad_y) = (22.0, 12.0);
        let (bw, bh) = (lw as f32 + pad_x * 2.0, lh as f32 + pad_y * 2.0);
        let x = ((w as f32) - bw) / 2.0;
        let y = 64.0 - 16.0 * (1.0 - ease(t) as f32);
        snap.push_opacity(t);
        let shape = gsk::RoundedRect::from_rect(graphene::Rect::new(x, y, bw, bh), bh / 2.0);
        snap.append_outset_shadow(&shape, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.45), 0.0, 6.0, 0.0, 18.0);
        snap.push_rounded_clip(&shape);
        snap.append_color(
            &gdk::RGBA::new(1.0, 0.47, 0.0, 1.0),
            &graphene::Rect::new(x, y, bw, bh),
        );
        snap.pop();
        snap.save();
        snap.translate(&graphene::Point::new(x + pad_x, y + pad_y));
        snap.append_layout(&layout, &gdk::RGBA::WHITE);
        snap.restore();
        snap.pop();
    }

    /// Move the pointer to `to` over `ms`, calling `each` at every step. `to` is asked again at
    /// every step, so the pointer keeps up with something that moves (the deck sliding back into
    /// view as a carried tile leaves the tray).
    pub async fn glide_with(&self, to: impl Fn() -> (f64, f64), ms: u64, mut each: impl FnMut((f64, f64))) {
        let from = self.cursor.get();
        let t0 = Instant::now();
        loop {
            let t = (t0.elapsed().as_secs_f64() * 1000.0 / paced(ms).max(1) as f64).min(1.0);
            let e = ease(t);
            let to = to();
            let p = (from.0 + (to.0 - from.0) * e, from.1 + (to.1 - from.1) * e);
            self.cursor.set(p);
            each(p);
            if t >= 1.0 {
                break;
            }
            wait(12).await;
        }
    }

    pub async fn glide(&self, to: (f64, f64), ms: u64) {
        self.glide_with(move || to, ms, |_| {}).await;
    }

    pub async fn glide_to(&self, w: &impl IsA<gtk::Widget>, ms: u64) {
        self.glide(self.center(w), ms).await;
    }

    /// The press of a click: the ring, a short hold.
    pub async fn press(&self) {
        self.pressed.set(true);
        self.ripple.set(Some((self.cursor.get(), Instant::now())));
        wait(110).await;
    }

    pub fn release(&self) {
        self.pressed.set(false);
    }

    pub async fn click(&self) {
        self.press().await;
        self.release();
        wait(60).await;
    }

    pub fn carry(&self, what: Option<(gdk::Paintable, f64, f64)>) {
        *self.carried.borrow_mut() = what;
    }

    pub fn banner(&self, text: Option<&str>) {
        *self.banner.borrow_mut() = text.map(|t| (t.to_string(), Instant::now()));
    }
}

/// An arrow pointer at `(x, y)`, black with a white rim and a soft shadow.
fn draw_pointer(snap: &gtk::Snapshot, x: f64, y: f64, scale: f32) {
    let b = gsk::PathBuilder::new();
    let pts = [
        (0.0, 0.0),
        (0.0, 17.0),
        (4.2, 13.2),
        (7.0, 19.4),
        (9.9, 18.2),
        (7.2, 12.2),
        (12.6, 12.2),
    ];
    b.move_to(pts[0].0, pts[0].1);
    for (px, py) in &pts[1..] {
        b.line_to(*px, *py);
    }
    b.close();
    let path = b.to_path();
    snap.save();
    snap.translate(&graphene::Point::new(x as f32, y as f32));
    snap.scale(scale, scale);
    snap.save();
    snap.translate(&graphene::Point::new(0.8, 1.4));
    snap.append_fill(&path, gsk::FillRule::Winding, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.3));
    snap.restore();
    snap.append_fill(
        &path,
        gsk::FillRule::Winding,
        &gdk::RGBA::new(0.08, 0.08, 0.08, 1.0),
    );
    let stroke = gsk::Stroke::new(1.3);
    stroke.set_line_join(gsk::LineJoin::Round);
    snap.append_stroke(&path, &stroke, &gdk::RGBA::WHITE);
    snap.restore();
}

/// Popovers that are showing, anywhere under `root`.
fn open_popovers(root: &gtk::Widget, out: &mut Vec<gtk::Popover>) {
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Some(p) = c.downcast_ref::<gtk::Popover>()
            && p.is_visible()
        {
            out.push(p.clone());
        }
        open_popovers(&c, out);
        child = c.next_sibling();
    }
}

/// Depth-first search for a widget that is showing.
pub fn find(root: &impl IsA<gtk::Widget>, pred: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    let root = root.as_ref();
    if pred(root) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if c.is_visible()
            && let Some(found) = find(&c, pred)
        {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn all(root: &impl IsA<gtk::Widget>, pred: &dyn Fn(&gtk::Widget) -> bool, out: &mut Vec<gtk::Widget>) {
    let root = root.as_ref();
    if pred(root) {
        out.push(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if c.is_visible() {
            all(&c, pred, out);
        }
        child = c.next_sibling();
    }
}

/// The label showing `text`, mapped.
fn label(root: &impl IsA<gtk::Widget>, text: &str) -> Option<gtk::Widget> {
    find(root, &|w| {
        w.is_mapped() && w.downcast_ref::<gtk::Label>().is_some_and(|l| l.label() == text)
    })
}

fn ancestor<T: IsA<gtk::Widget>>(w: &gtk::Widget) -> Option<T> {
    w.ancestor(T::static_type()).and_downcast::<T>()
}

/// A preferences row (`adw::ActionRow`, `ComboRow`, `ExpanderRow`) titled `title`.
fn row_titled<T: IsA<gtk::Widget> + IsA<adw::PreferencesRow>>(
    root: &impl IsA<gtk::Widget>,
    title: &str,
) -> Option<T> {
    find(root, &|w| {
        w.is_mapped() && w.downcast_ref::<T>().is_some_and(|r| r.title() == title)
    })
    .and_downcast::<T>()
}

fn button_tooltip(root: &impl IsA<gtk::Widget>, tip: &str) -> Option<gtk::Widget> {
    find(root, &|w| {
        w.is_mapped()
            && (w.is::<gtk::Button>() || w.is::<gtk::MenuButton>() || w.is::<gtk::ToggleButton>())
            && w.tooltip_text().is_some_and(|t| t.starts_with(tip))
    })
}

/// The deck's 13 keys on the canvas, in order.
fn deck_keys(window: &gtk::Window) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    all(
        window,
        &|w| w.has_css_class("deck-key") && w.is::<gtk::Button>(),
        &mut out,
    );
    out
}

fn deck_screen(window: &gtk::Window) -> Option<gtk::Widget> {
    find(window, &|w| {
        w.has_css_class("deck-screen") && w.is::<gtk::Button>()
    })
}

struct Ctx {
    rec: Rc<Rec>,
    state: State,
    tray: Rc<Tray>,
    window: gtk::Window,
}

impl Ctx {
    fn need(&self, what: &str, w: Option<gtk::Widget>) -> gtk::Widget {
        w.unwrap_or_else(|| {
            eprintln!("demo-reel: no {what} on screen");
            self.window.clone().upcast()
        })
    }

    fn library_id(&self, name: &str) -> Option<Uuid> {
        self.state
            .editor
            .borrow()
            .profile()
            .key_library
            .iter()
            .find(|l| l.name == name)
            .map(|l| l.id)
    }

    fn profile_id(&self, name: &str) -> Option<Uuid> {
        self.state
            .profiles
            .borrow()
            .iter()
            .find(|(_, n)| n == name)
            .map(|(id, _)| *id)
    }

    /// The pointer at window point `p` while a tray tile is carried: the window's follow, and the
    /// tiles' area when over it.
    fn carry_over(&self, p: (f64, f64)) {
        let root: gtk::Widget = self.window.clone().upcast();
        self.tray.follow_carried(&root, p.0, p.1);
        let pages = self.tray.pages();
        let inside = root
            .compute_point(pages, &graphene::Point::new(p.0 as f32, p.1 as f32))
            .filter(|q| pages.contains(q.x() as f64, q.y() as f64));
        match inside {
            Some(q) => {
                self.tray.hover_at(q.x() as f64, q.y() as f64);
            }
            None => self.tray.show_aim(crate::ui::tile_drop::Aim::Nothing),
        }
    }

    fn pages_point(&self, p: (f64, f64)) -> Option<(f64, f64)> {
        self.window
            .compute_point(self.tray.pages(), &graphene::Point::new(p.0 as f32, p.1 as f32))
            .map(|q| (q.x() as f64, q.y() as f64))
    }

    /// Pick up library key `name` from the tray.
    async fn pick_tile(&self, name: &str) -> Option<Uuid> {
        let id = self.library_id(name)?;
        let (_, disc) = self.tray.tile(id)?;
        self.rec.glide_to(&disc, 700).await;
        self.rec.press().await;
        let (w, h) = (disc.width() as f64, disc.height() as f64);
        self.rec
            .carry(Some((gtk::WidgetPaintable::new(Some(&disc)).upcast(), w, h)));
        self.tray.carry_begin(id, Tab::Keys);
        Some(id)
    }

    async fn carry_to(&self, to: (f64, f64), ms: u64) {
        self.rec.glide_with(move || to, ms, |p| self.carry_over(p)).await;
    }

    fn let_go(&self) {
        self.rec.release();
        self.rec.carry(None);
    }
}

/// The scenes, in the order the video shows them.
pub const SCENES: &[&str] = &["keys", "widgets", "library", "autoswitch", "plugins"];

/// Record `scene` into `out`, then `done`.
pub fn run(window: &gtk::Window, state: &State, scene: String, out: PathBuf, done: Box<dyn FnOnce()>) {
    let Some(tray) = Tray::last() else {
        eprintln!("demo-reel: no tray");
        done();
        return;
    };
    let ctx = Rc::new(Ctx {
        rec: Rec::new(window),
        state: state.clone(),
        tray,
        window: window.clone(),
    });
    glib::MainContext::default().spawn_local(async move {
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        setup(&ctx, &scene).await;
        if let Err(e) = ctx.rec.start(&out) {
            eprintln!("demo-reel: ffmpeg: {e}");
            done();
            return;
        }
        match scene.as_str() {
            "keys" => keys(&ctx).await,
            "widgets" => widgets(&ctx).await,
            "library" => library(&ctx).await,
            "autoswitch" => autoswitch(&ctx).await,
            "plugins" => plugins(&ctx).await,
            other => eprintln!("demo-reel: no scene {other} (scenes: {})", SCENES.join(", ")),
        }
        let ok = ctx.rec.stop();
        eprintln!(
            "demo-reel: {scene} -> {} ({})",
            out.display(),
            if ok { "ok" } else { "FAILED" }
        );
        done();
    });
}

/// Before the camera rolls: the page and selection the scene starts from.
async fn setup(c: &Ctx, scene: &str) {
    c.state.select(|s| s.clear());
    if scene == "library" {
        c.state.go_to_page(1);
    }
    wait(900).await;
}

/// Set up a key: open a page, drag an action onto an empty key, give it an effect.
async fn keys(c: &Ctx) {
    let rec = &c.rec;
    wait(700).await;
    // The Extras page.
    let extras = c.need("Extras page", label(&c.window, "Extras"));
    rec.glide_to(&extras, 800).await;
    rec.click().await;
    if let Some(row) = ancestor::<gtk::ListBoxRow>(&extras) {
        row.activate();
    }
    wait(800).await;
    // Drag Next Track from the action list onto key 3.
    let entry = c.need("Next Track action", label(&c.window, "Next Track"));
    rec.glide_to(&entry, 900).await;
    let row = ancestor::<gtk::ListBoxRow>(&entry);
    let action = row.as_ref().and_then(row_action);
    rec.press().await;
    if let Some(a) = &action {
        c.state.dragging_action.set(true);
        c.state.notify(Change::Drag);
        let look = ulanzi_core::catalog::default_look(a);
        if let Some(t) = crate::ui::canvas::key_texture(&look, &crate::ui::canvas::profile_dir(&c.state)) {
            rec.carry(Some((t.upcast(), 72.0, 72.0)));
        }
    }
    let keys = deck_keys(&c.window);
    let target = 2u8;
    if let Some(k) = keys.get(target as usize) {
        rec.glide_to(k, 1100).await;
    }
    rec.release();
    rec.carry(None);
    c.state.dragging_action.set(false);
    c.state.notify(Change::Drag);
    if let Some(a) = action {
        c.state.edit(|e| e.drop_action(Slot::Key(target), a));
        c.state.select(|s| s.click(Slot::Key(target)));
    }
    wait(1200).await;
    // Effects: open the row, pick Chase (lower ones open past the frame).
    if let Some(effects) = row_titled::<adw::ExpanderRow>(&c.window, "Effects") {
        rec.glide_to(&effects, 800).await;
        rec.click().await;
        effects.set_expanded(true);
        wait(700).await;
        let combo =
            find(&effects, &|w| w.is_mapped() && w.is::<adw::ComboRow>()).and_downcast::<adw::ComboRow>();
        if let Some(combo) = combo {
            rec.glide_to(&combo, 700).await;
            rec.click().await;
            gtk::prelude::WidgetExt::activate(&combo);
            wait(700).await;
            let names: Vec<String> = combo
                .model()
                .map(|m| {
                    (0..m.n_items())
                        .filter_map(|i| {
                            m.item(i)
                                .and_downcast::<gtk::StringObject>()
                                .map(|s| s.string().to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let pick = names.iter().position(|n| n == "Chase").unwrap_or(0) as u32;
            if let Some(item) = find(&c.window, &|w| {
                w.is_mapped()
                    && w.ancestor(gtk::Popover::static_type()).is_some()
                    && w.downcast_ref::<gtk::Label>()
                        .is_some_and(|l| l.label() == "Chase")
            }) {
                rec.glide_to(&item, 600).await;
                rec.click().await;
            }
            combo.set_selected(pick);
            close_popovers(&c.window);
            wait(2200).await;
        }
    }
    rec.glide((rec.cursor.get().0 - 260.0, rec.cursor.get().1 + 120.0), 900)
        .await;
    wait(800).await;
}

/// The action a row of the action list gives, from its drag.
fn row_action(row: &gtk::ListBoxRow) -> Option<ulanzi_core::model::Action> {
    let src = row
        .observe_controllers()
        .into_iter()
        .filter_map(|o| o.ok().and_downcast::<gtk::DragSource>())
        .next()?;
    let cp = src.emit_by_name::<Option<gdk::ContentProvider>>("prepare", &[&0.0f64, &0.0f64])?;
    let s: String = cp.value(glib::Type::STRING).ok()?.get().ok()?;
    match parse_payload(&s)? {
        Payload::Action(a) => Some(a),
        _ => None,
    }
}

fn close_popovers(root: &gtk::Window) {
    let mut ps = Vec::new();
    open_popovers(root.upcast_ref(), &mut ps);
    for p in ps {
        p.popdown();
    }
}

/// The wide screen: add widgets, see each one on the deck.
async fn widgets(c: &Ctx) {
    let rec = &c.rec;
    wait(600).await;
    let screen = c.need("wide screen", deck_screen(&c.window));
    rec.glide_to(&screen, 900).await;
    rec.click().await;
    c.state.select(|s| s.click(Slot::Screen));
    wait(1000).await;
    for kind in ["System", "Timer", "Clock"] {
        let Some(add) = find(&c.window, &|w| {
            w.is_mapped()
                && w.downcast_ref::<gtk::MenuButton>()
                    .is_some_and(|m| m.label().as_deref() == Some("Add Widget"))
        })
        .and_downcast::<gtk::MenuButton>() else {
            eprintln!("demo-reel: no Add Widget");
            break;
        };
        rec.glide_to(&add, 800).await;
        rec.click().await;
        add.popup();
        wait(600).await;
        let item = add.popover().and_then(|p| {
            find(&p, &|w| {
                w.downcast_ref::<gtk::Label>().is_some_and(|l| l.label() == kind)
            })
        });
        if let Some(item) = item {
            rec.glide_to(&item, 600).await;
            rec.click().await;
            if let Some(row) = ancestor::<gtk::ListBoxRow>(&item)
                && let Some(list) = row.parent().and_downcast::<gtk::ListBox>()
            {
                list.emit_by_name::<()>("row-activated", &[&row]);
            }
        }
        wait(1800).await;
    }
    wait(600).await;
}

/// The Library: open it, drag a key onto the deck, reorder, turn the page at the edge.
async fn library(c: &Ctx) {
    let rec = &c.rec;
    wait(600).await;
    let open = c.need("Library button", button_tooltip(&c.window, "Library"));
    rec.glide_to(&open, 900).await;
    rec.click().await;
    WidgetExt::activate_action(&c.window, "win.library-tray", None).ok();
    wait(1200).await;
    // Onto the deck: "Starting" to key 4 of this page.
    if let Some(id) = c.pick_tile("Starting").await {
        let key = deck_keys(&c.window).get(3).cloned();
        if let Some(key) = key {
            rec.glide_with(|| rec.center(&key), 1300, |p| c.carry_over(p))
                .await;
        }
        wait(250).await;
        c.let_go();
        c.state.edit(|e| e.place_key(3, id));
        c.state.select(|s| s.click(Slot::Key(3)));
        c.tray.carry_end();
        wait(1400).await;
    }
    c.state.select(|s| s.clear());
    // Swap two tiles: "Ending" onto "Main".
    if c.pick_tile("Ending").await.is_some() {
        let target = c
            .library_id("Main")
            .and_then(|id| c.tray.tile(id))
            .map(|(_, d)| rec.center(&d));
        if let Some(to) = target {
            c.carry_to(to, 1000).await;
            wait(500).await;
            c.let_go();
            if let Some(q) = c.pages_point(to) {
                c.tray.drop_at(q.0, q.1);
            }
            c.tray.carry_end();
        }
        wait(1000).await;
    }
    // In between two tiles: "Mic" between "Previous" and "Play / Pause".
    if c.pick_tile("Mic").await.is_some() {
        let a = c
            .library_id("Previous")
            .and_then(|id| c.tray.tile(id))
            .map(|(_, d)| rec.bounds(&d));
        let b = c
            .library_id("Play / Pause")
            .and_then(|id| c.tray.tile(id))
            .map(|(_, d)| rec.bounds(&d));
        if let (Some(Some(a)), Some(Some(b))) = (a, b) {
            let to = (
                ((a.x() + a.width() + b.x()) / 2.0) as f64,
                (a.y() + a.height() / 2.0) as f64,
            );
            c.carry_to(to, 1100).await;
            wait(700).await;
            c.let_go();
            if let Some(q) = c.pages_point(to) {
                c.tray.drop_at(q.0, q.1);
            }
            c.tray.carry_end();
        }
        wait(1100).await;
    }
    // Hold one at the tray's right edge: the page turns.
    if c.pick_tile("Lock").await.is_some() {
        let frame = c.tray.frame().compute_bounds(&c.window);
        if let Some(f) = frame {
            let y = rec.cursor.get().1;
            let edge = ((f.x() + f.width()) as f64 - 2.0, y);
            c.carry_to(edge, 1200).await;
            for _ in 0..60 {
                c.carry_over(edge);
                wait(25).await;
            }
            // On the new page, onto "Undo": the two trade places and nothing else moves (put in
            // between, the tile after it would go back to page 1, as the Library is one list).
            let between = || {
                c.library_id("Undo")
                    .and_then(|id| c.tray.tile(id))
                    .map(|(_, d)| rec.center(&d))
                    .unwrap_or(((f.x() + f.width() * 0.3) as f64, y))
            };
            rec.glide_with(between, 1000, |p| c.carry_over(p)).await;
            wait(600).await;
            let at = between();
            c.let_go();
            if let Some(q) = c.pages_point(at) {
                c.tray.drop_at(q.0, q.1);
            }
            c.tray.carry_end();
        }
        wait(1200).await;
    }
}

/// Profiles that follow the app in front.
async fn autoswitch(c: &Ctx) {
    let rec = &c.rec;
    wait(600).await;
    // Video Editing's apps.
    if let Some(row) = label(&c.window, "Video Editing") {
        rec.glide_to(&row, 900).await;
        rec.click().await;
        if let Some(id) = c.profile_id("Video Editing") {
            crate::ui::autoswitch::open(&c.state, &c.window, id, "Video Editing");
        }
        // The sandbox has no desktop to ask which window is in front, so the dialog says auto-switch
        // is not available here; on a real desktop that line says it works. Leave it out.
        for _ in 0..20 {
            if let Some(row) = row_titled::<adw::ActionRow>(&c.window, "Not available here") {
                if let Some(g) = ancestor::<adw::PreferencesGroup>(row.upcast_ref()) {
                    g.set_visible(false);
                }
                break;
            }
            wait(20).await;
        }
        wait(2400).await;
        if let Some(d) = c
            .window
            .downcast_ref::<adw::ApplicationWindow>()
            .and_then(|w| w.visible_dialog())
        {
            if let Some(close) = button_tooltip(&d, "Close").or_else(|| label(&d, "Done")) {
                rec.glide_to(&close, 800).await;
                rec.click().await;
            }
            d.close();
        }
        wait(800).await;
    }
    rec.glide(
        (c.window.width() as f64 * 0.6, c.window.height() as f64 * 0.75),
        900,
    )
    .await;
    for (text, class, desktop, title) in [
        (
            "Kdenlive is in front",
            "kdenlive",
            "org.kde.kdenlive",
            "Untitled - Kdenlive",
        ),
        (
            "OBS Studio is in front",
            "obs",
            "com.obsproject.Studio",
            "OBS Studio",
        ),
        ("Firefox is in front", "firefox", "firefox", "Mozilla Firefox"),
    ] {
        rec.banner(Some(text));
        if let Some(cl) = c.state.client.borrow().clone() {
            let (class, desktop, title) = (class.to_string(), desktop.to_string(), title.to_string());
            crate::runtime::run(
                async move { cl.window_activated(&class, &desktop, &title).await },
                |r| {
                    if let Err(e) = r {
                        eprintln!("demo-reel: focus: {e}");
                    }
                },
            );
        }
        wait(2600).await;
    }
    rec.banner(None);
    wait(600).await;
}

/// The plugin store.
async fn plugins(c: &Ctx) {
    let rec = &c.rec;
    wait(600).await;
    let open = c.need("Plugins button", button_tooltip(&c.window, "Plugins"));
    rec.glide_to(&open, 900).await;
    rec.click().await;
    // Straight to the store: the sandbox's installed plugins have no OBS or Home Assistant to
    // reach, so their rows ask for attention.
    crate::ui::plugins_window::open(&c.state, &c.window, Some("discover"));
    wait(1400).await;
    let dialog = c
        .window
        .downcast_ref::<adw::ApplicationWindow>()
        .and_then(|w| w.visible_dialog())
        .and_downcast::<adw::PreferencesDialog>();
    let Some(dialog) = dialog else {
        eprintln!("demo-reel: no Plugins dialog");
        return;
    };
    for name in ["Home Assistant", "OBS Studio"] {
        if let Some(card) = label(&dialog, name) {
            rec.glide_to(&card, 900).await;
            wait(1200).await;
        }
    }
    // Down the list of plugins.
    if let Some(sw) = find(&dialog, &|w| {
        w.is_mapped()
            && w.downcast_ref::<gtk::ScrolledWindow>()
                .is_some_and(|s| s.vadjustment().upper() > s.vadjustment().page_size() + 50.0)
    })
    .and_downcast::<gtk::ScrolledWindow>()
    {
        let b = rec.bounds(&sw);
        if let Some(b) = b {
            rec.glide(
                (
                    (b.x() + b.width() * 0.5) as f64,
                    (b.y() + b.height() * 0.6) as f64,
                ),
                900,
            )
            .await;
        }
        let adj = sw.vadjustment();
        let to = (adj.upper() - adj.page_size()).min(400.0);
        let t0 = std::time::Instant::now();
        loop {
            let t = (t0.elapsed().as_secs_f64() / 1.4).min(1.0);
            adj.set_value(to * ease(t));
            if t >= 1.0 {
                break;
            }
            wait(12).await;
        }
    }
    wait(1800).await;
}
