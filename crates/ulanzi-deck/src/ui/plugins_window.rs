//! The Plugins window (the puzzle button in the top bar): **Installed** lists the plugins with their
//! status and settings, and installs a file; **Discover** is the plugin store, searchable, with
//! category chips that wrap, a card per plugin and one button that installs, updates or opens it.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;
use ulanzi_core::plugin::store::{Listing, Offer, StoreEntry};

use crate::runtime;
use crate::state::{Change, State};
use crate::ui::integrations;

/// Open the window; `show` picks a page (`installed`, `discover`) or a plugin (`plugin:<id>`).
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>, show: Option<&str>) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Plugins")
        .content_width(760)
        .content_height(680)
        .build();
    let installed = adw::PreferencesPage::builder()
        .name("installed")
        .title("Installed")
        .icon_name("application-x-addon-symbolic")
        .build();
    installed.add(&integrations::plugins_group(state, &dialog));
    installed.add(&integrations::developer_group(state, &dialog));
    dialog.add(&installed);
    dialog.add(&discover_page(state, &dialog));
    match show {
        Some(p) if p.starts_with("plugin:") => {
            dialog.push_subpage(&integrations::subpage(state, &dialog, &p["plugin:".len()..]));
        }
        Some(p) => dialog.set_visible_page_name(p),
        None => {
            // Nothing installed yet: start where plugins come from.
            let none = state.plugins.borrow().loaded && state.plugins.borrow().list.is_empty();
            if none {
                dialog.set_visible_page_name("discover");
            }
        }
    }
    dialog.present(Some(parent));
}

thread_local! {
    /// Open Discover pages, told how installs are going.
    static OPEN: RefCell<Vec<Weak<Discover>>> = const { RefCell::new(Vec::new()) };
}

/// The service says how far installing `id` is: the open Discover pages show it on its card.
pub fn install_progress(id: &str, stage: &str, done: u64, total: u64) {
    OPEN.with(|o| {
        o.borrow_mut().retain(|d| d.upgrade().is_some());
        for d in o.borrow().iter().filter_map(Weak::upgrade) {
            d.show_progress(id, stage, done, total);
        }
    });
}

/// What a card under install says and how full its bar is (`None`: an animated bar).
pub fn progress_words(stage: &str, done: u64, total: u64) -> (String, Option<f64>) {
    let mb = |b: u64| b as f64 / 1_000_000.0;
    match stage {
        "downloading" if total > 0 => (
            format!("Downloading · {:.1} of {:.1} MB", mb(done), mb(total)),
            Some((done as f64 / total as f64).clamp(0.0, 1.0)),
        ),
        "downloading" => ("Downloading…".into(), None),
        "checking" => ("Checking that it is safe…".into(), None),
        "installing" => ("Installing…".into(), None),
        _ => ("Getting ready…".into(), None),
    }
}

/// The Discover page and what it shows now.
struct Discover {
    state: State,
    dialog: glib::WeakRef<adw::PreferencesDialog>,
    search: gtk::SearchEntry,
    chips: gtk::FlowBox,
    grid: gtk::FlowBox,
    stack: gtk::Stack,
    error: adw::StatusPage,
    note: gtk::Label,
    listing: RefCell<Option<Listing>>,
    /// The chosen category; empty for All.
    category: RefCell<String>,
    /// Plugins being installed now.
    busy: RefCell<BTreeSet<String>>,
    /// The progress bar and words on each card under install.
    bars: RefCell<std::collections::HashMap<String, (gtk::ProgressBar, gtk::Label)>>,
    /// The last progress per plugin, so a redrawn card shows it straight away.
    last: RefCell<std::collections::HashMap<String, (String, u64, u64)>>,
}

