//! The action list: a search field, a row of category buttons (All · built-in categories · one
//! per plugin) and the actions in collapsible sections, A–Z. The side panel shows it as the
//! library (drag an action onto the deck, or click it to assign it to the selection); the key
//! editor's action picker shows the same list in a popover. What it shows comes from
//! [`crate::action_list`].

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::catalog;
use ulanzi_core::model::Action;

use crate::action_list::{self, Catalog, Entry, Group, Memory, View};
use crate::state::{Change, State};
use crate::ui::action_editor::action_chip;
use crate::ui::canvas::action_payload;

/// What the list is for.
pub(crate) enum Use {
    /// The side panel: rows can be dragged onto the deck; a click assigns to the selection.
    Library,
    /// The action picker: a click hands a fresh action to `on_pick`.
    Picker {
        include_multi: bool,
        on_pick: Box<dyn Fn(Action)>,
    },
}

/// A built action list.
pub(crate) struct ActionList {
    pub root: gtk::Box,
    pub search: gtk::SearchEntry,
    pub scroller: gtk::ScrolledWindow,
    /// Read the plugins again and redraw; the query and category stay.
    pub reload: Rc<dyn Fn()>,
    inner: Weak<Inner>,
}

/// The side panel's library.
pub struct Library {
    pub root: gtk::Widget,
    pub search: gtk::SearchEntry,
}

struct Inner {
    state: State,
    purpose: Use,
    catalog: RefCell<Catalog>,
    view: RefCell<View>,
    search: gtk::SearchEntry,
    tabs: gtk::Box,
    list: gtk::Box,
    tab_scroller: gtk::ScrolledWindow,
    /// Set while the category buttons are rebuilt, so their toggles don't redraw the list.
    building_tabs: Cell<bool>,
    /// The query the shown list was made for.
    shown_query: RefCell<String>,
    /// The sections folded away on screen now.
    folded: RefCell<HashSet<String>>,
}

impl Inner {
    fn memory(&self) -> &RefCell<Memory> {
        match self.purpose {
            Use::Library => &self.state.library_memory,
            Use::Picker { .. } => &self.state.picker_memory,
        }
    }

    fn include_multi(&self) -> bool {
        match self.purpose {
            Use::Library => true,
            Use::Picker { include_multi, .. } => include_multi,
        }
    }

    fn fresh_catalog(&self) -> Catalog {
        Catalog::new(&self.state.plugins.borrow(), self.include_multi())
    }

    /// The remembered category, or All when it is gone.
    fn category(&self) -> String {
        let wanted = self.memory().borrow().category.clone();
        self.catalog.borrow().resolve(&wanted)
    }

    fn set_category(self: &Rc<Self>, id: &str) {
        self.memory().borrow_mut().category = id.to_string();
        self.render_tabs();
        self.render_list();
    }

    fn reload(self: &Rc<Self>) {
        *self.catalog.borrow_mut() = self.fresh_catalog();
        self.render_tabs();
        self.render_list();
    }

    fn render_tabs(self: &Rc<Self>) {
        self.building_tabs.set(true);
        while let Some(c) = self.tabs.first_child() {
            self.tabs.remove(&c);
        }
        let current = self.category();
        let mut first: Option<gtk::ToggleButton> = None;
        for tab in self.catalog.borrow().tabs() {
            let b = gtk::ToggleButton::builder()
                .label(&tab.label)
                .active(tab.id == current)
                .css_classes(["category-tab"])
                .build();
            match &first {
                Some(f) => b.set_group(Some(f)),
                None => first = Some(b.clone()),
            }
            let weak = Rc::downgrade(self);
            let id = tab.id.clone();
            b.connect_toggled(move |b| {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                if b.is_active() && !inner.building_tabs.get() {
                    inner.memory().borrow_mut().category = id.clone();
                    inner.render_list();
                }
            });
            self.tabs.append(&b);
        }
        self.building_tabs.set(false);
        self.scroll_to_chosen_tab_soon();
    }

