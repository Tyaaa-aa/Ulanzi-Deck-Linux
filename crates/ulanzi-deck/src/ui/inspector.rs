//! Right-hand panel: the inspector for the selection, or the action library.
//!
//! The inspector is rebuilt when the selection, the page or the profile changes from elsewhere
//! (undo, a drop on the canvas, the daemon). Its own edits only refresh the preview and summaries,
//! so typing and dragging keep focus.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use ulanzi_core::Color;
use ulanzi_core::catalog;
use ulanzi_core::edit::{Editor, Field, Mixed, Slot};
use ulanzi_core::model::*;

use crate::state::{Change, State};
use crate::ui::action_editor::{self, How, OnChange};
use crate::ui::canvas;

pub struct Panel {
    pub root: gtk::Widget,
    pub header: adw::HeaderBar,
    /// Show the action list (keeping the selection, so a click there assigns to it) and put the
    /// typing focus in its search field (Ctrl+L).
    pub show_library: Rc<dyn Fn()>,
    /// The × and Escape: while the action list is shown over a selection (Ctrl+L), go back to
    /// the key's editor and return true; otherwise return false (the caller deselects).
    pub back_to_key: Rc<dyn Fn() -> bool>,
}

/// The panel's page: the key editor for a selection, else the action list. `library_asked`
/// (Ctrl+L) shows the list over a selection.
pub fn panel_page(has_selection: bool, library_asked: bool) -> &'static str {
    if has_selection && !library_asked {
        "inspector"
    } else {
        "library"
    }
}

/// What the panel's × (and Escape) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDoes {
    /// The action list is shown over a selection (Ctrl+L): back to the key's editor.
    BackToKey,
    /// Select nothing; the action list comes back.
    Deselect,
}

impl CloseDoes {
    pub fn tooltip(self) -> &'static str {
        match self {
            CloseDoes::BackToKey => "Back to the key",
            CloseDoes::Deselect => "Close",
        }
    }
}

pub fn close_does(has_selection: bool, library_asked: bool) -> CloseDoes {
    if has_selection && library_asked {
        CloseDoes::BackToKey
    } else {
        CloseDoes::Deselect
    }
}

/// A part of the editor's page, top to bottom in the order [`sections`] gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Image and colours.
    Look,
    /// The label's text and place.
    Label,
    /// What a press (a tap, for the screen; every zone, for a dial) does.
    Press,
    /// What holding does.
    Hold,
    /// What the wide screen shows.
    Display,
    /// Things done to several selected keys at once.
    Keys,
}

impl Section {
    /// The part's widget name (for tests and the inspector in GTK's debugger).
    fn name(self) -> &'static str {
        match self {
            Section::Look => "look",
            Section::Label => "label",
            Section::Press => "press",
            Section::Hold => "hold",
            Section::Display => "display",
            Section::Keys => "keys",
        }
    }
}

/// The parts the editor shows for `slots` (the selection), top to bottom; empty when
/// there is nothing to edit. `screen` is what the wide screen is set to show.
pub fn sections(slots: &[Slot], screen: &ScreenConfig) -> Vec<Section> {
    use Section::*;
    match slots {
        [] => vec![],
        [Slot::Key(_)] => vec![Press, Label, Hold, Look],
        // Its four zones (see `Dial`) together.
        [Slot::Dial(_)] => vec![Press],
        [Slot::Small(_)] => vec![Press],
        [Slot::Screen] => match screen {
            ScreenConfig::Custom { .. } => vec![Display, Press],
            _ => vec![Display],
        },
        _ if slots.iter().any(|s| matches!(s, Slot::Key(_))) => vec![Label, Look, Keys],
        _ => vec![],
    }
}

/// Shared by every row of one build of the inspector.
struct Ctx {
    state: State,
    /// Light refreshes after the inspector's own edits.
    refreshers: RefCell<Vec<Box<dyn Fn()>>>,
    rebuild: RefCell<Option<Rc<dyn Fn()>>>,
}

type C = Rc<Ctx>;

impl Ctx {
    /// Apply an edit made in the inspector.
    fn commit(self: &Rc<Self>, how: How, f: impl FnOnce(&mut Editor)) {
        let state = &self.state;
        state.inspector_edit.set(true);
        match how {
            How::Typing(key) => state.edit_merged(key, f),
            How::Set | How::Structure => state.edit(f),
        }
        state.inspector_edit.set(false);
        if how == How::Structure {
            self.schedule_rebuild();
        } else {
            for r in self.refreshers.borrow().iter() {
                r();
            }
        }
    }

    fn schedule_rebuild(&self) {
        if let Some(r) = self.rebuild.borrow().clone() {
            glib::idle_add_local_once(move || r());
        }
    }

    fn on_refresh(&self, f: impl Fn() + 'static) {
        self.refreshers.borrow_mut().push(Box::new(f));
    }
}

impl Panel {
    pub fn build(state: &State) -> Panel {
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        // The preview and the section switcher stay put; each section scrolls on its own, and
        // only when it is taller than the panel.
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        let clamp = adw::Clamp::builder()
            .maximum_size(420)
            .vexpand(true)
            .child(&content)
            .build();
        stack.add_named(&clamp, Some("inspector"));

        // Top of the panel: close the key editor (select nothing) and go back to the action list,
        // or, while the list is shown over a selection, back to the key's editor.
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text(CloseDoes::Deselect.tooltip())
            .visible(false)
            .build();
        let library_asked = Rc::new(std::cell::Cell::new(false));
        // Show the page and the × for the current selection and `library_asked`.
        let show_page: Rc<dyn Fn()> = {
            let (state, stack, asked, close) = (
                state.clone(),
                stack.downgrade(),
                library_asked.clone(),
                close.downgrade(),
            );
            Rc::new(move || {
                let has = !state.selection.borrow().is_empty();
                if let Some(stack) = stack.upgrade() {
                    stack.set_visible_child_name(panel_page(has, asked.get()));
                }
                if let Some(close) = close.upgrade() {
                    close.set_visible(has);
                    close.set_tooltip_text(Some(close_does(has, asked.get()).tooltip()));
                }
            })
        };
        let back_to_key: Rc<dyn Fn() -> bool> = {
            let (state, asked, show_page) = (state.clone(), library_asked.clone(), show_page.clone());
            Rc::new(move || {
                let has = !state.selection.borrow().is_empty();
                if close_does(has, asked.get()) != CloseDoes::BackToKey {
                    return false;
                }
                asked.set(false);
                show_page();
                true
            })
        };
        {
            let back = back_to_key.clone();
            close.connect_clicked(move |b| {
                if !back() {
                    let _ = b.activate_action("win.deselect", None);
                }
            });
        }

        // A click in the list over a selection assigns the action: the key's editor comes back.
        let library = {
            let (asked, show_page) = (library_asked.clone(), show_page.clone());
            crate::ui::library::build(state, move || {
                asked.set(false);
                show_page();
            })
        };
        stack.add_named(&library.root, Some("library"));

        let ctx: C = Rc::new(Ctx {
            state: state.clone(),
            refreshers: RefCell::new(Vec::new()),
            rebuild: RefCell::new(None),
        });
        let rebuild: Rc<dyn Fn()> = {
            let (ctx_weak, content, show_page) = (Rc::downgrade(&ctx), content.clone(), show_page.clone());
            Rc::new(move || {
                let Some(ctx) = ctx_weak.upgrade() else {
                    return;
                };
                ctx.refreshers.borrow_mut().clear();
                // Typing in a new set of rows starts a new undo step.
                ctx.state.editor.borrow_mut().set_merge(None);
                while let Some(c) = content.first_child() {
                    content.remove(&c);
                }
                fill(&ctx, &content);
                // With nothing selected the panel shows the actions to drag onto the deck.
                show_page();
            })
        };
        *ctx.rebuild.borrow_mut() = Some(rebuild.clone());
        rebuild();

        let show_library: Rc<dyn Fn()> = {
            let (asked, search, show_page) =
                (library_asked.clone(), library.search.clone(), show_page.clone());
            let state = state.clone();
            Rc::new(move || {
                asked.set(!state.selection.borrow().is_empty());
                show_page();
                search.grab_focus();
            })
        };
        {
            let (ctx, asked) = (ctx.clone(), library_asked.clone());
            // The daemon announces the profile list after every save; only a real change to it
            // (the Switch Profile choices) is worth a rebuild that would drop the typing focus.
            let known_profiles = RefCell::new(state.profiles.borrow().clone());
            // Only a plugin change the editor shows is worth a rebuild (not a plugin retrying).
            let known_plugins = RefCell::new(state.plugins.borrow().clone());
            state.on(move |c| match c {
                Change::Plugins => {
                    let now = ctx.state.plugins.borrow().clone();
                    if known_plugins.borrow().editor_differs(&now) {
                        *known_plugins.borrow_mut() = now;
                        ctx.schedule_rebuild();
                    }
                }
                Change::Profiles => {
                    let now = ctx.state.profiles.borrow().clone();
                    if *known_profiles.borrow() != now {
                        *known_profiles.borrow_mut() = now;
                        ctx.schedule_rebuild();
                    }
                }
                Change::Selection => {
                    // Picking something on the deck shows its editor.
                    asked.set(false);
                    ctx.schedule_rebuild();
                }
                Change::Page => ctx.schedule_rebuild(),
                Change::Profile if !ctx.state.inspector_edit.get() => ctx.schedule_rebuild(),
                _ => {}
            });
        }

        let view = adw::ToolbarView::new();
        let header = adw::HeaderBar::builder()
            .show_title(false)
            .show_start_title_buttons(false)
            .build();
        header.pack_start(&close);
        view.add_top_bar(&header);
        view.set_content(Some(&stack));
        Panel {
            root: view.upcast(),
            header,
            show_library,
            back_to_key,
        }
    }
}

