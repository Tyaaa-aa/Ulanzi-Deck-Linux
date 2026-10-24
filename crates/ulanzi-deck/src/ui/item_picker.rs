//! The dialog that picks a plugin item (a scene, an entity…) from the plugin's own list
//! (`PluginListItems`). The words and decisions come from [`crate::plugins`]; this is the GTK
//! around them.
//!
//! Every answer is tagged with a generation: only the newest search's answer is shown. Typing an
//! id by hand is the fallback, offered when the plugin can't list (an error, an empty list, or a
//! plugin built for contract 1.0, which has no list at all).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::plugin::{Field, PickItem, Picker, PickerStyle};

use crate::client::ClientError;
use crate::plugins::{
    SHOW_MAX, can_list, can_tick, capped, current_first, empty_message, group_items, group_names,
    icon_lookup, limit_note, manual_ids, max_picks, offer_manual, shown_items, toggle_id,
};
use crate::runtime;
use crate::state::State;

fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// The open dialog. Held by its widgets' handlers (and an answer on its way); it holds its
/// widgets weakly, so closing the dialog frees everything.
struct Picking {
    state: State,
    plugin: String,
    picker: Picker,
    /// The plugin can list (contract 1.1 or later); else only manual entry.
    lists: bool,
    /// How many entries a multi picker may tick (the field's `max`).
    limit: Option<usize>,
    /// What was stored when the dialog opened; kept visible even when the list lacks it.
    current: Vec<String>,
    /// The ticked entries (multi), in the order they were picked.
    selected: RefCell<Vec<String>>,
    /// Every entry seen, by id, so the row can show labels after a pick.
    known: RefCell<HashMap<String, PickItem>>,
    /// The tick widgets on screen (multi), made insensitive once the limit is reached.
    ticks: RefCell<Vec<(String, glib::WeakRef<gtk::Widget>)>>,
    /// Bumped by every ask; an answer for an older one is dropped.
    generation: Cell<u32>,
    /// The groups of the first unfiltered answer (the drop-down's entries after "All").
    groups: RefCell<Option<Vec<String>>>,
    /// Set while the drop-down's entries are replaced, so that is not taken as the user's choice.
    filling: Cell<bool>,
    dialog: glib::WeakRef<adw::Dialog>,
    search: glib::WeakRef<gtk::SearchEntry>,
    bar: glib::WeakRef<gtk::Box>,
    by_hand: glib::WeakRef<gtk::Button>,
    group: glib::WeakRef<gtk::DropDown>,
    stack: glib::WeakRef<gtk::Stack>,
    content: glib::WeakRef<gtk::Box>,
    status: glib::WeakRef<adw::StatusPage>,
    retry: glib::WeakRef<gtk::Button>,
    manual: glib::WeakRef<adw::EntryRow>,
    on_done: Box<dyn Fn(Vec<PickItem>)>,
}

impl Picking {
    /// The group picked in the drop-down; `""` for all.
    fn group_filter(&self) -> String {
        let Some(d) = self.group.upgrade() else {
            return String::new();
        };
        let i = d.selected() as usize;
        match (i, self.groups.borrow().as_ref()) {
            (0, _) | (_, None) => String::new(),
            (i, Some(g)) => g.get(i - 1).cloned().unwrap_or_default(),
        }
    }

    fn query(&self) -> String {
        self.search
            .upgrade()
            .map(|s| s.text().to_string())
            .unwrap_or_default()
    }

    /// Show a page. The search bar hides on the manual page, so a stray keystroke can't replace
    /// a half-typed entry with a new list.
    fn show(&self, page: &str) {
        if let Some(s) = self.stack.upgrade() {
            s.set_visible_child_name(page);
        }
        if let Some(b) = self.bar.upgrade() {
            b.set_visible(self.lists && page != "manual");
        }
    }

    fn page(&self) -> Option<glib::GString> {
        self.stack.upgrade().and_then(|s| s.visible_child_name())
    }