    /// Bring the chosen category's button into view once it has its size.
    fn scroll_to_chosen_tab_soon(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(inner) = weak.upgrade() {
                inner.scroll_to_chosen_tab();
            }
        });
    }

    fn scroll_to_chosen_tab(&self) {
        let mut c = self.tabs.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            let Some(b) = w.downcast_ref::<gtk::ToggleButton>() else {
                continue;
            };
            if !b.is_active() {
                continue;
            }
            let Some(p) = b.compute_point(&self.tabs, &gtk::graphene::Point::new(0.0, 0.0)) else {
                return;
            };
            let adj = self.tab_scroller.hadjustment();
            let v = scroll_into_view(
                adj.value(),
                adj.page_size(),
                f64::from(p.x()),
                f64::from(b.width()),
            );
            adj.set_value(v);
            return;
        }
    }

    fn render_list(self: &Rc<Self>) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let query = self.search.text().to_string();
        let category = self.category();
        let view = self.catalog.borrow().view(&category, &query);
        if matches!(self.purpose, Use::Library) {
            self.list.append(
                &gtk::Label::builder()
                    .label("Drag an action onto a key, dial or the screen. Click one to give it to the selection.")
                    .wrap(true)
                    .xalign(0.0)
                    .css_classes(["dim-label", "caption"])
                    .build(),
            );
        }
        if view.is_empty() {
            let label = self
                .catalog
                .borrow()
                .tabs()
                .into_iter()
                .find(|t| t.id == category)
                .map(|t| t.label)
                .unwrap_or_default();
            let (title, detail) = action_list::empty_text(&query, &category, &label);
            self.list.append(
                &adw::StatusPage::builder()
                    .icon_name("edit-find-symbolic")
                    .title(title)
                    .description(detail.unwrap_or_default())
                    .css_classes(["compact"])
                    .build(),
            );
        }
        let collapsed = self.memory().borrow().collapsed.clone();
        let mut folded = HashSet::new();
        for g in &view.groups {
            let rows = self.group_list(g);
            if !view.headers {
                self.list.append(&rows);
                continue;
            }
            let open = action_list::expanded(&collapsed, &g.id, &query);
            if !open {
                folded.insert(g.id.clone());
            }
            let revealer = gtk::Revealer::builder()
                .reveal_child(open)
                .transition_type(gtk::RevealerTransitionType::SlideDown)
                .child(&rows)
                .build();
            self.list.append(&self.group_header(g, &revealer));
            self.list.append(&revealer);
        }
        if let Some(text) = action_list::elsewhere_label(view.elsewhere) {
            let b = gtk::Button::builder()
                .label(text)
                .halign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let weak = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    inner.set_category(action_list::ALL);
                }
            });
            self.list.append(&b);
        }
        *self.view.borrow_mut() = view;
        *self.folded.borrow_mut() = folded;
        *self.shown_query.borrow_mut() = query;
    }

    /// A section's header: the plugin's icon, the name, the count and an arrow; a click opens or
    /// closes the section (remembered unless searching).
    fn group_header(self: &Rc<Self>, g: &Group, revealer: &gtk::Revealer) -> gtk::Button {
        let b = gtk::Box::builder().spacing(8).build();
        if g.plugin {
            b.append(&plugin_icon(g.icon.as_deref(), 16));
        }
        b.append(
            &gtk::Label::builder()
                .label(&g.title)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["heading"])
                .build(),
        );
        b.append(
            &gtk::Label::builder()
                .label(g.entries.len().to_string())
                .css_classes(["dim-label", "caption"])
                .build(),
        );
        let arrow = gtk::Image::from_icon_name(arrow_icon(revealer.reveals_child()));
        b.append(&arrow);
        let button = gtk::Button::builder()
            .child(&b)
            .css_classes(["flat", "group-header"])
            .build();
        let (weak, id, revealer) = (Rc::downgrade(self), g.id.clone(), revealer.downgrade());
        button.connect_clicked(move |_| {
            let (Some(inner), Some(revealer)) = (weak.upgrade(), revealer.upgrade()) else {
                return;
            };
            let open = !revealer.reveals_child();
            revealer.set_reveal_child(open);
            arrow.set_icon_name(Some(arrow_icon(open)));
            if open {
                inner.folded.borrow_mut().remove(&id);
            } else {
                inner.folded.borrow_mut().insert(id.clone());
            }
            if inner.search.text().trim().is_empty() {
                let mut m = inner.memory().borrow_mut();
                if open {
                    m.collapsed.remove(&id);
                } else {
                    m.collapsed.insert(id.clone());
                }
            }
        });
        button
    }

    fn group_list(self: &Rc<Self>, g: &Group) -> gtk::ListBox {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list", "action-rows"])
            .build();
        for e in &g.entries {
            list.append(&self.entry_row(e));
        }
        let actions: Vec<Action> = g.entries.iter().map(|e| e.action.clone()).collect();
        let weak = Rc::downgrade(self);
        list.connect_row_activated(move |_, row| {
            let (Some(inner), Ok(i)) = (weak.upgrade(), usize::try_from(row.index())) else {
                return;
            };
            if let Some(a) = actions.get(i) {
                inner.pick(a.clone());
            }
        });
        list
    }

    fn entry_row(self: &Rc<Self>, e: &Entry) -> gtk::ListBoxRow {
        let b = gtk::Box::builder().spacing(10).build();
        if e.plugin.is_some() {
            b.append(&plugin_icon(e.icon.as_deref(), 24));
        } else {
            b.append(&action_chip(&self.state, Some(&e.action), 24));
        }
        b.append(
            &gtk::Label::builder()
                .label(&e.title)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        let what = match &e.plugin {
            Some(p) => format!("{} ({p})", e.title),
            None => e.title.clone(),
        };
        let tooltip = match self.purpose {
            Use::Library => format!("{what}: drag onto the deck, or click to assign"),
            Use::Picker { .. } => what,
        };
        let row = gtk::ListBoxRow::builder().child(&b).tooltip_text(tooltip).build();
        if matches!(self.purpose, Use::Library) {
            row.add_controller(drag_source(&self.state, e.action.clone()));
        }
        row
    }

    fn pick(&self, action: Action) {
        match &self.purpose {
            Use::Library => assign(&self.state, action),
            Use::Picker { on_pick, .. } => on_pick(action),
        }
    }
}