/// Fill the inspector for the current selection; false when nothing is selected.
fn fill(ctx: &C, content: &gtk::Box) -> bool {
    let slots = ctx.state.selection.borrow().slots();
    let keys = ctx.state.selection.borrow().keys();
    let screen = ctx.state.editor.borrow().profile().screen.clone();
    let list = sections(&slots, &screen);
    if let [slot] = slots.as_slice()
        && ctx.state.editor.borrow().is_foreign(*slot)
    {
        foreign_view(ctx, content, *slot);
        return true;
    }
    let view = match slots.as_slice() {
        [] => return false,
        [Slot::Key(k)] => key_view(ctx, content, *k),
        [Slot::Dial(d)] => dial_view(ctx, content, *d),
        [Slot::Small(s)] => small_view(ctx, content, *s),
        [Slot::Screen] => screen_view(ctx, content, &screen),
        _ if !keys.is_empty() => multi_view(ctx, content, keys),
        _ => {
            content.append(&padded(title_block(
                &format!("{} Controls Selected", slots.len()),
                "Select keys only to edit them together",
            )));
            View::default()
        }
    };
    add_sections(content, &list, view);
    true
}

/// What a view puts below its fixed header: a page per section, and a row under them that
/// stays in sight.
#[derive(Default)]
struct View {
    pages: Vec<(Section, gtk::Box)>,
    footer: Option<gtk::Widget>,
}

impl View {
    fn page(&mut self, section: Section) -> gtk::Box {
        let page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_bottom(18)
            .margin_start(12)
            .margin_end(12)
            .build();
        self.pages.push((section, page.clone()));
        page
    }
}

/// Side margins for the fixed rows above and below the sections (the pages have their own).
fn padded(w: impl IsA<gtk::Widget>) -> impl IsA<gtk::Widget> {
    w.set_margin_start(12);
    w.set_margin_end(12);
    w
}

/// The sections in `list` order on one scrolling page (most-used first, so a key is set up
/// without switching anything), then the footer.
fn add_sections(content: &gtk::Box, list: &[Section], mut view: View) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for &section in list {
        let Some(i) = view.pages.iter().position(|(s, _)| *s == section) else {
            debug_assert!(false, "no page for {section:?}");
            continue;
        };
        let (_, page) = view.pages.remove(i);
        page.set_widget_name(section.name());
        body.append(&page);
    }
    debug_assert!(view.pages.is_empty(), "pages the model does not list");
    if !list.is_empty() {
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .css_classes(["inspector-body"])
            .build();
        content.append(&scroller);
    }
    if let Some(f) = view.footer {
        f.set_margin_top(6);
        f.set_margin_bottom(18);
        content.append(&padded(f));
    }
}

fn title_block(title: &str, subtitle: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["title-2"])
            .build(),
    );
    if !subtitle.is_empty() {
        b.append(
            &gtk::Label::builder()
                .label(subtitle)
                .xalign(0.0)
                .wrap(true)
                .css_classes(["dim-label"])
                .build(),
        );
    }
    b
}

fn page_name(state: &State) -> String {
    let e = state.editor.borrow();
    let p = e.page();
    format!(
        "Page {} · {}",
        p + 1,
        e.profile().pages.get(p).map(|p| p.name.as_str()).unwrap_or("")
    )
}

fn key_look(state: &State, k: u8) -> KeyLook {
    state
        .editor
        .borrow()
        .key(k)
        .map(|k| k.look.clone())
        .unwrap_or_default()
}

/// What the key preview shows: a cycling key's first step, a live label value by its name.
fn shown_look(state: &State, k: u8) -> KeyLook {
    let e = state.editor.borrow();
    let Some(key) = e.key(k) else {
        return KeyLook::default();
    };
    let mut look = key.shown_look();
    look.label.text = state
        .plugins
        .borrow()
        .shown_label(key.tap.as_ref(), &look.label.text);
    look
}

// ---------------------------------------------------------------------------------------------
// Single key

/// A control from a newer version: what it needs, and the one thing that can be done to it.
fn foreign_view(ctx: &C, content: &gtk::Box, slot: Slot) {
    let what = match slot {
        Slot::Key(_) => "key",
        Slot::Dial(_) => "dial",
        Slot::Small(_) => "button",
        Slot::Screen => "screen",
    };
    let state = &ctx.state;
    let min_app = state.editor.borrow().profile().min_app.clone();
    let sentence = ulanzi_core::model::foreign::needs_sentence(what, min_app.as_deref());
    content.append(&padded(title_block("Made With a Newer Version", &sentence)));
    let clear = gtk::Button::builder()
        .label(if what == "key" { "Clear Key" } else { "Clear" })
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    let st = state.clone();
    clear.connect_clicked(move |_| {
        st.edit(|e| match slot {
            Slot::Key(k) => e.clear(&[k]),
            other => e.clear_foreign(other),
        });
    });
    content.append(&padded(clear));
    if state
        .update
        .borrow()
        .as_ref()
        .is_some_and(|s| s.offered.is_some())
    {
        let update = gtk::Button::builder()
            .label("Update")
            .halign(gtk::Align::Center)
            .css_classes(["suggested-action", "pill"])
            .build();
        let st = state.clone();
        update.connect_clicked(move |b| crate::ui::updates::open(&st, b));
        content.append(&padded(update));
    }
}

