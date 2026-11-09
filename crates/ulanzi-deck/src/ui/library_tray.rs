//! The Library tray under the deck: library dials to drag onto the deck or click to edit. Opening
//! it grows the tray at the bottom and slides the deck up only as far as needed, never past its
//! dial row.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::model::{LibraryDial, Usage};
use uuid::Uuid;

use crate::state::{Change, State};
use crate::ui::canvas;
use crate::ui::dial_swap::DialRow;

/// Dials per row, and rows per page at most.
pub const PER_ROW: usize = 6;
pub const MAX_ROWS: usize = 4;
/// Library dials on a full page: New Dial takes the last spot.
const PER_PAGE: usize = PER_ROW * MAX_ROWS - 1;

/// Room under the tray, and between the tray and the deck.
const MARGIN: f64 = 12.0;
const GAP: f64 = 16.0;
const ROW_GAP: i32 = 8;
const ANIM_MS: u32 = 250;

const HINT: &str = "Drag a dial onto the deck to place it · click one to edit it";

/// How many pages the tray has, and how many rows the shown page needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub pages: usize,
    pub rows: usize,
}

/// The layout for `items` library dials on page `page` (0-based); New Dial ends every page.
pub fn layout(items: usize, page: usize) -> Layout {
    let pages = items.div_ceil(PER_PAGE).max(1);
    let page = page.min(pages - 1);
    let slots = (items - page * PER_PAGE).min(PER_PAGE) + 1;
    Layout {
        pages,
        rows: slots.div_ceil(PER_ROW),
    }
}

/// The tray's tabs (phase 2 adds Keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dials,
}

type OnToggle = Box<dyn Fn(bool)>;

/// A library dial's tile, kept so edits update it in place.
struct Item {
    id: Uuid,
    disc: gtk::Box,
    face: canvas::DialFace,
    name: gtk::Label,
    usage: gtk::Label,
    pin: gtk::Image,
}

pub struct Tray {
    state: State,
    me: RefCell<Weak<Tray>>,
    /// Holds the deck and the tray, and places both.
    stage: gtk::Overlay,
    deck: gtk::Widget,
    body: gtk::Widget,
    /// Kept in view however far the deck slides (the dial row).
    keep: gtk::Widget,
    dial_row: DialRow,
    /// Shown less as the tray opens (the page dots and hints under the deck).
    fade: Vec<gtk::Widget>,
    /// The tray itself, clipped while it grows.
    frame: gtk::ScrolledWindow,
    content: gtk::Box,
    /// The pages side by side; `sizer` gives the area its height.
    pages: gtk::Overlay,
    sizer: gtk::Box,
    grids: RefCell<Vec<gtk::Grid>>,
    items: RefCell<Vec<Item>>,
    /// The library as the tiles show it, so unrelated edits change nothing.
    drawn: RefCell<Vec<(LibraryDial, Usage)>>,
    /// Sits under the page in the same area, so it eases in and out with the height.
    pager: gtk::Box,
    with_pager: Cell<bool>,
    dots: gtk::Box,
    prev: gtk::Button,
    next: gtk::Button,
    open: Cell<bool>,
    /// The library changed while the tray was shut.
    stale: Cell<bool>,
    /// How far open, 0 to 1.
    shown: Cell<f64>,
    grow: RefCell<Option<adw::TimedAnimation>>,
    /// The page the grids are drawn at (between two while sliding) and the page area's height.
    offset: Cell<f64>,
    height: Cell<f64>,
    from: Cell<(f64, f64)>,
    to: Cell<(f64, f64)>,
    slide: RefCell<Option<adw::TimedAnimation>>,
    last_size: Cell<(i32, i32)>,
    on_toggle: RefCell<Option<OnToggle>>,
}