/// The scroll position that shows `[x, x + width)` in a view of `page` starting at `value`,
/// moving as little as possible.
fn scroll_into_view(value: f64, page: f64, x: f64, width: f64) -> f64 {
    if x < value {
        x
    } else if x + width > value + page {
        (x + width - page).min(x)
    } else {
        value
    }
}

fn arrow_icon(open: bool) -> &'static str {
    if open {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    }
}

/// The plugin's icon when the icon theme has it, else the add-on icon.
fn plugin_icon(icon: Option<&str>, size: i32) -> gtk::Image {
    let name = icon
        .and_then(crate::plugins::icon_lookup)
        .filter(|n| gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(n)))
        .unwrap_or("application-x-addon-symbolic");
    let i = gtk::Image::from_icon_name(name);
    i.set_pixel_size(size);
    i.set_size_request(size, size);
    i.set_valign(gtk::Align::Center);
    i
}

/// Dragging a row carries the action, as the canvas expects it.
fn drag_source(state: &State, action: Action) -> gtk::DragSource {
    let drag = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
    {
        let payload = action_payload(&action);
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&payload.to_value())));
    }
    {
        let state = state.clone();
        drag.connect_drag_begin(move |src, _| {
            state.dragging_action.set(true);
            state.notify(Change::Drag);
            let look = catalog::default_look(&action);
            if let Some(t) = crate::ui::canvas::key_texture(&look, &crate::ui::canvas::profile_dir(&state)) {
                src.set_icon(Some(&t), 24, 24);
            }
        });
    }
    {
        let state = state.clone();
        drag.connect_drag_end(move |_, _, _| {
            state.dragging_action.set(false);
            state.notify(Change::Drag);
        });
    }
    drag
}

/// Give the action to every selected control (one undo step).
fn assign(state: &State, action: Action) {
    let slots = state.selection.borrow().slots();
    if slots.is_empty() {
        crate::ui::toast(state, "Select a key first, or drag the action onto the deck");
        return;
    }
    state.edit(|e| {
        e.set_merge(Some("assign"));
        for s in &slots {
            e.drop_action(*s, action.clone());
        }
        e.set_merge(None);
    });
}