fn key_view(ctx: &C, content: &gtk::Box, k: u8) -> View {
    let state = &ctx.state;
    let mut view = View::default();
    let header = gtk::Box::builder().spacing(16).build();
    header.append(&key_preview(ctx, k));
    let titles = title_block(&format!("Key {}", k + 1), &page_name(state));
    titles.set_valign(gtk::Align::Center);
    header.append(&titles);
    content.append(&padded(header));

    let problem = state
        .editor
        .borrow()
        .key(k)
        .and_then(|key| crate::ui::canvas::key_problem(state, key));
    if let Some(p) = problem {
        let banner = adw::Banner::builder()
            .title(format!("⚠ {}", glib::markup_escape_text(&p)))
            .revealed(true)
            .build();
        banner.add_css_class("key-problem");
        content.append(&padded(banner));
    }

    let (look, label) = appearance_groups(ctx, vec![k]);
    view.page(Section::Look).append(&look);
    view.page(Section::Label).append(&label);

    let (tap, hold) = {
        let e = state.editor.borrow();
        let key = e.key(k);
        (key.and_then(|k| k.tap.clone()), key.and_then(|k| k.hold.clone()))
    };
    let actions = adw::PreferencesGroup::builder().title("Press").build();
    let write_tap: Write = {
        let ctx = ctx.clone();
        Rc::new(move |a: Option<Action>, how: How, picked: bool| {
            let before = key_look(&ctx.state, k);
            ctx.commit(how, |e| {
                let hold = e.key(k).and_then(|k| k.hold.clone());
                match a {
                    // A freshly picked action also gives an untouched key its look.
                    Some(a) if picked => e.drop_action(Slot::Key(k), a),
                    a => e.set_key_actions(k, a, hold),
                }
            });
            // A new action (or app) can change the icon, colours and label: show them.
            if how != How::Structure && key_look(&ctx.state, k) != before {
                ctx.schedule_rebuild();
            }
        })
    };
    for w in slot_rows(
        ctx,
        &format!("key{k}/tap"),
        tap.as_ref(),
        write_tap,
        "Choose what this key does",
    ) {
        actions.add(&w);
    }
    view.page(Section::Press).append(&actions);

    let hold_group = adw::PreferencesGroup::new();
    let write_hold: Write = {
        let ctx = ctx.clone();
        Rc::new(move |a: Option<Action>, how: How, _| {
            ctx.commit(how, |e| {
                let tap = e.key(k).and_then(|k| k.tap.clone());
                e.set_key_actions(k, tap, a)
            })
        })
    };
    // Rarely used: one row until it has an action, and folded unless opened before.
    let held = slot_expander(
        ctx,
        &format!("key{k}/hold"),
        "When Held",
        hold.as_ref(),
        write_hold,
    );
    held.set_tooltip_text(Some(
        "Runs instead of Press when the key is held for half a second",
    ));
    hold_group.add(&held);
    // Repeat While Holding and Sequence use the key's release themselves.
    let macro_needs_release = {
        let e = state.editor.borrow();
        match e.key(k).and_then(|k| k.tap.as_ref()) {
            Some(Action::Macro { id, own }) => own
                .as_deref()
                .or_else(|| e.library().get(*id))
                .is_some_and(|m| matches!(m.trigger, Trigger::WhileHeld { .. } | Trigger::Sequence)),
            _ => false,
        }
    };
    hold_group.set_visible(!macro_needs_release);
    view.page(Section::Hold).append(&hold_group);

    let clear = gtk::Button::builder()
        .label("Clear Key")
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    {
        let ctx = ctx.clone();
        clear.connect_clicked(move |_| ctx.commit(How::Structure, |e| e.clear(&[k])));
    }
    // Clears the look and both actions, so it sits under every section.
    view.footer = Some(clear.upcast());
    view
}

fn key_preview(ctx: &C, k: u8) -> gtk::Widget {
    // A fixed size: a picture would ask for the rendered key's full size, taking room the
    // sections below need.
    let picture = gtk::Image::builder().pixel_size(106).build();
    let label = gtk::Label::builder()
        .css_classes(["deck-key-label"])
        .valign(gtk::Align::End)
        .halign(gtk::Align::Center)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(12)
        .build();
    let overlay = gtk::Overlay::builder().child(&picture).build();
    overlay.add_overlay(&label);
    let frame = gtk::Box::builder()
        .css_classes(["key-preview"])
        .valign(gtk::Align::Center)
        .build();
    frame.set_size_request(120, 120);
    frame.append(&overlay);
    let refresh = {
        let state = ctx.state.clone();
        move || {
            let mut look = shown_look(&state, k);
            if let Some(f) = state.preview.borrow().as_ref() {
                ulanzi_core::edit::apply_field(&mut look, f);
            }
            let tex = canvas::key_texture(&look, &canvas::profile_dir(&state));
            picture.set_paintable(tex.as_ref());
            let text = if look.label.visible {
                look.label.text.as_str()
            } else {
                ""
            };
            label.set_label(text);
            label.set_visible(!text.is_empty());
        }
    };
    refresh();
    let refresh = Rc::new(refresh);
    {
        let refresh = refresh.clone();
        ctx.on_refresh(move || refresh());
    }
    {
        // Live previews (hovered colours); weak so a rebuilt inspector drops it.
        let weak = Rc::downgrade(&refresh);
        ctx.state.on_while(move |c| {
            let Some(r) = weak.upgrade() else {
                return false;
            };
            if c == Change::Preview {
                r()
            }
            true
        });
    }
    frame.upcast()
}

// ---------------------------------------------------------------------------------------------
// Appearance (one key or many)

fn source_summary(s: &ImageSource) -> String {
    match s {
        ImageSource::None => "None".into(),
        ImageSource::Icon { name, .. } => name.replace('-', " "),
        ImageSource::Image { path } => std::path::Path::new(path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone()),
        ImageSource::Text { content } => format!("Text “{content}”"),
        ImageSource::App { desktop_id } => ulanzi_core::apps::lookup(desktop_id)
            .map(|a| a.name)
            .unwrap_or_else(|| desktop_id.clone()),
        ImageSource::Steam { appid } => format!("Steam game {appid}"),
        ImageSource::Emoji { ch } => ch.clone(),
        ImageSource::Plugin { icon, .. } => std::path::Path::new(icon)
            .file_stem()
            .map(|f| f.to_string_lossy().replace(['-', '_'], " "))
            .unwrap_or_else(|| icon.clone()),
    }
}

fn mixed_text<T>(m: &Mixed<T>, f: impl Fn(&T) -> String) -> String {
    match m {
        Mixed::One(v) => f(v),
        Mixed::Mixed => "Mixed".into(),
        Mixed::None => String::new(),
    }
}

/// Label variables the daemon fills in.
const VARIABLES: [(&str, &str); 10] = [
    ("cpu", "CPU usage %"),
    ("ram", "Memory usage %"),
    ("cpu_temp", "CPU temperature"),
    ("time", "Time"),
    ("date", "Date"),
    ("media_title", "Song title"),
    ("media_artist", "Artist"),
    ("media_status", "Playing / Paused"),
    ("page", "Page number"),
    ("profile", "Profile name"),
];

/// What a macro's key can show while it plays.
const MACRO_LABELS: [(&str, &str); 5] = [
    ("{macro.step}/{macro.total}", "Step of total"),
    ("{macro.step}", "Step number"),
    ("{macro.percent}%", "Percent done"),
    ("{macro.name}", "Macro name"),
    ("{macro.running}", "Playing (1) or not (0)"),
];