impl Tray {
    /// A tray under `deck`; `body` is the deck drawing and `keep` the row that stays in view.
    pub fn new(
        state: &State,
        deck: &gtk::Widget,
        body: &gtk::Widget,
        keep: &gtk::Widget,
        dial_row: &DialRow,
        fade: Vec<gtk::Widget>,
    ) -> Rc<Tray> {
        let stage = gtk::Overlay::builder().overflow(gtk::Overflow::Hidden).build();
        stage.add_css_class("canvas-area");
        stage.add_overlay(deck);
        stage.set_measure_overlay(deck, true);

        let hint = gtk::Label::builder()
            .label(HINT)
            .css_classes(["tray-hint"])
            .hexpand(true)
            .xalign(1.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let help = gtk::Button::builder()
            .label("?")
            .css_classes(["circular", "tray-help"])
            .tooltip_text("How the Library Works")
            .action_name("win.library-guide")
            .valign(gtk::Align::Center)
            .build();
        let header = gtk::Box::builder().spacing(10).margin_bottom(6).build();
        header.append(&hint);
        header.append(&help);

        let sizer = gtk::Box::builder().hexpand(true).build();
        let pages = gtk::Overlay::builder()
            .child(&sizer)
            .overflow(gtk::Overflow::Hidden)
            .build();

        let round = |icon: &str, tip: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .css_classes(["circular", "tray-turn"])
                .valign(gtk::Align::Center)
                .build()
        };
        let prev = round("pan-start-symbolic", "Previous Page");
        let next = round("pan-end-symbolic", "Next Page");
        let dots = gtk::Box::builder().valign(gtk::Align::Center).build();
        let pager = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .margin_top(12)
            .build();
        pager.append(&prev);
        pager.append(&dots);
        pager.append(&next);

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["library-tray"])
            .build();
        content.append(&header);
        content.append(&pages);
        pages.add_overlay(&pager);
        // Never scrolled: the frame only clips the tray while it grows and shrinks.
        let frame = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .css_classes(["library-frame"])
            .visible(false)
            .build();
        stage.add_overlay(&frame);