    /// Done, Save or Enter: the typed ids while the manual page shows, else the ticked entries.
    fn commit(&self) {
        let ids = if self.page().as_deref() == Some("manual") {
            let text = self
                .manual
                .upgrade()
                .map(|m| m.text().to_string())
                .unwrap_or_default();
            manual_ids(&text, self.picker.multi)
        } else {
            self.selected.borrow().clone()
        };
        self.finish(&capped(ids, self.limit));
    }

    /// Tick widgets of unticked entries go insensitive once the limit is reached.
    fn update_ticks(&self) {
        let selected = self.selected.borrow();
        for (id, w) in self.ticks.borrow().iter() {
            if let Some(w) = w.upgrade() {
                w.set_sensitive(can_tick(&selected, id, self.limit));
            }
        }
    }

    fn toggled(&self, id: &str, on: bool) {
        toggle_id(&mut self.selected.borrow_mut(), id, on);
        self.update_ticks();
    }

    /// The entries for `ids`, labelled when the plugin listed them, else by the id.
    fn picks(&self, ids: &[String]) -> Vec<PickItem> {
        let known = self.known.borrow();
        ids.iter()
            .map(|id| {
                known.get(id).cloned().unwrap_or_else(|| PickItem {
                    id: id.clone(),
                    label: id.clone(),
                    subtitle: None,
                    group: None,
                    icon: None,
                    state: None,
                })
            })
            .collect()
    }

    /// Close the dialog and hand `ids` to the caller.
    fn finish(&self, ids: &[String]) {
        let picks = self.picks(ids);
        if let Some(d) = self.dialog.upgrade() {
            d.close();
        }
        (self.on_done)(picks);
    }
}