/// The look of `keys` (image, colours, icon size and live state) folded into one row, since a
/// fresh action already gives the key its look; and their label.
fn appearance_groups(ctx: &C, keys: Vec<u8>) -> (adw::PreferencesGroup, adw::PreferencesGroup) {
    let state = ctx.state.clone();
    let group = adw::PreferencesGroup::new();
    let look = adw::ExpanderRow::builder().title("Look").build();
    look.add_prefix(&gtk::Image::from_icon_name("applications-graphics-symbolic"));
    state.remember_expanded(&look, "look");
    group.add(&look);
    let label_group = adw::PreferencesGroup::new();
    let keys: Rc<Vec<u8>> = Rc::new(keys);
    let (source, bg, fg, label, visible, scale, dynamic, pos) = {
        let e = state.editor.borrow();
        (
            e.shared_look(&keys, |l| l.source.clone()),
            e.shared_look(&keys, |l| l.background),
            e.shared_look(&keys, |l| l.foreground),
            e.shared_look(&keys, |l| l.label.text.clone()),
            e.shared_look(&keys, |l| l.label.visible),
            e.shared_look(&keys, |l| (l.icon_scale * 100.0).round() as i32),
            e.shared_look(&keys, |l| l.dynamic),
            e.shared_look(&keys, |l| l.label.position),
        )
    };
    let set = {
        let (ctx, keys) = (ctx.clone(), keys.clone());
        move |how: How, f: Field| ctx.commit(how, |e| e.set_field(&keys, f))
    };

    look.set_subtitle(&glib::markup_escape_text(&match (&source, &bg) {
        (Mixed::Mixed, _) | (_, Mixed::Mixed) => "Mixed".to_string(),
        (Mixed::One(ImageSource::None), _) => "No image".to_string(),
        (Mixed::One(s), _) => source_summary(s),
        _ => String::new(),
    }));
    // Image
    let image = adw::ActionRow::builder()
        .title("Image")
        .subtitle(mixed_text(&source, source_summary))
        .build();
    let choose = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .action_name("win.pick-image")
        .build();
    image.add_suffix(&choose);
    if !matches!(source, Mixed::One(ImageSource::None)) {
        let remove = gtk::Button::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text("Remove image")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let set = set.clone();
        remove.connect_clicked(move |_| set(How::Structure, Field::Source(ImageSource::None)));
        image.add_suffix(&remove);
    }
    look.add_row(&image);

    // Colours
    for (title, value, is_bg) in [("Background", bg, true), ("Icon and Text", fg, false)] {
        let row = adw::ActionRow::builder().title(title).build();
        let field = move |c: Color| {
            if is_bg {
                Field::Background(c)
            } else {
                Field::Foreground(c)
            }
        };
        let current = match value {
            Mixed::One(c) => Some(c),
            _ => {
                row.set_subtitle("Mixed");
                None
            }
        };
        let preview = {
            let state = state.clone();
            move |c: Option<Color>| {
                *state.preview.borrow_mut() = c.map(field);
                state.notify(Change::Preview);
            }
        };
        let pick = {
            let (set, r) = (set.clone(), row.clone());
            move |c: Color| {
                r.set_subtitle("");
                set(How::Typing(if is_bg { "bg" } else { "fg" }), field(c));
            }
        };
        let button = crate::ui::color_popover::color_button(current, preview, pick);
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));
        look.add_row(&row);
    }

    // Label text with a variables menu.
    // A cycling key shows each step's label; this one only when a step has none.
    let cycling = keys.iter().all(|k| {
        matches!(
            state.editor.borrow().key(*k).and_then(|k| k.tap.as_ref()),
            Some(Action::Cycle { .. })
        )
    });
    let label_row = adw::EntryRow::builder()
        .title(if label == Mixed::Mixed {
            "Label (mixed)"
        } else if cycling {
            "Label (for steps without one)"
        } else {
            "Label"
        })
        .text(match &label {
            Mixed::One(t) => t.as_str(),
            _ => "",
        })
        .build();
    {
        let set = set.clone();
        label_row.connect_changed(move |r| set(How::Typing("label"), Field::LabelText(r.text().to_string())));
    }
    let vars = gtk::MenuButton::builder()
        .label("{ }")
        .tooltip_text("Insert live information")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let vars_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let pop = gtk::Popover::builder().child(&vars_list).build();
    for (name, what) in VARIABLES {
        let b = gtk::Button::builder().css_classes(["flat"]).build();
        let line = gtk::Box::builder().spacing(12).build();
        line.append(
            &gtk::Label::builder()
                .label(what)
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        line.append(
            &gtk::Label::builder()
                .label(format!("{{{name}}}"))
                .css_classes(["dim-label", "monospace"])
                .build(),
        );
        b.set_child(Some(&line));
        let (entry, pop) = (label_row.clone(), pop.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            let mut pos = entry.position();
            entry.insert_text(&format!("{{{name}}}"), &mut pos);
            entry.set_position(pos);
            entry.grab_focus_without_selecting();
        });
        vars_list.append(&b);
    }
    vars.set_popover(Some(&pop));
    label_row.add_suffix(&vars);
    // Live values the keys' plugin action offers (a temperature, a state), picked by name
    // instead of typed as {…} codes.
    let choices = label_choices(&state, &keys);
    if !choices.is_empty() {
        let mut names = vec!["Text".to_string()];
        names.extend(choices.iter().map(|c| c.name.clone()));
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let selected = match &label {
            Mixed::One(t) => choices.iter().position(|c| &c.text == t).map_or(0, |i| i + 1),
            _ => 0,
        };
        let shows = adw::ComboRow::builder()
            .title("Label Shows")
            .model(&gtk::StringList::new(&names))
            .selected(selected as u32)
            .build();
        label_row.set_visible(selected == 0);
        let (set, entry) = (set.clone(), label_row.clone());
        shows.connect_selected_notify(move |r| match r.selected() as usize {
            0 => {
                entry.set_visible(true);
                entry.set_text("");
                entry.grab_focus();
            }
            i => {
                entry.set_visible(false);
                if let Some(c) = choices.get(i - 1) {
                    set(How::Set, Field::LabelText(c.text.clone()));
                }
            }
        });
        label_group.add(&shows);
    }
    label_group.add(&label_row);

    // Say what "Default" means right now (the position set in Preferences).
    let default_position = format!(
        "Default ({})",
        match state.settings.borrow().label_style.align.as_str() {
            "top" => "Top",
            "center" => "Middle",
            _ => "Bottom",
        }
    );
    let position = adw::ComboRow::builder()
        .title("Label Position")
        .model(&gtk::StringList::new(&[
            default_position.as_str(),
            "Top",
            "Middle",
            "Bottom",
            "Hidden",
        ]))
        .selected(position_index(&visible, &pos))
        .build();
    let hint = "Default is set in Preferences";
    position.set_subtitle(if visible == Mixed::Mixed || pos == Mixed::Mixed {
        "Mixed"
    } else {
        hint
    });
    {
        let (ctx, keys) = (ctx.clone(), keys.clone());
        position.connect_selected_notify(move |r| {
            r.set_subtitle(hint);
            let (visible, p) = position_choice(r.selected());
            // One undo step for both.
            ctx.commit(How::Set, |e| {
                e.set_field(&keys, Field::LabelVisible(visible));
                if visible {
                    e.set_field(&keys, Field::LabelPosition(p));
                }
            });
        });
    }
    label_group.add(&position);

    let size = adw::SpinRow::with_range(30.0, 100.0, 5.0);
    size.set_title("Icon Size");
    size.set_subtitle(&match scale {
        Mixed::Mixed => "Mixed".into(),
        _ => "Percent of the key".to_string(),
    });
    size.set_value(match scale {
        Mixed::One(v) => v as f64,
        _ => 70.0,
    });
    {
        let set = set.clone();
        size.connect_value_notify(move |r| {
            set(How::Typing("scale"), Field::IconScale((r.value() / 100.0) as f32))
        });
    }
    look.add_row(&size);

    let live = adw::SwitchRow::builder()
        .title("Show Live State")
        .subtitle("The icon and colour follow what the key controls (on, off, playing, cooling)")
        .active(dynamic != Mixed::One(false))
        .build();
    live.connect_active_notify(move |r| set(How::Set, Field::Dynamic(r.is_active())));
    look.add_row(&live);
    (group, label_group)
}

