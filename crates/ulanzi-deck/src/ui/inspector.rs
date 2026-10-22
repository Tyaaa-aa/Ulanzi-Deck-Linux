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
    pub fn build(state: &State, library_toggle: &gtk::ToggleButton) -> Panel {
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let clamp = adw::Clamp::builder().maximum_size(420).child(&content).build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        stack.add_named(&scroller, Some("inspector"));

        stack.add_named(&crate::ui::library::build(state), Some("library"));

        let ctx: C = Rc::new(Ctx {
            state: state.clone(),
            refreshers: RefCell::new(Vec::new()),
            rebuild: RefCell::new(None),
        });
        let rebuild: Rc<dyn Fn()> = {
            let (ctx_weak, content, stack, toggle) = (
                Rc::downgrade(&ctx),
                content.clone(),
                stack.clone(),
                library_toggle.clone(),
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
                stack.set_visible_child_name(if has && !toggle.is_active() {
                    "inspector"
                } else {
                    "library"
                });
            })
        };
        *ctx.rebuild.borrow_mut() = Some(rebuild.clone());
        rebuild();

        {
            let (stack, ctx) = (stack.clone(), ctx.clone());
            library_toggle.connect_toggled(move |t| {
                if t.is_active() {
                    stack.set_visible_child_name("library");
                } else {
                    ctx.schedule_rebuild();
                }
            });
        }
        {
            let (ctx, toggle) = (ctx.clone(), library_toggle.clone());
            // The daemon announces the profile list after every save; only a real change to it
            // (the Switch Profile choices) is worth a rebuild that would drop the typing focus.
            let known_profiles = RefCell::new(state.profiles.borrow().clone());
            state.on(move |c| match c {
                Change::Profiles => {
                    let now = ctx.state.profiles.borrow().clone();
                    if *known_profiles.borrow() != now {
                        *known_profiles.borrow_mut() = now;
                        ctx.schedule_rebuild();
                    }
                }
                Change::Selection => {
                    // Picking something on the deck shows its inspector.
                    if toggle.is_active() && !ctx.state.selection.borrow().is_empty() {
                        toggle.set_active(false);
                    }
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
        view.add_top_bar(&header);
        view.set_content(Some(&stack));
        Panel {
            root: view.upcast(),
            header,
        }
    }
}

/// Fill the inspector for the current selection; false when nothing is selected.
fn fill(ctx: &C, content: &gtk::Box) -> bool {
    let slots = ctx.state.selection.borrow().slots();
    let keys = ctx.state.selection.borrow().keys();
    match slots.as_slice() {
        [] => return false,
        [Slot::Key(k)] => key_view(ctx, content, *k),
        [Slot::Dial(d)] => dial_view(ctx, content, *d),
        [Slot::Small(s)] => small_view(ctx, content, *s),
        [Slot::Screen] => screen_view(ctx, content),
        _ if !keys.is_empty() => multi_view(ctx, content, keys),
        _ => {
            content.append(&title_block(
                &format!("{} Controls Selected", slots.len()),
                "Select keys only to edit them together",
            ));
        }
    }
    true
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

fn key_view(ctx: &C, content: &gtk::Box, k: u8) {
    let state = &ctx.state;
    let header = gtk::Box::builder().spacing(16).build();
    header.append(&key_preview(ctx, k));
    let titles = title_block(&format!("Key {}", k + 1), &page_name(state));
    titles.set_valign(gtk::Align::Center);
    header.append(&titles);
    content.append(&header);

    content.append(&appearance_group(ctx, vec![k]));

    let (tap, hold) = {
        let e = state.editor.borrow();
        let key = e.key(k);
        (key.and_then(|k| k.tap.clone()), key.and_then(|k| k.hold.clone()))
    };
    let actions = adw::PreferencesGroup::builder().title("Action").build();
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
    content.append(&actions);

    let hold_group = adw::PreferencesGroup::builder()
        .title("Hold")
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
    content.append(&hold_group);

    let clear = gtk::Button::builder()
        .label("Clear Key")
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    {
        let ctx = ctx.clone();
        clear.connect_clicked(move |_| ctx.commit(How::Structure, |e| e.clear(&[k])));
    }
    content.append(&clear);
}

fn key_preview(ctx: &C, k: u8) -> gtk::Widget {
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .build();
    picture.set_size_request(106, 106);
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
        ctx.state.on(move |c| {
            if c == Change::Preview
                && let Some(r) = weak.upgrade()
            {
                r()
            }
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

fn appearance_group(ctx: &C, keys: Vec<u8>) -> adw::PreferencesGroup {
    let state = ctx.state.clone();
    let group = adw::PreferencesGroup::builder().title("Appearance").build();
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
    group.add(&label_row);

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
    group.add(&show);

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
    group.add(&position);

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
    group
}

// ---------------------------------------------------------------------------------------------
// Several keys

fn multi_view(ctx: &C, content: &gtk::Box, keys: Vec<u8>) {
    content.append(&title_block(
        &format!("{} Keys Selected", keys.len()),
        "Changes apply to all of them",
    ));
    content.append(&appearance_group(ctx, keys.clone()));

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
    content.append(&group);

    let clear = gtk::Button::builder()
        .label(format!("Clear {} Keys", keys.len()))
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    {
        let ctx = ctx.clone();
        clear.connect_clicked(move |_| ctx.commit(How::Structure, |e| e.clear(&keys)));
    }
    content.append(&clear);
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
    let spec = action.and_then(catalog::spec_for);
    let card = adw::ActionRow::builder()
        .title(spec.map(|s| s.title).unwrap_or("No Action"))
        .subtitle(
            action
                .map(catalog::describe)
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
            let card = card.clone();
            Rc::new(move |a: Action, how: How| {
                card.set_subtitle(&catalog::describe(&a));
                write(Some(a), how, false);
            })
        };
        rows.extend(action_editor::rows_for(state, path, a, on));
    }
    rows
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
        .subtitle(action.map(catalog::describe).unwrap_or_else(|| "Nothing".into()))
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
                let (exp, write) = (exp.clone(), write.clone());
                Rc::new(move |a: Action, how: How| {
                    exp.set_subtitle(&catalog::describe(&a));
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

fn dial_view(ctx: &C, content: &gtk::Box, d: u8) {
    content.append(&title_block(
        &format!("Dial {}", d + 1),
        &format!("{} · turn it, press it, or hold it down", page_name(&ctx.state)),
    ));
    let dial = ctx.state.editor.borrow().dials()[d as usize].clone();
    let group = adw::PreferencesGroup::new();
    type Pick = fn(&mut Dial) -> &mut Option<Action>;
    let zones: [(&str, Option<&Action>, Pick); 4] = [
        ("Turn Left", dial.left.as_ref(), |d| &mut d.left),
        ("Press", dial.press.as_ref(), |d| &mut d.press),
        ("Turn Right", dial.right.as_ref(), |d| &mut d.right),
        ("Hold", dial.hold.as_ref(), |d| &mut d.hold),
    ];
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
    content.append(&group);

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
    content.append(&all);
}

fn small_view(ctx: &C, content: &gtk::Box, s: u8) {
    let side = if s == 0 { "Left" } else { "Right" };
    content.append(&title_block(
        &format!("{side} Button"),
        "The small button beside the dials",
    ));
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
    content.append(&group);
}

/// The screen's display choices, in combo order.
const SCREEN_MODES: [&str; 8] = [
    "Clock",
    "Clock, Date and Weekday",
    "Clock and Weekday",
    "Clock and Date",
    "Large Clock",
    "System Stats",
    "Now Playing",
    "Blank",
];

fn screen_mode_index(s: &ScreenConfig) -> usize {
    match s {
        ScreenConfig::FirmwareClock { style } => ClockStyle::ALL.iter().position(|c| c == style).unwrap_or(0),
        ScreenConfig::FirmwareStats => 5,
        ScreenConfig::Custom { widgets, .. } => match widgets.first() {
            Some(Widget::NowPlaying) => 6,
            _ => 7,
        },
    }
}

fn screen_for_mode(i: usize, old: &ScreenConfig) -> ScreenConfig {
    let (tap, hold) = match old {
        ScreenConfig::Custom { tap, hold, .. } => (tap.clone(), hold.clone()),
        _ => (None, None),
    };
    match i {
        0..=4 => ScreenConfig::FirmwareClock {
            style: ClockStyle::ALL[i],
        },
        5 => ScreenConfig::FirmwareStats,
        6 => ScreenConfig::Custom {
            widgets: vec![Widget::NowPlaying],
            tap,
            hold,
        },
        _ => ScreenConfig::Custom {
            widgets: vec![Widget::Blank],
            tap,
            hold,
        },
    }
}

fn screen_view(ctx: &C, content: &gtk::Box) {
    content.append(&title_block(
        "Wide Screen",
        "The large display at the bottom right",
    ));
    let screen = ctx.state.editor.borrow().profile().screen.clone();
    let group = adw::PreferencesGroup::builder().title("Shows").build();
    let mode = adw::ComboRow::builder()
        .title("Display")
        .model(&gtk::StringList::new(&SCREEN_MODES))
        .selected(screen_mode_index(&screen) as u32)
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
    content.append(&group);

    match &screen {
        ScreenConfig::Custom { tap, hold, .. } => {
            let group = adw::PreferencesGroup::builder().title("When Pressed").build();
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
            content.append(&group);
        }
        _ => {
            content.append(
                &gtk::Label::builder()
                    .label(
                        "The deck draws the clock and stats itself, so the screen can’t be \
                         pressed in these modes. Choose Now Playing or Blank to give it an action.",
                    )
                    .wrap(true)
                    .xalign(0.0)
                    .css_classes(["dim-label", "caption"])
                    .build(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_modes_round_trip() {
        let old = ScreenConfig::Custom {
            widgets: vec![Widget::NowPlaying],
            tap: Some(Action::NextPage),
            hold: None,
        };
        for i in 0..SCREEN_MODES.len() {
            assert_eq!(screen_mode_index(&screen_for_mode(i, &old)), i);
        }
        // Switching between custom modes keeps the tap action.
        assert!(matches!(
            screen_for_mode(7, &old),
            ScreenConfig::Custom {
                tap: Some(Action::NextPage),
                ..
            }
        ));
    }
}