/// Open the picker for field `f` (picker `picker`) of `plugin` over `parent`. `current` is what is
/// stored now; `on_done` gets the picked entries (an id the list lacks comes back labelled with
/// itself) on a pick, Done or a manual entry. A multi picker ticks at most the field's `max`
/// entries. Closing the dialog changes nothing.
pub fn open(
    parent: &impl IsA<gtk::Widget>,
    state: &State,
    plugin: &str,
    f: &Field,
    picker: &Picker,
    current: Vec<String>,
    on_done: impl Fn(Vec<PickItem>) + 'static,
) {
    let lists = state
        .plugins
        .borrow()
        .manifests
        .get(plugin)
        .is_none_or(|m| can_list(&m.contract));
    let multi = picker.multi;
    let title = f.title();
    let limit = max_picks(f, multi);

    let header = adw::HeaderBar::new();
    let done = gtk::Button::builder()
        .label("Done")
        .css_classes(["suggested-action"])
        .build();
    let cancel = gtk::Button::with_label("Cancel");
    let clear = gtk::Button::with_label("Clear");
    if multi {
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);
        header.pack_start(&cancel);
        header.pack_end(&done);
    } else if !current.is_empty() {
        header.pack_start(&clear);
    }

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search")
        .search_delay(150)
        .hexpand(true)
        .build();
    let group = gtk::DropDown::from_strings(&["All"]);
    group.set_visible(false);
    group.set_tooltip_text(Some("Group"));
    let bar = gtk::Box::builder()
        .spacing(6)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(6)
        .visible(lists)
        .build();
    bar.append(&search);
    bar.append(&group);

    let spinner = adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(6)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    let retry = gtk::Button::builder()
        .label("Retry")
        .css_classes(["pill", "suggested-action"])
        .halign(gtk::Align::Center)
        .build();
    let by_hand = gtk::Button::builder()
        .label("Enter manually…")
        .css_classes(["pill"])
        .halign(gtk::Align::Center)
        .build();
    let buttons = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    buttons.append(&retry);
    buttons.append(&by_hand);
    let status = adw::StatusPage::builder()
        .icon_name("dialog-information-symbolic")
        .child(&buttons)
        .build();
    status.add_css_class("compact");

    let manual_text = if multi {
        current.join(", ")
    } else {
        current.first().cloned().unwrap_or_default()
    };
    let manual = adw::EntryRow::builder()
        .title(escape(title))
        .text(manual_text)
        .build();
    let manual_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    manual_list.append(&manual);
    let mut hint_text = match (lists, multi) {
        (false, false) => "This plugin can’t list its choices. Type the one you want.",
        (false, true) => "This plugin can’t list its choices. Type them, separated by commas.",
        (true, false) => "Type it exactly as the plugin knows it.",
        (true, true) => "Type them exactly as the plugin knows them, separated by commas.",
    }
    .to_string();
    if let Some(n) = limit_note(limit) {
        hint_text = format!("{hint_text} {n}");
    }
    let hint = gtk::Label::builder()
        .label(hint_text)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    // A multi picker commits with the header's Done (which takes the typed ids on this page);
    // a single one has no Done, so the page has its own Save.
    let save = gtk::Button::builder()
        .label("Save")
        .css_classes(["pill", "suggested-action"])
        .halign(gtk::Align::Center)
        .visible(!multi)
        .build();
    let back = gtk::Button::builder()
        .label("Show the list")
        .css_classes(["pill"])
        .halign(gtk::Align::Center)
        .visible(lists)
        .build();
    let manual_page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    manual_page.append(&manual_list);
    manual_page.append(&hint);
    manual_page.append(&save);
    manual_page.append(&back);

    let stack = gtk::Stack::new();
    stack.add_named(&spinner, Some("loading"));
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&status, Some("status"));
    stack.add_named(&manual_page, Some("manual"));
    stack.set_visible_child_name(if lists { "loading" } else { "manual" });

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&bar);
    view.set_content(Some(&stack));
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(420)
        .content_height(560)
        .child(&view)
        .build();
    if multi {
        dialog.set_default_widget(Some(&done));
    }

    let p = Rc::new(Picking {
        state: state.clone(),
        plugin: plugin.to_string(),
        picker: picker.clone(),
        lists,
        limit,
        selected: RefCell::new(current.clone()),
        current,
        known: RefCell::default(),
        ticks: RefCell::default(),
        generation: Cell::new(0),
        groups: RefCell::default(),
        filling: Cell::new(false),
        dialog: dialog.downgrade(),
        search: search.downgrade(),
        bar: bar.downgrade(),
        by_hand: by_hand.downgrade(),
        group: group.downgrade(),
        stack: stack.downgrade(),
        content: content.downgrade(),
        status: status.downgrade(),
        retry: retry.downgrade(),
        manual: manual.downgrade(),
        on_done: Box::new(on_done),
    });

    {
        let p = p.clone();
        done.connect_clicked(move |_| p.commit());
    }
    {
        let d = dialog.downgrade();
        cancel.connect_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                d.close();
            }
        });
    }
    {
        let p = p.clone();
        clear.connect_clicked(move |_| p.finish(&[]));
    }
    {
        let p = p.clone();
        search.connect_search_changed(move |_| ask(&p));
    }
    {
        let p = p.clone();
        group.connect_selected_notify(move |_| {
            if !p.filling.get() {
                ask(&p);
            }
        });
    }
    {
        let p = p.clone();
        retry.connect_clicked(move |_| ask(&p));
    }
    {
        let p = p.clone();
        back.connect_clicked(move |_| ask(&p));
    }
    {
        let p = p.clone();
        by_hand.connect_clicked(move |_| {
            p.show("manual");
            if let Some(m) = p.manual.upgrade() {
                m.grab_focus();
            }
        });
    }
    {
        let p = p.clone();
        save.connect_clicked(move |_| p.commit());
    }
    {
        let p = p.clone();
        manual.connect_entry_activated(move |_| p.commit());
    }

    dialog.present(Some(parent));
    if lists {
        ask(&p);
        search.grab_focus();
    } else {
        p.show("manual");
        manual.grab_focus();
    }
}