/// The label values the keys offer: a macro's, or their plugin action's when they all have the same one.
fn label_choices(state: &State, keys: &[u8]) -> Vec<ulanzi_core::plugin::ManifestLabel> {
    let e = state.editor.borrow();
    let is_macro = |k: &u8| matches!(e.key(*k).and_then(|k| k.tap.as_ref()), Some(Action::Macro { .. }));
    if !keys.is_empty() && keys.iter().all(is_macro) {
        return MACRO_LABELS
            .iter()
            .map(|(text, title)| ulanzi_core::plugin::ManifestLabel {
                name: (*title).to_string(),
                text: (*text).to_string(),
                track: Vec::new(),
            })
            .collect();
    }
    let mut actions = keys.iter().map(|k| match e.key(*k).and_then(|k| k.tap.as_ref()) {
        Some(Action::Plugin { plugin, action, .. }) => Some((plugin.clone(), action.clone())),
        _ => None,
    });
    let Some(Some(first)) = actions.next() else {
        return Vec::new();
    };
    if actions.any(|a| a.as_ref() != Some(&first)) {
        return Vec::new();
    }
    state
        .plugins
        .borrow()
        .manifests
        .get(&first.0)
        .and_then(|m| m.action(&first.1))
        .map(|a| a.labels.clone())
        .unwrap_or_default()
}

/// The Label Position entry for the keys' visibility and position: Default, Top, Middle, Bottom
/// or Hidden (4).
fn position_index(visible: &Mixed<bool>, pos: &Mixed<Option<LabelPosition>>) -> u32 {
    if *visible == Mixed::One(false) {
        return 4;
    }
    match pos {
        Mixed::One(Some(p)) => 1 + LabelPosition::ALL.iter().position(|x| x == p).unwrap_or(0) as u32,
        _ => 0,
    }
}

/// What entry `i` of Label Position sets: (shown, position).
fn position_choice(i: u32) -> (bool, Option<LabelPosition>) {
    match i {
        0 => (true, None),
        4 => (false, None),
        i => (true, LabelPosition::ALL.get(i as usize - 1).copied()),
    }
}

// ---------------------------------------------------------------------------------------------
// Several keys

fn multi_view(ctx: &C, content: &gtk::Box, keys: Vec<u8>) -> View {
    let mut view = View::default();
    content.append(&padded(title_block(
        &format!("{} Keys Selected", keys.len()),
        "Changes apply to all of them",
    )));
    let (look, label) = appearance_groups(ctx, keys.clone());
    view.page(Section::Look).append(&look);
    view.page(Section::Label).append(&label);

    let group = adw::PreferencesGroup::builder().title("Keys").build();
    let keys = Rc::new(keys);

    let reset = adw::ButtonRow::builder()
        .title("Reset to Action Defaults")
        .start_icon_name("edit-undo-symbolic")
        .build();
    {
        let (ctx, keys) = (ctx.clone(), keys.clone());
        reset.connect_activated(move |_| ctx.commit(How::Structure, |e| e.reset_to_default_look(&keys)));
    }
    group.add(&reset);

    let copy = adw::ButtonRow::builder()
        .title("Copy Keys")
        .start_icon_name("edit-copy-symbolic")
        .build();
    {
        let (ctx, keys) = (ctx.clone(), keys.clone());
        copy.connect_activated(move |_| {
            ctx.state.editor.borrow_mut().copy_keys(&keys);
            crate::ui::toast(&ctx.state, "Copied. Select a key and paste with Ctrl+V");
        });
    }
    group.add(&copy);

    // Move to another page.
    let pages: Vec<(usize, String)> = {
        let e = ctx.state.editor.borrow();
        e.profile()
            .pages
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != e.page())
            .map(|(i, p)| (i, format!("{} · {}", i + 1, p.name)))
            .collect()
    };
    let move_row = adw::ActionRow::builder()
        .title("Move to Page")
        .subtitle(if pages.is_empty() {
            "Add another page first"
        } else {
            "Uses free slots on that page"
        })
        .build();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let pop = gtk::Popover::builder().child(&list).build();
    for (i, name) in pages.iter().cloned() {
        let b = gtk::Button::builder().label(name).css_classes(["flat"]).build();
        let (ctx, keys, pop) = (ctx.clone(), keys.clone(), pop.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            ctx.commit(How::Structure, |e| e.move_to_page(&keys, i));
            ctx.state.select(|s| s.clear());
        });
        list.append(&b);
    }
    let mb = gtk::MenuButton::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .popover(&pop)
        .sensitive(!pages.is_empty())
        .build();
    move_row.add_suffix(&mb);
    group.add(&move_row);
    let page = view.page(Section::Keys);
    page.append(&group);

    let clear = gtk::Button::builder()
        .label(format!("Clear {} Keys", keys.len()))
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    {
        let ctx = ctx.clone();
        clear.connect_clicked(move |_| ctx.commit(How::Structure, |e| e.clear(&keys)));
    }
    // Like a single key's Clear Key: under every section.
    view.footer = Some(clear.upcast());
    view
}

// ---------------------------------------------------------------------------------------------
// Action slots

/// Writes a slot's action: (action, how, picked from the list).
type Write = Rc<dyn Fn(Option<Action>, How, bool)>;

/// The action card (chip, title, summary, Change / Remove) followed by the parameter rows.
fn slot_rows(
    ctx: &C,
    path: &str,
    action: Option<&Action>,
    write: Write,
    empty_hint: &str,
) -> Vec<gtk::Widget> {
    let state = &ctx.state;
    let title = match action {
        Some(Action::Plugin { plugin, action, .. }) => state.plugins.borrow().action_title(plugin, action),
        Some(a) => catalog::spec_for(a)
            .map(|s| s.title)
            .unwrap_or("No Action")
            .to_string(),
        None => "No Action".to_string(),
    };
    let card = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title).as_str())
        .subtitle(
            action
                .map(|a| card_subtitle(state, a))
                .unwrap_or_else(|| empty_hint.to_string()),
        )
        .css_classes(["action-card"])
        .build();
    card.add_prefix(&action_editor::action_chip(state, action, 36));
    let picker = {
        let write = write.clone();
        action_editor::picker_button(
            state,
            if action.is_some() { "Change" } else { "Choose" },
            true,
            true,
            move |a| write(Some(a), How::Structure, true),
        )
    };
    card.add_suffix(&picker);
    if action.is_some() {
        let remove = gtk::Button::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text("Remove action")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let write = write.clone();
        remove.connect_clicked(move |_| write(None, How::Structure, false));
        card.add_suffix(&remove);
    }
    let mut rows: Vec<gtk::Widget> = vec![card.clone().upcast()];
    if let Some(a) = action {
        let on: OnChange = {
            let (card, state) = (card.clone(), state.clone());
            Rc::new(move |a: Action, how: How| {
                card.set_subtitle(&card_subtitle(&state, &a));
                write(Some(a), how, false);
            })
        };
        rows.extend(action_editor::rows_for(state, path, a, on));
    }
    rows
}

