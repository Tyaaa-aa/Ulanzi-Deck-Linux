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

/// A page of the key editor, picked with the switcher under the key preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Image and colours.
    Look,
    /// The label's text and place.
    Label,
    /// What a press (a tap, for the screen) does.
    Press,
    /// What holding does.
    Hold,
    TurnLeft,
    TurnRight,
    /// What the wide screen shows.
    Display,
    /// Things done to several selected keys at once.
    Keys,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Look => "Look",
            Section::Label => "Label",
            Section::Press => "Press",
            Section::Hold => "Hold",
            // Short, so a dial's four fit the panel; the tooltip and the page say "Turn".
            Section::TurnLeft => "Left",
            Section::TurnRight => "Right",
            Section::Display => "Display",
            Section::Keys => "Keys",
        }
    }

    /// The longer name, where the switcher's label is shortened.
    pub fn long_title(self) -> &'static str {
        match self {
            Section::TurnLeft => "Turn left",
            Section::TurnRight => "Turn right",
            s => s.title(),
        }
    }

    /// The section's page name in the editor's stack.
    fn name(self) -> &'static str {
        match self {
            Section::Look => "look",
            Section::Label => "label",
            Section::Press => "press",
            Section::Hold => "hold",
            Section::TurnLeft => "left",
            Section::TurnRight => "right",
            Section::Display => "display",
            Section::Keys => "keys",
        }
    }
}

/// The sections the editor shows for `slots` (the selection), in switcher order; empty when
/// there is nothing to edit. `screen` is what the wide screen is set to show.
pub fn sections(slots: &[Slot], screen: &ScreenConfig) -> Vec<Section> {
    use Section::*;
    match slots {
        [] => vec![],
        [Slot::Key(_)] => vec![Look, Label, Press, Hold],
        // The zones a dial has (see `Dial`).
        [Slot::Dial(_)] => vec![TurnLeft, TurnRight, Press, Hold],
        [Slot::Small(_)] => vec![Press],
        [Slot::Screen] => match screen {
            ScreenConfig::Custom { .. } => vec![Display, Press],
            _ => vec![Display],
        },
        _ if slots.iter().any(|s| matches!(s, Slot::Key(_))) => vec![Look, Label, Keys],
        _ => vec![],
    }
}

/// The section to open: the one last picked while it is there, else the first.
pub fn initial_section(remembered: Option<Section>, available: &[Section]) -> Option<Section> {
    remembered
        .filter(|r| available.contains(r))
        .or_else(|| available.first().copied())
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

        let library = crate::ui::library::build(state);
        stack.add_named(&library.root, Some("library"));

        // Top of the panel: close the key editor (select nothing) and go back to the action list.
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Close")
            .action_name("win.deselect")
            .visible(false)
            .build();
        let library_asked = Rc::new(std::cell::Cell::new(false));

        let ctx: C = Rc::new(Ctx {
            state: state.clone(),
            refreshers: RefCell::new(Vec::new()),
            rebuild: RefCell::new(None),
        });
        let rebuild: Rc<dyn Fn()> = {
            let (ctx_weak, content, stack, asked, close) = (
                Rc::downgrade(&ctx),
                content.clone(),
                stack.clone(),
                library_asked.clone(),
                close.clone(),
            );
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
                let has = fill(&ctx, &content);
                // With nothing selected the panel shows the actions to drag onto the deck.
                stack.set_visible_child_name(panel_page(has, asked.get()));
                close.set_visible(has);
            })
        };
        *ctx.rebuild.borrow_mut() = Some(rebuild.clone());
        rebuild();

        let show_library: Rc<dyn Fn()> = {
            let (stack, asked, search) = (stack.clone(), library_asked.clone(), library.search.clone());
            let state = state.clone();
            Rc::new(move || {
                asked.set(!state.selection.borrow().is_empty());
                stack.set_visible_child_name("library");
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
        }
    }
}

