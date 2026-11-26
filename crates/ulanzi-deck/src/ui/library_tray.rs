//! The Library tray under the deck: library keys and dials (one tab each) to drag onto the deck or
//! click to pick. Opening it grows the tray at the bottom and slides the deck up only as far as
//! needed, never past its dial row.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ulanzi_core::model::{Action, LibraryDial, LibraryKey, MacroLibrary, Usage};
use uuid::Uuid;

use crate::state::{Change, LibraryKind, State};
use crate::ui::canvas;
use crate::ui::tile_drop::{self, Aim, Slot};

pub mod drag_test;

thread_local! {
    /// The window's tray, for the scripted drag test ([`drag_test`]).
    static LAST: RefCell<Weak<Tray>> = const { RefCell::new(Weak::new()) };
}
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
/// The tray dropping to its peek strip while a tile is carried out, and springing back.
const PEEK_DOWN_MS: u32 = 140;
const PEEK_UP_MS: u32 = 180;
/// A tile held at an edge of the tray turns the page after this. After a turn the edge rests
/// `EDGE_PAUSE_MS` before it can turn again, so a held tile does not race through the pages.
const EDGE_TURN_MS: u64 = 180;
const EDGE_PAUSE_MS: u64 = 900;
/// The edge's one orange pulse when it turns the page.
const EDGE_FLASH_MS: u64 = 450;
/// A held tile turns the page at the tray's outer edges: in its side padding, this far into the
/// tiles' area, or up to `EDGE_OUT` past the tray's side (pushing past the edge counts).
const EDGE_IN: f64 = 4.0;
const EDGE_OUT: f64 = 48.0;
/// Tiles sliding to their new places after a drop.
const REORDER_MS: u32 = 260;

const HINT: &str = "Drag a dial onto the deck to place it, or a dial from the deck down here to add it. \
Click one to edit it, right-click to copy, paste or delete it. Click ? for the tour.";
const KEY_HINT: &str = "Drag a key onto the deck to place it, or a key from the deck down here to add it. \
Click one to edit it, right-click to copy, paste or delete it. Click ? for the tour.";

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

/// The tray's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Keys,
    Dials,
}

impl Tab {
    /// The first-time guide about this tab's library.
    pub fn guide(self) -> crate::ui::library_guide::Kind {
        match self {
            Tab::Keys => crate::ui::library_guide::Kind::Keys,
            Tab::Dials => crate::ui::library_guide::Kind::Dials,
        }
    }

    fn index(self) -> usize {
        match self {
            Tab::Keys => 0,
            Tab::Dials => 1,
        }
    }
}

/// One library item as a tile shows it.
#[derive(Debug, Clone, PartialEq)]
enum Entry {
    Dial(LibraryDial),
    Key(LibraryKey),
}

impl Entry {
    fn id(&self) -> Uuid {
        match self {
            Entry::Dial(d) => d.id,
            Entry::Key(k) => k.id,
        }
    }

    /// Whether the search `query` (lowercase) finds it: by any text in it (name, labels, file and
    /// picture names, commands, plugin settings), or by what its actions are called, the plugins
    /// they come from and the macros they run.
    fn matches(&self, query: &str, plugins: &crate::plugins::Plugins, macros: &MacroLibrary) -> bool {
        if query.is_empty() {
            return true;
        }
        let value = match self {
            Entry::Key(k) => serde_json::to_value(k),
            Entry::Dial(d) => serde_json::to_value(d),
        };
        let mut words = Vec::new();
        if let Ok(v) = value {
            search_words(&v, plugins, macros, &mut words);
        }
        words.iter().any(|w| w.to_lowercase().contains(query))
    }
}

/// Every text in `v`, and for each action in it (a cycle's steps too) its title, its summary,
/// its plugin's name and its macro's name.
fn search_words(
    v: &serde_json::Value,
    plugins: &crate::plugins::Plugins,
    macros: &MacroLibrary,
    out: &mut Vec<String>,
) {
    match v {
        serde_json::Value::String(t) => out.push(t.clone()),
        serde_json::Value::Array(a) => a.iter().for_each(|v| search_words(v, plugins, macros, out)),
        serde_json::Value::Object(m) => {
            if m.get("type").is_some_and(|t| t.is_string())
                && let Ok(a) = serde_json::from_value::<Action>(v.clone())
            {
                out.push(plugins.describe(&a));
                if let Some(spec) = ulanzi_core::catalog::spec_for(&a) {
                    out.push(spec.title.to_string());
                }
                match &a {
                    Action::Plugin { plugin, .. } => out.push(plugins.plugin_name(plugin)),
                    Action::Macro { id, own } => {
                        if let Some(m) = own.as_deref().or_else(|| macros.get(*id)) {
                            out.push(m.name.clone());
                        }
                    }
                    _ => {}
                }
            }
            m.values().for_each(|v| search_words(v, plugins, macros, out));
        }
        _ => {}
    }
}

type OnToggle = Box<dyn Fn(bool)>;
/// What a library dial's menu item does, given the dial's tile.
type TileAction = Box<dyn Fn(&State, &gtk::Button)>;

/// A library item's tile, kept so edits update it in place.
struct Item {
    id: Uuid,
    /// What the tile shows (picture, name, usage): this moves aside to open a gap, and hides
    /// while a picture of it slides to its new place; the button around it stays put. Before
    /// `button`, so it lets go first and the button is never finalized with it still held.
    column: gtk::Box,
    button: gtk::Button,
    disc: gtk::Box,
    /// A dial's face, or a key's picture.
    face: Option<canvas::DialFace>,
    picture: Option<gtk::Image>,
    name: gtk::Label,
    usage: gtk::Label,
    pin: gtk::Image,
}

pub struct Tray {
    /// Holds the tray's content, with the page-turn lights over its sides.
    rim: gtk::Overlay,
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
    drawn: RefCell<Vec<(Entry, Usage)>>,
    /// The tab shown, its switch, and the page each tab was on.
    tab: Cell<Tab>,
    tabs: [gtk::ToggleButton; 2],
    /// The Keys | Dials switch; hidden on a deck without dials.
    switch: gtk::Box,
    tab_pages: Cell<[usize; 2]>,
    help: gtk::Button,
    /// Shown once the library fills more than one page; `query` is what it holds, lowercase.
    search: gtk::SearchEntry,
    query: RefCell<String>,
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
    /// The top row; while a tile is carried onto the deck the tray drops to just this
    /// (the peek strip), with `peek_hint` in it.
    header: gtk::Box,
    peek_hint: gtk::Label,
    /// How far down to the peek strip, 0 to 1, and whether it is going there.
    peek: Cell<f64>,
    peeked: Cell<bool>,
    /// The window watches where a carried tile is.
    watching: Cell<bool>,
    /// Glowing edges shown while a tile is dragged, to turn to the page before or after.
    edges: [gtk::Box; 2],
    edge_timer: RefCell<Option<glib::SourceId>>,
    /// When an edge last turned the page, for the pause after it.
    edge_turned: Cell<Option<std::time::Instant>>,
    /// The side a carried tile is held at (-1 left, 1 right).
    edge_at: Cell<Option<isize>>,
    /// What letting go of the carried tile would do, as shown.
    aim: Cell<Aim>,
    /// The mark between two tiles where a carried tile would go.
    gap_mark: gtk::Box,
    /// The dashed ring round the picture a carried tile would swap with. Drawn over the tiles,
    /// not on the tile, so the picture that slides the tile away never carries it.
    swap_mark: gtk::Box,
    /// Pictures of tiles sliding to their new places after a drop, over the pages.
    ghosts: gtk::Fixed,
    reorder_anim: RefCell<Option<adw::TimedAnimation>>,
    peek_anim: RefCell<Option<adw::TimedAnimation>>,
    /// The page the grids are drawn at (between two while sliding) and the page area's height.
    offset: Cell<f64>,
    height: Cell<f64>,
    from: Cell<(f64, f64)>,
    to: Cell<(f64, f64)>,
    slide: RefCell<Option<adw::TimedAnimation>>,
    last_size: Cell<(i32, i32)>,
    /// The library dial being dragged from its tile, so the others can take it to reorder.
    moving: Cell<Option<Uuid>>,
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