/// The action card's subtitle: the plugin's name for a plugin action (the title is the action's
/// name), else the action's summary.
fn card_subtitle(state: &State, a: &Action) -> String {
    let text = match a {
        Action::Plugin { plugin, .. } => state.plugins.borrow().plugin_name(plugin),
        Action::Macro { .. } => action_editor::macro_title(state, a),
        _ => catalog::describe(a),
    };
    glib::markup_escape_text(&text).to_string()
}

/// A collapsible slot (dial zones, screen tap/hold): title, summary, picker; parameters inside.
fn slot_expander(
    ctx: &C,
    path: &str,
    title: &str,
    action: Option<&Action>,
    write: Write,
) -> adw::ExpanderRow {
    let state = &ctx.state;
    let exp = adw::ExpanderRow::builder()
        .title(title)
        .subtitle(
            action
                .map(|a| glib::markup_escape_text(&state.plugins.borrow().describe(a)).to_string())
                .unwrap_or_else(|| "Nothing".into()),
        )
        .build();
    exp.add_prefix(&action_editor::action_chip(state, action, 32));
    state.remember_expanded(&exp, path);
    // A dial turn is many presses: no place for a macro.
    let lower = path.to_lowercase();
    let turning = lower.ends_with("left") || lower.ends_with("right");
    let picker = {
        let (write, state, path) = (write.clone(), state.clone(), path.to_string());
        action_editor::picker_button(
            &state.clone(),
            if action.is_some() { "Change" } else { "Choose" },
            true,
            !turning,
            move |a| {
                // Open the row so its settings are right there.
                state.expanded.borrow_mut().insert(path.clone());
                write(Some(a), How::Structure, true)
            },
        )
    };
    exp.add_suffix(&picker);
    match action {
        Some(a) => {
            let remove = adw::ButtonRow::builder()
                .title("Remove Action")
                .start_icon_name("edit-clear-symbolic")
                .build();
            let on: OnChange = {
                let (exp, write, state) = (exp.clone(), write.clone(), state.clone());
                Rc::new(move |a: Action, how: How| {
                    exp.set_subtitle(&glib::markup_escape_text(&state.plugins.borrow().describe(&a)));
                    write(Some(a), how, false);
                })
            };
            let rows = action_editor::rows_for(state, path, a, on);
            exp.set_enable_expansion(true);
            for r in rows {
                exp.add_row(&r);
            }
            remove.connect_activated(move |_| write(None, How::Structure, false));
            exp.add_row(&remove);
        }
        None => exp.set_enable_expansion(false),
    }
    exp
}

// ---------------------------------------------------------------------------------------------
// Dials, small buttons, wide screen

fn dial_view(ctx: &C, content: &gtk::Box, d: u8) -> View {
    let mut view = View::default();
    content.append(&padded(title_block(
        &format!("Dial {}", d + 1),
        &format!("{} · turn it, press it, or hold it down", page_name(&ctx.state)),
    )));
    let dial = ctx.state.editor.borrow().dials()[d as usize].clone();
    type Pick = fn(&mut Dial) -> &mut Option<Action>;
    // The path names are the old expander titles, which the undo merging keys on.
    let zones: [(&str, Option<&Action>, Pick); 4] = [
        ("Turn Left", dial.left.as_ref(), |d| &mut d.left),
        ("Turn Right", dial.right.as_ref(), |d| &mut d.right),
        ("Press", dial.press.as_ref(), |d| &mut d.press),
        ("Hold", dial.hold.as_ref(), |d| &mut d.hold),
    ];
    // One list: each zone opens to its settings.
    let group = adw::PreferencesGroup::new();
    for (title, action, pick) in zones {
        let write: Write = {
            let ctx = ctx.clone();
            Rc::new(move |a, how, _| ctx.commit(how, |e| e.set_dial(d, |dial| *pick(dial) = a)))
        };
        group.add(&slot_expander(
            ctx,
            &format!("dial{d}/{title}"),
            title,
            action,
            write,
        ));
    }
    view.page(Section::Press).append(&group);

    let all = adw::PreferencesGroup::builder()
        .description(
            "Each page has its own dials. New pages start with the dials of the page you add them from.",
        )
        .build();
    let copy = adw::ButtonRow::builder()
        .title("Use This Dial on Every Page")
        .start_icon_name("edit-copy-symbolic")
        .build();
    {
        let ctx = ctx.clone();
        copy.connect_activated(move |_| {
            ctx.commit(How::Set, |e| e.dial_to_all_pages(d));
            crate::ui::toast(
                &ctx.state,
                &format!("Dial {} is now the same on every page", d + 1),
            );
        });
    }
    all.add(&copy);
    // The same for every zone, so it stays under the sections.
    view.footer = Some(all.upcast());
    view
}

fn small_view(ctx: &C, content: &gtk::Box, s: u8) -> View {
    let mut view = View::default();
    let side = if s == 0 { "Left" } else { "Right" };
    content.append(&padded(title_block(
        &format!("{side} Button"),
        "The small button beside the dials",
    )));
    let action = {
        let e = ctx.state.editor.borrow();
        let b = &e.profile().small_buttons;
        if s == 0 { b.left.clone() } else { b.right.clone() }
    };
    let group = adw::PreferencesGroup::builder().title("Action").build();
    let write: Write = {
        let ctx = ctx.clone();
        Rc::new(move |a, how, _| ctx.commit(how, |e| e.set_small_button(s, a)))
    };
    for w in slot_rows(
        ctx,
        &format!("small{s}"),
        action.as_ref(),
        write,
        "Choose what this button does",
    ) {
        group.add(&w);
    }
    view.page(Section::Press).append(&group);
    view
}

/// The screen's display choices, in combo order.
const SCREEN_MODES: [&str; 7] = [
    "Clock",
    "Clock, Date and Weekday",
    "Clock and Weekday",
    "Clock and Date",
    "Large Clock",
    "System Stats",
    "Widgets",
];

fn screen_mode_index(s: &ScreenConfig) -> usize {
    match s {
        ScreenConfig::FirmwareClock { style } => ClockStyle::ALL.iter().position(|c| c == style).unwrap_or(0),
        ScreenConfig::FirmwareStats => 5,
        ScreenConfig::Custom { .. } => 6,
    }
}

fn screen_for_mode(i: usize, old: &ScreenConfig) -> ScreenConfig {
    match i {
        0..=4 => ScreenConfig::FirmwareClock {
            style: ClockStyle::ALL[i],
        },
        5 => ScreenConfig::FirmwareStats,
        // Back to widgets keeps the widgets already set up.
        _ => match old {
            ScreenConfig::Custom { .. } => old.clone(),
            _ => ScreenConfig::custom(vec![Widget::now_playing(), Widget::clock()], None, None),
        },
    }
}