/// Build an action list for `purpose`.
pub(crate) fn action_list(state: &State, purpose: Use) -> ActionList {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search actions")
        .build();
    let tabs = gtk::Box::builder().spacing(4).build();
    let tab_scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&tabs)
        .css_classes(["category-tabs"])
        .build();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    list.set_margin_bottom(12);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    root.append(&search);
    root.append(&tab_scroller);
    root.append(&scroller);

    let inner = Rc::new(Inner {
        state: state.clone(),
        purpose,
        catalog: RefCell::new(Catalog::default()),
        view: RefCell::new(View::default()),
        search: search.clone(),
        tabs,
        list,
        tab_scroller: tab_scroller.clone(),
        building_tabs: Cell::new(false),
        shown_query: RefCell::default(),
        folded: RefCell::default(),
    });
    // The picker fills itself each time it opens (see `reload`); the key editor builds several
    // pickers on every rebuild, and most are never opened.
    if matches!(inner.purpose, Use::Library) {
        inner.reload();
    }
    {
        let weak = Rc::downgrade(&inner);
        search.connect_search_changed(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.render_list();
            }
        });
    }
    {
        // Enter picks the first action shown in an open section, and only for a query.
        let weak = Rc::downgrade(&inner);
        search.connect_activate(move |s| {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            // Typing is redrawn after a short delay: Enter right after typing sees the new list.
            if *inner.shown_query.borrow() != s.text().as_str() {
                inner.render_list();
            }
            let query = s.text();
            let first = inner
                .view
                .borrow()
                .enter_pick(&query, &inner.folded.borrow())
                .map(|e| e.action.clone());
            if let Some(a) = first {
                inner.pick(a);
            }
        });
    }
    let reload: Rc<dyn Fn()> = {
        let weak = Rc::downgrade(&inner);
        Rc::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.reload();
            }
        })
    };
    {
        // Opening the panel (or the picker): show the chosen category's button.
        let weak = Rc::downgrade(&inner);
        root.connect_map(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.scroll_to_chosen_tab_soon();
            }
        });
    }
    let weak = Rc::downgrade(&inner);
    // The list lives as long as its root: the root's handler is what holds it (the list's own
    // handlers hold it weakly).
    keep_alive(&root, inner);
    ActionList {
        root,
        search,
        scroller,
        reload,
        inner: weak,
    }
}

/// Keep `inner` until `root` is destroyed.
fn keep_alive(root: &gtk::Box, inner: Rc<Inner>) {
    let held = RefCell::new(Some(inner));
    root.connect_destroy(move |_| {
        held.borrow_mut().take();
    });
}

/// Whether the plugins' actions differ from what `inner` shows.
fn plugins_changed(inner: &Weak<Inner>) -> Option<bool> {
    let inner = inner.upgrade()?;
    let now = inner.fresh_catalog();
    Some(*inner.catalog.borrow() != now)
}