/// Fill the inspector for the current selection; false when nothing is selected.
fn fill(ctx: &C, content: &gtk::Box) -> bool {
    let slots = ctx.state.selection.borrow().slots();
    let keys = ctx.state.selection.borrow().keys();
    let screen = ctx.state.editor.borrow().profile().screen.clone();
    let list = sections(&slots, &screen);
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
    add_sections(ctx, content, &list, view);
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

/// The switcher (with more than one section), the sections in `list` order and the footer.
/// Each page is built once per rebuild; switching only changes the visible one.
fn add_sections(ctx: &C, content: &gtk::Box, list: &[Section], mut view: View) {
    let stack = gtk::Stack::builder()
        .vexpand(true)
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    let switcher = gtk::Box::builder()
        .spacing(4)
        .halign(gtk::Align::Center)
        .css_classes(["section-switcher"])
        .build();
    switcher.update_property(&[gtk::accessible::Property::Label("Sections")]);
    let open = initial_section(ctx.state.inspector_section.get(), list);
    let mut first: Option<gtk::ToggleButton> = None;
    for &section in list {
        let Some(i) = view.pages.iter().position(|(s, _)| *s == section) else {
            debug_assert!(false, "no page for {section:?}");
            continue;
        };
        let (_, page) = view.pages.remove(i);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&page)
            .build();
        stack.add_named(&scroller, Some(section.name()));
        let b = gtk::ToggleButton::builder()
            .label(section.title())
            .active(open == Some(section))
            .css_classes(["category-tab"])
            .build();
        if section.long_title() != section.title() {
            b.set_tooltip_text(Some(section.long_title()));
        }
        match &first {
            Some(f) => b.set_group(Some(f)),
            None => first = Some(b.clone()),
        }
        let (stack, state) = (stack.clone(), ctx.state.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                stack.set_visible_child_name(section.name());
                // Kept for the next key, until the app quits.
                state.inspector_section.set(Some(section));
            }
        });
        switcher.append(&b);
    }
    debug_assert!(view.pages.is_empty(), "pages the model does not list");
    if let Some(s) = open {
        stack.set_visible_child_name(s.name());
    }
    if list.len() > 1 {
        // Scrolls sideways rather than widen the panel, should a font make the pills too wide.
        let scroller = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(true)
            .halign(gtk::Align::Center)
            .child(&switcher)
            .build();
        content.append(&padded(scroller));
    }
    if !list.is_empty() {
        content.append(&stack);
    }
    if let Some(f) = view.footer {
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

// ---------------------------------------------------------------------------------------------
// Single key

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
    // The switcher names the section, so the groups need no title.
    let actions = adw::PreferencesGroup::new();
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

    let hold_group = adw::PreferencesGroup::builder()
        .description("Runs instead when the key is held for half a second")
        .build();
    let write_hold: Write = {
        let ctx = ctx.clone();
        Rc::new(move |a: Option<Action>, how: How, _| {
            ctx.commit(how, |e| {
                let tap = e.key(k).and_then(|k| k.tap.clone());
                e.set_key_actions(k, tap, a)
            })
        })
    };
    for w in slot_rows(
        ctx,
        &format!("key{k}/hold"),
        hold.as_ref(),
        write_hold,
        "Nothing (tap action only)",
    ) {
        hold_group.add(&w);
    }
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
            let mut look = key_look(&state, k);
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

/// The look of `keys`: image, colours, icon size and live state; and their label.
fn appearance_groups(ctx: &C, keys: Vec<u8>) -> (adw::PreferencesGroup, adw::PreferencesGroup) {
    let state = ctx.state.clone();
    let group = adw::PreferencesGroup::new();
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
    group.add(&image);

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
        group.add(&row);
    }

    // Label text with a variables menu.
    let label_row = adw::EntryRow::builder()
        .title(if label == Mixed::Mixed {
            "Label (mixed)"
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
    label_group.add(&label_row);

    let show = adw::SwitchRow::builder()
        .title("Show Label")
        .active(visible != Mixed::One(false))
        .build();
    if visible == Mixed::Mixed {
        show.set_subtitle("Mixed");
    }
    {
        let set = set.clone();
        show.connect_active_notify(move |r| {
            r.set_subtitle("");
            set(How::Set, Field::LabelVisible(r.is_active()))
        });
    }
    label_group.add(&show);

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
        ]))
        .selected(match pos {
            Mixed::One(Some(p)) => 1 + LabelPosition::ALL.iter().position(|x| *x == p).unwrap_or(0) as u32,
            _ => 0,
        })
        .build();
    position.set_subtitle(match pos {
        Mixed::Mixed => "Mixed",
        _ => "Default is set in Preferences",
    });
    {
        let set = set.clone();
        position.connect_selected_notify(move |r| {
            r.set_subtitle("Default is set in Preferences");
            let p = match r.selected() {
                0 => None,
                i => LabelPosition::ALL.get(i as usize - 1).copied(),
            };
            set(How::Set, Field::LabelPosition(p))
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
    group.add(&size);

    let live = adw::SwitchRow::builder()
        .title("Show Live State")
        .subtitle("Play/pause, mute and similar actions update their icon")
        .active(dynamic != Mixed::One(false))
        .build();
    live.connect_active_notify(move |r| set(How::Set, Field::Dynamic(r.is_active())));
    group.add(&live);
    (group, label_group)
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
    let picker = {
        let (write, state, path) = (write.clone(), state.clone(), path.to_string());
        action_editor::picker_button(
            &state.clone(),
            if action.is_some() { "Change" } else { "Choose" },
            true,
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
    let zones: [(Section, &str, &str, Option<&Action>, Pick); 4] = [
        (
            Section::TurnLeft,
            "Turn Left",
            "Choose what turning left does",
            dial.left.as_ref(),
            |d| &mut d.left,
        ),
        (
            Section::TurnRight,
            "Turn Right",
            "Choose what turning right does",
            dial.right.as_ref(),
            |d| &mut d.right,
        ),
        (
            Section::Press,
            "Press",
            "Choose what pressing the dial does",
            dial.press.as_ref(),
            |d| &mut d.press,
        ),
        (
            Section::Hold,
            "Hold",
            "Nothing (press action only)",
            dial.hold.as_ref(),
            |d| &mut d.hold,
        ),
    ];
    for (section, title, hint, action, pick) in zones {
        let write: Write = {
            let ctx = ctx.clone();
            Rc::new(move |a, how, _| ctx.commit(how, |e| e.set_dial(d, |dial| *pick(dial) = a)))
        };
        let group = adw::PreferencesGroup::builder()
            .title(section.long_title())
            .build();
        for w in slot_rows(ctx, &format!("dial{d}/{title}"), action, write, hint) {
            group.add(&w);
        }
        view.page(section).append(&group);
    }

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
    fn each_control_gets_its_own_sections() {
        use Section::*;
        let clock = ScreenConfig::FirmwareClock {
            style: ClockStyle::ALL[0],
        };
        let widgets = ScreenConfig::custom(vec![Widget::clock()], None, None);
        // The look takes two sections, so each fits a small window without scrolling.
        assert_eq!(sections(&[Slot::Key(3)], &clock), [Look, Label, Press, Hold]);
        // A dial has no look: its four zones, in the order they sit on the dial's card.
        assert_eq!(
            sections(&[Slot::Dial(1)], &clock),
            [TurnLeft, TurnRight, Press, Hold]
        );
        assert_eq!(sections(&[Slot::Small(0)], &clock), [Press]);
        // The screen can only be pressed when the app draws it.
        assert_eq!(sections(&[Slot::Screen], &clock), [Display]);
        assert_eq!(sections(&[Slot::Screen], &widgets), [Display, Press]);
        assert_eq!(
            sections(&[Slot::Key(0), Slot::Key(1)], &clock),
            [Look, Label, Keys]
        );
        // Keys mixed with other controls: the keys are edited together.
        assert_eq!(
            sections(&[Slot::Key(0), Slot::Dial(0)], &clock),
            [Look, Label, Keys]
        );
        assert!(sections(&[Slot::Dial(0), Slot::Small(1)], &clock).is_empty());
        assert!(sections(&[], &clock).is_empty());
    }

    #[test]
    fn section_titles_and_names_are_distinct() {
        use Section::*;
        let all = [Look, Label, Press, Hold, TurnLeft, TurnRight, Display, Keys];
        for (i, a) in all.iter().enumerate() {
            assert!(!a.title().is_empty());
            for b in &all[i + 1..] {
                assert_ne!(a.title(), b.title());
                assert_ne!(a.name(), b.name());
            }
        }
        assert_eq!(TurnLeft.title(), "Left");
        assert_eq!(TurnLeft.long_title(), "Turn left");
        assert_eq!(Press.long_title(), "Press");
    }

    #[test]
    fn the_last_section_stays_open_while_there() {
        use Section::*;
        let key = [Look, Label, Press, Hold];
        let dial = [TurnLeft, TurnRight, Press, Hold];
        assert_eq!(initial_section(None, &key), Some(Look));
        assert_eq!(initial_section(Some(Hold), &key), Some(Hold));
        assert_eq!(initial_section(Some(Label), &key), Some(Label));
        // From a key's Hold to a dial's Hold, and its Press back to a key's Press.
        assert_eq!(initial_section(Some(Hold), &dial), Some(Hold));
        assert_eq!(initial_section(Some(Press), &key), Some(Press));
        // Not there: the first.
        assert_eq!(initial_section(Some(Look), &dial), Some(TurnLeft));
        assert_eq!(initial_section(Some(Display), &key), Some(Look));
        assert_eq!(initial_section(Some(Look), &[]), None);
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

    /// The editor's section stack (the library has none with these page names).
    fn section_stack(panel: &Panel) -> Option<gtk::Stack> {
        all::<gtk::Stack>(&panel.root).into_iter().find(|s| {
            ["look", "left", "press", "display"]
                .iter()
                .any(|n| s.child_by_name(n).is_some())
        })
    }

    /// The switcher's buttons, as (label, active).
    fn switcher(panel: &Panel) -> Vec<(String, bool)> {
        all::<gtk::ToggleButton>(&panel.root)
            .into_iter()
            .filter(|b| b.parent().is_some_and(|p| p.has_css_class("section-switcher")))
            .map(|b| (b.label().unwrap_or_default().to_string(), b.is_active()))
            .collect()
    }

    fn select(state: &State, slot: Slot) {
        state.select(|s| {
            s.clear();
            s.click(slot)
        });
        while glib::MainContext::default().iteration(false) {}
    }

    #[test]
    fn the_editor_switches_sections_and_keeps_the_last_one() {
        crate::ui::with_gtk(|| {
            let state = state();
            let panel = Panel::build(&state);
            select(&state, Slot::Key(0));
            let names = |p: &Panel| switcher(p).into_iter().map(|(l, _)| l).collect::<Vec<_>>();
            assert_eq!(names(&panel), ["Look", "Label", "Press", "Hold"]);
            assert!(switcher(&panel)[0].1, "Look first");
            let stack = section_stack(&panel).expect("sections");
            assert_eq!(stack.visible_child_name().as_deref(), Some("look"));

            // Picking Hold shows it and is remembered for the next key and for a dial.
            let hold = all::<gtk::ToggleButton>(&panel.root)
                .into_iter()
                .find(|b| b.label().as_deref() == Some("Hold"))
                .expect("Hold");
            hold.set_active(true);
            assert_eq!(stack.visible_child_name().as_deref(), Some("hold"));
            assert_eq!(state.inspector_section.get(), Some(Section::Hold));
            select(&state, Slot::Key(5));
            let stack = section_stack(&panel).expect("sections");
            assert_eq!(stack.visible_child_name().as_deref(), Some("hold"));
            select(&state, Slot::Dial(0));
            assert_eq!(names(&panel), ["Left", "Right", "Press", "Hold"]);
            let stack = section_stack(&panel).expect("sections");
            assert_eq!(stack.visible_child_name().as_deref(), Some("hold"));
            // A small button has one section: no switcher, and Hold stays remembered.
            select(&state, Slot::Small(0));
            assert!(switcher(&panel).is_empty());
            let stack = section_stack(&panel).expect("sections");
            assert_eq!(stack.visible_child_name().as_deref(), Some("press"));
            select(&state, Slot::Key(1));
            assert_eq!(
                section_stack(&panel)
                    .expect("sections")
                    .visible_child_name()
                    .as_deref(),
                Some("hold")
            );
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

    /// At the default window size (1280×800) and at 1280×720, every section of a typical key
    /// and of a dial fits below the preview and the switcher, so nothing scrolls. The panel is
    /// 320 wide there (see window.rs: the inner split's minimum) and runs the window's height.
    #[test]
    fn sections_fit_without_scrolling_at_the_default_and_a_small_size() {
        crate::ui::with_gtk(|| {
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
                    let stack = section_stack(&panel).expect("sections");
                    pump_until(|| stack.height() > 0);
                    // The test display is smaller than 800px: give the panel its size directly.
                    panel.root.allocate(320, height, -1, None);
                    assert_eq!(panel.root.height(), height);
                    let (min_width, ..) = panel.root.measure(gtk::Orientation::Horizontal, -1);
                    assert!(min_width <= 320, "{slot:?} widens the panel to {min_width}px");
                    let mut page = stack.first_child();
                    while let Some(scroller) = page {
                        page = scroller.next_sibling();
                        let child = scroller.first_child().expect("page");
                        let (_, natural, _, _) = child.measure(gtk::Orientation::Vertical, stack.width());
                        let name = stack.page(&scroller).name().unwrap_or_default();
                        eprintln!(
                            "{height}px window, {slot:?} {name}: needs {natural}px of {}px",
                            stack.height()
                        );
                        assert!(
                            natural <= stack.height(),
                            "{slot:?} {name} needs {natural}px but has {}px at {height}px",
                            stack.height()
                        );
                    }
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
