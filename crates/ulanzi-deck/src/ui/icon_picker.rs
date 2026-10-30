//! The image picker: icons, brand logos, installed apps, emoji, Steam games or a file of your own.
//! Picking applies to every selected key as one undo step.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ulanzi_core::Color;
use ulanzi_core::apps::AppEntry;
use ulanzi_core::edit::Field;
use ulanzi_core::import::import_image;
use ulanzi_core::model::{IconSet, ImageSource};
use ulanzi_core::render::IconLibrary;

use crate::state::State;
use crate::ui::action_editor::installed_apps;
use crate::ui::canvas::profile_dir;

/// Copy a picture into the profile and show it on `keys`. Decoding runs off the UI thread, so a
/// huge photo never freezes the window; `done` gets whether it worked.
pub fn import_into(state: &State, path: &Path, keys: &[u8], done: impl FnOnce(bool) + 'static) {
    if keys.is_empty() {
        done(false);
        return;
    }
    let (path, dir, keys) = (path.to_path_buf(), profile_dir(state), keys.to_vec());
    let state = state.clone();
    crate::runtime::run(
        async move { tokio::task::spawn_blocking(move || import_image(&path, &dir)).await },
        move |r| match r {
            Ok(Ok(rel)) => {
                state.edit(|e| e.set_field(&keys, Field::Source(ImageSource::Image { path: rel })));
                done(true);
            }
            Ok(Err(e)) => {
                crate::ui::toast(&state, &e.to_string());
                done(false);
            }
            Err(_) => done(false),
        },
    );
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

/// Finding each app's icon searches the icon themes, so this belongs off the UI thread.
fn app_items(apps: &[AppEntry]) -> Vec<Item> {
    apps.iter()
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

/// Pictures kept at most: enough for several full scrolls, far fewer than the icons there are.
const MAX_TEXTURES: usize = 1500;

thread_local! {
    /// Kept between openings, so the picker shows at once the second time.
    static TEXTURES: Textures = Rc::default();
    /// The apps with their icons, found once like the app list itself.
    static APP_ITEMS: RefCell<Option<Rc<Vec<Item>>>> = const { RefCell::new(None) };
}

fn cache_key(v: &Visual, fg: Color) -> Option<String> {
    match v {
        Visual::Icon(set, name) => Some(format!("{set:?}/{name}/{fg:?}")),
        Visual::File(p) => Some(p.to_string_lossy().into_owned()),
        _ => None,
    }
}

fn remember(cache: &Textures, key: String, texture: Option<gdk::Texture>) {
    let mut cache = cache.borrow_mut();
    if cache.len() >= MAX_TEXTURES {
        cache.clear();
    }
    cache.insert(key, texture);
}

/// A bundled icon, drawn now: it takes a fraction of a millisecond.
fn icon_texture(set: IconSet, name: &str, key: String, cache: &Textures, fg: Color) -> Option<gdk::Texture> {
    let cached = cache.borrow().get(&key).cloned();
    cached.unwrap_or_else(|| {
        let texture = IconLibrary::bundled()
            .icon_png(set, name, 64, fg)
            .and_then(|png| gdk::Texture::from_bytes(&glib::Bytes::from(&png)).ok());
        remember(cache, key, texture.clone());
        texture
    })
}

/// Show a picture file on `image`, decoding it off the UI thread: cover art is large enough to
/// stutter the scrolling. `current` says whether the cell still shows this item when it is ready.
fn show_file(
    image: &gtk::Image,
    path: &Path,
    key: String,
    cache: &Textures,
    current: impl Fn() -> bool + 'static,
) {
    let cached = cache.borrow().get(&key).cloned();
    if let Some(texture) = cached {
        match texture {
            Some(t) => image.set_paintable(Some(&t)),
            None => image.set_icon_name(Some("image-missing-symbolic")),
        }
        return;
    }
    image.set_paintable(gdk::Paintable::NONE);
    let (path, image, cache) = (path.to_path_buf(), image.clone(), cache.clone());
    crate::runtime::run(
        async move { tokio::task::spawn_blocking(move || gdk::Texture::from_filename(path).ok()).await },
        move |r| {
            let texture = r.ok().flatten();
            remember(&cache, key, texture.clone());
            if current() {
                match texture {
                    Some(t) => image.set_paintable(Some(&t)),
                    None => image.set_icon_name(Some("image-missing-symbolic")),
                }
            }
        },
    );
}

/// A searchable, virtualised grid of items; `on_pick` gets the chosen source.
fn grid(
    items: Rc<Vec<Item>>,
    search: &gtk::SearchEntry,
    cache: &Textures,
    fill_cells: bool,
    on_pick: Rc<dyn Fn(ImageSource)>,
) -> gtk::Widget {
    let indexes: Vec<String> = (0..items.len()).map(|i| i.to_string()).collect();
    let model = gtk::StringList::new(&indexes.iter().map(String::as_str).collect::<Vec<_>>());
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
            let bound = li.item();
            let Some(i) = bound
                .clone()
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
                Visual::Icon(set, name) => {
                    emoji.set_visible(false);
                    image.set_visible(true);
                    let key = cache_key(&item.visual, fg).unwrap_or_default();
                    match icon_texture(*set, name, key, &cache, fg) {
                        Some(t) => image.set_paintable(Some(&t)),
                        None => image.set_icon_name(Some("image-missing-symbolic")),
                    }
                }
                Visual::File(path) => {
                    emoji.set_visible(false);
                    image.set_visible(true);
                    let key = cache_key(&item.visual, fg).unwrap_or_default();
                    let li = li.clone();
                    show_file(&image, path, key, &cache, move || li.item() == bound);
                }
                Visual::Placeholder => {
                    emoji.set_visible(false);
                    image.set_visible(true);
                    image.set_icon_name(Some("image-missing-symbolic"));
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
        .vexpand(true)
        .build()
        .upcast()
}

/// Put `items` in `holder`: a grid, or `empty` when there are none.
fn fill(
    holder: &gtk::Box,
    items: Rc<Vec<Item>>,
    search: &gtk::SearchEntry,
    cache: &Textures,
    fill_cells: bool,
    on_pick: Rc<dyn Fn(ImageSource)>,
    empty: impl FnOnce() -> gtk::Widget,
) {
    while let Some(child) = holder.first_child() {
        holder.remove(&child);
    }
    if items.is_empty() {
        holder.append(&empty());
    } else {
        holder.append(&grid(items, search, cache, fill_cells, on_pick));
    }
}

/// A spinner for a tab whose items are still loading.
fn loading() -> gtk::Widget {
    adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .vexpand(true)
        .hexpand(true)
        .build()
        .upcast()
}

fn no_apps() -> gtk::Widget {
    empty_page("application-x-executable-symbolic", "No Apps Found", "")
}

fn no_games() -> gtk::Widget {
    empty_page(
        "input-gaming-symbolic",
        "No Steam Games",
        "Games installed with Steam show up here with their cover art",
    )
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
    let current = state.editor.borrow().key(keys[0]).map(|k| k.look.source.clone());
    show(state, parent, tab, current, Target::Keys(Rc::new(keys)));
}

/// Open the picker and hand the chosen image to `on_pick` (a picture file is copied into the
/// profile first), for images that are not a key's own (a cycle step's).
pub fn choose(
    state: &State,
    parent: &impl IsA<gtk::Widget>,
    current: Option<ImageSource>,
    on_pick: impl Fn(ImageSource) + 'static,
) {
    show(state, parent, None, current, Target::Pick(Rc::new(on_pick)));
}

/// Where a picked image goes.
#[derive(Clone)]
enum Target {
    Keys(Rc<Vec<u8>>),
    Pick(Rc<dyn Fn(ImageSource)>),
}

fn show(
    state: &State,
    parent: &impl IsA<gtk::Widget>,
    tab: Option<&str>,
    current: Option<ImageSource>,
    target: Target,
) {
    let dialog = adw::Dialog::builder()
        .title("Choose an Image")
        .content_width(640)
        .content_height(620)
        .build();
    // A tab is built when first shown, and tabs are not sized to the largest: measuring every grid
    // when the dialog opens took a full second.
    let stack = adw::ViewStack::builder()
        .hhomogeneous(false)
        .vhomogeneous(false)
        .build();
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
        let (state, target, dialog) = (state.clone(), target.clone(), dialog.clone());
        Rc::new(move |source| {
            match &target {
                Target::Keys(keys) => state.edit(|e| e.set_field(keys, Field::Source(source))),
                Target::Pick(f) => f(source),
            }
            dialog.close();
        })
    };
    let cache = TEXTURES.with(Rc::clone);

    type Build = Box<dyn FnOnce(&gtk::Box)>;
    let tabs: Rc<RefCell<HashMap<&'static str, (gtk::Box, Build)>>> = Rc::default();
    let page = |w: &gtk::Widget, name: &str, title: &str, icon: &str| {
        stack.add_titled_with_icon(w, Some(name), title, icon);
    };
    let add_tab = |name: &'static str, title: &str, icon: &str, build: Build| {
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page(holder.upcast_ref(), name, title, icon);
        tabs.borrow_mut().insert(name, (holder, build));
    };
    {
        let (search, cache, pick) = (search.clone(), cache.clone(), pick.clone());
        add_tab(
            "icons",
            "Icons",
            "shapes-symbolic",
            Box::new(move |holder| {
                let items = Rc::new(icon_items(IconSet::Lucide));
                holder.append(&grid(items, &search, &cache, false, pick));
            }),
        );
    }
    {
        let (search, cache, pick) = (search.clone(), cache.clone(), pick.clone());
        add_tab(
            "brands",
            "Brands",
            "starred-symbolic",
            Box::new(move |holder| {
                let items = Rc::new(icon_items(IconSet::Brands));
                holder.append(&grid(items, &search, &cache, false, pick));
                holder.append(
                    &gtk::Label::builder()
                        .label("Logos from Simple Icons. Trademarks belong to their owners.")
                        .css_classes(["dim-label", "caption"])
                        .margin_top(6)
                        .margin_bottom(8)
                        .build(),
                );
            }),
        );
    }
    {
        let (search, cache, pick) = (search.clone(), cache.clone(), pick.clone());
        add_tab(
            "apps",
            "Apps",
            "view-app-grid-symbolic",
            Box::new(move |holder| {
                if let Some(items) = APP_ITEMS.with_borrow(Clone::clone) {
                    fill(holder, items, &search, &cache, false, pick, no_apps);
                    return;
                }
                holder.append(&loading());
                let apps = installed_apps().to_vec();
                let holder = holder.clone();
                crate::runtime::run(
                    async move { tokio::task::spawn_blocking(move || app_items(&apps)).await },
                    move |r| {
                        let items = Rc::new(r.unwrap_or_default());
                        APP_ITEMS.with_borrow_mut(|a| *a = Some(items.clone()));
                        fill(&holder, items, &search, &cache, false, pick, no_apps);
                    },
                );
            }),
        );
    }
    {
        let (search, cache, pick) = (search.clone(), cache.clone(), pick.clone());
        add_tab(
            "emoji",
            "Emoji",
            "emoji-people-symbolic",
            Box::new(move |holder| {
                let items = Rc::new(emoji_items());
                holder.append(&grid(items, &search, &cache, false, pick));
            }),
        );
    }
    {
        let (search, cache, pick) = (search.clone(), cache.clone(), pick.clone());
        add_tab(
            "steam",
            "Steam",
            "input-gaming-symbolic",
            Box::new(move |holder| {
                holder.append(&loading());
                let holder = holder.clone();
                crate::runtime::run(
                    async move { tokio::task::spawn_blocking(steam_items).await },
                    move |r| {
                        let items = Rc::new(r.unwrap_or_default());
                        fill(&holder, items, &search, &cache, true, pick, no_games);
                    },
                );
            }),
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
        let (state, target, dialog) = (state.clone(), target.clone(), dialog.clone());
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
            let (state, target, dialog) = (state.clone(), target.clone(), dialog.clone());
            fd.open(window.as_ref(), None::<&gio::Cancellable>, move |res| {
                let Some(path) = res.ok().and_then(|f| f.path()) else {
                    return;
                };
                let dialog = dialog.clone();
                match &target {
                    Target::Keys(keys) => import_into(&state, &path, keys, move |ok| {
                        if ok {
                            dialog.close();
                        }
                    }),
                    Target::Pick(f) => {
                        let (f, dir, st) = (f.clone(), profile_dir(&state), state.clone());
                        crate::runtime::run(
                            async move { tokio::task::spawn_blocking(move || import_image(&path, &dir)).await },
                            move |r| match r {
                                Ok(Ok(rel)) => {
                                    f(ImageSource::Image { path: rel });
                                    dialog.close();
                                }
                                Ok(Err(e)) => crate::ui::toast(&st, &e.to_string()),
                                Err(_) => {}
                            },
                        );
                    }
                }
            });
        });
    }
    page(file_page.upcast_ref(), "file", "File", "folder-pictures-symbolic");

    let build_tab = move |name: &str| {
        // Out of the map first: building may ask for it again.
        let tab = tabs.borrow_mut().remove(name);
        if let Some((holder, build)) = tab {
            build(&holder);
        }
    };
    // The search only makes sense for the grids.
    {
        let (search_bar, build_tab) = (search_bar.clone(), build_tab.clone());
        stack.connect_visible_child_name_notify(move |s| {
            let name = s.visible_child_name();
            search_bar.set_visible(name.as_deref() != Some("file"));
            build_tab(name.as_deref().unwrap_or_default());
        });
    }
    // Start on the tab that matches the current image.
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
    build_tab(stack.visible_child_name().as_deref().unwrap_or_default());
    dialog.present(Some(parent));
    search.grab_focus();
}