/// The library panel.
pub fn build(state: &State) -> Library {
    let list = action_list(state, Use::Library);
    list.root.set_margin_start(12);
    list.root.set_margin_end(12);
    list.root.set_margin_top(6);
    {
        // New, removed or broken plugins change the categories and the actions; a plugin that
        // only retries its connection does not, and the list (and the typing) stays as it is.
        let root = list.root.downgrade();
        let reload = list.reload.clone();
        let inner = list.inner.clone();
        state.on_while(move |c| {
            if root.upgrade().is_none() {
                return false;
            }
            if c == Change::Plugins && plugins_changed(&inner) == Some(true) {
                reload();
            }
            true
        });
    }
    Library {
        root: list.root.upcast(),
        search: list.search,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use ulanzi_core::catalog::Category;
    use ulanzi_core::model::{Model, Profile};

    fn state() -> State {
        AppState::new(Profile::new_default("Default", Model::D200X))
    }

    fn tab_buttons(inner: &Inner) -> Vec<gtk::ToggleButton> {
        let mut out = Vec::new();
        let mut c = inner.tabs.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            if let Ok(t) = w.downcast::<gtk::ToggleButton>() {
                out.push(t);
            }
        }
        out
    }

    #[test]
    fn picker_list_searches_switches_categories_and_picks() {
        crate::ui::with_gtk(|| {
            let state = state();
            let picked: Rc<RefCell<Vec<Action>>> = Rc::default();
            let p = picked.clone();
            let list = action_list(
                &state,
                Use::Picker {
                    include_multi: false,
                    on_pick: Box::new(move |a| p.borrow_mut().push(a)),
                },
            );
            let inner = list.inner.upgrade().expect("alive");
            assert!(
                tab_buttons(&inner).is_empty(),
                "a picker fills itself when it opens"
            );
            (list.reload)();
            let tabs = tab_buttons(&inner);
            assert_eq!(tabs.len(), 1 + Category::ALL.len());
            assert!(tabs[0].is_active(), "All first");
            assert_eq!(tabs[0].label().as_deref(), Some("All"));

            // Enter without a query picks nothing.
            list.search.emit_activate();
            assert!(picked.borrow().is_empty());
            // Enter in the search field picks the first match.
            list.search.set_text("volume up");
            inner.render_list();
            list.search.emit_activate();
            assert_eq!(picked.borrow().len(), 1);
            assert_eq!(
                picked.borrow()[0],
                (catalog::catalog()
                    .iter()
                    .find(|s| s.id == "volume_up")
                    .expect("spec")
                    .make)()
            );

            // A category button narrows the list and is remembered.
            tabs[1].set_active(true);
            assert_eq!(state.picker_memory.borrow().category, "cat:navigation");
            assert!(!tabs[0].is_active());
            list.search.set_text("");
            inner.render_list();
            assert_eq!(
                inner.view.borrow().len(),
                catalog::catalog()
                    .iter()
                    .filter(|s| s.category == Category::Navigation)
                    .count()
            );
            // A rebuilt list starts on the remembered category; the library keeps its own.
            drop(inner);
            drop(list);
            let again = action_list(
                &state,
                Use::Picker {
                    include_multi: false,
                    on_pick: Box::new(|_| {}),
                },
            );
            let inner = again.inner.upgrade().expect("alive");
            (again.reload)();
            assert!(tab_buttons(&inner)[1].is_active());
            assert_eq!(state.library_memory.borrow().category, action_list::ALL);
        });
    }

    #[test]
    fn the_chosen_tab_scrolls_into_view() {
        // Already visible: stays.
        assert_eq!(scroll_into_view(0.0, 300.0, 100.0, 80.0), 0.0);
        // Off to the right: its right edge lines up with the view's.
        assert_eq!(scroll_into_view(0.0, 300.0, 500.0, 80.0), 280.0);
        // Off to the left: its left edge lines up.
        assert_eq!(scroll_into_view(400.0, 300.0, 120.0, 80.0), 120.0);
        // Wider than the view: its start shows.
        assert_eq!(scroll_into_view(0.0, 50.0, 200.0, 80.0), 200.0);
    }

    #[test]
    fn the_list_goes_away_with_its_widget() {
        crate::ui::with_gtk(|| {
            let state = state();
            let list = action_list(&state, Use::Library);
            let weak = list.inner.clone();
            assert!(weak.upgrade().is_some());
            drop(list);
            assert!(weak.upgrade().is_none(), "the list leaked");

            let lib = build(&state);
            let before = state.listener_count();
            drop(lib);
            state.notify(Change::Plugins);
            assert_eq!(state.listener_count(), before - 1, "the plugin listener stays");
        });
    }

    #[test]
    fn library_rows_can_be_dragged() {
        crate::ui::with_gtk(|| {
            let state = state();
            let list = action_list(&state, Use::Library);
            let inner = list.inner.upgrade().expect("alive");
            let row = find_row(&inner.list).expect("a row");
            let drags = row
                .observe_controllers()
                .into_iter()
                .filter_map(|o| o.ok().and_downcast::<gtk::DragSource>())
                .count();
            assert_eq!(drags, 1);
        });
    }

    fn find_row(w: &impl IsA<gtk::Widget>) -> Option<gtk::ListBoxRow> {
        let mut c = w.as_ref().first_child();
        while let Some(child) = c {
            if let Ok(r) = child.clone().downcast::<gtk::ListBoxRow>() {
                return Some(r);
            }
            if let Some(r) = find_row(&child) {
                return Some(r);
            }
            c = child.next_sibling();
        }
        None
    }
}