fn screen_view(ctx: &C, content: &gtk::Box, screen: &ScreenConfig) -> View {
    let mut view = View::default();
    content.append(&padded(title_block(
        "Wide Screen",
        "The large display at the bottom right",
    )));
    let display = view.page(Section::Display);
    let group = adw::PreferencesGroup::builder().title("Shows").build();
    let mode = adw::ComboRow::builder()
        .title("Display")
        .model(&gtk::StringList::new(&SCREEN_MODES))
        .selected(screen_mode_index(screen) as u32)
        .build();
    {
        let ctx = ctx.clone();
        mode.connect_selected_notify(move |r| {
            let i = r.selected() as usize;
            ctx.commit(How::Structure, |e| {
                let new = screen_for_mode(i, &e.profile().screen);
                e.set_screen(new)
            })
        });
    }
    group.add(&mode);
    display.append(&group);

    match screen {
        ScreenConfig::Custom { tap, hold, .. } => {
            let edit: crate::ui::widgets_editor::EditScreen = {
                let ctx = ctx.clone();
                Rc::new(move |how, f| {
                    ctx.commit(how, |e| {
                        let mut s = e.profile().screen.clone();
                        f(&mut s);
                        e.set_screen(s)
                    })
                })
            };
            for g in crate::ui::widgets_editor::groups(&ctx.state, screen, edit) {
                display.append(&g);
            }
            let group = adw::PreferencesGroup::builder()
                .title("When Pressed")
                .description("Without a tap action, tapping plays or pauses Now Playing, starts the timer, or shows the next widget.")
                .build();
            for (title, action, is_tap) in [("Tap", tap, true), ("Hold", hold, false)] {
                let write: Write = {
                    let ctx = ctx.clone();
                    Rc::new(move |a, how, _| {
                        ctx.commit(how, |e| {
                            let mut s = e.profile().screen.clone();
                            if let ScreenConfig::Custom { tap, hold, .. } = &mut s {
                                if is_tap {
                                    *tap = a;
                                } else {
                                    *hold = a;
                                }
                            }
                            e.set_screen(s)
                        })
                    })
                };
                group.add(&slot_expander(
                    ctx,
                    &format!("screen/{title}"),
                    title,
                    action.as_ref(),
                    write,
                ));
            }
            view.page(Section::Press).append(&group);
        }
        _ => {
            display.append(
                &gtk::Label::builder()
                    .label(
                        "The deck draws the clock and stats itself, so the screen can’t be \
                         pressed in these modes. Choose Widgets to draw your own and give it an action.",
                    )
                    .wrap(true)
                    .xalign(0.0)
                    .css_classes(["dim-label", "caption"])
                    .build(),
            );
        }
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_shows_the_editor_for_a_selection_unless_the_list_was_asked_for() {
        assert_eq!(panel_page(true, false), "inspector");
        assert_eq!(panel_page(true, true), "library");
        assert_eq!(panel_page(false, false), "library");
        assert_eq!(panel_page(false, true), "library");
    }

    #[test]
    fn the_close_button_goes_back_to_the_key_while_the_list_is_asked_for() {
        assert_eq!(close_does(true, true), CloseDoes::BackToKey);
        assert_eq!(close_does(true, false), CloseDoes::Deselect);
        assert_eq!(close_does(false, true), CloseDoes::Deselect);
        assert_eq!(CloseDoes::BackToKey.tooltip(), "Back to the key");
        assert_eq!(CloseDoes::Deselect.tooltip(), "Close");
    }

    #[test]
    fn the_list_asked_over_a_key_gives_way_to_its_editor() {
        crate::ui::with_gtk(|| {
            let state = state();
            let panel = Panel::build(&state);
            let page = |p: &Panel| {
                all::<gtk::Stack>(&p.root)
                    .into_iter()
                    .find(|s| s.child_by_name("library").is_some())
                    .and_then(|s| s.visible_child_name())
                    .map(|n| n.to_string())
            };
            let close = |p: &Panel| {
                all::<gtk::Button>(&p.header.clone().upcast())
                    .into_iter()
                    .find(|b| b.icon_name().as_deref() == Some("window-close-symbolic"))
                    .expect("×")
            };
            select(&state, Slot::Key(0));
            assert_eq!(page(&panel).as_deref(), Some("inspector"));
            assert_eq!(close(&panel).tooltip_text().as_deref(), Some("Close"));

            // Ctrl+L, then the × (or Escape): back to the key, still selected.
            (panel.show_library)();
            assert_eq!(page(&panel).as_deref(), Some("library"));
            assert_eq!(close(&panel).tooltip_text().as_deref(), Some("Back to the key"));
            // Rebuilds (an edit from elsewhere) keep the list.
            state.notify(Change::Page);
            while glib::MainContext::default().iteration(false) {}
            assert_eq!(page(&panel).as_deref(), Some("library"));
            assert!((panel.back_to_key)());
            assert_eq!(page(&panel).as_deref(), Some("inspector"));
            assert!(!state.selection.borrow().is_empty());
            assert_eq!(close(&panel).tooltip_text().as_deref(), Some("Close"));
            // Not asked: the caller deselects.
            assert!(!(panel.back_to_key)());

            // Ctrl+L, then a click in the list assigns and shows the editor again.
            state.edit(|e| e.set_key_actions(0, None, None));
            while glib::MainContext::default().iteration(false) {}
            (panel.show_library)();
            let library = all::<gtk::Stack>(&panel.root)
                .into_iter()
                .find_map(|s| s.child_by_name("library"))
                .expect("library page");
            let row = all::<gtk::ListBoxRow>(&library)
                .into_iter()
                .next()
                .expect("an action row");
            row.activate();
            pump_until(|| page(&panel).as_deref() == Some("inspector"));
            assert_eq!(page(&panel).as_deref(), Some("inspector"));
            let tap = state.editor.borrow().key(0).and_then(|k| k.tap.clone());
            assert!(tap.is_some(), "the click assigned the action");
            // Later rebuilds keep the editor.
            state.notify(Change::Page);
            while glib::MainContext::default().iteration(false) {}
            assert_eq!(page(&panel).as_deref(), Some("inspector"));

            // Ctrl+L, then another key picked on the deck: its editor.
            (panel.show_library)();
            select(&state, Slot::Key(3));
            pump_until(|| page(&panel).as_deref() == Some("inspector"));
            assert_eq!(page(&panel).as_deref(), Some("inspector"));
        });
    }

    #[test]
    fn each_control_gets_its_own_sections() {
        use Section::*;
        let clock = ScreenConfig::FirmwareClock {
            style: ClockStyle::ALL[0],
        };
        let widgets = ScreenConfig::custom(vec![Widget::clock()], None, None);
        // What a key does first, its name next; hold and look are folded until opened.
        assert_eq!(sections(&[Slot::Key(3)], &clock), [Press, Label, Hold, Look]);
        // A dial's four zones sit together.
        assert_eq!(sections(&[Slot::Dial(1)], &clock), [Press]);
        assert_eq!(sections(&[Slot::Small(0)], &clock), [Press]);
        // The screen can only be pressed when the app draws it.
        assert_eq!(sections(&[Slot::Screen], &clock), [Display]);
        assert_eq!(sections(&[Slot::Screen], &widgets), [Display, Press]);
        assert_eq!(
            sections(&[Slot::Key(0), Slot::Key(1)], &clock),
            [Label, Look, Keys]
        );
        // Keys mixed with other controls: the keys are edited together.
        assert_eq!(
            sections(&[Slot::Key(0), Slot::Dial(0)], &clock),
            [Label, Look, Keys]
        );
        assert!(sections(&[Slot::Dial(0), Slot::Small(1)], &clock).is_empty());
        assert!(sections(&[], &clock).is_empty());
    }

    #[test]
    fn section_names_are_distinct() {
        use Section::*;
        let all = [Look, Label, Press, Hold, Display, Keys];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.name(), b.name());
            }
        }
    }

    #[test]
    fn label_position_covers_hidden() {
        assert_eq!(position_index(&Mixed::One(true), &Mixed::One(None)), 0);
        assert_eq!(
            position_index(&Mixed::One(true), &Mixed::One(Some(LabelPosition::Middle))),
            2
        );
        assert_eq!(
            position_index(&Mixed::One(false), &Mixed::One(Some(LabelPosition::Top))),
            4
        );
        for i in 0..5 {
            let (shown, p) = position_choice(i);
            let back = position_index(&Mixed::One(shown), &Mixed::One(p));
            assert_eq!(back, i, "entry {i}");
        }
    }

    fn state() -> State {
        crate::state::AppState::new(Profile::new_default("Default", Model::D200X))
    }

    fn all<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Vec<T> {
        let mut out = Vec::new();
        let mut c = root.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            if let Ok(t) = w.clone().downcast::<T>() {
                out.push(t);
            }
            out.extend(all::<T>(&w));
        }
        out
    }

    /// Run the main loop until `done` (at most a few seconds).
    fn pump_until(done: impl Fn() -> bool) {
        let main = glib::MainContext::default();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            while main.iteration(false) {}
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// The editor's scrolling page.
    fn body(panel: &Panel) -> Option<gtk::ScrolledWindow> {
        all::<gtk::ScrolledWindow>(&panel.root)
            .into_iter()
            .find(|s| s.has_css_class("inspector-body"))
    }

    /// The parts on the page, top to bottom.
    fn parts(panel: &Panel) -> Vec<String> {
        let Some(page) = body(panel).and_then(|b| b.child()).and_then(|v| v.first_child()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut c = page.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            out.push(w.widget_name().to_string());
        }
        out
    }

    fn select(state: &State, slot: Slot) {
        state.select(|s| {
            s.clear();
            s.click(slot)
        });
        while glib::MainContext::default().iteration(false) {}
    }

    #[test]
    fn the_editor_shows_every_part_on_one_page() {
        crate::ui::with_gtk(|| {
            let state = state();
            let panel = Panel::build(&state);
            select(&state, Slot::Key(0));
            assert_eq!(parts(&panel), ["press", "label", "hold", "look"]);
            assert!(
                all::<gtk::ToggleButton>(&panel.root)
                    .iter()
                    .all(|b| !b.has_css_class("category-tab")),
                "no section switcher"
            );
            // Hold and Look are folded rows.
            let titles: Vec<String> = all::<adw::ExpanderRow>(&panel.root)
                .iter()
                .map(|e| e.title().to_string())
                .collect();
            assert!(
                titles.contains(&"When Held".to_string()) && titles.contains(&"Look".to_string()),
                "{titles:?}"
            );
            // A dial: its four zones in one list.
            select(&state, Slot::Dial(0));
            assert_eq!(parts(&panel), ["press"]);
            let zones: Vec<String> = all::<adw::ExpanderRow>(&panel.root)
                .iter()
                .map(|e| e.title().to_string())
                .collect();
            assert_eq!(zones, ["Turn Left", "Turn Right", "Press", "Hold"]);
        });
    }

    #[test]
    fn rebuilding_the_editor_leaves_no_listeners_behind() {
        crate::ui::with_gtk(|| {
            let state = state();
            let _panel = Panel::build(&state);
            select(&state, Slot::Key(0));
            state.notify(Change::Preview);
            let before = state.listener_count();
            for k in [1, 2, 0, 3, 0] {
                select(&state, Slot::Key(k));
                // Listeners of the old build go on the next change.
                state.notify(Change::Preview);
            }
            assert_eq!(state.listener_count(), before);
        });
    }

    /// A key with an action on press and on hold, as most keys are, and a volume dial.
    fn typical_key(state: &State) {
        state.edit(|e| {
            e.drop_action(
                Slot::Key(0),
                Action::Shortcut {
                    keys: "Ctrl+C".into(),
                },
            );
            let tap = e.key(0).and_then(|k| k.tap.clone());
            e.set_key_actions(
                0,
                tap,
                Some(Action::Command {
                    cmd: "notify-send hi".into(),
                    terminal: false,
                }),
            );
            e.set_field(&[0], Field::LabelText("Copy".into()));
            // A dial turning the volume, the most common setup.
            let make = |id: &str| (catalog::catalog().iter().find(|s| s.id == id).expect("spec").make)();
            e.set_dial(0, |d| {
                d.left = Some(make("volume_down"));
                d.right = Some(make("volume_up"));
                d.press = Some(make("mute"));
            });
        });
    }

    /// At the default window size (1280×800) and at 1280×720, the whole page of a typical key
    /// and of a dial (folded rows folded) fits below the preview, so nothing scrolls. The panel is
    /// 320 wide there (see window.rs: the inner split's minimum) and runs the window's height.
    #[test]
    fn sections_fit_without_scrolling_at_the_default_and_a_small_size() {
        // The sizes depend on the fonts installed; the margins are about 12px. Fail on them only
        // when asked (`ULANZI_LAYOUT_TEST=1`), otherwise print them.
        let strict = std::env::var("ULANZI_LAYOUT_TEST").is_ok_and(|v| v == "1");
        let check = move |ok: bool, msg: String| {
            if strict {
                assert!(ok, "{msg}");
            } else if !ok {
                eprintln!("layout (not enforced without ULANZI_LAYOUT_TEST=1): {msg}");
            }
        };
        crate::ui::with_gtk(move || {
            let css = gtk::CssProvider::new();
            css.load_from_string(include_str!("../style.css"));
            gtk::style_context_add_provider_for_display(
                &gtk::gdk::Display::default().expect("display"),
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            for height in [800, 720] {
                let state = state();
                typical_key(&state);
                let panel = Panel::build(&state);
                let win = gtk::Window::builder()
                    .decorated(false)
                    .default_width(320)
                    .default_height(height)
                    .child(&panel.root)
                    .build();
                win.present();
                for slot in [Slot::Key(0), Slot::Dial(0)] {
                    select(&state, slot);
                    let scroller = body(&panel).expect("page");
                    pump_until(|| scroller.height() > 0);
                    // The test display is smaller than 800px: give the panel its size directly.
                    panel.root.allocate(320, height, -1, None);
                    assert_eq!(panel.root.height(), height);
                    let (min_width, ..) = panel.root.measure(gtk::Orientation::Horizontal, -1);
                    check(
                        min_width <= 320,
                        format!("{slot:?} widens the panel to {min_width}px"),
                    );
                    let child = scroller.child().expect("page");
                    let (_, natural, _, _) = child.measure(gtk::Orientation::Vertical, scroller.width());
                    eprintln!(
                        "{height}px window, {slot:?}: needs {natural}px of {}px",
                        scroller.height()
                    );
                    check(
                        natural <= scroller.height(),
                        format!(
                            "{slot:?} needs {natural}px but has {}px at {height}px",
                            scroller.height()
                        ),
                    );
                }
                win.destroy();
            }
        });
    }

    #[test]
    fn screen_modes_round_trip() {
        let old = ScreenConfig::custom(vec![Widget::now_playing()], Some(Action::NextPage), None);
        for i in 0..SCREEN_MODES.len() {
            assert_eq!(screen_mode_index(&screen_for_mode(i, &old)), i);
        }
        // Choosing Widgets again keeps the widgets and the tap action.
        assert!(matches!(
            screen_for_mode(6, &old),
            ScreenConfig::Custom {
                tap: Some(Action::NextPage),
                ..
            }
        ));
    }
}