fn discover_page(state: &State, dialog: &adw::PreferencesDialog) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .name("discover")
        .title("Discover")
        .icon_name("system-software-install-symbolic")
        .build();
    let group = adw::PreferencesGroup::new();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search plugins")
        .hexpand(true)
        .build();
    // Chips wrap onto more lines rather than scroll sideways.
    let chips = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(6)
        .row_spacing(6)
        .max_children_per_line(12)
        .homogeneous(false)
        .build();
    let note = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .css_classes(["dim-label", "caption"])
        .build();
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(2)
        .column_spacing(12)
        .row_spacing(12)
        .valign(gtk::Align::Start)
        .build();
    let spinner = adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .margin_top(48)
        .build();
    let error = adw::StatusPage::builder()
        .icon_name("network-offline-symbolic")
        .title("Can’t Reach the Plugin Store")
        .css_classes(["compact"])
        .build();
    let retry = gtk::Button::builder()
        .label("Try Again")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();
    error.set_child(Some(&retry));
    let empty = adw::StatusPage::builder()
        .icon_name("edit-find-symbolic")
        .title("No Plugins Match")
        .description("Try other words or another category")
        .css_classes(["compact"])
        .build();
    let stack = gtk::Stack::builder().vhomogeneous(false).build();
    stack.add_named(&spinner, Some("loading"));
    stack.add_named(&error, Some("error"));
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&grid, Some("list"));
    body.append(&search);
    body.append(&chips);
    body.append(&note);
    body.append(&stack);
    group.add(&body);
    page.add(&group);

    let d = Rc::new(Discover {
        state: state.clone(),
        dialog: dialog.downgrade(),
        search: search.clone(),
        chips,
        grid,
        stack,
        error,
        note,
        listing: RefCell::default(),
        category: RefCell::default(),
        busy: RefCell::default(),
        bars: RefCell::default(),
        last: RefCell::default(),
    });
    OPEN.with(|o| o.borrow_mut().push(Rc::downgrade(&d)));
    {
        let weak = Rc::downgrade(&d);
        search.connect_search_changed(move |_| {
            if let Some(d) = weak.upgrade() {
                d.render();
            }
        });
    }
    {
        let weak = Rc::downgrade(&d);
        retry.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                d.fetch(true);
            }
        });
    }
    {
        // Installed versions changed (an install here, a removal on the Installed page).
        let weak = Rc::downgrade(&d);
        state.on_while(move |c| {
            let Some(d) = weak.upgrade() else {
                return false;
            };
            if c == Change::Plugins {
                d.render();
            }
            true
        });
    }
    d.fetch(false);
    // Held by the page: dropped with the window.
    let held = RefCell::new(Some(d));
    page.connect_destroy(move |_| {
        held.borrow_mut().take();
    });
    page
}