/// Ask the plugin for the entries matching the search and the group; only the newest answer is
/// shown.
fn ask(p: &Rc<Picking>) {
    let g = p.generation.get().wrapping_add(1);
    p.generation.set(g);
    let Some(c) = p.state.client.borrow().clone() else {
        show_error(p, &ClientError::NotRunning.to_string());
        return;
    };
    let (query, group) = (p.query(), p.group_filter());
    // Keep the list on screen while a search runs; show the spinner otherwise.
    if p.page().as_deref() != Some("list") {
        p.show("loading");
    }
    let (plugin, picker) = (p.plugin.clone(), p.picker.id.clone());
    let (q, gr) = (query.clone(), group.clone());
    let p = p.clone();
    runtime::run(
        async move { c.plugin_list_items(&plugin, &picker, &q, &gr).await },
        move |r| {
            if p.generation.get() != g || p.dialog.upgrade().is_none() {
                return;
            }
            match r {
                Ok(items) => show_items(&p, &query, &group, items),
                Err(e) => show_error(&p, &e.to_string()),
            }
        },
    );
}

fn show_error(p: &Picking, message: &str) {
    if let Some(s) = p.status.upgrade() {
        s.set_icon_name(Some("dialog-warning-symbolic"));
        s.set_title("Couldn’t get the list");
        s.set_description(Some(&escape(message)));
    }
    offer(p, true);
    p.show("status");
}

/// Retry and "Enter manually…" on the status page, only when `shown`.
fn offer(p: &Picking, shown: bool) {
    if let Some(r) = p.retry.upgrade() {
        r.set_visible(shown);
    }
    if let Some(b) = p.by_hand.upgrade() {
        b.set_visible(shown);
    }
}

/// The group drop-down's entries, once, from the first unfiltered answer.
fn fill_groups(p: &Picking, items: &[PickItem]) {
    // An empty answer (a plugin still connecting, say) leaves the groups for the next one.
    if p.groups.borrow().is_some() || items.is_empty() {
        return;
    }
    let names = group_names(items);
    if let Some(d) = p.group.upgrade()
        && p.picker.groups
        && !names.is_empty()
    {
        let mut entries = vec!["All"];
        entries.extend(names.iter().map(String::as_str));
        p.filling.set(true);
        d.set_model(Some(&gtk::StringList::new(&entries)));
        d.set_selected(0);
        p.filling.set(false);
        d.set_visible(true);
    }
    *p.groups.borrow_mut() = Some(names);
}

fn show_items(p: &Rc<Picking>, query: &str, group: &str, items: Vec<PickItem>) {
    let unfiltered = query.trim().is_empty() && group.is_empty();
    if unfiltered {
        fill_groups(p, &items);
    }
    {
        let mut known = p.known.borrow_mut();
        for i in &items {
            known.insert(i.id.clone(), i.clone());
        }
    }
    if items.is_empty() {
        if let Some(s) = p.status.upgrade() {
            s.set_icon_name(Some("edit-find-symbolic"));
            s.set_title(&empty_message(query, group));
            s.set_description(None);
        }
        // A search that matches nothing is not something to retry or type instead; an empty
        // list may fill once the plugin is connected.
        offer(p, offer_manual(false, query, group));
        p.show("status");
        return;
    }
    let Some(content) = p.content.upgrade() else {
        return;
    };
    while let Some(c) = content.first_child() {
        content.remove(&c);
    }
    p.ticks.borrow_mut().clear();
    // The stored value comes first, within the cap on shown rows; an id the plugin no longer
    // lists stays there, marked, instead of vanishing.
    let all = current_first(items, &p.current, unfiltered);
    let (shown, note) = shown_items(&all, SHOW_MAX);
    let limit = if p.picker.multi { limit_note(p.limit) } else { None };
    if let Some(note) = limit.into_iter().chain(note).reduce(|a, b| format!("{a} {b}")) {
        content.append(
            &gtk::Label::builder()
                .label(note)
                .wrap(true)
                .xalign(0.0)
                .css_classes(["dim-label"])
                .build(),
        );
    }
    let sections = if p.picker.groups {
        group_items(shown)
    } else {
        vec![(String::new(), shown.to_vec())]
    };
    for (name, entries) in sections {
        if !name.is_empty() {
            content.append(
                &gtk::Label::builder()
                    .label(name)
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .build(),
            );
        }
        content.append(&match p.picker.style {
            PickerStyle::List => list(p, &entries).upcast::<gtk::Widget>(),
            PickerStyle::Grid => grid(p, &entries).upcast(),
        });
    }
    p.update_ticks();
    p.show("list");
}