        let tray = Rc::new(Tray {
            state: state.clone(),
            me: RefCell::default(),
            stage,
            deck: deck.clone(),
            body: body.clone(),
            keep: keep.clone(),
            dial_row: dial_row.clone(),
            fade,
            frame,
            content,
            pages,
            sizer,
            grids: RefCell::default(),
            items: RefCell::default(),
            drawn: RefCell::default(),
            pager,
            with_pager: Cell::new(false),
            dots,
            prev,
            next,
            open: Cell::new(false),
            stale: Cell::new(true),
            shown: Cell::new(0.0),
            grow: RefCell::default(),
            offset: Cell::new(0.0),
            height: Cell::new(0.0),
            from: Cell::new((0.0, 0.0)),
            to: Cell::new((0.0, 0.0)),
            slide: RefCell::default(),
            last_size: Cell::new((0, 0)),
            on_toggle: RefCell::default(),
        });
        *tray.me.borrow_mut() = Rc::downgrade(&tray);
        tray.wire();
        tray
    }

    /// The deck with the tray under it.
    pub fn widget(&self) -> gtk::Widget {
        self.stage.clone().upcast()
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Called with the new state whenever the tray opens or closes.
    pub fn connect_toggled(&self, f: impl Fn(bool) + 'static) {
        *self.on_toggle.borrow_mut() = Some(Box::new(f));
    }

    pub fn open(&self, _tab: Tab) {
        if self.open.replace(true) {
            return;
        }
        if self.stale.get() {
            self.refresh();
        }
        // Sized afresh: the dials may have changed size while it was shut.
        self.go(self.state.tray_page.get());
        self.frame.set_visible(true);
        self.run_grow(1.0);
        self.toggled(true);
    }

    pub fn close(&self) {
        if !self.open.replace(false) {
            return;
        }
        self.dial_row.set_marked(None);
        self.run_grow(0.0);
        self.toggled(false);
    }

    /// Outline deck dial `n` (dashed, in the accent colour) as where a dial will land.
    pub fn mark_drop_spot(&self, n: Option<u8>) {
        self.dial_row.set_marked(n);
    }

    /// Smaller dials for narrow windows, as on the deck.
    pub fn set_compact(&self, compact: bool) {
        canvas::set_class(&self.content, "compact", compact);
    }

    fn toggled(&self, open: bool) {
        if let Some(f) = self.on_toggle.borrow().as_ref() {
            f(open);
        }
    }

    fn wire(&self) {
        let me = self.me.borrow().clone();
        {
            let me = me.clone();
            self.stage
                .connect_get_child_position(move |_, child| me.upgrade()?.place(child));
        }
        {
            let me = me.clone();
            self.pages
                .connect_get_child_position(move |pages, child| me.upgrade()?.place_page(pages, child));
        }
        let grow_target = {
            let me = me.clone();
            adw::CallbackAnimationTarget::new(move |v| {
                if let Some(t) = me.upgrade() {
                    t.shown.set(v);
                    for w in &t.fade {
                        w.set_opacity(1.0 - v);
                    }
                    t.stage.queue_allocate();
                }
            })
        };
        let grow = eased(&self.stage, &grow_target);
        {
            let me = me.clone();
            grow.connect_done(move |_| {
                if let Some(t) = me.upgrade()
                    && !t.open.get()
                {
                    t.frame.set_visible(false);
                }
            });
        }
        *self.grow.borrow_mut() = Some(grow);
        let slide_target = {
            let me = me.clone();
            adw::CallbackAnimationTarget::new(move |v| {
                if let Some(t) = me.upgrade() {
                    let ((o0, h0), (o1, h1)) = (t.from.get(), t.to.get());
                    t.set_view(o0 + (o1 - o0) * v, h0 + (h1 - h0) * v);
                }
            })
        };
        *self.slide.borrow_mut() = Some(eased(&self.stage, &slide_target));
        for (b, step) in [(&self.prev, -1isize), (&self.next, 1)] {
            let me = me.clone();
            b.connect_clicked(move |_| {
                if let Some(t) = me.upgrade() {
                    let page = t.state.tray_page.get().saturating_add_signed(step);
                    t.go(page);
                }
            });
        }
        let st = self.state.clone();
        st.on(move |c| {
            let Some(t) = me.upgrade() else { return };
            match c {
                Change::Profile | Change::Plugins => {
                    if t.open.get() {
                        t.refresh();
                    } else {
                        t.stale.set(true);
                    }
                }
                Change::Selection => t.show_picked(),
                _ => {}
            }
        });
    }

    /// Where the deck and the tray go in the stage.
    fn place(&self, child: &gtk::Widget) -> Option<gdk::Rectangle> {
        let (w, h) = (self.stage.width(), self.stage.height());
        // The deck lays out again at a new size; place everything again once it has.
        if self.last_size.replace((w, h)) != (w, h) && self.shown.get() > 0.0 {
            let stage = self.stage.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(s) = stage.upgrade() {
                    s.queue_allocate();
                }
            });
        }
        // The page's height changed under it (smaller dials when compact): ease to the new one.
        let sliding = self
            .slide
            .borrow()
            .as_ref()
            .is_some_and(|a| a.state() == adw::AnimationState::Playing);
        if child == &self.deck
            && self.open.get()
            && !sliding
            && (self.page_height(self.state.tray_page.get()) - self.height.get()).abs() > 0.5
        {
            let me = self.me.borrow().clone();
            glib::idle_add_local_once(move || {
                if let Some(t) = me.upgrade() {
                    t.go(t.state.tray_page.get());
                }
            });
        }
        let (w, h) = (w as f64, h as f64);
        let body = self.body.compute_bounds(&self.deck);
        let (bx, bw) = body.map_or((0.0, w), |b| (b.x() as f64, b.width() as f64));
        // Under the deck, never narrower than its dials need (the deck scrolls sideways then too).
        let need = self.content.measure(gtk::Orientation::Horizontal, -1).0 as f64;
        let tw = bw.min(w - 2.0 * MARGIN).max(need);
        let mut th = match self.shown.get() {
            s if s > 0.0 => s * self.content.measure(gtk::Orientation::Vertical, tw as i32).1 as f64,
            _ => 0.0,
        };
        let shift = match (body, self.keep.compute_bounds(&self.deck)) {
            (Some(b), Some(k)) if th > 0.0 => {
                let bottom = b.y() as f64 + b.height() as f64;
                let shift = (bottom + GAP - (h - MARGIN - th))
                    .min(k.y() as f64 - MARGIN)
                    .max(0.0);
                // Too short a window: the tray stops under the deck and scrolls.
                th = th.min(h - MARGIN - (bottom - shift + GAP)).max(0.0);
                shift
            }
            _ => 0.0,
        };
        if child == &self.deck {
            return Some(gdk::Rectangle::new(0, -shift.round() as i32, w as i32, h as i32));
        }
        let top = h - MARGIN - th;
        let x = bx + (bw - tw) / 2.0;
        Some(gdk::Rectangle::new(
            x.round() as i32,
            top.round() as i32,
            tw.round() as i32,
            th.round() as i32,
        ))
    }

    /// Page `i` sits `i - offset` page widths from the left.
    fn place_page(&self, pages: &gtk::Overlay, child: &gtk::Widget) -> Option<gdk::Rectangle> {
        let w = pages.width();
        if child == &self.pager {
            // At the bottom of the area but never over the pages drawn there, so coming and
            // going it is only uncovered or clipped as the height eases.
            let q = child.measure(gtk::Orientation::Vertical, w).1;
            let grid = |i: f64| {
                self.grids
                    .borrow()
                    .get(i as usize)
                    .map_or(0.0, |g| g.measure(gtk::Orientation::Vertical, w).1 as f64)
            };
            let o = self.offset.get().max(0.0);
            let (a, b) = (grid(o.floor()), grid(o.ceil()));
            let drawn = a + (b - a) * o.fract();
            let y = (pages.height() - q).max(drawn.round() as i32);
            return Some(gdk::Rectangle::new(0, y, w, q));
        }
        let i = self
            .grids
            .borrow()
            .iter()
            .position(|g| g.upcast_ref::<gtk::Widget>() == child)?;
        let nat = child.measure(gtk::Orientation::Vertical, w).1;
        let x = (i as f64 - self.offset.get()) * w as f64;
        Some(gdk::Rectangle::new(x.round() as i32, 0, w, nat))
    }

    fn set_view(&self, offset: f64, height: f64) {
        self.offset.set(offset);
        self.height.set(height);
        self.sizer.set_size_request(-1, height.round() as i32);
        self.pages.queue_allocate();
    }

    fn run_grow(&self, to: f64) {
        if let Some(a) = self.grow.borrow().as_ref() {
            a.set_value_from(self.shown.get());
            a.set_value_to(to);
            a.reset();
            a.play();
        }
    }

    /// Show page `page` (sliding to it while the tray is open).
    pub fn go(&self, page: usize) {
        let items = self.state.editor.borrow().profile().dial_library.len();
        let l = layout(items, page);
        let page = page.min(l.pages - 1);
        self.state.tray_page.set(page);
        self.show_pager(l.pages, page);
        let to = (page as f64, self.page_height(page));
        let slide = self.slide.borrow();
        match slide.as_ref() {
            Some(a) if self.shown.get() > 0.0 && self.height.get() > 0.0 => {
                self.from.set((self.offset.get(), self.height.get()));
                self.to.set(to);
                a.reset();
                a.play();
            }
            _ => self.set_view(to.0, to.1),
        }
    }

    /// Page `page`'s own height (so a short last page shrinks the tray), and the pager's when
    /// there are pages.
    fn page_height(&self, page: usize) -> f64 {
        let w = self.pages.width();
        let height = self.grids.borrow().get(page).map_or(0.0, |g| {
            g.measure(gtk::Orientation::Vertical, if w > 0 { w } else { -1 })
                .1 as f64
        });
        let pager = match self.with_pager.get() {
            true => self.pager.measure(gtk::Orientation::Vertical, -1).1 as f64,
            false => 0.0,
        };
        height + pager
    }

    fn show_pager(&self, pages: usize, page: usize) {
        self.with_pager.set(pages > 1);
        self.pager.set_sensitive(pages > 1);
        self.prev.set_sensitive(page > 0);
        self.next.set_sensitive(page + 1 < pages);
        while let Some(c) = self.dots.first_child() {
            self.dots.remove(&c);
        }
        for i in 0..pages {
            let dot = gtk::Box::builder()
                .css_classes(["dot"])
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .build();
            let b = gtk::Button::builder()
                .child(&dot)
                .css_classes(["flat", "circular", "tray-dot"])
                .tooltip_text(format!("Page {}", i + 1))
                .build();
            canvas::set_class(&b, "current", i == page);
            let me = self.me.borrow().clone();
            b.connect_clicked(move |_| {
                if let Some(t) = me.upgrade() {
                    t.go(i);
                }
            });
            self.dots.append(&b);
        }
    }

    /// Show the library as it is now: nothing when unchanged, the tiles in place when the same
    /// dials are there, else laid out again.
    fn refresh(&self) {
        self.stale.set(false);
        let now: Vec<(LibraryDial, Usage)> = {
            let e = self.state.editor.borrow();
            let p = e.profile();
            p.dial_library
                .iter()
                .map(|d| (d.clone(), p.dial_usage(d.id)))
                .collect()
        };
        if *self.drawn.borrow() == now && !self.grids.borrow().is_empty() {
            return;
        }
        let same = {
            let items = self.items.borrow();
            items.len() == now.len() && items.iter().zip(&now).all(|(i, (l, _))| i.id == l.id)
        };
        if same && !self.grids.borrow().is_empty() {
            for (item, (lib, usage)) in self.items.borrow().iter().zip(&now) {
                fill(item, lib, *usage);
            }
        } else {
            self.rebuild(&now);
        }
        *self.drawn.borrow_mut() = now;
    }

    /// Lay out the library's dials again.
    fn rebuild(&self, now: &[(LibraryDial, Usage)]) {
        for g in self.grids.take() {
            self.pages.remove_overlay(&g);
        }
        self.items.borrow_mut().clear();
        let (library, usage): (Vec<LibraryDial>, Vec<Usage>) = now.iter().cloned().unzip();
        let pages = layout(library.len(), 0).pages;
        for p in 0..pages {
            let grid = gtk::Grid::builder()
                .row_spacing(ROW_GAP)
                .column_homogeneous(true)
                .valign(gtk::Align::Start)
                .build();
            let start = p * PER_PAGE;
            let end = (start + PER_PAGE).min(library.len());
            let mut cells: Vec<gtk::Widget> = (start..end)
                .map(|i| self.dial_item(&library[i], usage[i]).upcast())
                .collect();
            cells.push(self.new_item().upcast());
            // Blanks keep six columns on a short page.
            while cells.len() < PER_ROW {
                cells.push(gtk::Box::builder().hexpand(true).build().upcast());
            }
            for (i, c) in cells.iter().enumerate() {
                grid.attach(c, (i % PER_ROW) as i32, (i / PER_ROW) as i32, 1, 1);
            }
            self.pages.add_overlay(&grid);
            self.grids.borrow_mut().push(grid);
        }
        self.show_picked();
        self.go(self.state.tray_page.get());
    }

    /// The face of a library dial in a round well, as on the deck.
    fn disc(&self, child: &impl IsA<gtk::Widget>) -> gtk::Box {
        let disc = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["tray-dial"])
            .halign(gtk::Align::Center)
            .build();
        // Sized by the stylesheet (smaller when compact).
        child.set_hexpand(true);
        child.set_vexpand(true);
        disc.append(child);
        // Keep that from stretching the disc itself.
        disc.set_hexpand(false);
        disc.set_vexpand(false);
        disc
    }

    /// A tile: `room` (the disc), a name and a usage line under it.
    fn item(&self, room: &gtk::Overlay, name: &str) -> (gtk::Button, gtk::Label, gtk::Label) {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .build();
        column.append(room);
        let name = gtk::Label::builder()
            .label(name)
            .css_classes(["tray-name"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(12)
            .build();
        column.append(&name);
        let usage = gtk::Label::builder()
            .css_classes(["tray-usage"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        column.append(&usage);
        let button = gtk::Button::builder()
            .child(&column)
            .css_classes(["flat", "tray-item"])
            .hexpand(true)
            .build();
        (button, name, usage)
    }

    fn dial_item(&self, lib: &LibraryDial, usage: Usage) -> gtk::Button {
        let zones = gtk::DrawingArea::new();
        let face = canvas::dial_face(&zones);
        let disc = self.disc(&face.overlay);
        let room = canvas::with_room(&disc, 6, 4, 2);
        let pin = canvas::slot_badge("pin", gtk::Align::End);
        room.add_overlay(&pin);
        let (button, name, usage_label) = self.item(&room, "");
        let id = lib.id;
        {
            let state = self.state.clone();
            button.connect_clicked(move |_| state.select_library(id));
        }
        // Drag it onto a dial on the deck to place it there.
        let source = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
        source.connect_prepare(move |_, _, _| {
            Some(gdk::ContentProvider::for_value(
                &canvas::library_dial_payload(id).to_value(),
            ))
        });
        {
            let disc = disc.clone();
            source.connect_drag_begin(move |src, _| {
                let icon = gtk::WidgetPaintable::new(Some(&disc));
                src.set_icon(Some(&icon), disc.width() / 2, disc.height() / 2);
            });
        }
        button.add_controller(source);
        let item = Item {
            id,
            disc,
            face,
            name,
            usage: usage_label,
            pin,
        };
        fill(&item, lib, usage);
        self.items.borrow_mut().push(item);
        button
    }

    /// + New Dial: a blank library dial, picked and in view.
    fn new_item(&self) -> gtk::Button {
        let plus = gtk::Image::builder()
            .icon_name("list-add-symbolic")
            .pixel_size(20)
            .build();
        let disc = self.disc(&plus);
        disc.add_css_class("new");
        let (button, _, _) = self.item(&canvas::with_room(&disc, 6, 4, 2), "New Dial");
        let me = self.me.borrow().clone();
        button.connect_clicked(move |_| {
            let Some(t) = me.upgrade() else { return };
            let mut id = None;
            t.state.edit(|e| id = Some(e.new_library_dial()));
            let Some(id) = id else { return };
            let items = t.state.editor.borrow().profile().dial_library.len();
            t.go(items.saturating_sub(1) / PER_PAGE);
            t.state.select_library(id);
        });
        button
    }

    /// Ring the picked library dial.
    fn show_picked(&self) {
        let picked = *self.state.selected_library.borrow();
        for item in self.items.borrow().iter() {
            canvas::set_class(&item.disc, "selected", picked == Some(item.id));
        }
    }
}

/// Show `lib` and where it is used on its tile.
fn fill(item: &Item, lib: &LibraryDial, usage: Usage) {
    item.face.show(&lib.dial);
    item.name.set_label(match lib.dial.name.trim() {
        "" => "Dial",
        n => n,
    });
    item.usage.set_label(&usage.text());
    canvas::set_class(&item.usage, "pinned", usage.pinned);
    item.pin.set_visible(usage.pinned);
}

/// A 0 to 1 animation on `w`'s frame clock, eased out.
fn eased(w: &impl IsA<gtk::Widget>, target: &impl IsA<adw::AnimationTarget>) -> adw::TimedAnimation {
    adw::TimedAnimation::builder()
        .widget(w)
        .value_from(0.0)
        .value_to(1.0)
        .duration(ANIM_MS)
        .easing(adw::Easing::EaseOutCubic)
        .target(target)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_fits_rows() {
        assert_eq!(layout(0, 0), Layout { pages: 1, rows: 1 });
        assert_eq!(layout(5, 0), Layout { pages: 1, rows: 1 });
        assert_eq!(layout(6, 0), Layout { pages: 1, rows: 2 });
        assert_eq!(layout(23, 0), Layout { pages: 1, rows: 4 });
        assert_eq!(layout(24, 0), Layout { pages: 2, rows: 4 });
        assert_eq!(layout(24, 1), Layout { pages: 2, rows: 1 });
        assert_eq!(layout(50, 2), Layout { pages: 3, rows: 1 });
        assert_eq!(layout(50, 1).rows, 4);
        // A page past the end shows the last one.
        assert_eq!(layout(24, 9), Layout { pages: 2, rows: 1 });
    }
}