/// Whether `e` belongs in `category` (empty: all) and matches every word of `query`.
pub fn shown(e: &StoreEntry, category: &str, query: &str) -> bool {
    let in_category = category.is_empty() || e.categories.iter().any(|c| c == category);
    let hay = format!("{} {} {} {}", e.name, e.id, e.description, e.categories.join(" ")).to_lowercase();
    in_category && query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// A category id as a chip says it: `smart-home` → "Smart home".
pub fn category_title(c: &str) -> String {
    let spaced = c.replace(['-', '_'], " ");
    let mut chars = spaced.chars();
    chars
        .next()
        .map(|f| f.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// What the card's button says and whether it acts.
pub fn button_text(offer: &Offer, busy: bool) -> (String, bool) {
    match offer {
        _ if busy => ("Installing…".into(), false),
        Offer::Install => ("Install".into(), true),
        Offer::Update => ("Update".into(), true),
        Offer::Installed => ("Open".into(), true),
        Offer::NeedsApp(v) => (format!("Needs Ulanzi Deck {v}"), false),
    }
}

impl Discover {
    fn fetch(self: &Rc<Self>, refresh: bool) {
        self.stack.set_visible_child_name("loading");
        let Some(c) = self.state.client.borrow().clone() else {
            self.failed("The Ulanzi Deck service isn’t running");
            return;
        };
        let weak = Rc::downgrade(self);
        runtime::run(async move { c.plugin_store_index(refresh).await }, move |r| {
            let Some(d) = weak.upgrade() else { return };
            match r {
                Ok(listing) => {
                    *d.listing.borrow_mut() = Some(listing);
                    d.render_chips();
                    d.render();
                }
                Err(e) => d.failed(&e.to_string()),
            }
        });
    }

    fn failed(&self, why: &str) {
        self.error.set_description(Some(&glib::markup_escape_text(why)));
        self.stack.set_visible_child_name("error");
    }

    fn render_chips(self: &Rc<Self>) {
        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        let Some(listing) = self.listing.borrow().clone() else {
            return;
        };
        let mut categories: Vec<String> = listing
            .index
            .plugins
            .iter()
            .flat_map(|p| p.categories.clone())
            .collect();
        categories.sort();
        categories.dedup();
        // One category or none: the chips would only say what the list shows.
        self.chips.set_visible(categories.len() > 1);
        let current = self.category.borrow().clone();
        let mut first: Option<gtk::ToggleButton> = None;
        for c in std::iter::once(String::new()).chain(categories) {
            let label = if c.is_empty() {
                "All".to_string()
            } else {
                category_title(&c)
            };
            let b = gtk::ToggleButton::builder()
                .label(&label)
                .active(c == current)
                .css_classes(["category-tab"])
                .build();
            match &first {
                Some(f) => b.set_group(Some(f)),
                None => first = Some(b.clone()),
            }
            let weak: Weak<Self> = Rc::downgrade(self);
            b.connect_toggled(move |b| {
                if let (true, Some(d)) = (b.is_active(), weak.upgrade()) {
                    *d.category.borrow_mut() = c.clone();
                    d.render();
                }
            });
            self.chips.append(&b);
        }
    }

    fn render(self: &Rc<Self>) {
        let Some(listing) = self.listing.borrow().clone() else {
            return;
        };
        while let Some(c) = self.grid.first_child() {
            self.grid.remove(&c);
        }
        self.bars.borrow_mut().clear();
        self.note.set_visible(listing.offline);
        self.note
            .set_label("The plugin store can’t be reached right now: this is the list from last time.");
        let (category, query) = (self.category.borrow().clone(), self.search.text().to_string());
        let installed = self.state.plugins.borrow().list.clone();
        let mut any = false;
        for e in listing
            .index
            .plugins
            .iter()
            .filter(|e| shown(e, &category, &query))
        {
            let have = installed.iter().find(|r| r.id == e.id).map(|r| r.version.clone());
            self.grid.append(&self.card(e, &listing, have.as_deref()));
            any = true;
        }
        self.stack
            .set_visible_child_name(if any { "list" } else { "empty" });
    }

    fn card(self: &Rc<Self>, e: &StoreEntry, listing: &Listing, installed: Option<&str>) -> gtk::Widget {
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["card", "store-card"])
            .build();
        let top = gtk::Box::builder().spacing(12).build();
        top.append(&store_icon(listing, &e.id, 48));
        let titles = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        titles.append(
            &gtk::Label::builder()
                .label(&e.name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["heading"])
                .build(),
        );
        let maker = if e.official {
            "Ulanzi Deck Linux".to_string()
        } else {
            format!(
                "by {}",
                e.author
                    .as_deref()
                    .filter(|a| !a.trim().is_empty())
                    .unwrap_or("an unnamed author")
            )
        };
        let mut line = format!("{maker} · Version {}", e.version);
        if let Some(v) = installed.filter(|v| *v != e.version) {
            line.push_str(&format!(" · you have {v}"));
        }
        titles.append(
            &gtk::Label::builder()
                .label(&line)
                .xalign(0.0)
                .css_classes(["dim-label", "caption"])
                .build(),
        );
        top.append(&titles);
        card.append(&top);
        card.append(
            &gtk::Label::builder()
                .label(&e.description)
                .xalign(0.0)
                .wrap(true)
                .lines(3)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .vexpand(true)
                .valign(gtk::Align::Start)
                .css_classes(["dim-label"])
                .build(),
        );
        let bottom = gtk::Box::builder().spacing(8).build();
        let badge = |icon: &str, text: &str, tip: &str| {
            let b = gtk::Box::builder().spacing(4).tooltip_text(tip).build();
            b.append(&gtk::Image::from_icon_name(icon));
            b.append(&gtk::Label::builder().label(text).css_classes(["caption"]).build());
            b
        };
        // Everything in the store is signed; official ones say who made them.
        if let Some(t) = trust_badge(e.official, true) {
            bottom.append(&t);
        }
        if !e.tested_on_hardware {
            let untested = badge(
                "dialog-information-symbolic",
                "Not tested on a deck yet",
                "Report how it works for you",
            );
            untested.add_css_class("dim-label");
            bottom.append(&untested);
        }
        let offer = e.offer(installed);
        if self.busy.borrow().contains(&e.id) {
            card.append(&bottom);
            card.append(&self.progress_box(&e.id));
            return card.upcast();
        }
        let (text, acts) = button_text(&offer, false);
        let button = gtk::Button::builder()
            .label(&text)
            .sensitive(acts)
            .hexpand(true)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        if matches!(offer, Offer::Install | Offer::Update) {
            button.add_css_class("suggested-action");
        }
        {
            let (weak, id, name) = (Rc::downgrade(self), e.id.clone(), e.name.clone());
            button.connect_clicked(move |_| {
                let Some(d) = weak.upgrade() else { return };
                match offer {
                    Offer::Installed => d.open_plugin(&id),
                    _ => d.install(&id, &name),
                }
            });
        }
        bottom.append(&button);
        card.append(&bottom);
        card.upcast()
    }

    /// The bar and words a card shows while its plugin installs.
    fn progress_box(&self, id: &str) -> gtk::Box {
        let b = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let bar = gtk::ProgressBar::builder().pulse_step(0.08).build();
        let words = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["caption", "dim-label"])
            .build();
        b.append(&bar);
        b.append(&words);
        self.bars
            .borrow_mut()
            .insert(id.to_string(), (bar.clone(), words.clone()));
        let (stage, done, total) = self
            .last
            .borrow()
            .get(id)
            .cloned()
            .unwrap_or(("preparing".into(), 0, 0));
        set_progress(&bar, &words, &stage, done, total);
        // Animated while there is no number to show.
        let weak = bar.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
            let Some(bar) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if bar.widget_name() == "pulse" {
                bar.pulse();
            }
            glib::ControlFlow::Continue
        });
        b
    }

    fn show_progress(&self, id: &str, stage: &str, done: u64, total: u64) {
        self.last
            .borrow_mut()
            .insert(id.to_string(), (stage.to_string(), done, total));
        if let Some((bar, words)) = self.bars.borrow().get(id) {
            set_progress(bar, words, stage, done, total);
        }
    }

    fn open_plugin(&self, id: &str) {
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.push_subpage(&integrations::subpage(&self.state, &dialog, id));
        }
    }

    fn install(self: &Rc<Self>, id: &str, name: &str) {
        let Some(c) = self.state.client.borrow().clone() else {
            return;
        };
        self.busy.borrow_mut().insert(id.to_string());
        self.render();
        let (weak, id, name) = (Rc::downgrade(self), id.to_string(), name.to_string());
        let wanted = id.clone();
        runtime::run(async move { c.plugin_store_install(&wanted).await }, move |r| {
            let Some(d) = weak.upgrade() else { return };
            d.busy.borrow_mut().remove(&id);
            d.last.borrow_mut().remove(&id);
            crate::ui::window::refresh_plugins(&d.state);
            d.render();
            let Some(dialog) = d.dialog.upgrade() else { return };
            match r {
                Ok(_) => {
                    let toast = adw::Toast::builder()
                        .title(format!("{name} is installed"))
                        .button_label("Set Up")
                        .build();
                    let (weak, id) = (Rc::downgrade(&d), id.clone());
                    toast.connect_button_clicked(move |_| {
                        if let Some(d) = weak.upgrade() {
                            d.open_plugin(&id);
                        }
                    });
                    dialog.add_toast(toast);
                }
                Err(e) => {
                    let alert = adw::AlertDialog::builder()
                        .heading(format!("Can’t Install {name}"))
                        .body(e.to_string())
                        .default_response("close")
                        .close_response("close")
                        .build();
                    alert.add_response("close", "Close");
                    alert.present(Some(&dialog));
                }
            }
        });
    }
}