/// A themed icon for the entry, only when the icon theme has it.
fn icon_of(i: &PickItem) -> Option<gtk::Image> {
    let name = icon_lookup(i.icon.as_deref()?)?;
    let display = gdk::Display::default()?;
    gtk::IconTheme::for_display(&display)
        .has_icon(name)
        .then(|| gtk::Image::from_icon_name(name))
}

fn state_label(i: &PickItem) -> Option<gtk::Label> {
    let s = i.state.as_deref().filter(|s| !s.is_empty())?;
    Some(
        gtk::Label::builder()
            .label(s)
            .valign(gtk::Align::Center)
            .css_classes(["dim-label"])
            .build(),
    )
}

fn list(p: &Rc<Picking>, entries: &[PickItem]) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    for i in entries {
        let row = adw::ActionRow::builder()
            .title(escape(&i.label))
            .activatable(true)
            .build();
        if let Some(s) = i.subtitle.as_deref().filter(|s| !s.is_empty()) {
            row.set_subtitle(&escape(s));
        }
        if let Some(icon) = icon_of(i) {
            row.add_prefix(&icon);
        }
        if let Some(l) = state_label(i) {
            row.add_suffix(&l);
        }
        if p.picker.multi {
            let check = gtk::CheckButton::builder()
                .active(p.selected.borrow().contains(&i.id))
                .valign(gtk::Align::Center)
                .build();
            p.ticks
                .borrow_mut()
                .push((i.id.clone(), check.upcast_ref::<gtk::Widget>().downgrade()));
            let (p, id) = (p.clone(), i.id.clone());
            check.connect_toggled(move |c| p.toggled(&id, c.is_active()));
            row.add_prefix(&check);
            row.set_activatable_widget(Some(&check));
        } else {
            if p.current.contains(&i.id) {
                row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            let (p, id) = (p.clone(), i.id.clone());
            row.connect_activated(move |_| p.finish(std::slice::from_ref(&id)));
        }
        list.append(&row);
    }
    list
}

fn tile(i: &PickItem) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    if let Some(icon) = icon_of(i) {
        icon.set_icon_size(gtk::IconSize::Large);
        b.append(&icon);
    }
    b.append(
        &gtk::Label::builder()
            .label(&i.label)
            .wrap(true)
            .justify(gtk::Justification::Center)
            .max_width_chars(14)
            .build(),
    );
    if let Some(l) = state_label(i) {
        l.set_halign(gtk::Align::Center);
        b.append(&l);
    }
    if let Some(s) = i.subtitle.as_deref().filter(|s| !s.is_empty()) {
        b.set_tooltip_text(Some(s));
    }
    b
}

fn grid(p: &Rc<Picking>, entries: &[PickItem]) -> gtk::FlowBox {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .row_spacing(6)
        .column_spacing(6)
        .build();
    for i in entries {
        let child: gtk::Widget = if p.picker.multi {
            let b = gtk::ToggleButton::builder()
                .child(&tile(i))
                .active(p.selected.borrow().contains(&i.id))
                .build();
            p.ticks
                .borrow_mut()
                .push((i.id.clone(), b.upcast_ref::<gtk::Widget>().downgrade()));
            let (p, id) = (p.clone(), i.id.clone());
            b.connect_toggled(move |b| p.toggled(&id, b.is_active()));
            b.upcast()
        } else {
            let b = gtk::Button::builder().child(&tile(i)).build();
            if p.current.contains(&i.id) {
                b.add_css_class("suggested-action");
            }
            let (p, id) = (p.clone(), i.id.clone());
            b.connect_clicked(move |_| p.finish(std::slice::from_ref(&id)));
            b.upcast()
        };
        flow.append(&child);
    }
    flow
}
