//! The image picker: icons, brand logos, installed apps, emoji, Steam games or a file of your own.
//! Picking applies to every selected key as one undo step.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ulanzi_core::Color;
use ulanzi_core::edit::Field;
use ulanzi_core::import::import_image;
use ulanzi_core::model::{IconSet, ImageSource};
use ulanzi_core::render::IconLibrary;

use crate::state::State;
use crate::ui::action_editor::installed_apps;
use crate::ui::canvas::profile_dir;

/// Copy a picture into the profile and show it on `keys`.
pub fn import_into(state: &State, path: &Path, keys: &[u8]) -> bool {
    if keys.is_empty() {
        return false;
    }
    match import_image(path, &profile_dir(state)) {
        Ok(rel) => {
            state.edit(|e| e.set_field(keys, Field::Source(ImageSource::Image { path: rel })));
            true
        }
        Err(e) => {
            crate::ui::toast(state, &e.to_string());
            false
        }
    }
}

/// What a grid cell shows.
#[derive(Clone)]
enum Visual {
    Icon(IconSet, &'static str),
    File(PathBuf),
    Emoji(&'static str),
    Placeholder,
}

#[derive(Clone)]
struct Item {
    label: String,
    hay: String,
    visual: Visual,
    source: ImageSource,
}

/// Common emoji (character, search words).
const EMOJI: &[(&str, &str)] = &[
    ("😀", "grin smile happy"),
    ("😂", "joy laugh tears"),
    ("😎", "cool sunglasses"),
    ("🤔", "thinking hmm"),
    ("😴", "sleep zzz"),
    ("🥳", "party celebrate"),
    ("😡", "angry mad"),
    ("😭", "cry sob"),
    ("👍", "thumbs up yes ok"),
    ("👎", "thumbs down no"),
    ("👋", "wave hello bye"),
    ("👏", "clap applause"),
    ("🙏", "pray thanks please"),
    ("💪", "strong muscle"),
    ("❤️", "heart love red"),
    ("💔", "broken heart"),
    ("⭐", "star favourite"),
    ("✨", "sparkles magic"),
    ("🔥", "fire hot lit"),
    ("⚡", "lightning power zap"),
    ("💡", "idea light bulb"),
    ("🔔", "bell notification"),
    ("🔕", "bell mute silent"),
    ("🔊", "speaker loud volume"),
    ("🔇", "mute speaker"),
    ("🎤", "microphone mic sing"),
    ("🎧", "headphones music"),
    ("🎵", "music note"),
    ("🎶", "music notes"),
    ("🎮", "game controller gaming"),
    ("🕹️", "joystick arcade"),
    ("🎬", "clapper film movie"),
    ("📷", "camera photo"),
    ("🎥", "video camera"),
    ("📺", "tv television"),
    ("💻", "laptop computer"),
    ("🖥️", "desktop computer monitor"),
    ("⌨️", "keyboard"),
    ("🖱️", "mouse"),
    ("📱", "phone mobile"),
    ("☎️", "telephone call"),
    ("📞", "call phone"),
    ("✉️", "mail envelope email"),
    ("📧", "email mail"),
    ("💬", "chat message speech"),
    ("📝", "note memo write"),
    ("📁", "folder files"),
    ("📂", "open folder"),
    ("🗑️", "trash bin delete"),
    ("📅", "calendar date"),
    ("⏰", "alarm clock"),
    ("⏱️", "stopwatch timer"),
    ("⏯️", "play pause"),
    ("▶️", "play"),
    ("⏸️", "pause"),
    ("⏹️", "stop"),
    ("⏭️", "next track"),
    ("⏮️", "previous track"),
    ("🔴", "red circle record live"),
    ("🟢", "green circle go"),
    ("🟡", "yellow circle"),
    ("🔵", "blue circle"),
    ("⚫", "black circle"),
    ("⚪", "white circle"),
    ("✅", "check done yes"),
    ("❌", "cross no cancel"),
    ("⚠️", "warning caution"),
    ("⛔", "no entry stop"),
    ("🔒", "lock locked"),
    ("🔓", "unlock open"),
    ("🔑", "key password"),
    ("⚙️", "gear settings"),
    ("🛠️", "tools build"),
    ("🔧", "wrench fix"),
    ("🔨", "hammer"),
    ("🧹", "broom clean"),
    ("🔍", "search magnifier"),
    ("🌐", "globe web internet"),
    ("🏠", "home house"),
    ("🚀", "rocket launch"),
    ("✈️", "plane travel"),
    ("🚗", "car"),
    ("☀️", "sun day bright"),
    ("🌙", "moon night"),
    ("🌧️", "rain weather"),
    ("❄️", "snow cold"),
    ("☕", "coffee break"),
    ("🍕", "pizza food"),
    ("🍺", "beer drink"),
    ("🎉", "party popper celebrate"),
    ("🎁", "gift present"),
    ("🏆", "trophy win"),
    ("💰", "money bag"),
    ("📈", "chart up stocks"),
    ("📉", "chart down"),
    ("🐱", "cat"),
    ("🐶", "dog"),
    ("🦊", "fox"),
    ("🐧", "penguin linux"),
    ("👻", "ghost"),
    ("💀", "skull"),
    ("🤖", "robot bot"),
    ("👾", "alien game"),
    ("🧠", "brain think"),
    ("👀", "eyes look"),
    ("💤", "sleep zzz"),
    ("🔋", "battery"),
    ("🔌", "plug power"),
    ("📡", "satellite antenna stream"),
    ("🎙️", "studio microphone podcast"),
    ("🛑", "stop sign"),
    ("➕", "plus add"),
    ("➖", "minus remove"),
    ("⬆️", "up arrow"),
    ("⬇️", "down arrow"),
    ("⬅️", "left arrow back"),
    ("➡️", "right arrow forward"),
    ("🔁", "repeat loop"),
    ("🔀", "shuffle"),
];

fn fg_for_theme() -> Color {
    if adw::StyleManager::default().is_dark() {
        Color::rgb(0xee, 0xee, 0xee)
    } else {
        Color::rgb(0x2e, 0x34, 0x36)
    }
}

fn icon_items(set: IconSet) -> Vec<Item> {
    IconLibrary::bundled()
        .names(set)
        .map(|name| Item {
            label: name.replace('-', " "),
            hay: name.replace('-', " "),
            visual: Visual::Icon(set, name),
            source: ImageSource::Icon {
                set,
                name: name.to_string(),
            },
        })
        .collect()
}

fn app_items() -> Vec<Item> {
    installed_apps()
        .iter()
        .map(|a| Item {
            label: a.name.clone(),
            hay: format!("{} {} {}", a.name, a.desktop_id, a.keywords.join(" ")).to_lowercase(),
            visual: ulanzi_core::apps::app_icon(&a.desktop_id)
                .map(Visual::File)
                .unwrap_or(Visual::Placeholder),
            source: ImageSource::App {
                desktop_id: a.desktop_id.clone(),
            },
        })
        .collect()
}

fn emoji_items() -> Vec<Item> {
    EMOJI
        .iter()
        .map(|(ch, words)| Item {
            label: words.split(' ').next().unwrap_or("").to_string(),
            hay: words.to_string(),
            visual: Visual::Emoji(ch),
            source: ImageSource::Emoji { ch: ch.to_string() },
        })
        .collect()
}

fn steam_items() -> Vec<Item> {
    let Some(root) = ulanzi_core::steam::default_root() else {
        return Vec::new();
    };
    let mut games = ulanzi_core::steam::scan(&root);
    games.sort_by_key(|g| g.name.to_lowercase());
    games
        .into_iter()
        .map(|g| Item {
            hay: g.name.to_lowercase(),
            label: g.name,
            visual: g.art.map(Visual::File).unwrap_or(Visual::Placeholder),
            source: ImageSource::Steam { appid: g.appid },
        })
        .collect()
}

type Textures = Rc<RefCell<HashMap<String, Option<gdk::Texture>>>>;

fn texture_for(v: &Visual, cache: &Textures, fg: Color) -> Option<gdk::Texture> {
    let key = match v {
        Visual::Icon(set, name) => format!("{set:?}/{name}"),
        Visual::File(p) => p.to_string_lossy().into_owned(),
        _ => return None,
    };
    cache
        .borrow_mut()
        .entry(key)
        .or_insert_with(|| match v {
            Visual::Icon(set, name) => IconLibrary::bundled()
                .icon_png(*set, name, 64, fg)
                .and_then(|png| gdk::Texture::from_bytes(&glib::Bytes::from(&png)).ok()),
            Visual::File(p) => gdk::Texture::from_filename(p).ok(),
            _ => None,
        })
        .clone()
}

/// A searchable, virtualised grid of items; `on_pick` gets the chosen source.
fn grid(
    items: Vec<Item>,
    search: &gtk::SearchEntry,
    cache: &Textures,
    fill_cells: bool,
    on_pick: Rc<dyn Fn(ImageSource)>,
) -> gtk::Widget {
    let items = Rc::new(items);
    let model = gtk::StringList::new(&[]);
    for i in 0..items.len() {
        model.append(&i.to_string());
    }
    let filter = {
        let (items, search) = (items.clone(), search.clone());
        gtk::CustomFilter::new(move |obj| {
            let q = search.text().to_lowercase();
            let Some(i) = obj
                .downcast_ref::<gtk::StringObject>()
                .and_then(|s| s.string().parse::<usize>().ok())
            else {
                return false;
            };
            q.split_whitespace().all(|w| items[i].hay.contains(w))
        })
    };
    {
        let filter = filter.clone();
        search.connect_search_changed(move |_| filter.changed(gtk::FilterChange::Different));
    }
    let filtered = gtk::FilterListModel::new(Some(model), Some(filter));
    let selection = gtk::NoSelection::new(Some(filtered.clone()));

    let factory = gtk::SignalListItemFactory::new();
    let cell = if fill_cells { 96 } else { 72 };
    factory.connect_setup(move |_, obj| {
        let Some(li) = obj.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
        b.set_size_request(cell, cell + 8);
        let image = gtk::Image::builder()
            .pixel_size(if fill_cells { 72 } else { 40 })
            .css_classes(["picker-image"])
            .build();
        let emoji = gtk::Label::builder()
            .css_classes(["picker-emoji"])
            .visible(false)
            .build();
        let label = gtk::Label::builder()
            .css_classes(["caption"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(10)
            .build();
        b.append(&image);
        b.append(&emoji);
        b.append(&label);
        li.set_child(Some(&b));
    });
    {
        let (items, cache) = (items.clone(), cache.clone());
        let fg = fg_for_theme();
        factory.connect_bind(move |_, obj| {
            let Some(li) = obj.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(i) = li
                .item()
                .and_downcast::<gtk::StringObject>()
                .and_then(|s| s.string().parse::<usize>().ok())
            else {
                return;
            };
            let item = &items[i];
            let Some(b) = li.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(image) = b.first_child().and_downcast::<gtk::Image>() else {
                return;
            };
            let Some(emoji) = image.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(label) = emoji.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            label.set_label(&item.label);
            b.set_tooltip_text(Some(&item.label));
            match &item.visual {
                Visual::Emoji(ch) => {
                    image.set_visible(false);
                    emoji.set_visible(true);
                    emoji.set_label(ch);
                }
                v => {
                    emoji.set_visible(false);
                    image.set_visible(true);
                    match texture_for(v, &cache, fg) {
                        Some(t) => image.set_paintable(Some(&t)),
                        None => image.set_icon_name(Some("image-missing-symbolic")),
                    }
                }
            }
        });
    }
    let view = gtk::GridView::builder()
        .model(&selection)
        .factory(&factory)
        .min_columns(3)
        .max_columns(8)
        .single_click_activate(true)
        .css_classes(["picker-grid"])
        .build();
    view.connect_activate(move |_, pos| {
        let Some(i) = filtered
            .item(pos)
            .and_downcast::<gtk::StringObject>()
            .and_then(|s| s.string().parse::<usize>().ok())
        else {
            return;
        };
        on_pick(items[i].source.clone());
    });
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&view)
        .build()
        .upcast()
}

fn empty_page(icon: &str, title: &str, description: &str) -> gtk::Widget {
    adw::StatusPage::builder()
        .icon_name(icon)
        .title(title)
        .description(description)
        .css_classes(["compact"])
        .build()
        .upcast()
}

/// Open the picker for the selected keys.
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>) {
    open_at(state, parent, None);
}

/// Open the picker on a given tab (`icons`, `brands`, `apps`, `emoji`, `steam`, `file`).
pub fn open_at(state: &State, parent: &impl IsA<gtk::Widget>, tab: Option<&str>) {
    let keys = state.selection.borrow().keys();
    if keys.is_empty() {
        crate::ui::toast(state, "Select one or more keys first");
        return;
    }
    let keys = Rc::new(keys);
    let dialog = adw::Dialog::builder()
        .title("Choose an Image")
        .content_width(640)
        .content_height(620)
        .build();
    let stack = adw::ViewStack::new();
    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    let header = adw::HeaderBar::builder().title_widget(&switcher).build();
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search")
        .hexpand(true)
        .build();
    let search_bar = gtk::Box::builder()
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(6)
        .build();
    search_bar.append(&search);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&search_bar);
    view.set_content(Some(&stack));
    dialog.set_child(Some(&view));

    let pick: Rc<dyn Fn(ImageSource)> = {
        let (state, keys, dialog) = (state.clone(), keys.clone(), dialog.clone());
        Rc::new(move |source| {
            state.edit(|e| e.set_field(&keys, Field::Source(source)));
            dialog.close();
        })
    };
    let cache: Textures = Rc::new(RefCell::new(HashMap::new()));

    let page = |w: &gtk::Widget, name: &str, title: &str, icon: &str| {
        stack.add_titled_with_icon(w, Some(name), title, icon);
    };
    page(
        &grid(icon_items(IconSet::Lucide), &search, &cache, false, pick.clone()),
        "icons",
        "Icons",
        "shapes-symbolic",
    );
    let brands = gtk::Box::new(gtk::Orientation::Vertical, 0);
    brands.append(&grid(
        icon_items(IconSet::Brands),
        &search,
        &cache,
        false,
        pick.clone(),
    ));
    brands.append(
        &gtk::Label::builder()
            .label("Logos from Simple Icons. Trademarks belong to their owners.")
            .css_classes(["dim-label", "caption"])
            .margin_top(6)
            .margin_bottom(8)
            .build(),
    );
    page(brands.upcast_ref(), "brands", "Brands", "starred-symbolic");
    let apps = app_items();
    if apps.is_empty() {
        page(
            &empty_page("application-x-executable-symbolic", "No Apps Found", ""),
            "apps",
            "Apps",
            "view-app-grid-symbolic",
        );
    } else {
        page(
            &grid(apps, &search, &cache, false, pick.clone()),
            "apps",
            "Apps",
            "view-app-grid-symbolic",
        );
    }
    page(
        &grid(emoji_items(), &search, &cache, false, pick.clone()),
        "emoji",
        "Emoji",
        "emoji-people-symbolic",
    );
    let games = steam_items();
    if games.is_empty() {
        page(
            &empty_page(
                "input-gaming-symbolic",
                "No Steam Games",
                "Games installed with Steam show up here with their cover art",
            ),
            "steam",
            "Steam",
            "input-gaming-symbolic",
        );
    } else {
        page(
            &grid(games, &search, &cache, true, pick.clone()),
            "steam",
            "Steam",
            "input-gaming-symbolic",
        );
    }

    // Your own picture.
    let file_page = adw::StatusPage::builder()
        .icon_name("image-x-generic-symbolic")
        .title("Use Your Own Picture")
        .description("PNG, JPEG, WebP, GIF or SVG. You can also drop a file straight onto a key.")
        .css_classes(["compact"])
        .build();
    let choose = gtk::Button::builder()
        .label("Choose a File…")
        .halign(gtk::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    file_page.set_child(Some(&choose));
    {
        let (state, keys, dialog) = (state.clone(), keys.clone(), dialog.clone());
        choose.connect_clicked(move |b| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_mime_type("image/*");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let fd = gtk::FileDialog::builder()
                .title("Choose a Picture")
                .filters(&filters)
                .modal(true)
                .build();
            let window = b.root().and_downcast::<gtk::Window>();
            let (state, keys, dialog) = (state.clone(), keys.clone(), dialog.clone());
            fd.open(window.as_ref(), None::<&gio::Cancellable>, move |res| {
                if let Some(path) = res.ok().and_then(|f| f.path())
                    && import_into(&state, &path, &keys)
                {
                    dialog.close();
                }
            });
        });
    }
    page(file_page.upcast_ref(), "file", "File", "folder-pictures-symbolic");

    // The search only makes sense for the grids.
    {
        let search_bar = search_bar.clone();
        stack.connect_visible_child_name_notify(move |s| {
            search_bar.set_visible(s.visible_child_name().as_deref() != Some("file"));
        });
    }
    // Start on the tab that matches the current image.
    let current = state.editor.borrow().key(keys[0]).map(|k| k.look.source.clone());
    stack.set_visible_child_name(tab.unwrap_or(match current {
        Some(ImageSource::Icon {
            set: IconSet::Brands, ..
        }) => "brands",
        Some(ImageSource::App { .. }) => "apps",
        Some(ImageSource::Emoji { .. }) => "emoji",
        Some(ImageSource::Steam { .. }) => "steam",
        Some(ImageSource::Image { .. }) => "file",
        _ => "icons",
    }));
    dialog.present(Some(parent));
    search.grab_focus();
}