fn set_progress(bar: &gtk::ProgressBar, words: &gtk::Label, stage: &str, done: u64, total: u64) {
    let (text, fraction) = progress_words(stage, done, total);
    words.set_label(&text);
    match fraction {
        Some(f) => {
            bar.set_widget_name("");
            bar.set_fraction(f);
        }
        None => bar.set_widget_name("pulse"),
    }
}

/// The small pill that says how far a plugin is trusted: **Official** (made by the Ulanzi Deck
/// project, as the signed store list says) or **Verified** (signed); nothing for unsigned ones.
pub fn trust_badge(official: bool, verified: bool) -> Option<gtk::Label> {
    let (text, css, tip) = match (official, verified) {
        (true, true) => (
            "Official",
            "official",
            "Made by the Ulanzi Deck project. Signed, and checked before it installs.",
        ),
        (_, true) => (
            "Verified",
            "verified",
            "From the plugin store. Signed, and checked before it installs.",
        ),
        _ => return None,
    };
    Some(
        gtk::Label::builder()
            .label(text)
            .tooltip_text(tip)
            .valign(gtk::Align::Center)
            .css_classes(["trust-badge", css])
            .build(),
    )
}

/// A store plugin's icon from the copy the service keeps, `size` px; the add-on icon until then.
fn store_icon(listing: &Listing, id: &str, size: i32) -> gtk::Image {
    let px = u32::try_from(size * 2).unwrap_or(96);
    let image = listing
        .icons
        .get(id)
        .and_then(|p| ulanzi_core::render::icon_file_png(p, px, None))
        .and_then(|png| gtk::gdk::Texture::from_bytes(&glib::Bytes::from(&png)).ok())
        .map(|t| gtk::Image::from_paintable(Some(&t)))
        .unwrap_or_else(|| gtk::Image::from_icon_name("application-x-addon-symbolic"));
    image.set_pixel_size(size);
    image.set_size_request(size, size);
    image.set_valign(gtk::Align::Center);
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulanzi_core::plugin::store::StorePackage;

    fn entry(id: &str, name: &str, categories: &[&str]) -> StoreEntry {
        StoreEntry {
            id: id.into(),
            name: name.into(),
            version: "1.2.0".into(),
            description: "Lights, fans and air conditioners".into(),
            categories: categories.iter().map(|c| c.to_string()).collect(),
            contract: "1.2".into(),
            min_app: None,
            tested_on_hardware: true,
            icon: None,
            readme: None,
            package: StorePackage {
                url: String::new(),
                signature: String::new(),
                sha256: String::new(),
                size: 0,
            },
        }
    }

    #[test]
    fn search_and_categories_narrow_the_store() {
        let ha = entry("ha", "Home Assistant", &["smart-home"]);
        assert!(shown(&ha, "", ""));
        assert!(shown(&ha, "smart-home", "fans home"), "every word, anywhere");
        assert!(!shown(&ha, "streaming", ""));
        assert!(!shown(&ha, "", "obs"));
        assert_eq!(category_title("smart-home"), "Smart home");
    }

    #[test]
    fn install_progress_reads_plainly() {
        assert_eq!(
            progress_words("downloading", 1_200_000, 3_400_000),
            ("Downloading · 1.2 of 3.4 MB".to_string(), Some(1.2 / 3.4))
        );
        assert_eq!(progress_words("checking", 0, 0).1, None, "no number: animated");
        assert_eq!(progress_words("installing", 0, 0).0, "Installing…");
        assert_eq!(progress_words("preparing", 0, 0).0, "Getting ready…");
    }

    #[test]
    fn the_button_follows_what_is_installed() {
        let mut e = entry("ha", "Home Assistant", &[]);
        assert_eq!(button_text(&e.offer(None), false), ("Install".into(), true));
        assert_eq!(
            button_text(&e.offer(Some("1.1.0")), false),
            ("Update".into(), true)
        );
        assert_eq!(button_text(&e.offer(Some("1.2.0")), false), ("Open".into(), true));
        assert_eq!(button_text(&e.offer(None), true), ("Installing…".into(), false));
        e.min_app = Some("99.0.0".into());
        assert_eq!(
            button_text(&e.offer(None), false),
            ("Needs Ulanzi Deck 99.0.0".into(), false)
        );
    }
}