        // Keys | Dials, linked like a view switcher.
        let keys_tab = gtk::ToggleButton::builder().label("Keys").active(true).build();
        let dials_tab = gtk::ToggleButton::builder()
            .label("Dials")
            .group(&keys_tab)
            .build();
        let switch = gtk::Box::builder().css_classes(["linked", "tray-tabs"]).build();
        switch.append(&keys_tab);
        switch.append(&dials_tab);
        // The hint shows only on hovering ?.
        let help = gtk::Button::builder()
            .label("?")
            .css_classes(["circular", "tray-help"])
            .tooltip_text(KEY_HINT)
            .action_name("win.library-guide")
            .valign(gtk::Align::Center)
            .build();
        let shut = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .css_classes(["circular", "flat", "tray-help"])
            .tooltip_text("Close Library")
            .valign(gtk::Align::Center)
            .build();
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search")
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        search.add_css_class("tray-search");
        let header = gtk::Box::builder().spacing(6).margin_bottom(6).build();
        header.append(&switch);
        let peek_hint = gtk::Label::builder()
            .label("Drop here to cancel")
            .css_classes(["tray-title"])
            .margin_start(6)
            .visible(false)
            .build();
        header.append(&peek_hint);
        header.append(&search);
        // Keeps ? and close at the end with the switch hidden.
        header.append(&gtk::Box::builder().hexpand(true).build());
        header.append(&help);
        header.append(&shut);
        let sizer = gtk::Box::builder().hexpand(true).build();
        let pages = gtk::Overlay::builder()
            .child(&sizer)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["tray-pages"])
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
        // The page-turn lights sit over the tray's own sides.
        let rim = gtk::Overlay::builder().child(&content).build();
        // Never scrolled: the frame only clips the tray while it grows and shrinks.
        let frame = gtk::ScrolledWindow::builder()
            .child(&rim)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .css_classes(["library-frame"])
            .visible(false)
            .build();
        stage.add_overlay(&frame);

        let tray = Rc::new(Tray {
            rim,
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
            tab: Cell::new(Tab::Keys),
            tabs: [keys_tab, dials_tab],
            switch,
            tab_pages: Cell::new([0, 0]),
            help,
            search,
            query: RefCell::default(),
            pager,
            with_pager: Cell::new(false),
            dots,
            prev,
            next,
            open: Cell::new(false),
            stale: Cell::new(true),
            shown: Cell::new(0.0),
            grow: RefCell::default(),
            header,
            peek_hint,
            peek: Cell::new(0.0),
            peeked: Cell::new(false),
            watching: Cell::new(false),
            edges: [-1isize, 1].map(|step| {
                let edge = gtk::Box::builder()
                    .css_classes(["tray-edge", if step < 0 { "prev" } else { "next" }])
                    .overflow(gtk::Overflow::Hidden)
                    .halign(if step < 0 {
                        gtk::Align::Start
                    } else {
                        gtk::Align::End
                    })
                    .valign(gtk::Align::Fill)
                    .visible(false)
                    .can_target(false)
                    .build();
                // The orange pulse as the page turns, over the soft light.
                edge.append(
                    &gtk::Box::builder()
                        .css_classes(["tray-edge-flash"])
                        .hexpand(true)
                        .vexpand(true)
                        .build(),
                );
                edge
            }),
            edge_timer: RefCell::default(),
            edge_turned: Cell::new(None),
            edge_at: Cell::new(None),
            aim: Cell::new(Aim::Nothing),
            gap_mark: gtk::Box::builder()
                .css_classes(["tray-gap-mark"])
                .width_request(4)
                .can_target(false)
                .build(),
            swap_mark: gtk::Box::builder()
                .css_classes(["tray-swap-mark"])
                .can_target(false)
                .build(),
            ghosts: gtk::Fixed::builder().can_target(false).build(),
            reorder_anim: RefCell::default(),
            peek_anim: RefCell::default(),
            offset: Cell::new(0.0),
            height: Cell::new(0.0),
            from: Cell::new((0.0, 0.0)),
            to: Cell::new((0.0, 0.0)),
            slide: RefCell::default(),
            last_size: Cell::new((0, 0)),
            moving: Cell::new(None),
            on_toggle: RefCell::default(),
        });
        *tray.me.borrow_mut() = Rc::downgrade(&tray);
        {
            let me = Rc::downgrade(&tray);
            shut.connect_clicked(move |_| {
                if let Some(t) = me.upgrade() {
                    t.close();
                }
            });
        }
        for (i, tab) in [Tab::Keys, Tab::Dials].into_iter().enumerate() {
            let me = Rc::downgrade(&tray);
            tray.tabs[i].connect_toggled(move |b| {
                if b.is_active()
                    && let Some(t) = me.upgrade()
                {
                    t.set_tab(tab);
                }
            });
        }
        {
            let me = Rc::downgrade(&tray);
            tray.search.connect_search_changed(move |e| {
                if let Some(t) = me.upgrade() {
                    *t.query.borrow_mut() = e.text().trim().to_lowercase();
                    t.state.tray_page.set(0);
                    // Laid out again, so "No keys match" names the new text.
                    t.drawn.borrow_mut().clear();
                    t.items.borrow_mut().clear();
                    t.refresh();
                }
            });
        }
        tray.accept_deck_dials();
        tray.wire();
        tray.peek_when_carried_out();
        tray.turn_at_edges();
        tray.take_moved_tiles();
        LAST.with(|l| *l.borrow_mut() = Rc::downgrade(&tray));
        tray
    }

    /// While a tile is dragged in the tray, holding it at a side where there is another page
    /// turns to it: the side lights up softly under the tile, pulses orange once as the page
    /// turns, and rests a moment before it turns again. Where the tile is comes from the
    /// window's watch ([`Tray::peek_when_carried_out`]); the sides themselves take no pointer.
    fn turn_at_edges(&self) {
        for edge in &self.edges {
            self.rim.add_overlay(edge);
        }
        self.pages.add_overlay(&self.ghosts);
        self.ghosts.put(&self.gap_mark, 0.0, 0.0);
        self.ghosts.put(&self.swap_mark, 0.0, 0.0);
    }

    /// Which outer edge of the tray `(x, y)` in `from` is at (-1 left, 1 right): the tray's side
    /// padding, a few pixels into the tiles' area, or just past the tray's side, at the tray's
    /// height.
    fn edge_zone(&self, from: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Option<isize> {
        let pt = |w: &gtk::Widget, x: f64, y: f64, to: &gtk::Widget| {
            w.compute_point(to, &gtk::graphene::Point::new(x as f32, y as f32))
                .map(|p| (p.x() as f64, p.y() as f64))
        };
        let frame: &gtk::Widget = self.frame.upcast_ref();
        let (px, py) = pt(from.upcast_ref(), x, y, frame)?;
        if !self.frame.is_visible() || py < 0.0 || py > self.frame.height() as f64 {
            return None;
        }
        // Where the tiles' area starts and ends across the tray.
        let pages: &gtk::Widget = self.pages.upcast_ref();
        let (left, _) = pt(pages, 0.0, 0.0, frame)?;
        let (right, _) = pt(pages, self.pages.width() as f64, 0.0, frame)?;
        let w = self.frame.width() as f64;
        if px > -EDGE_OUT && px < left + EDGE_IN {
            Some(-1)
        } else if px > right - EDGE_IN && px < w + EDGE_OUT {
            Some(1)
        } else {
            None
        }
    }

    /// [`Tray::edge_zone`] while a tile is carried, where there is a page to turn to.
    fn edge_step(&self, from: &impl IsA<gtk::Widget>, x: f64, y: f64) -> Option<isize> {
        self.moving.get()?;
        let step = self.edge_zone(from, x, y)?;
        self.edges[if step < 0 { 0 } else { 1 }]
            .is_visible()
            .then_some(step)
    }

    /// The carried tile is now held at side `step`, or at none.
    fn edge_hover(&self, step: Option<isize>) {
        if self.edge_at.replace(step) == step {
            return;
        }
        self.stop_edge_timer();
        for (edge, s) in self.edges.iter().zip([-1isize, 1]) {
            if step == Some(s) {
                edge.add_css_class("hover");
            } else {
                edge.remove_css_class("hover");
            }
        }
        if let Some(step) = step {
            self.show_aim(Aim::Nothing);
            self.arm_edge(step, if step < 0 { 0 } else { 1 });
        }
    }

    /// Turn the page `step` once the tile has been held long enough and the last turn's pause is
    /// over; while the tile stays, arm again for the next one.
    fn arm_edge(&self, step: isize, i: usize) {
        self.stop_edge_timer();
        let pause = self
            .edge_turned
            .get()
            .map(|at| std::time::Duration::from_millis(EDGE_PAUSE_MS).saturating_sub(at.elapsed()))
            .unwrap_or_default();
        let wait = pause.max(std::time::Duration::from_millis(EDGE_TURN_MS));
        let me = self.me.borrow().clone();
        let id = glib::timeout_add_local_once(wait, move || {
            let Some(t) = me.upgrade() else { return };
            t.edge_timer.borrow_mut().take();
            if !t.turn_by(step) {
                return;
            }
            t.edge_turned.set(Some(std::time::Instant::now()));
            let edge = t.edges[i].clone();
            edge.remove_css_class("flash");
            edge.add_css_class("flash");
            glib::timeout_add_local_once(std::time::Duration::from_millis(EDGE_FLASH_MS), move || {
                edge.remove_css_class("flash")
            });
            if t.edges[i].is_visible() && t.edge_at.get() == Some(step) {
                t.arm_edge(step, i);
            }
        });
        *self.edge_timer.borrow_mut() = Some(id);
    }

    fn stop_edge_timer(&self) {
        if let Some(id) = self.edge_timer.borrow_mut().take() {
            id.remove();
        }
    }

    /// Turn one page back (-1) or on (1) while a tile is dragged; false when there is none.
    fn turn_by(&self, step: isize) -> bool {
        if self.moving.get().is_none() {
            return false;
        }
        let page = self.state.tray_page.get();
        let pages = layout(self.count(), page).pages;
        let Some(to) = page.checked_add_signed(step).filter(|p| *p < pages) else {
            return false;
        };
        self.go(to);
        self.show_edges();
        true
    }

    /// The glowing edges show while a tile is dragged, each only where there is a page to go to.
    fn show_edges(&self) {
        let dragging = self.moving.get().is_some();
        let page = self.state.tray_page.get();
        let pages = layout(self.count(), page).pages;
        for (edge, show) in self.edges.iter().zip([page > 0, page + 1 < pages]) {
            let show = dragging && show;
            if !show {
                edge.remove_css_class("hover");
            }
            edge.set_visible(show);
        }
        let held = self.edge_at.get();
        if held.is_some_and(|s| !self.edges[if s < 0 { 0 } else { 1 }].is_visible()) || !dragging {
            self.edge_at.set(None);
        }
        if !dragging {
            self.stop_edge_timer();
        }
    }

    /// A tile carried up out of the tray drops the tray to its peek strip, so the whole deck is in
    /// reach; back over the strip, the tray rises again. The drag's end brings it back too.
    fn peek_when_carried_out(&self) {
        // Watched on the whole window, not the tray: a quick flick can leave the tray between two
        // pointer events, so the tray never sees the pointer go.
        let me = self.me.borrow().clone();
        self.stage.connect_realize(move |stage| {
            let Some(root) = stage.root() else { return };
            let root: gtk::Widget = root.upcast();
            // Once per window, however often the stage is realized.
            let Some(t) = me.upgrade() else { return };
            if t.watching.replace(true) {
                return;
            }
            let motion = gtk::DropControllerMotion::new();
            let follow = {
                let (me, root) = (me.clone(), root.downgrade());
                move |x: f64, y: f64| {
                    let (Some(t), Some(root)) = (me.upgrade(), root.upgrade()) else {
                        return;
                    };
                    if t.state.library_drag.get().is_none() {
                        return;
                    }
                    let over_tray = root
                        .compute_point(&t.frame, &gtk::graphene::Point::new(x as f32, y as f32))
                        .is_some_and(|p| t.frame.contains(p.x() as f64, p.y() as f64));
                    // Pushed just past the tray's side is still the tray: that edge turns pages.
                    let beside = t.moving.get().is_some() && t.edge_zone(&root, x, y).is_some();
                    t.set_peek(!(over_tray || beside));
                    t.edge_hover(t.edge_step(&root, x, y));
                }
            };
            {
                let follow = follow.clone();
                motion.connect_enter(move |_, x, y| follow(x, y));
            }
            motion.connect_motion(move |_, x, y| follow(x, y));
            {
                let me = me.clone();
                motion.connect_leave(move |_| {
                    // Not a peek: GTK reports this leave as a drop ends too, before the drag's
                    // end; peeking here dropped the tray for a moment on every drop.
                    if let Some(t) = me.upgrade() {
                        tracing::debug!(target: "dnd", "window leave");
                        t.edge_hover(None);
                    }
                });
            }
            root.add_controller(motion);
        });
    }

    /// Drop to the peek strip (`true`) or rise back to the tray's height.
    fn set_peek(&self, down: bool) {
        if self.peeked.replace(down) == down {
            return;
        }
        self.peek_hint.set_visible(down);
        if let Some(a) = self.peek_anim.borrow().as_ref() {
            a.pause();
            a.set_value_from(self.peek.get());
            a.set_value_to(if down { 1.0 } else { 0.0 });
            // Down quickly; up with a little bounce.
            a.set_duration(if down { PEEK_DOWN_MS } else { PEEK_UP_MS });
            a.set_easing(if down {
                adw::Easing::EaseOutCubic
            } else {
                adw::Easing::EaseOutBack
            });
            a.play();
        }
    }

    /// The page dots and hints under the deck show as the tray goes. They stay hidden while a
    /// tile is carried out: the tray keeps the room under the deck then.
    fn fade_under_deck(&self) {
        let covered = self.shown.get();
        for w in &self.fade {
            w.set_opacity(1.0 - covered.clamp(0.0, 1.0));
        }
    }

    /// Show `tab`, on the page it was on.
    pub fn set_tab(&self, tab: Tab) {
        // A D200 has no dials, so it has no dial library to show.
        let tab = if self.with_dials() { tab } else { Tab::Keys };
        self.switch.set_visible(self.with_dials());
        if !self.tabs[tab.index()].is_active() {
            // Comes back here through the toggle.
            self.tabs[tab.index()].set_active(true);
            return;
        }
        let was = self.tab.replace(tab);
        if was == tab && !self.grids.borrow().is_empty() {
            return;
        }
        let mut pages = self.tab_pages.get();
        pages[was.index()] = self.state.tray_page.get();
        self.tab_pages.set(pages);
        self.state.tray_page.set(pages[tab.index()]);
        self.help.set_tooltip_text(Some(match tab {
            Tab::Keys => KEY_HINT,
            Tab::Dials => HINT,
        }));
        // A search is for one tab.
        self.query.borrow_mut().clear();
        self.search.set_text("");
        // The other tab's tiles go, even when both tabs list nothing.
        self.drawn.borrow_mut().clear();
        self.items.borrow_mut().clear();
        for g in self.grids.take() {
            self.pages.remove_overlay(&g);
        }
        if self.open.get() {
            self.refresh();
            self.first_guide(200);
        } else {
            self.stale.set(true);
        }
    }

    fn with_dials(&self) -> bool {
        self.state.model() == ulanzi_core::model::Model::D200X
    }

    /// The items the shown tab lists, found by the search.
    fn count(&self) -> usize {
        self.drawn.borrow().len()
    }

    /// The deck with the tray under it.
    pub fn widget(&self) -> gtk::Widget {
        self.stage.clone().upcast()
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// The tab shown.
    pub fn tab(&self) -> Tab {
        self.tab.get()
    }

    /// Called with the new state whenever the tray opens or closes.
    pub fn connect_toggled(&self, f: impl Fn(bool) + 'static) {
        *self.on_toggle.borrow_mut() = Some(Box::new(f));
    }

    pub fn open(&self, tab: Tab) {
        self.set_tab(tab);
        if self.open.replace(true) {
            return;
        }
        self.state.tray_open.set(true);
        self.state.notify(Change::Tray);
        if self.stale.get() {
            self.refresh();
        }
        // Sized afresh: the dials may have changed size while it was shut.
        self.go(self.state.tray_page.get());
        self.frame.set_visible(true);
        self.run_grow(1.0);
        self.toggled(true);
        // The first time, the guide follows once the tray has grown.
        self.first_guide(ANIM_MS as u64 + 100);
    }

    /// Open the shown tab's guide after `delay_ms`, the first time only.
    fn first_guide(&self, delay_ms: u64) {
        let kind = self.tab.get().guide();
        if !crate::ui::library_guide::should_show(kind) {
            return;
        }
        crate::ui::library_guide::mark_shown(kind);
        let (state, stage, me) = (
            self.state.clone(),
            self.stage.downgrade(),
            self.me.borrow().clone(),
        );
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay_ms), move || {
            // Not over a tray that was shut meanwhile.
            let still_open = me.upgrade().is_some_and(|t| t.is_open());
            if let Some(stage) = stage.upgrade().filter(|_| still_open) {
                crate::ui::library_guide::open(&state, &stage, kind);
            }
        });
    }

    pub fn close(&self) {
        if !self.open.replace(false) {
            return;
        }
        self.state.tray_open.set(false);
        self.state.notify(Change::Tray);
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

    /// A key or dial dragged from the deck into the tray is added to the library.
    fn accept_deck_dials(&self) {
        // The deck's keys and dials move; library tiles are copied.
        let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        {
            let content = self.content.clone();
            target.connect_enter(move |_, _, _| {
                content.add_css_class("drop-target");
                gdk::DragAction::MOVE
            });
        }
        {
            let content = self.content.clone();
            target.connect_leave(move |_| content.remove_css_class("drop-target"));
        }
        let me = self.me.borrow().clone();
        target.connect_drop(move |_, value, _, _| {
            let Some(t) = me.upgrade() else { return false };
            t.content.remove_css_class("drop-target");
            let (tab, added) = match value.get::<String>().ok().and_then(|s| canvas::parse_payload(&s)) {
                Some(canvas::Payload::DeckDial(n)) => {
                    let mut added = None;
                    t.state.edit(|e| added = e.add_dial_to_library(n));
                    (Tab::Dials, added)
                }
                // Carried from another page: the number is that page's, not this one's.
                Some(canvas::Payload::Key(_))
                    if t.state
                        .carried_key
                        .get()
                        .is_some_and(|(page, _)| page != t.state.editor.borrow().page()) =>
                {
                    return false;
                }
                Some(canvas::Payload::Key(n)) => {
                    let mut added = None;
                    t.state.edit(|e| added = e.add_key_to_library(n));
                    (Tab::Keys, added)
                }
                _ => return false,
            };
            if added.is_some() {
                // Shown where it landed: the last page of its tab.
                t.set_tab(tab);
                t.go(t.count().saturating_sub(1) / PER_PAGE);
            }
            added.is_some()
        });
        self.frame.add_controller(target);
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
                    t.fade_under_deck();
                    t.stage.queue_allocate();
                }
            })
        };
        let grow = eased(&self.stage, &grow_target);
        let peek_target = {
            let me = me.clone();
            adw::CallbackAnimationTarget::new(move |v| {
                if let Some(t) = me.upgrade() {
                    // The bounce overshoots: never above the tray's own height.
                    t.peek.set(v.clamp(0.0, 1.0));
                    t.fade_under_deck();
                    t.stage.queue_allocate();
                }
            })
        };
        *self.peek_anim.borrow_mut() = Some(eased(&self.stage, &peek_target));
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
            let turn = {
                let me = me.clone();
                move || {
                    if let Some(t) = me.upgrade() {
                        let page = t.state.tray_page.get().saturating_add_signed(step);
                        t.go(page);
                    }
                }
            };
            b.connect_clicked(move |_| turn());
        }
        let st = self.state.clone();
        st.on(move |c| {
            let Some(t) = me.upgrade() else { return };
            match c {
                Change::Profile | Change::Plugins => {
                    if !t.with_dials() && t.tab.get() == Tab::Dials {
                        t.set_tab(Tab::Keys);
                    }
                    t.switch.set_visible(t.with_dials());
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
        // A tile carried out: the whole deck shows where it sits, and the tray keeps the room
        // left under it, in whole rows of tiles, down to just its top row.
        let peek = self.peek.get();
        if peek > 0.0 && th > 0.0 {
            let room = body.map_or(th, |b| h - MARGIN - (b.y() as f64 + b.height() as f64 + GAP));
            let fit = fit_tray(room, self.peek_height(), th, &self.row_bottoms(), self.pad());
            th += (fit - th) * peek;
        }
        let shift = match (body, self.keep.compute_bounds(&self.deck)) {
            (Some(b), Some(k)) if th > 0.0 => {
                let bottom = b.y() as f64 + b.height() as f64;
                // The deck only ever moves up out of the way.
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

    /// The tray's padding above its top row (kept under the last row shown, too).
    fn pad(&self) -> f64 {
        self.header
            .compute_bounds(&self.content)
            .map_or(12.0, |b| b.y() as f64)
    }

    /// The peek strip's height: the tray's top row with its padding.
    fn peek_height(&self) -> f64 {
        let row = self
            .header
            .compute_bounds(&self.content)
            .map_or(40.0, |b| (b.y() + b.height()) as f64);
        row + self.pad()
    }

    /// Where each row of tiles ends, from the tray's top, smallest first.
    fn row_bottoms(&self) -> Vec<f64> {
        let mut rows: Vec<f64> = self
            .items
            .borrow()
            .iter()
            .filter_map(|i| i.usage.compute_bounds(&self.content))
            .map(|b| ((b.y() + b.height()) as f64).round())
            .collect();
        rows.sort_by(f64::total_cmp);
        rows.dedup();
        rows
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
        let l = layout(self.count(), page);
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
        let now: Vec<(Entry, Usage)> = {
            let e = self.state.editor.borrow();
            let p = e.profile();
            match self.tab.get() {
                Tab::Dials => p
                    .dial_library
                    .iter()
                    .map(|d| (Entry::Dial(d.clone()), p.dial_usage(d.id)))
                    .collect(),
                Tab::Keys => p
                    .key_library
                    .iter()
                    .map(|k| (Entry::Key(k.clone()), p.key_usage(k.id)))
                    .collect(),
            }
        };
        // Search once there is more than a page, and while a search is typed.
        let query = self.query.borrow().clone();
        self.search.set_visible(now.len() > PER_PAGE || !query.is_empty());
        let now: Vec<(Entry, Usage)> = {
            let (plugins, e) = (self.state.plugins.borrow(), self.state.editor.borrow());
            now.into_iter()
                .filter(|(l, _)| l.matches(&query, &plugins, e.library()))
                .collect()
        };
        if *self.drawn.borrow() == now && !self.grids.borrow().is_empty() {
            return;
        }
        let same = {
            let items = self.items.borrow();
            items.len() == now.len() && items.iter().zip(&now).all(|(i, (l, _))| i.id == l.id())
        };
        if same && !self.grids.borrow().is_empty() {
            let dir = canvas::profile_dir(&self.state);
            for (item, (lib, usage)) in self.items.borrow().iter().zip(&now) {
                fill(item, lib, *usage, &dir);
            }
        } else {
            self.rebuild(&now);
        }
        *self.drawn.borrow_mut() = now;
    }

    /// Lay out the shown tab's items again.
    fn rebuild(&self, now: &[(Entry, Usage)]) {
        for g in self.grids.take() {
            self.pages.remove_overlay(&g);
        }
        // The pager counts these.
        *self.drawn.borrow_mut() = now.to_vec();
        self.items.borrow_mut().clear();
        let (library, usage): (Vec<Entry>, Vec<Usage>) = now.iter().cloned().unzip();
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
                .map(|i| self.lib_item(&library[i], usage[i]).upcast())
                .collect();
            let searching = !self.query.borrow().is_empty();
            if !searching {
                cells.push(match self.tab.get() {
                    Tab::Dials => self.new_item().upcast(),
                    Tab::Keys => self.add_key_item().upcast(),
                });
            } else if library.is_empty() {
                let what = if self.tab.get() == Tab::Keys {
                    "keys"
                } else {
                    "dials"
                };
                let none = gtk::Label::builder()
                    .label(format!("No {what} match “{}”.", self.search.text().trim()))
                    .css_classes(["dim-label"])
                    .margin_top(12)
                    .margin_bottom(12)
                    .build();
                grid.attach(&none, 0, 0, PER_ROW as i32, 1);
            }
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
        // The sliding pictures stay over the tiles.
        if self.ghosts.parent().is_some() {
            self.pages.remove_overlay(&self.ghosts);
        }
        self.pages.add_overlay(&self.ghosts);
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

    /// A library dial's or key's tile.
    fn lib_item(&self, lib: &Entry, usage: Usage) -> gtk::Button {
        let (face, picture, disc) = match lib {
            Entry::Dial(_) => {
                let face = canvas::dial_face(&gtk::DrawingArea::new());
                let disc = self.disc(&face.overlay);
                (Some(face), None, disc)
            }
            Entry::Key(_) => {
                let picture = gtk::Image::new();
                let disc = self.disc(&picture);
                disc.add_css_class("tray-key");
                // The picture takes the tile's rounded corners.
                disc.set_overflow(gtk::Overflow::Hidden);
                (None, Some(picture), disc)
            }
        };
        let room = canvas::with_room(&disc, 6, 4, 2);
        let pin = canvas::slot_badge("pin", gtk::Align::End);
        room.add_overlay(&pin);
        let (button, name, usage_label) = self.item(&room, "");
        let id = lib.id();
        let kind = match lib {
            Entry::Dial(_) => Tab::Dials,
            Entry::Key(_) => Tab::Keys,
        };
        // Ctrl-click adds or takes out one item, Shift-click picks a run of them (held keys read
        // as on the deck).
        {
            let state = self.state.clone();
            button.connect_clicked(move |b| match canvas::select_op(canvas::modifiers(b)) {
                canvas::SelectOp::Only => state.select_library(id),
                canvas::SelectOp::Toggle => state.toggle_library(id),
                canvas::SelectOp::Range => {
                    let order: Vec<Uuid> = {
                        let e = state.editor.borrow();
                        match kind {
                            Tab::Keys => e.profile().key_library.iter().map(|l| l.id).collect(),
                            Tab::Dials => e.profile().dial_library.iter().map(|l| l.id).collect(),
                        }
                    };
                    state.extend_library(id, &order);
                }
            });
        }
        // Drag it onto the deck to place it there.
        let source = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
        source.connect_prepare(move |_, _, _| {
            let payload = match kind {
                Tab::Dials => canvas::library_dial_payload(id),
                Tab::Keys => canvas::library_key_payload(id),
            };
            Some(gdk::ContentProvider::for_value(&payload.to_value()))
        });
        let me = self.me.borrow().clone();
        {
            let (disc, me) = (disc.clone(), me.clone());
            source.connect_drag_begin(move |src, _| {
                let icon = gtk::WidgetPaintable::new(Some(&disc));
                src.set_icon(Some(&icon), disc.width() / 2, disc.height() / 2);
                if let Some(t) = me.upgrade() {
                    t.carry_begin(id, kind);
                }
            });
        }
        {
            let me = me.clone();
            source.connect_drag_end(move |_, _, _| {
                if let Some(t) = me.upgrade() {
                    t.carry_end();
                }
            });
        }
        button.add_controller(source);
        self.tile_menu(&button, id, kind);
        let item = Item {
            id,
            column: button
                .child()
                .and_downcast::<gtk::Box>()
                .expect("a tile holds a column"),
            button: button.clone(),
            disc,
            face,
            picture,
            name,
            usage: usage_label,
            pin,
        };
        fill(&item, lib, usage, &canvas::profile_dir(&self.state));
        self.items.borrow_mut().push(item);
        button
    }

    /// A library tile was picked up (a drag started on it).
    fn carry_begin(&self, id: Uuid, kind: Tab) {
        tracing::debug!(target: "dnd", "carry begin {id}");
        self.moving.set(Some(id));
        self.aim.set(Aim::Nothing);
        self.pages.add_css_class("carrying");
        self.show_edges();
        self.state.library_drag.set(Some(match kind {
            Tab::Keys => LibraryKind::Key,
            Tab::Dials => LibraryKind::Dial,
        }));
        // The tile stays in place, faded, until it lands somewhere. Faded on the button, not on
        // what it shows: the picture that slides it to its new place is taken from that.
        if let Some(i) = self.items.borrow().iter().find(|i| i.id == id) {
            i.button.add_css_class("carried");
            i.button.set_opacity(0.35);
        }
    }

    /// The carried tile was let go (dropped or not).
    fn carry_end(&self) {
        tracing::debug!(target: "dnd", "carry end");
        if let Some(id) = self.moving.take()
            && let Some(i) = self.items.borrow().iter().find(|i| i.id == id)
        {
            i.button.remove_css_class("carried");
            i.button.set_opacity(1.0);
        }
        self.show_aim(Aim::Nothing);
        self.pages.remove_css_class("carrying");
        self.show_edges();
        self.state.library_drag.set(None);
        self.set_peek(false);
    }

    /// One drop target over the whole page area takes the carried tile: on a tile's picture the
    /// two swap places, between two pictures it goes into that gap and the tiles after it shift
    /// on. What is under the pointer comes from where the tiles are laid out
    /// ([`tile_drop::aim_at`]), so tiles moving aside to open a gap never change it.
    fn take_moved_tiles(&self) {
        let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
        let me = self.me.borrow().clone();
        {
            let me = me.clone();
            // Only this tray's own tiles; a deck key or dial goes on to the tray's own target.
            target.connect_accept(move |_, _| me.upgrade().is_some_and(|t| t.moving.get().is_some()));
        }
        {
            let me = me.clone();
            target.connect_enter(move |_, x, y| {
                if let Some(t) = me.upgrade() {
                    t.hover_at(x, y);
                }
                gdk::DragAction::COPY
            });
        }
        {
            let me = me.clone();
            target.connect_motion(move |_, x, y| {
                if let Some(t) = me.upgrade() {
                    t.hover_at(x, y);
                }
                gdk::DragAction::COPY
            });
        }
        {
            let me = me.clone();
            target.connect_leave(move |_| {
                if let Some(t) = me.upgrade() {
                    t.show_aim(Aim::Nothing);
                }
            });
        }
        target.connect_drop(move |_, _, x, y| me.upgrade().is_some_and(|t| t.drop_at(x, y)));
        self.pages.add_controller(target);
    }

    /// The shown tiles on this page, where they are laid out in the page area.
    fn slots(&self) -> Vec<Slot> {
        let page = self.state.tray_page.get();
        self.items
            .borrow()
            .iter()
            .enumerate()
            .skip(page * PER_PAGE)
            .take(PER_PAGE)
            .filter_map(|(k, i)| {
                // The button never moves; the picture is in its middle (the column that moves
                // aside is inside it, so the picture's own bounds can be off by that).
                let b = i.button.compute_bounds(&self.pages)?;
                let d = i.disc.compute_bounds(&i.button)?;
                let mid = b.x() as f64 + b.width() as f64 / 2.0;
                let half = d.width() as f64 / 2.0;
                Some(Slot {
                    index: k,
                    row: (k % PER_PAGE) / PER_ROW,
                    top: b.y() as f64,
                    bottom: (b.y() + b.height()) as f64,
                    pic_left: mid - half,
                    pic_right: mid + half,
                    pic_top: (b.y() + d.y()) as f64,
                    pic_bottom: (b.y() + d.y() + d.height()) as f64,
                })
            })
            .collect()
    }

    /// The carried tile is over `(x, y)` in the page area: show what letting go there would do.
    fn hover_at(&self, x: f64, y: f64) -> Aim {
        let aim = match self.moving.get() {
            Some(id) if self.edge_step(&self.pages, x, y).is_none() => {
                let carried = self.items.borrow().iter().position(|i| i.id == id);
                tile_drop::aim_at(&self.slots(), x, y, carried, self.aim.get())
            }
            _ => Aim::Nothing,
        };
        self.show_aim(aim);
        aim
    }

    /// Show `aim`: the swapped tile's picture outlined, or the gap's two tiles moved apart with a
    /// mark between them.
    fn show_aim(&self, aim: Aim) {
        if self.aim.replace(aim) == aim {
            return;
        }
        tracing::debug!(target: "dnd", "aim {aim:?}");
        let slots = self.slots();
        let (left, right) = tile_drop::gap_neighbours(&slots, aim);
        for (k, item) in self.items.borrow().iter().enumerate() {
            let swap = aim == Aim::Swap(k);
            let classes = [
                ("swap-target", swap),
                ("scooch-left", left == Some(k)),
                ("scooch-right", right == Some(k)),
            ];
            for (class, on) in classes {
                if on {
                    item.button.add_css_class(class);
                } else {
                    item.button.remove_css_class(class);
                }
            }
        }
        match aim {
            Aim::Swap(k) => match slots.iter().find(|s| s.index == k) {
                Some(s) => {
                    const RING: f64 = 5.0;
                    self.swap_mark.set_size_request(
                        (s.pic_right - s.pic_left + 2.0 * RING).round() as i32,
                        (s.pic_bottom - s.pic_top + 2.0 * RING).round() as i32,
                    );
                    self.ghosts
                        .move_(&self.swap_mark, s.pic_left - RING, s.pic_top - RING);
                    if self.tab.get() == Tab::Dials {
                        self.swap_mark.add_css_class("round");
                    } else {
                        self.swap_mark.remove_css_class("round");
                    }
                    self.swap_mark.add_css_class("shown");
                }
                None => self.swap_mark.remove_css_class("shown"),
            },
            _ => self.swap_mark.remove_css_class("shown"),
        }
        match tile_drop::gap_mark(&slots, aim) {
            Some((x, top, bottom)) => {
                let w = self.gap_mark.width_request().max(1) as f64;
                self.gap_mark
                    .set_size_request(w as i32, (bottom - top).round() as i32);
                self.ghosts.move_(&self.gap_mark, x - w / 2.0, top);
                self.gap_mark.add_css_class("shown");
            }
            None => self.gap_mark.remove_css_class("shown"),
        }
    }

    /// The carried tile was let go at `(x, y)` in the page area.
    fn drop_at(&self, x: f64, y: f64) -> bool {
        let aim = self.hover_at(x, y);
        tracing::debug!(target: "dnd", "drop {aim:?}");
        let (Some(from), kind) = (self.moving.get(), self.tab.get()) else {
            return false;
        };
        let shown: Vec<Uuid> = self.items.borrow().iter().map(|i| i.id).collect();
        let edit: Box<dyn FnOnce(&mut ulanzi_core::edit::Editor)> = match aim {
            Aim::Nothing => {
                self.show_aim(Aim::Nothing);
                return false;
            }
            Aim::Swap(k) => {
                let Some(&with) = shown.get(k) else { return false };
                Box::new(move |e| match kind {
                    Tab::Dials => e.swap_library_dial(from, with),
                    Tab::Keys => e.swap_library_key(from, with),
                })
            }
            Aim::Gap { before, .. } => {
                let order: Vec<Uuid> = {
                    let e = self.state.editor.borrow();
                    match kind {
                        Tab::Dials => e.profile().dial_library.iter().map(|l| l.id).collect(),
                        Tab::Keys => e.profile().key_library.iter().map(|l| l.id).collect(),
                    }
                };
                let Some(to) = insert_to(&order, &shown, from, before) else {
                    self.show_aim(Aim::Nothing);
                    return false;
                };
                Box::new(move |e| match kind {
                    Tab::Dials => e.move_library_dial(from, to),
                    Tab::Keys => e.move_library_key(from, to),
                })
            }
        };
        self.reorder(from, (x, y), edit);
        true
    }

    /// Make a reorder `edit`, then slide the tiles on this page from where they are seen now (moved
    /// aside or not) to their new places. The carried tile `moved` comes from where it was let go,
    /// `landed`. Its new places are known before the edit (the same slots, in a new order), so
    /// the slide starts in the same frame: the new tiles never show before it.
    fn reorder(
        &self,
        moved: Uuid,
        landed: (f64, f64),
        edit: Box<dyn FnOnce(&mut ulanzi_core::edit::Editor)>,
    ) {
        if let Some(a) = self.reorder_anim.take() {
            a.skip();
        }
        let page = self.state.tray_page.get();
        let on_page = |k: usize| k / PER_PAGE == page;
        // Where each slot of this page is laid out, and where what a tile shows sits in it when not
        // moved aside (the same in every tile).
        let mut cells = Vec::new();
        let mut seen: Vec<(Uuid, gtk::graphene::Rect, gdk::Paintable)> = Vec::new();
        let inset = {
            let items = self.items.borrow();
            items
                .iter()
                .enumerate()
                .filter(|(k, i)| {
                    on_page(*k)
                        && !i.button.has_css_class("scooch-left")
                        && !i.button.has_css_class("scooch-right")
                })
                .find_map(|(_, i)| {
                    let (b, c) = (
                        i.button.compute_bounds(&self.ghosts)?,
                        i.column.compute_bounds(&self.ghosts)?,
                    );
                    Some((c.x() - b.x(), c.y() - b.y()))
                })
                .unwrap_or((0.0, 0.0))
        };
        {
            let items = self.items.borrow();
            for (k, i) in items.iter().enumerate() {
                if i.id == moved {
                    // Held by its picture's middle where it was let go.
                    let (cw, ch) = (i.column.width() as f32, i.column.height() as f32);
                    let (px, py) = i
                        .disc
                        .compute_bounds(&i.column)
                        .map_or((cw / 2.0, ch / 2.0), |d| {
                            (d.x() + d.width() / 2.0, d.y() + d.height() / 2.0)
                        });
                    let r = gtk::graphene::Rect::new(landed.0 as f32 - px, landed.1 as f32 - py, cw, ch);
                    seen.push((
                        i.id,
                        r,
                        gtk::WidgetPaintable::new(Some(&i.column)).current_image(),
                    ));
                    if on_page(k)
                        && let Some(b) = i.button.compute_bounds(&self.ghosts)
                    {
                        cells.push(b);
                    }
                    continue;
                }
                if !on_page(k) {
                    continue;
                }
                let (Some(b), Some(c)) = (
                    i.button.compute_bounds(&self.ghosts),
                    i.column.compute_bounds(&self.ghosts),
                ) else {
                    continue;
                };
                cells.push(b);
                seen.push((
                    i.id,
                    c,
                    gtk::WidgetPaintable::new(Some(&i.column)).current_image(),
                ));
            }
        }
        self.aim.set(Aim::Nothing);
        self.gap_mark.remove_css_class("shown");
        self.swap_mark.remove_css_class("shown");
        self.state.edit(edit);
        // The tray laid itself out again (new tiles, same slots); slide into them.
        let mut moves = Vec::new();
        let mut appear = Vec::new();
        let mut kept = Vec::new();
        {
            let items = self.items.borrow();
            for (k, item) in items.iter().enumerate().filter(|(k, _)| on_page(*k)) {
                kept.push(item.id);
                let Some(cell) = cells.get(k - page * PER_PAGE) else {
                    continue;
                };
                match seen.iter().find(|(id, _, _)| *id == item.id) {
                    Some((_, from, picture)) => {
                        let to = gtk::graphene::Rect::new(
                            cell.x() + inset.0,
                            cell.y() + inset.1,
                            from.width(),
                            from.height(),
                        );
                        if (to.x() - from.x()).abs() < 0.5 && (to.y() - from.y()).abs() < 0.5 {
                            continue;
                        }
                        tracing::debug!(target: "dnd", "tile {k} slides from {:?} to {:?}", (from.x(), from.y(), from.width(), from.height()), (to.x(), to.y(), to.width(), to.height()));
                        let ghost = self.ghost(picture, from);
                        item.column.set_opacity(0.0);
                        moves.push((ghost, item.column.clone(), *from, to));
                    }
                    // Pushed in from another page: fades in where it is.
                    None => {
                        item.column.set_opacity(0.0);
                        appear.push(item.column.clone());
                    }
                }
            }
        }
        // Pushed off this page: fades out where it was.
        let leave: Vec<gtk::Picture> = seen
            .iter()
            .filter(|(id, _, _)| !kept.contains(id))
            .map(|(_, from, picture)| self.ghost(picture, from))
            .collect();
        tracing::debug!(target: "dnd", "slide {} tiles, {} in, {} out", moves.len(), appear.len(), leave.len());
        if moves.is_empty() && appear.is_empty() && leave.is_empty() {
            return;
        }
        let parts = Rc::new((moves, appear, leave));
        let target = {
            let (parts, ghosts) = (parts.clone(), self.ghosts.clone());
            adw::CallbackAnimationTarget::new(move |v| {
                let (moves, appear, leave) = &*parts;
                for (ghost, _, from, to) in moves {
                    let x = from.x() as f64 + (to.x() - from.x()) as f64 * v;
                    let y = from.y() as f64 + (to.y() - from.y()) as f64 * v;
                    ghosts.move_(ghost, x, y);
                }
                for column in appear {
                    column.set_opacity(v);
                }
                for ghost in leave {
                    ghost.set_opacity(1.0 - v);
                }
            })
        };
        let anim = adw::TimedAnimation::builder()
            .widget(&self.ghosts)
            .value_from(0.0)
            .value_to(1.0)
            .duration(REORDER_MS)
            .easing(adw::Easing::EaseOutCubic)
            .target(&target)
            .build();
        {
            let ghosts = self.ghosts.clone();
            anim.connect_done(move |_| {
                let (moves, appear, leave) = &*parts;
                for (ghost, column, _, _) in moves {
                    ghosts.remove(ghost);
                    column.set_opacity(1.0);
                }
                for column in appear {
                    column.set_opacity(1.0);
                }
                for ghost in leave {
                    ghosts.remove(ghost);
                }
            });
        }
        anim.play();
        *self.reorder_anim.borrow_mut() = Some(anim);
    }

    /// A still picture of a tile at `at` in the page area, over the tiles.
    fn ghost(&self, picture: &gdk::Paintable, at: &gtk::graphene::Rect) -> gtk::Picture {
        let ghost = gtk::Picture::builder()
            .paintable(picture)
            .can_shrink(false)
            .can_target(false)
            .width_request(at.width().round() as i32)
            .height_request(at.height().round() as i32)
            .build();
        self.ghosts.put(&ghost, at.x() as f64, at.y() as f64);
        ghost
    }

    /// Right-click a library item: Copy, Paste (greyed out with nothing copied) and Delete, for
    /// every picked item when it is one of them.
    fn tile_menu(&self, button: &gtk::Button, id: Uuid, kind: Tab) {
        let actions = gio::SimpleActionGroup::new();
        let add = |name: &str, f: TileAction| {
            let a = gio::SimpleAction::new(name, None);
            let (state, button) = (self.state.clone(), button.downgrade());
            a.connect_activate(move |_, _| {
                if let Some(b) = button.upgrade() {
                    f(&state, &b);
                }
            });
            actions.add_action(&a);
        };
        add(
            "copy",
            Box::new(|s, _| {
                copy_picked(s);
            }),
        );
        add(
            "paste",
            Box::new(|s, _| {
                paste_picked(s);
            }),
        );
        add(
            "delete",
            Box::new(|s, b| {
                delete_picked(s, b);
            }),
        );
        button.insert_action_group("tile", Some(&actions));
        let pop = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        pop.set_has_arrow(true);
        pop.set_parent(button);
        // Let go of the menu with the tile (tiles are made again on every reorder).
        {
            let pop = pop.clone();
            button.connect_destroy(move |_| pop.unparent());
        }
        let right = gtk::GestureClick::builder().button(3).build();
        let state = self.state.clone();
        right.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            // Keep a multi-pick that holds this item.
            if !state.library_selection().contains(&id) {
                state.select_library(id);
            }
            let can_paste = match kind {
                Tab::Dials => state.editor.borrow().has_dial_clip(),
                Tab::Keys => state.editor.borrow().has_keys_clip(),
            };
            let count = picked(&state).map_or(1, |(_, ids)| ids.len());
            let what = if kind == Tab::Keys { "Key" } else { "Dial" };
            pop.set_menu_model(Some(&tile_menu(can_paste, count, what)));
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        });
        button.add_controller(right);
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

    /// The last tile on the Keys tab: a blank library key, opened in the key editor.
    fn add_key_item(&self) -> gtk::Button {
        let plus = gtk::Image::builder()
            .icon_name("list-add-symbolic")
            .pixel_size(20)
            .build();
        let disc = self.disc(&plus);
        disc.add_css_class("new");
        disc.add_css_class("tray-key");
        let (button, _, _) = self.item(&canvas::with_room(&disc, 6, 4, 2), "New Key");
        let me = self.me.borrow().clone();
        button.connect_clicked(move |_| {
            let Some(t) = me.upgrade() else { return };
            let mut id = None;
            t.state.edit(|e| id = Some(e.new_library_key()));
            let Some(id) = id else { return };
            let items = t.state.editor.borrow().profile().key_library.len();
            t.go(items.saturating_sub(1) / PER_PAGE);
            t.state.select_library(id);
        });
        button
    }

    /// Ctrl+V with nothing picked on the deck or here, after copying library tiles: they go in
    /// at the end of their library, which opens on them, picked. False when what was copied did
    /// not come from the library.
    pub fn paste_new(&self) -> bool {
        let (keys, from_library) = {
            let e = self.state.editor.borrow();
            (e.has_keys_clip(), e.clip_from_library())
        };
        let tab = if keys { Tab::Keys } else { Tab::Dials };
        if !from_library || (tab == Tab::Dials && !self.with_dials()) {
            return false;
        }
        let mut new = Vec::new();
        self.state.edit(|e| {
            new = match tab {
                Tab::Keys => e.paste_into_key_library(None),
                Tab::Dials => e.paste_into_dial_library(None),
            }
        });
        let Some(&first) = new.first() else {
            return true;
        };
        // The new items show, whatever was searched for.
        if !self.query.borrow().is_empty() {
            self.query.borrow_mut().clear();
            self.search.set_text("");
            self.drawn.borrow_mut().clear();
            self.items.borrow_mut().clear();
        }
        self.open(tab);
        self.refresh();
        let items = {
            let e = self.state.editor.borrow();
            match tab {
                Tab::Keys => e.profile().key_library.len(),
                Tab::Dials => e.profile().dial_library.len(),
            }
        };
        self.go((items - new.len()) / PER_PAGE);
        self.state.select_library(first);
        for id in &new[1..] {
            self.state.toggle_library(*id);
        }
        true
    }

    /// Ring the picked library items.
    fn show_picked(&self) {
        let picked = self.state.library_selection();
        for item in self.items.borrow().iter() {
            canvas::set_class(&item.disc, "selected", picked.contains(&item.id));
        }
    }
}

/// Which tab library item `id` belongs to.
fn kind_of(state: &State, id: Uuid) -> Option<Tab> {
    let e = state.editor.borrow();
    let p = e.profile();
    if p.library_key(id).is_some() {
        Some(Tab::Keys)
    } else if p.library_dial(id).is_some() {
        Some(Tab::Dials)
    } else {
        None
    }
}

/// The picked library items and their tab; `None` when no library item is picked.
fn picked(state: &State) -> Option<(Tab, Vec<Uuid>)> {
    let ids = state.library_selection();
    let kind = kind_of(state, *ids.last()?)?;
    let ids = ids
        .into_iter()
        .filter(|id| kind_of(state, *id) == Some(kind))
        .collect();
    Some((kind, ids))
}

/// Ctrl+C with library items picked: copy them. False when none is picked.
pub fn copy_picked(state: &State) -> bool {
    let Some((kind, ids)) = picked(state) else {
        return false;
    };
    let mut e = state.editor.borrow_mut();
    match kind {
        Tab::Keys => e.copy_library_keys(&ids),
        Tab::Dials => e.copy_library_dials(&ids),
    }
    true
}

/// Ctrl+V with library items picked: what was copied goes into that library as new items after
/// the one shown, and those are picked. False when no library item is picked.
pub fn paste_picked(state: &State) -> bool {
    let Some((kind, _)) = picked(state) else {
        return false;
    };
    let after = *state.selected_library.borrow();
    let can = match kind {
        Tab::Keys => state.editor.borrow().has_keys_clip(),
        Tab::Dials => state.editor.borrow().has_dial_clip(),
    };
    if !can {
        let what = if kind == Tab::Keys { "keys" } else { "dials" };
        crate::ui::toast(
            state,
            &format!("Copy {what} first, from the deck or the library."),
        );
        return true;
    }
    let mut new = Vec::new();
    state.edit(|e| {
        new = match kind {
            Tab::Keys => e.paste_into_key_library(after),
            Tab::Dials => e.paste_into_dial_library(after),
        }
    });
    if let Some(first) = new.first() {
        state.select_library(*first);
        for id in &new[1..] {
            state.toggle_library(*id);
        }
    }
    true
}

/// Delete with library items picked: ask, then delete them. False when none is picked.
pub fn delete_picked(state: &State, parent: &impl IsA<gtk::Widget>) -> bool {
    let Some((kind, ids)) = picked(state) else {
        return false;
    };
    match kind {
        Tab::Keys => crate::ui::key_library::delete_library_keys(state, ids, parent),
        Tab::Dials => crate::ui::dial_inspector::delete_library_dials(state, ids, parent),
    }
    true
}

/// Ctrl+A with a library item picked: pick every item of its library. False when none is picked.
pub fn select_all(state: &State) -> bool {
    let Some((kind, _)) = picked(state) else {
        return false;
    };
    let ids: Vec<Uuid> = {
        let e = state.editor.borrow();
        match kind {
            Tab::Keys => e.profile().key_library.iter().map(|l| l.id).collect(),
            Tab::Dials => e.profile().dial_library.iter().map(|l| l.id).collect(),
        }
    };
    if let (Some(first), Some(last)) = (ids.first(), ids.last()) {
        state.select_library(*first);
        state.extend_library(*last, &ids);
    }
    true
}

/// A library item's right-click menu for `count` picked items of `what` ("Key" or "Dial");
/// Paste is greyed out with nothing to paste.
fn tile_menu(can_paste: bool, count: usize, what: &str) -> gio::Menu {
    // Shown with the window shortcuts that do the same.
    let item = |section: &gio::Menu, label: &str, action: &str, shortcut: &str| {
        let it = gio::MenuItem::new(Some(label), Some(action));
        if let Some(accel) = crate::ui::key_routing::accel_for(shortcut) {
            it.set_attribute_value("accel", Some(&accel.to_variant()));
        }
        section.append_item(&it);
    };
    let many = |verb: &str| match count {
        1 => verb.to_string(),
        n => format!("{verb} {n} {what}s"),
    };
    let m = gio::Menu::new();
    let clip = gio::Menu::new();
    item(&clip, &many("Copy"), "tile.copy", "copy");
    // An action nobody installs leaves the item insensitive.
    item(
        &clip,
        "Paste",
        if can_paste { "tile.paste" } else { "tile.no-paste" },
        "paste",
    );
    m.append_section(None, &clip);
    let gone = gio::Menu::new();
    let delete = match count {
        1 => "Delete from Library".to_string(),
        n => format!("Delete {n} {what}s from Library"),
    };
    item(&gone, &delete, "tile.delete", "clear");
    m.append_section(None, &gone);
    m
}

/// Show `lib` and where it is used on its tile; key pictures come from `dir`.
fn fill(item: &Item, lib: &Entry, usage: Usage, dir: &std::path::Path) {
    let name = match lib {
        Entry::Dial(d) => {
            if let Some(face) = &item.face {
                face.show(&d.dial);
            }
            d.dial.name.trim()
        }
        Entry::Key(k) => {
            if let Some(p) = &item.picture {
                p.set_paintable(canvas::key_texture(&k.key.shown_look(), dir).as_ref());
            }
            k.name.trim()
        }
    };
    item.name.set_label(match (name, lib) {
        ("", Entry::Dial(_)) => "Dial",
        ("", Entry::Key(_)) => "Key",
        (n, _) => n,
    });
    item.usage.set_label(&usage.text());
    canvas::set_class(&item.usage, "pinned", usage.pinned);
    item.pin.set_visible(usage.pinned);
}

/// How tall the tray stays while a tile is carried out, with `room` left under the deck: all of
/// it (`full`) if it fits, else as many whole rows of tiles as fit (rows end at `rows`, plus `pad`
/// under them), and never less than the top row (`strip`).
/// Where in the library (`order`) the carried item `from` goes for the gap before shown tile
/// `gap` (`shown` are the tiles on screen, fewer than `order` while searching); `None` when that
/// is where it already is.
fn insert_to(order: &[Uuid], shown: &[Uuid], from: Uuid, gap: usize) -> Option<usize> {
    let pos = |id: &Uuid| order.iter().position(|o| o == id);
    let f = pos(&from)?;
    // The library place of the gap: before the tile after it, or after the tile before it.
    let at = match shown.get(gap) {
        Some(next) => pos(next)?,
        None => pos(shown.get(gap.checked_sub(1)?)?)? + 1,
    };
    let to = if at > f { at - 1 } else { at };
    (to != f).then_some(to)
}

fn fit_tray(room: f64, strip: f64, full: f64, rows: &[f64], pad: f64) -> f64 {
    if room >= full {
        return full;
    }
    rows.iter()
        .map(|r| r + pad)
        .filter(|h| *h <= room && *h < full)
        .fold(strip, f64::max)
        .min(full)
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

    fn tray_with(model: ulanzi_core::model::Model) -> (crate::state::State, Rc<Tray>) {
        let state = crate::state::AppState::new(ulanzi_core::model::Profile::new_default("T", model));
        crate::ui::library_guide::hold_back();
        let canvas = canvas::build(&state);
        (state, canvas.tray.clone())
    }

    #[test]
    fn tabs_switch_back_and_forth_with_empty_libraries() {
        crate::ui::with_gtk(|| {
            let (state, t) = tray_with(ulanzi_core::model::Model::D200X);
            t.open(Tab::Keys);
            state.edit(|e| {
                e.new_library_dial();
            });
            t.tabs[1].set_active(true);
            assert_eq!(t.tab(), Tab::Dials);
            assert_eq!(t.items.borrow().len(), 1);
            // Back to Keys, whose library is empty: its own tiles show again.
            t.tabs[0].set_active(true);
            assert_eq!(t.tab(), Tab::Keys);
            assert!(t.items.borrow().is_empty(), "no dial tiles left on the Keys tab");
            t.tabs[1].set_active(true);
            assert_eq!(t.items.borrow().len(), 1, "the dial is back");
        });
    }

    #[test]
    fn a_carried_tile_leaves_the_tray_whole_rows_that_fit() {
        let rows = [150.0, 260.0, 370.0];
        // Room for all of it: it stays as it is.
        assert_eq!(fit_tray(500.0, 58.0, 420.0, &rows, 12.0), 420.0);
        // Room for two rows.
        assert_eq!(fit_tray(300.0, 58.0, 420.0, &rows, 12.0), 272.0);
        // Not even one row: just the strip.
        assert_eq!(fit_tray(100.0, 58.0, 420.0, &rows, 12.0), 58.0);
        // No room at all still keeps the strip.
        assert_eq!(fit_tray(-20.0, 58.0, 420.0, &rows, 12.0), 58.0);
    }

    #[test]
    fn carrying_a_tile_out_drops_to_the_peek_strip_and_back() {
        crate::ui::with_gtk(|| {
            let (state, t) = tray_with(ulanzi_core::model::Model::D200X);
            t.open(Tab::Keys);
            t.set_peek(true);
            assert!(t.peeked.get());
            assert!(t.peek_hint.is_visible(), "the strip says how to cancel");
            // Still open throughout, so the header button does not flicker.
            assert!(state.tray_open.get());
            t.set_peek(false);
            assert!(!t.peeked.get());
            assert!(!t.peek_hint.is_visible());
            assert!(state.tray_open.get());
        });
    }

    #[test]
    fn a_d200_has_no_dials_tab() {
        crate::ui::with_gtk(|| {
            let (_, t) = tray_with(ulanzi_core::model::Model::D200);
            t.open(Tab::Dials);
            assert_eq!(t.tab(), Tab::Keys);
            assert!(!t.switch.is_visible());
        });
    }

    #[test]
    fn tile_menu_greys_out_paste_without_a_copy() {
        let action = |m: &gio::Menu, section: i32, i: i32| {
            let s = m
                .item_link(section, "section")
                .expect("section")
                .downcast::<gio::Menu>()
                .expect("menu");
            s.item_attribute_value(i, "action", None)
                .and_then(|v| v.get::<String>())
                .expect("action")
        };
        assert_eq!(action(&tile_menu(true, 1, "Key"), 0, 1), "tile.paste");
        assert_eq!(action(&tile_menu(false, 1, "Key"), 0, 1), "tile.no-paste");
        assert_eq!(action(&tile_menu(false, 1, "Key"), 1, 0), "tile.delete");
        crate::ui::assert_menu_labels(&tile_menu(true, 3, "Key"));
        let labels = crate::ui::menu_labels(&tile_menu(true, 3, "Key"));
        assert!(labels.contains(&"Copy 3 Keys".into()), "{labels:?}");
        assert!(
            labels.contains(&"Delete 3 Keys from Library".into()),
            "{labels:?}"
        );
    }

    #[test]
    fn search_shows_past_one_page_and_finds_by_anything_in_a_key() {
        crate::ui::with_gtk(|| {
            let (state, t) = tray_with(ulanzi_core::model::Model::D200X);
            t.open(Tab::Keys);
            let mut ids = Vec::new();
            state.edit(|e| ids = (0..PER_PAGE).map(|_| e.new_library_key()).collect());
            state.edit(|e| {
                e.rename_library_key(ids[0], "Lamp");
                e.set_library_key(ids[1], |k| k.tap = Some(Action::NextPage));
                e.set_library_key(ids[2], |k| {
                    k.tap = Some(Action::PlaySound {
                        path: "/home/me/sounds/Scream.ogg".into(),
                        device: None,
                        volume: 100,
                    });
                    k.hold = Some(Action::Plugin {
                        plugin: "obs".into(),
                        action: "scene".into(),
                        params: toml::Table::new(),
                    });
                });
                e.set_library_key(ids[3], |k| {
                    k.look.source = ulanzi_core::model::ImageSource::Image {
                        path: "icons/cat.png".into(),
                    }
                });
            });
            t.refresh();
            assert!(!t.search.is_visible(), "one page");
            state.edit(|e| {
                e.new_library_key();
            });
            t.refresh();
            assert!(t.search.is_visible());
            t.search.set_text("lamp");
            *t.query.borrow_mut() = "lamp".into();
            t.refresh();
            assert_eq!(t.count(), 1);
            *t.query.borrow_mut() = "next page".into();
            t.drawn.borrow_mut().clear();
            t.refresh();
            assert_eq!(t.items.borrow()[0].id, ids[1]);
            // A sound file, a plugin, a picture.
            for (q, id) in [("scream", ids[2]), ("obs", ids[2]), ("cat.png", ids[3])] {
                *t.query.borrow_mut() = q.into();
                t.drawn.borrow_mut().clear();
                t.refresh();
                assert_eq!((t.count(), t.items.borrow()[0].id), (1, id), "{q}");
            }
            *t.query.borrow_mut() = "nothing like it".into();
            t.drawn.borrow_mut().clear();
            t.refresh();
            assert_eq!(t.count(), 0);
        });
    }

    #[test]
    fn paste_with_nothing_picked_adds_library_tiles_back_to_the_library() {
        crate::ui::with_gtk(|| {
            let (state, t) = tray_with(ulanzi_core::model::Model::D200X);
            let id = state.editor.borrow_mut().new_library_key();
            state.select_library(id);
            assert!(copy_picked(&state));
            state.select(|s| s.clear());
            assert!(t.paste_new());
            let lib: Vec<Uuid> = state
                .editor
                .borrow()
                .profile()
                .key_library
                .iter()
                .map(|l| l.id)
                .collect();
            assert_eq!(lib.len(), 2);
            assert!(t.is_open());
            assert_eq!(state.library_selection(), vec![lib[1]]);
            // Keys copied from the deck are not the library's to take.
            state.editor.borrow_mut().copy_keys(&[0]);
            assert!(!t.paste_new());
        });
    }

    #[test]
    fn library_items_pick_copy_paste_and_delete_together() {
        crate::ui::with_gtk(|| {
            let (state, _t) = tray_with(ulanzi_core::model::Model::D200X);
            let mut ids = Vec::new();
            state.edit(|e| ids = (0..4).map(|_| e.new_library_key()).collect());
            state.select_library(ids[0]);
            state.extend_library(ids[2], &ids);
            assert_eq!(state.library_selection(), ids[..3].to_vec());
            state.toggle_library(ids[1]);
            assert_eq!(state.library_selection(), vec![ids[0], ids[2]]);
            assert!(copy_picked(&state));
            assert!(paste_picked(&state));
            // Two new keys after the one shown, picked.
            let lib: Vec<Uuid> = state
                .editor
                .borrow()
                .profile()
                .key_library
                .iter()
                .map(|l| l.id)
                .collect();
            assert_eq!(lib.len(), 6);
            assert_eq!(state.library_selection(), lib[3..5].to_vec());
            // Nothing in use: deleted without asking, in one undo step.
            let w = gtk::Window::new();
            assert!(delete_picked(&state, &w));
            assert_eq!(state.editor.borrow().profile().key_library.len(), 4);
            assert!(state.library_selection().is_empty());
            state.undo();
            assert_eq!(state.editor.borrow().profile().key_library.len(), 6);
            assert!(!select_all(&state), "Ctrl+A needs a library pick");
            state.select_library(ids[3]);
            assert!(select_all(&state));
            assert_eq!(state.library_selection().len(), 6);
            // A deck pick is not a library pick.
            state.select(|s| s.click(ulanzi_core::edit::Slot::Key(0)));
            assert!(!copy_picked(&state));
        });
    }

    #[test]
    fn a_tile_dropped_between_two_goes_into_that_gap() {
        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        // Carried right: the tiles between shift back one.
        assert_eq!(insert_to(&ids, &ids, ids[0], 3), Some(2));
        // Carried left, and to the very end.
        assert_eq!(insert_to(&ids, &ids, ids[4], 1), Some(1));
        assert_eq!(insert_to(&ids, &ids, ids[1], 5), Some(4));
        // Either side of itself is where it is.
        assert_eq!(insert_to(&ids, &ids, ids[2], 2), None);
        assert_eq!(insert_to(&ids, &ids, ids[2], 3), None);
        // While searching, the gap is found in the whole library.
        let shown = [ids[0], ids[3]];
        assert_eq!(insert_to(&ids, &shown, ids[0], 2), Some(3));
    }

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
