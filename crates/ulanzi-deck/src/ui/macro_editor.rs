//! The macro editor: name, type, steps, recording and key feedback.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::catalog::describe;
use ulanzi_core::macro_record::MacroRecorder;
use ulanzi_core::model::{
    Action, Feedback, ImageSource, LookOverride, Macro, MacroEvent, MacroStep, MouseButton, PressAgain,
    Release, SequencePart, Trigger,
};
use uuid::Uuid;

use crate::runtime;
use crate::state::State;
use crate::ui::action_editor::{How, OnChange, base_key_name, picker_button, rows_for, sendable_alias};
use crate::ui::capture::Capture;
use crate::ui::macro_diagram::Diagram;
use crate::ui::macro_steps::{
    Chip, Direction, RECORD_HINT, chips, held_warning, move_chips, remove_chip, remove_selected, step_count,
    trigger_sentence,
};
use crate::ui::wrap_box::WrapBox;

/// Seconds before Try It plays, to click into the target app.
const TRY_COUNTDOWN: u32 = 3;
const DEFAULT_STANDARD_DELAY: u32 = 50;

/// What the editor opens on.
#[derive(Clone)]
pub enum MacroTarget {
    New,
    Library(Uuid),
    /// A macro action wherever it sits (a key's press or hold, a Cycle step): its own copy, its
    /// library macro, or a new one. `save` writes the action back.
    Action {
        action: Action,
        save: OnChange,
    },
}

/// What Done does.
#[derive(Clone)]
enum Saving {
    /// Into the library, and with `link`, the action follows it.
    Library(Option<OnChange>),
    /// Into the action's own copy.
    Own(OnChange),
}

fn write_action(save: &OnChange, id: Uuid, own: Option<Macro>) {
    save(
        Action::Macro {
            id,
            own: own.map(Box::new),
        },
        How::Structure,
    );
}

fn load(state: &State, target: MacroTarget) -> (Macro, Saving) {
    let library = |id: Uuid| state.editor.borrow().library().get(id).cloned();
    let new = || Macro::new("New Macro");
    match target {
        MacroTarget::New => (new(), Saving::Library(None)),
        MacroTarget::Library(id) => (library(id).unwrap_or_else(new), Saving::Library(None)),
        MacroTarget::Action { action, save } => match action {
            Action::Macro { own: Some(m), .. } => (*m, Saving::Own(save)),
            Action::Macro { id, own: None } => (library(id).unwrap_or_else(new), Saving::Library(Some(save))),
            _ => (new(), Saving::Library(Some(save))),
        },
    }
}

/// What to run after the macro changes.
type Redraw = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// Everything the editor's parts share.
#[derive(Clone)]
struct Ed {
    state: State,
    m: Rc<RefCell<Macro>>,
    window: adw::Window,
    steps: gtk::Box,
    warning: gtk::Label,
    redraw: Redraw,
    /// The macro as it was opened, to know whether closing loses anything.
    original: Rc<RefCell<Macro>>,
    /// Ends a recording in progress, keeping its steps.
    stop_recording: Redraw,
    /// How many keys in every profile follow this macro.
    users: Rc<Cell<u32>>,
    /// Try It's countdown or playback is running.
    trying: Rc<Cell<bool>>,
    /// Goes up with each Try It or stop, so the timers of an older one know to give up.
    try_run: Rc<Cell<u32>>,
    /// The first step of each selected chip.
    selected: Rc<RefCell<BTreeSet<usize>>>,
    /// The chip a Shift+click range starts from.
    anchor: Rc<Cell<Option<(SequencePart, usize)>>>,
    /// The profile and page this was opened on: an action is written back there.
    opened_on: (Uuid, usize),
}

impl Ed {
    /// Close without asking: everything is saved.
    fn close_saved(&self) {
        *self.original.borrow_mut() = self.m.borrow().clone();
        self.window.close();
    }

    /// Everything the editor's widgets hold of each other, so they go when the window does.
    fn let_go(&self) {
        *self.redraw.borrow_mut() = None;
        *self.stop_recording.borrow_mut() = None;
        while let Some(child) = self.steps.first_child() {
            self.steps.remove(&child);
        }
    }

    fn changed(&self) {
        let draw = self.redraw.borrow().clone();
        if let Some(draw) = draw {
            draw();
        }
    }

    /// Whether this is still the page the action sits on.
    fn in_place(&self) -> bool {
        let page = self.state.editor.borrow().page();
        if (self.state.profile_id(), page) == self.opened_on {
            return true;
        }
        crate::ui::toast(&self.state, "Go back to the page this key is on to save it");
        false
    }

    fn end_recording(&self) {
        let stop = self.stop_recording.borrow().clone();
        if let Some(stop) = stop {
            stop();
        }
    }

    /// A popover on a chip: gone when closed, then the steps are redrawn, which removes its parent.
    fn tidy(&self, popover: &gtk::Popover) {
        let ed = self.clone();
        popover.connect_closed(move |p| {
            let (ed, p) = (ed.clone(), p.clone());
            glib::idle_add_local_once(move || {
                p.unparent();
                ed.changed();
            });
        });
    }

    /// Which chips a macro of this type shows rows for.
    fn parts(&self) -> &'static [SequencePart] {
        if self.m.borrow().trigger == Trigger::Sequence {
            &[SequencePart::Press, SequencePart::Held, SequencePart::ReleaseKey]
        } else {
            &[SequencePart::Press]
        }
    }

    /// Redraw for a new selection: later, because a click on a chip is still going on.
    fn selection_changed(&self) {
        let ed = self.clone();
        glib::idle_add_local_once(move || ed.changed());
    }

    fn select_all(&self) {
        let m = self.m.borrow();
        let firsts = self
            .parts()
            .iter()
            .flat_map(|part| chips(&m, *part))
            .filter_map(|c| c.steps.first().copied());
        self.selected.borrow_mut().extend(firsts);
        self.selection_changed();
    }

    fn deselect(&self) {
        self.selected.borrow_mut().clear();
        self.selection_changed();
    }

    fn delete_selected(&self) {
        let selected = self.selected.borrow().clone();
        let parts = self.parts();
        self.edit(|m| remove_selected(m, parts, &selected));
    }

    fn edit(&self, f: impl FnOnce(&mut Macro)) {
        f(&mut self.m.borrow_mut());
        self.selected.borrow_mut().clear();
        self.anchor.set(None);
        self.changed();
    }
}

fn part_title(part: SequencePart) -> &'static str {
    match part {
        SequencePart::Press => "Press",
        SequencePart::Held => "While Held",
        SequencePart::ReleaseKey => "Release",
    }
}

fn new_step(event: MacroEvent, part: SequencePart) -> MacroStep {
    MacroStep {
        event,
        delay_ms: DEFAULT_STANDARD_DELAY,
        look: LookOverride::default(),
        part,
    }
}

/// Which macros can go in a Run Macro step: not this one, nor one that runs it.
fn runnable(state: &State, m: &Macro) -> Vec<Macro> {
    let e = state.editor.borrow();
    let library = e.library();
    library
        .macros
        .iter()
        .filter(|other| other.id != m.id)
        .filter(|other| {
            let mut test = m.clone();
            test.steps.push(new_step(
                MacroEvent::RunMacro { id: other.id },
                SequencePart::Press,
            ));
            !library.would_loop(&test)
        })
        .cloned()
        .collect()
}

fn ask_text(ed: &Ed, heading: &str, value: &str, done: impl Fn(String) + 'static) {
    let entry = gtk::Entry::builder().text(value).activates_default(true).build();
    let alert = adw::AlertDialog::builder()
        .heading(heading)
        .extra_child(&entry)
        .build();
    alert.add_response("cancel", "Cancel");
    alert.add_response("ok", "OK");
    alert.set_default_response(Some("ok"));
    alert.set_close_response("cancel");
    alert.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    alert.connect_response(Some("ok"), move |_, _| done(entry.text().to_string()));
    alert.present(Some(&ed.window));
}

fn ask_number(ed: &Ed, heading: &str, value: f64, max: f64, done: impl Fn(i32) + 'static) {
    let spin = gtk::SpinButton::with_range(-max, max, 1.0);
    spin.set_value(value);
    let alert = adw::AlertDialog::builder()
        .heading(heading)
        .extra_child(&spin)
        .build();
    alert.add_response("cancel", "Cancel");
    alert.add_response("ok", "OK");
    alert.set_default_response(Some("ok"));
    alert.set_close_response("cancel");
    alert.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    alert.connect_response(Some("ok"), move |_, _| done(spin.value() as i32));
    alert.present(Some(&ed.window));
}

fn ask_pick(ed: &Ed, heading: &str, names: Vec<String>, done: impl Fn(usize) + 'static) {
    let list = gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let drop = gtk::DropDown::builder()
        .model(&list)
        .enable_search(true)
        .expression(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<&gtk::Expression>,
            "string",
        ))
        .build();
    let alert = adw::AlertDialog::builder()
        .heading(heading)
        .extra_child(&drop)
        .build();
    alert.add_response("cancel", "Cancel");
    alert.add_response("ok", "OK");
    alert.set_default_response(Some("ok"));
    alert.set_close_response("cancel");
    alert.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    alert.connect_response(Some("ok"), move |_, _| done(drop.selected() as usize));
    alert.present(Some(&ed.window));
}

/// The **+** menu of one part.
fn add_menu(ed: &Ed, part: SequencePart) -> gtk::Widget {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let popover = gtk::Popover::builder().child(&list).build();
    let button = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add a step")
        .popover(&popover)
        .build();
    let item = |label: &str, f: Box<dyn Fn()>| {
        let b = gtk::Button::builder().label(label).css_classes(["flat"]).build();
        let p = popover.clone();
        b.connect_clicked(move |_| {
            p.popdown();
            f();
        });
        list.append(&b);
    };
    let add = |events: Vec<MacroEvent>| {
        let ed = ed.clone();
        move || {
            let steps: Vec<MacroStep> = events.iter().map(|e| new_step(e.clone(), part)).collect();
            ed.edit(|m| m.steps.extend(steps));
        }
    };
    {
        let ed = ed.clone();
        item(
            "Key…",
            Box::new(move || {
                let names = ulanzi_core::keys::all_sendable();
                let (ed2, shown) = (ed.clone(), names.clone());
                ask_pick(&ed, "Choose a Key", shown, move |i| {
                    if let Some(key) = names.get(i).cloned() {
                        ed2.edit(|m| {
                            m.steps
                                .push(new_step(MacroEvent::KeyDown { key: key.clone() }, part));
                            m.steps.push(new_step(MacroEvent::KeyUp { key }, part));
                        });
                    }
                });
            }),
        );
    }
    {
        let ed = ed.clone();
        item(
            "Text…",
            Box::new(move || {
                let ed2 = ed.clone();
                ask_text(&ed, "Type This Text", "", move |text| {
                    ed2.edit(|m| m.steps.push(new_step(MacroEvent::TypeText { text }, part)));
                });
            }),
        );
    }
    for (label, button) in [
        ("Left Click", MouseButton::Left),
        ("Right Click", MouseButton::Right),
        ("Middle Click", MouseButton::Middle),
    ] {
        let click = add(vec![
            MacroEvent::MouseDown { button },
            MacroEvent::MouseUp { button },
        ]);
        item(label, Box::new(click));
    }
    item(
        "Scroll Up",
        Box::new(add(vec![MacroEvent::Scroll { dx: 0, dy: -1 }])),
    );
    item(
        "Scroll Down",
        Box::new(add(vec![MacroEvent::Scroll { dx: 0, dy: 1 }])),
    );
    {
        let ed = ed.clone();
        item(
            "Move Pointer…",
            Box::new(move || {
                let ed2 = ed.clone();
                ask_number(&ed, "Move Right, in Pixels", 0.0, 10000.0, move |dx| {
                    let ed3 = ed2.clone();
                    ask_number(&ed2, "Move Down, in Pixels", 0.0, 10000.0, move |dy| {
                        ed3.edit(|m| m.steps.push(new_step(MacroEvent::MoveBy { dx, dy }, part)));
                    });
                });
            }),
        );
    }
    {
        let ed = ed.clone();
        item(
            "Pick a Spot…",
            Box::new(move || {
                let ed2 = ed.clone();
                crate::ui::pick_spot::open(&ed.state, move |x, y| {
                    ed2.edit(|m| m.steps.push(new_step(MacroEvent::MoveTo { x, y }, part)));
                });
            }),
        );
    }
    item("Wait", Box::new(add(vec![MacroEvent::Wait { ms: 500 }])));
    {
        let ed = ed.clone();
        item(
            "Run Macro…",
            Box::new(move || {
                let choices = runnable(&ed.state, &ed.m.borrow());
                if choices.is_empty() {
                    crate::ui::toast(&ed.state, "No other macro to run");
                    return;
                }
                let names = choices.iter().map(|m| m.name.clone()).collect();
                let ed2 = ed.clone();
                ask_pick(&ed, "Run Which Macro?", names, move |i| {
                    if let Some(id) = choices.get(i).map(|m| m.id) {
                        ed2.edit(|m| m.steps.push(new_step(MacroEvent::RunMacro { id }, part)));
                    }
                });
            }),
        );
    }
    let holder = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    holder.append(&button);
    {
        let ed = ed.clone();
        let action = picker_button(&ed.state.clone(), "Add Action", false, false, move |action| {
            ed.edit(|m| m.steps.push(new_step(MacroEvent::Action { action }, part)));
        });
        holder.append(&action);
    }
    holder.upcast()
}

const MOUSE_BUTTONS: [(&str, MouseButton); 5] = [
    ("Left", MouseButton::Left),
    ("Right", MouseButton::Right),
    ("Middle", MouseButton::Middle),
    ("Back", MouseButton::Back),
    ("Forward", MouseButton::Forward),
];

/// A number typed into `spin` counts: Enter saves it and closes, and so does closing.
fn save_on_close(popover: &gtk::Popover, spin: &gtk::SpinButton) {
    {
        let popover = popover.downgrade();
        spin.connect_activate(move |_| {
            if let Some(p) = popover.upgrade() {
                p.popdown();
            }
        });
    }
    let spin = spin.downgrade();
    popover.connect_closed(move |_| {
        if let Some(s) = spin.upgrade() {
            s.update();
        }
    });
}

/// A label and a whole number, reported as it changes.
fn number_row(
    popover: &gtk::Popover,
    label: &str,
    value: i32,
    max: i32,
    changed: impl Fn(i32) + 'static,
) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(8).build();
    row.append(
        &gtk::Label::builder()
            .label(label)
            .hexpand(true)
            .xalign(0.0)
            .build(),
    );
    let spin = gtk::SpinButton::with_range(-max as f64, max as f64, 1.0);
    spin.set_value(value as f64);
    spin.connect_value_changed(move |s| changed(s.value() as i32));
    save_on_close(popover, &spin);
    row.append(&spin);
    row.upcast()
}

/// One button for each key on a chip, to swap it for another.
fn key_choices(ed: &Ed, popover: &gtk::Popover, body: &gtk::Box, steps: &[usize]) {
    let mut seen: Vec<String> = Vec::new();
    {
        let m = ed.m.borrow();
        for step in steps.iter().filter_map(|i| m.steps.get(*i)) {
            if let MacroEvent::KeyDown { key } | MacroEvent::KeyUp { key } = &step.event
                && !seen.contains(key)
            {
                seen.push(key.clone());
            }
        }
    }
    for key in seen {
        let label = format!("Change {}…", ulanzi_core::catalog::pretty_combo(&key));
        let button = gtk::Button::with_label(&label);
        let (ed, popover, steps) = (ed.clone(), popover.clone(), steps.to_vec());
        button.connect_clicked(move |_| {
            popover.popdown();
            let names = ulanzi_core::keys::all_sendable();
            let (ed2, shown, old, steps) = (ed.clone(), names.clone(), key.clone(), steps.clone());
            ask_pick(&ed, "Choose a Key", shown, move |pick| {
                let Some(new) = names.get(pick).cloned() else {
                    return;
                };
                ed2.edit(|m| {
                    for i in &steps {
                        if let Some(MacroEvent::KeyDown { key } | MacroEvent::KeyUp { key }) =
                            m.steps.get_mut(*i).map(|step| &mut step.event)
                            && *key == old
                        {
                            *key = new.clone();
                        }
                    }
                });
            });
        });
        body.append(&button);
    }
}

/// A step's action, with how it's named on its chip.
fn action_text(action: &Action) -> String {
    if let Action::LaunchApp {
        desktop_id: Some(id), ..
    } = action
        && let Some(app) = crate::ui::action_editor::installed_apps()
            .iter()
            .find(|a| a.desktop_id == *id)
    {
        return format!("Launch {}", app.name);
    }
    describe(action)
}

/// The controls of step `i`'s action. They come back when the action changes, so a choice shows
/// straight away, on the chip behind them too.
fn fill_action(ed: &Ed, i: usize, group: &gtk::ListBox, title: &gtk::Label, chip: &gtk::Button) {
    let Some(MacroEvent::Action { action }) = ed.m.borrow().steps.get(i).map(|s| s.event.clone()) else {
        return;
    };
    title.set_label(&action_text(&action));
    while let Some(row) = group.first_child() {
        group.remove(&row);
    }
    let on: OnChange = {
        let (ed, group, title, chip) = (ed.clone(), group.clone(), title.clone(), chip.clone());
        Rc::new(move |action: Action, how: How| {
            if let Some(step) = ed.m.borrow_mut().steps.get_mut(i) {
                step.event = MacroEvent::Action { action };
            }
            let (ed, group, title, chip) = (ed.clone(), group.clone(), title.clone(), chip.clone());
            glib::idle_add_local_once(move || {
                if how == How::Structure {
                    fill_action(&ed, i, &group, &title, &chip);
                }
                let text = ed.m.borrow().steps.get(i).and_then(|s| match &s.event {
                    MacroEvent::Action { action } => Some(action_text(action)),
                    _ => None,
                });
                if let Some(text) = text {
                    title.set_label(&text);
                    chip.set_child(Some(&gtk::Label::new(Some(&text))));
                }
            });
        })
    };
    let path = format!("macro/{}/{i}", ed.m.borrow().id);
    for row in rows_for(&ed.state, &path, &action, on) {
        group.append(&row);
    }
}

/// What opens when a step is clicked: its values, its look with Custom feedback, and Delete.
fn step_popover(ed: &Ed, button: &gtk::Button, chip: &Chip, index: usize, part: SequencePart) {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .margin_bottom(6)
        .width_request(260)
        .build();
    let popover = gtk::Popover::builder().child(&body).build();
    popover.set_parent(button);
    ed.tidy(&popover);
    let first = chip.steps.first().copied();
    let m = ed.m.borrow().clone();
    if let Some(i) = first {
        match m.steps.get(i).map(|s| s.event.clone()) {
            Some(MacroEvent::TypeText { text }) => {
                let entry = gtk::Entry::builder().text(text).build();
                {
                    let popover = popover.downgrade();
                    entry.connect_activate(move |_| {
                        if let Some(p) = popover.upgrade() {
                            p.popdown();
                        }
                    });
                }
                let ed2 = ed.clone();
                entry.connect_changed(move |e| {
                    let text = e.text().to_string();
                    if let Some(s) = ed2.m.borrow_mut().steps.get_mut(i) {
                        s.event = MacroEvent::TypeText { text };
                    }
                });
                body.append(&entry);
            }
            Some(MacroEvent::Wait { ms }) => {
                let spin = gtk::SpinButton::with_range(0.0, 600_000.0, 10.0);
                spin.set_value(ms as f64);
                save_on_close(&popover, &spin);
                let ed2 = ed.clone();
                spin.connect_value_changed(move |s| {
                    let ms = s.value() as u32;
                    if let Some(step) = ed2.m.borrow_mut().steps.get_mut(i) {
                        step.event = MacroEvent::Wait { ms };
                    }
                });
                body.append(&spin);
            }
            Some(MacroEvent::Action { .. }) => {
                let title = gtk::Label::new(None);
                let group = gtk::ListBox::builder().css_classes(["boxed-list"]).build();
                body.append(&title);
                body.append(&group);
                let label = button.clone();
                fill_action(ed, i, &group, &title, &label);
            }
            Some(MacroEvent::KeyDown { .. } | MacroEvent::KeyUp { .. }) => {
                key_choices(ed, &popover, &body, &chip.steps);
            }
            Some(MacroEvent::MouseDown { button: current } | MacroEvent::MouseUp { button: current }) => {
                let names = MOUSE_BUTTONS.map(|(name, _)| name);
                let drop = gtk::DropDown::from_strings(&names);
                let at = MOUSE_BUTTONS.iter().position(|(_, b)| *b == current);
                drop.set_selected(at.unwrap_or(0) as u32);
                let (ed2, steps) = (ed.clone(), chip.steps.clone());
                drop.connect_selected_notify(move |d| {
                    let Some((_, button)) = MOUSE_BUTTONS.get(d.selected() as usize).copied() else {
                        return;
                    };
                    let mut m = ed2.m.borrow_mut();
                    for i in &steps {
                        if let Some(MacroEvent::MouseDown { button: b } | MacroEvent::MouseUp { button: b }) =
                            m.steps.get_mut(*i).map(|step| &mut step.event)
                        {
                            *b = button;
                        }
                    }
                });
                body.append(&drop);
            }
            Some(MacroEvent::Scroll { dx, dy }) => {
                let set = |ed: &Ed, across: bool| {
                    let ed = ed.clone();
                    move |value: i32| {
                        if let Some(MacroEvent::Scroll { dx, dy }) =
                            ed.m.borrow_mut().steps.get_mut(i).map(|s| &mut s.event)
                        {
                            *(if across { dx } else { dy }) = value;
                        }
                    }
                };
                body.append(&number_row(
                    &popover,
                    "Down (minus is up)",
                    dy,
                    100,
                    set(ed, false),
                ));
                body.append(&number_row(
                    &popover,
                    "Right (minus is left)",
                    dx,
                    100,
                    set(ed, true),
                ));
            }
            Some(MacroEvent::MoveBy { dx, dy }) => {
                let set = |ed: &Ed, across: bool| {
                    let ed = ed.clone();
                    move |value: i32| {
                        if let Some(MacroEvent::MoveBy { dx, dy }) =
                            ed.m.borrow_mut().steps.get_mut(i).map(|s| &mut s.event)
                        {
                            *(if across { dx } else { dy }) = value;
                        }
                    }
                };
                body.append(&number_row(
                    &popover,
                    "Right (minus is left)",
                    dx,
                    10_000,
                    set(ed, true),
                ));
                body.append(&number_row(
                    &popover,
                    "Down (minus is up)",
                    dy,
                    10_000,
                    set(ed, false),
                ));
            }
            Some(MacroEvent::MoveTo { .. }) => {
                let again = gtk::Button::with_label("Pick the Spot Again…");
                let (ed2, popover2) = (ed.clone(), popover.clone());
                again.connect_clicked(move |_| {
                    popover2.popdown();
                    let ed3 = ed2.clone();
                    crate::ui::pick_spot::open(&ed2.state, move |x, y| {
                        if let Some(step) = ed3.m.borrow_mut().steps.get_mut(i) {
                            step.event = MacroEvent::MoveTo { x, y };
                        }
                        ed3.changed();
                    });
                });
                body.append(&again);
            }
            Some(MacroEvent::RunMacro { .. }) => {
                let again = gtk::Button::with_label("Run a Different Macro…");
                let (ed2, popover2) = (ed.clone(), popover.clone());
                again.connect_clicked(move |_| {
                    popover2.popdown();
                    let choices = runnable(&ed2.state, &ed2.m.borrow());
                    if choices.is_empty() {
                        crate::ui::toast(&ed2.state, "No other macro to run");
                        return;
                    }
                    let names = choices.iter().map(|m| m.name.clone()).collect();
                    let ed3 = ed2.clone();
                    ask_pick(&ed2, "Run Which Macro?", names, move |pick| {
                        if let Some(id) = choices.get(pick).map(|m| m.id) {
                            ed3.edit(|m| {
                                if let Some(step) = m.steps.get_mut(i) {
                                    step.event = MacroEvent::RunMacro { id };
                                }
                            });
                        }
                    });
                });
                body.append(&again);
            }
            _ => {}
        }
        if m.feedback == Feedback::Custom {
            let label = gtk::Entry::builder()
                .placeholder_text("Label during this step")
                .build();
            if let Some(text) = m.steps.get(i).and_then(|s| s.look.label.clone()) {
                label.set_text(&text);
            }
            let ed2 = ed.clone();
            label.connect_changed(move |e| {
                let text = e.text().to_string();
                if let Some(step) = ed2.m.borrow_mut().steps.get_mut(i) {
                    step.look.label = (!text.is_empty()).then_some(text);
                }
            });
            body.append(&label);
            let image = gtk::Button::with_label("Image During This Step…");
            let ed2 = ed.clone();
            image.connect_clicked(move |b| {
                let ed3 = ed2.clone();
                let current: Option<ImageSource> =
                    ed2.m.borrow().steps.get(i).and_then(|s| s.look.source.clone());
                crate::ui::icon_picker::choose(&ed2.state, b, current, move |source| {
                    if let Some(step) = ed3.m.borrow_mut().steps.get_mut(i) {
                        step.look.source = Some(source);
                    }
                });
            });
            body.append(&image);
        }
    }
    let delete = gtk::Button::builder()
        .label("Delete")
        .css_classes(["destructive-action"])
        .build();
    {
        let (ed, popover) = (ed.clone(), popover.clone());
        delete.connect_clicked(move |_| {
            popover.popdown();
            ed.edit(|m| remove_chip(m, part, index));
        });
    }
    body.append(&delete);
    popover.popup();
}

fn delay_popover(ed: &Ed, button: &gtk::Button, step: usize, ms: u32) {
    let spin = gtk::SpinButton::with_range(0.0, 600_000.0, 1.0);
    spin.set_value(ms as f64);
    let popover = gtk::Popover::builder().child(&spin).build();
    popover.set_parent(button);
    ed.tidy(&popover);
    save_on_close(&popover, &spin);
    let ed2 = ed.clone();
    spin.connect_value_changed(move |s| {
        if let Some(step) = ed2.m.borrow_mut().steps.get_mut(step) {
            step.delay_ms = s.value() as u32;
        }
    });
    popover.popup();
}

/// Shown when pointing at a step.
const STEP_TIPS: &str = "Click to edit, drag to move\nCtrl+click to select, Shift+click to select a range\nCtrl+A selects every step";

const PART_STRIDE: u32 = 100_000;

fn drag_code(part: SequencePart, index: usize) -> u32 {
    let row = match part {
        SequencePart::Press => 0,
        SequencePart::Held => 1,
        SequencePart::ReleaseKey => 2,
    };
    row * PART_STRIDE + index as u32
}

/// The chip a drag carries, when it comes from `part`.
fn drag_index(code: u32, part: SequencePart) -> Option<usize> {
    let row = code / PART_STRIDE;
    (drag_code(part, 0) / PART_STRIDE == row).then_some((code % PART_STRIDE) as usize)
}

/// One step chip: click to edit, drag to reorder.
/// "Steps", how selecting works, and buttons for all of them.
fn steps_heading(ed: &Ed) -> gtk::Widget {
    let heading = gtk::Box::builder().spacing(6).build();
    heading.append(
        &gtk::Label::builder()
            .label("Steps")
            .css_classes(["dim-label", "caption"])
            .build(),
    );
    heading.append(
        &gtk::Image::builder()
            .icon_name("dialog-information-symbolic")
            .pixel_size(14)
            .css_classes(["dim-label"])
            .tooltip_text(STEP_TIPS)
            .build(),
    );
    let all = gtk::Button::builder()
        .label("Select All")
        .hexpand(true)
        .halign(gtk::Align::End)
        .css_classes(["flat", "macro-delay"])
        .tooltip_text("Ctrl+A")
        .build();
    {
        let ed = ed.clone();
        all.connect_clicked(move |_| ed.select_all());
    }
    heading.append(&all);
    let clear = gtk::Button::builder()
        .label("Clear All")
        .css_classes(["flat", "macro-delay"])
        .build();
    {
        let ed = ed.clone();
        clear.connect_clicked(move |_| confirm_clear(&ed));
    }
    heading.append(&clear);
    heading.upcast()
}

/// Clear All, after asking.
fn confirm_clear(ed: &Ed) {
    let count = step_count(&ed.m.borrow());
    if ed.m.borrow().steps.is_empty() {
        return;
    }
    let alert = adw::AlertDialog::builder()
        .heading("Clear All Steps?")
        .body(format!(
            "This removes {count}. Closing the window without saving brings them back."
        ))
        .close_response("cancel")
        .build();
    alert.add_response("cancel", "Cancel");
    alert.add_response("clear", "Clear All");
    alert.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
    let ed2 = ed.clone();
    alert.connect_response(Some("clear"), move |_, _| ed2.edit(|m| m.steps.clear()));
    alert.present(Some(&ed.window));
}

/// The app a Launch App chip opens: its id and name.
fn app_of(ed: &Ed, chip: &Chip) -> Option<(String, String)> {
    let m = ed.m.borrow();
    let step = m.steps.get(*chip.steps.first()?)?;
    let MacroEvent::Action {
        action: Action::LaunchApp {
            desktop_id: Some(id), ..
        },
    } = &step.event
    else {
        return None;
    };
    let apps = crate::ui::action_editor::installed_apps();
    let app = apps.iter().find(|a| a.desktop_id == *id)?;
    Some((id.clone(), app.name.clone()))
}

fn chip_widget(ed: &Ed, chip: &Chip, index: usize, part: SequencePart) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let app = app_of(ed, chip);
    match &app {
        Some((id, name)) => {
            let text = format!("Launch {name}");
            content.append(&crate::ui::app_picker::app_label(Some(id), &text, 16));
        }
        None => content.append(&gtk::Image::from_icon_name(chip.icon)),
    }
    let arrow = match chip.direction {
        Direction::Down => Some("go-down-symbolic"),
        Direction::Up => Some("go-up-symbolic"),
        Direction::Both => None,
    };
    if let Some(arrow) = arrow {
        content.append(&gtk::Image::from_icon_name(arrow));
    }
    if app.is_none() {
        content.append(&gtk::Label::new(Some(&chip.label)));
    }
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["macro-step"])
        .tooltip_text(STEP_TIPS)
        .build();
    let first = chip.steps.first().copied().unwrap_or_default();
    if ed.selected.borrow().contains(&first) {
        button.add_css_class("selected");
    }
    {
        let (ed, chip) = (ed.clone(), chip.clone());
        button.connect_clicked(move |b| step_popover(&ed, b, &chip, index, part));
    }
    // Ctrl+click and Shift+click select instead of opening the step.
    let select = gtk::GestureClick::new();
    select.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let ed = ed.clone();
        select.connect_pressed(move |g, _, _, _| {
            let held = g.current_event_state();
            let (ctrl, shift) = (
                held.contains(gdk::ModifierType::CONTROL_MASK),
                held.contains(gdk::ModifierType::SHIFT_MASK),
            );
            if !ctrl && !shift {
                ed.anchor.set(Some((part, index)));
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            let from = match ed.anchor.get() {
                Some((p, i)) if shift && p == part => i,
                _ => index,
            };
            if shift {
                let list = chips(&ed.m.borrow(), part);
                let range = from.min(index)..=from.max(index);
                let firsts = range.filter_map(|i| list.get(i)?.steps.first().copied());
                ed.selected.borrow_mut().extend(firsts);
            } else {
                let mut selected = ed.selected.borrow_mut();
                if !selected.remove(&first) {
                    selected.insert(first);
                }
            }
            if !shift {
                ed.anchor.set(Some((part, index)));
            }
            ed.selection_changed();
        });
    }
    button.add_controller(select);
    let drag = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
    let code = drag_code(part, index);
    drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&code.to_value())));
    button.add_controller(drag);
    let drop = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
    {
        let ed = ed.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Ok(code) = value.get::<u32>() else {
                return false;
            };
            // A step keeps to its row of a Sequence: which part it plays in is what it means.
            let Some(from) = drag_index(code, part) else {
                return false;
            };
            // A selected chip brings the rest of the selection with it.
            let group = dragged_with(&ed, part, from);
            ed.edit(|m| move_chips(m, part, &group, index));
            true
        });
    }
    button.add_controller(drop);
    button.upcast()
}

/// The chips that move with chip `from`: the whole selection if it is part of one.
fn dragged_with(ed: &Ed, part: SequencePart, from: usize) -> Vec<usize> {
    let list = chips(&ed.m.borrow(), part);
    let selected = ed.selected.borrow();
    let chosen = |i: &usize| {
        list.get(*i)
            .and_then(|c| c.steps.first())
            .is_some_and(|first| selected.contains(first))
    };
    if chosen(&from) {
        (0..list.len()).filter(chosen).collect()
    } else {
        vec![from]
    }
}

fn draw_steps(ed: &Ed) {
    while let Some(child) = ed.steps.first_child() {
        ed.steps.remove(&child);
    }
    let m = ed.m.borrow().clone();
    for part in ed.parts() {
        if m.trigger == Trigger::Sequence {
            ed.steps.append(
                &gtk::Label::builder()
                    .label(part_title(*part))
                    .css_classes(["dim-label", "caption"])
                    .xalign(0.0)
                    .build(),
            );
        }
        let list = chips(&m, *part);
        if list.is_empty() && m.trigger != Trigger::Sequence {
            ed.steps.append(
                &gtk::Label::builder()
                    .label("No steps yet. Press Record, or add one here.")
                    .css_classes(["dim-label"])
                    .xalign(0.0)
                    .build(),
            );
        }
        let flow = WrapBox::default();
        for (i, chip) in list.iter().enumerate() {
            flow.append(&chip_widget(ed, chip, i, *part));
            if let (Some(ms), Some(step)) = (chip.delay, chip.steps.last().copied()) {
                let b = gtk::Button::builder()
                    .label(format!("{ms} ms"))
                    .valign(gtk::Align::Center)
                    .css_classes(["flat", "macro-delay"])
                    .build();
                let ed2 = ed.clone();
                b.connect_clicked(move |b| delay_popover(&ed2, b, step, ms));
                flow.append(&b);
            }
        }
        flow.append(&add_menu(ed, *part));
        ed.steps.append(&flow);
    }
    let count = ed.selected.borrow().len();
    if count > 0 {
        let bar = gtk::Box::builder().spacing(6).build();
        bar.append(&gtk::Label::new(Some(&format!("{count} selected"))));
        let delete = gtk::Button::builder()
            .label("Delete")
            .css_classes(["destructive-action"])
            .build();
        {
            let ed = ed.clone();
            delete.connect_clicked(move |_| ed.delete_selected());
        }
        bar.append(&delete);
        let clear = gtk::Button::builder()
            .label("Deselect")
            .css_classes(["flat"])
            .build();
        {
            let ed = ed.clone();
            clear.connect_clicked(move |_| ed.deselect());
        }
        bar.append(&clear);
        ed.steps.append(&bar);
    }
    match held_warning(&m) {
        Some(w) => {
            ed.warning.set_label(&w);
            ed.warning.set_visible(true);
        }
        None => ed.warning.set_visible(false),
    }
}

fn trigger_cards(ed: &Ed) -> (gtk::Widget, gtk::Label) {
    let cards = gtk::Box::builder().spacing(6).homogeneous(true).build();
    let sentence = gtk::Label::builder()
        .css_classes(["dim-label", "caption"])
        .xalign(0.0)
        .wrap(true)
        .build();
    let current = ed.m.borrow().trigger;
    sentence.set_label(trigger_sentence(&current));
    let diagram = Diagram::new(current);
    let options = [
        ("Play Once", Trigger::default()),
        (
            "While Holding",
            Trigger::WhileHeld {
                on_release: Release::FinishPass,
            },
        ),
        ("Toggle", Trigger::Toggle),
        ("Sequence", Trigger::Sequence),
    ];
    let same = |a: &Trigger, b: &Trigger| std::mem::discriminant(a) == std::mem::discriminant(b);
    let mut first: Option<gtk::ToggleButton> = None;
    for (name, trigger) in options {
        let card = gtk::ToggleButton::builder()
            .label(name)
            .active(same(&current, &trigger))
            .css_classes(["flat", "macro-type"])
            .build();
        if let Some(f) = &first {
            card.set_group(Some(f));
        } else {
            first = Some(card.clone());
        }
        let (ed2, sentence2) = (ed.clone(), sentence.clone());
        card.connect_toggled(move |c| {
            if c.is_active() && !same(&ed2.m.borrow().trigger, &trigger) {
                sentence2.set_label(trigger_sentence(&trigger));
                ed2.edit(|m| m.trigger = trigger);
            }
        });
        // Pointing at a type shows that type, with the choice already made for the one in use.
        let motion = gtk::EventControllerMotion::new();
        {
            let (ed, diagram) = (ed.clone(), diagram.clone());
            motion.connect_enter(move |_, _, _| {
                let current = ed.m.borrow().trigger;
                diagram.show(if same(&current, &trigger) {
                    current
                } else {
                    trigger
                });
            });
        }
        card.add_controller(motion);
        cards.append(&card);
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.append(&cards);
    column.append(&diagram.root);
    let motion = gtk::EventControllerMotion::new();
    {
        let diagram = diagram.clone();
        motion.connect_enter(move |_, _, _| diagram.hover(true));
    }
    {
        let diagram = diagram.clone();
        motion.connect_leave(move |_| diagram.hover(false));
    }
    column.add_controller(motion);
    // Any change of type or of what it does afterwards plays once.
    let previous = ed.redraw.borrow().clone();
    {
        let ed = ed.clone();
        let chained: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(p) = &previous {
                p();
            }
            let trigger = ed.m.borrow().trigger;
            diagram.choose(trigger);
        });
        *ed.redraw.borrow_mut() = Some(chained);
    }
    (column.upcast(), sentence)
}

const AGAIN: [(&str, PressAgain); 4] = [
    ("Stops it", PressAgain::Stop),
    ("Restarts it", PressAgain::Restart),
    ("Does nothing", PressAgain::Ignore),
    ("Plays it again after", PressAgain::Queue),
];

const RELEASE: [(&str, Release); 2] = [
    ("Finishes the pass", Release::FinishPass),
    ("Stops at once", Release::StopNow),
];

/// What another press does, or what letting go does, for the types that ask.
///
/// Each question has its own list. A redraw only moves a list that is showing the wrong thing:
/// changing `selected` from inside its own notify makes GObject restart that notify, for ever.
fn trigger_option(ed: &Ed) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(8).build();
    let label = gtk::Label::new(None);
    row.append(&label);
    let again = gtk::DropDown::from_strings(&AGAIN.map(|(name, _)| name));
    let release = gtk::DropDown::from_strings(&RELEASE.map(|(name, _)| name));
    row.append(&again);
    row.append(&release);
    {
        let ed = ed.clone();
        again.connect_selected_notify(move |d| {
            let Some((_, chosen)) = AGAIN.get(d.selected() as usize).copied() else {
                return;
            };
            let current = ed.m.borrow().trigger;
            if matches!(current, Trigger::Once { again } if again != chosen) {
                ed.edit(|m| m.trigger = Trigger::Once { again: chosen });
            }
        });
    }
    {
        let ed = ed.clone();
        release.connect_selected_notify(move |d| {
            let Some((_, chosen)) = RELEASE.get(d.selected() as usize).copied() else {
                return;
            };
            let current = ed.m.borrow().trigger;
            if matches!(current, Trigger::WhileHeld { on_release } if on_release != chosen) {
                ed.edit(|m| m.trigger = Trigger::WhileHeld { on_release: chosen });
            }
        });
    }
    let draw: Rc<dyn Fn()> = {
        let (ed, row) = (ed.clone(), row.clone());
        Rc::new(move || {
            let trigger = ed.m.borrow().trigger;
            let shown = |drop: &gtk::DropDown, index: Option<usize>| {
                drop.set_visible(index.is_some());
                if let Some(index) = index.map(|i| i as u32).filter(|i| *i != drop.selected()) {
                    drop.set_selected(index);
                }
            };
            shown(
                &again,
                match trigger {
                    Trigger::Once { again } => AGAIN.iter().position(|(_, a)| *a == again),
                    _ => None,
                },
            );
            shown(
                &release,
                match trigger {
                    Trigger::WhileHeld { on_release } => RELEASE.iter().position(|(_, r)| *r == on_release),
                    _ => None,
                },
            );
            match trigger {
                Trigger::Once { .. } => label.set_label("Pressing again"),
                Trigger::WhileHeld { .. } => label.set_label("Letting go"),
                _ => {}
            }
            row.set_visible(matches!(
                trigger,
                Trigger::Once { .. } | Trigger::WhileHeld { .. }
            ));
        })
    };
    let previous = ed.redraw.borrow().clone();
    let chained: Rc<dyn Fn()> = {
        let draw = draw.clone();
        Rc::new(move || {
            if let Some(p) = &previous {
                p();
            }
            draw();
        })
    };
    *ed.redraw.borrow_mut() = Some(chained);
    draw();
    row.upcast()
}

fn delays_row(ed: &Ed) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(8).build();
    let current = ed.m.borrow().standard_delay_ms;
    let switch = gtk::Switch::builder()
        .active(current.is_some())
        .valign(gtk::Align::Center)
        .build();
    row.append(&switch);
    row.append(&gtk::Label::new(Some("Use Standard Delays")));
    let spin = gtk::SpinButton::with_range(1.0, 10_000.0, 10.0);
    spin.set_value(current.unwrap_or(DEFAULT_STANDARD_DELAY) as f64);
    spin.set_sensitive(current.is_some());
    row.append(&spin);
    row.append(&gtk::Label::new(Some("ms")));
    {
        let (ed, spin) = (ed.clone(), spin.clone());
        switch.connect_active_notify(move |s| {
            spin.set_sensitive(s.is_active());
            let delay = s.is_active().then(|| spin.value() as u32);
            ed.edit(|m| m.standard_delay_ms = delay);
        });
    }
    {
        let ed = ed.clone();
        spin.connect_value_changed(move |s| {
            let delay = s.value() as u32;
            let mut m = ed.m.borrow_mut();
            if m.standard_delay_ms.is_some() {
                m.standard_delay_ms = Some(delay);
            }
        });
    }
    row.upcast()
}

fn feedback_row(ed: &Ed) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(4).css_classes(["linked"]).build();
    let current = ed.m.borrow().feedback;
    let choices = [
        (
            "Off",
            Feedback::Off,
            "The key doesn't change while the macro plays",
        ),
        (
            "Progress Bar",
            Feedback::Progress,
            "A bar fills along the bottom of the key",
        ),
        (
            "Step of Total",
            Feedback::Count { show_total: true },
            "The key counts through the steps, like 2/12",
        ),
        (
            "Step Number",
            Feedback::Count { show_total: false },
            "The key shows the step it's on, like 2",
        ),
        (
            "Custom per Step",
            Feedback::Custom,
            "Give each step its own label and image: click a step to set them",
        ),
    ];
    let mut first: Option<gtk::ToggleButton> = None;
    for (label, feedback, tip) in choices {
        let b = gtk::ToggleButton::builder()
            .label(label)
            .tooltip_text(tip)
            .active(current == feedback)
            .build();
        if let Some(f) = &first {
            b.set_group(Some(f));
        } else {
            first = Some(b.clone());
        }
        let ed = ed.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                ed.edit(|m| m.feedback = feedback);
            }
        });
        row.append(&b);
    }
    row.upcast()
}

/// The recorder: Record, the hint, and a pad for the mouse.
fn recorder(ed: &Ed) -> (gtk::Button, gtk::Widget) {
    let record = gtk::Button::builder().label("Record").build();
    let hint = gtk::Label::builder()
        .label(RECORD_HINT)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();
    let pad = gtk::Label::builder()
        .label("Click and scroll here to record the mouse")
        .height_request(70)
        .css_classes(["card", "macro-pad"])
        .build();
    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .visible(false)
        .build();
    panel.append(&hint);
    panel.append(&pad);

    let live: Rc<RefCell<Option<MacroRecorder>>> = Rc::default();
    let capture: Rc<Capture> = Rc::default();
    let stop: Rc<dyn Fn()> = {
        let (ed, live, capture, panel, record) = (
            ed.clone(),
            live.clone(),
            capture.clone(),
            panel.clone(),
            record.downgrade(),
        );
        Rc::new(move || {
            capture.end();
            panel.set_visible(false);
            if let Some(record) = record.upgrade() {
                record.set_label("Record");
                record.remove_css_class("destructive-action");
            }
            if let Some(r) = live.borrow_mut().take() {
                let steps = r.finish();
                ed.edit(|m| m.steps.extend(steps));
            }
        })
    };
    *ed.stop_recording.borrow_mut() = Some(stop.clone());
    {
        let (ed, live, capture, panel, stop) = (
            ed.clone(),
            live.clone(),
            capture.clone(),
            panel.clone(),
            stop.clone(),
        );
        record.connect_clicked(move |b| {
            if live.borrow().is_some() {
                stop();
                return;
            }
            *live.borrow_mut() = Some(MacroRecorder::default());
            panel.set_visible(true);
            b.set_label("Stop");
            b.add_css_class("destructive-action");
            let stop = stop.clone();
            let lost = Rc::downgrade(&stop);
            capture.begin(ed.window.upcast_ref(), move || {
                if let Some(stop) = lost.upgrade() {
                    stop();
                }
            });
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let record_key = {
        let live = live.clone();
        move |c: &gtk::EventControllerKey, keyval: gdk::Key, code: u32, down: bool| -> glib::Propagation {
            let mut live = live.borrow_mut();
            let Some(r) = live.as_mut() else {
                return glib::Propagation::Proceed;
            };
            let Some(w) = c.widget() else {
                return glib::Propagation::Proceed;
            };
            let name = base_key_name(&w, keyval, code).to_ascii_lowercase();
            let name = sendable_alias(&name);
            if ulanzi_core::keys::is_sendable(name) {
                r.key(name, down, c.current_event_time() as u64);
            }
            glib::Propagation::Stop
        }
    };
    {
        let record_key = record_key.clone();
        keys.connect_key_pressed(move |c, k, code, _| record_key(c, k, code, true));
    }
    keys.connect_key_released(move |c, k, code, _| {
        record_key(c, k, code, false);
    });
    ed.window.add_controller(keys);

    let button_of = |n: u32| match n {
        2 => MouseButton::Middle,
        3 => MouseButton::Right,
        8 => MouseButton::Back,
        9 => MouseButton::Forward,
        _ => MouseButton::Left,
    };
    // Raw events: a click gesture gives up when the pointer moves, and would lose the release.
    let buttons = gtk::EventControllerLegacy::new();
    {
        let live = live.clone();
        buttons.connect_event(move |_, event| {
            let down = match event.event_type() {
                gdk::EventType::ButtonPress => true,
                gdk::EventType::ButtonRelease => false,
                _ => return glib::Propagation::Proceed,
            };
            if let (Some(r), Some(e)) = (
                live.borrow_mut().as_mut(),
                event.downcast_ref::<gdk::ButtonEvent>(),
            ) {
                r.button(button_of(e.button()), down, event.time() as u64);
            }
            glib::Propagation::Stop
        });
    }
    pad.add_controller(buttons);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    {
        let live = live.clone();
        scroll.connect_scroll(move |c, dx, dy| {
            let (dx, dy) = (dx.round() as i32, dy.round() as i32);
            if let Some(r) = live.borrow_mut().as_mut()
                && (dx, dy) != (0, 0)
            {
                r.scroll(dx, dy, c.current_event_time() as u64);
            }
            glib::Propagation::Stop
        });
    }
    pad.add_controller(scroll);
    (record, panel.upcast())
}

/// The bar for a key's own copy: back to the library, one way or another.
fn own_bar(ed: &Ed, save: &OnChange, library_name: Option<String>) -> gtk::Widget {
    let bar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let name = ed.m.borrow().name.clone();
    bar.append(
        &gtk::Label::builder()
            .label(format!(
                "{name} is this key's own copy. It only exists on this key, so it isn't in your \
                 Macros list until you save it as a new macro."
            ))
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label"])
            .build(),
    );
    let row = gtk::Box::builder().spacing(6).build();
    let new = gtk::Button::with_label("Save as New Macro");
    {
        let (ed, save) = (ed.clone(), save.clone());
        new.connect_clicked(move |_| {
            if !ed.in_place() {
                return;
            }
            let (ed2, save) = (ed.clone(), save.clone());
            let name = ed.m.borrow().name.clone();
            ask_text(&ed, "Name the New Macro", &name, move |name| {
                ed2.end_recording();
                let mut m = ed2.m.borrow().clone();
                m.id = Uuid::new_v4();
                m.name = name;
                m.previous = None;
                let id = m.id;
                ed2.state.edit(|e| {
                    e.save_macro(m);
                    e.merge_next();
                });
                write_action(&save, id, None);
                ed2.close_saved();
            });
        });
    }
    row.append(&new);
    let update_label = match &library_name {
        Some(n) => format!("Update {n}"),
        None => "Add to Library".to_string(),
    };
    let update = gtk::Button::with_label(&update_label);
    {
        let (ed, save) = (ed.clone(), save.clone());
        update.connect_clicked(move |_| update_library(&ed, &save));
    }
    row.append(&update);
    if library_name.is_some() {
        let back = gtk::Button::with_label("Use Library Version");
        let (ed, save) = (ed.clone(), save.clone());
        back.connect_clicked(move |_| {
            if !ed.in_place() {
                return;
            }
            write_action(&save, ed.m.borrow().id, None);
            crate::ui::toast(
                &ed.state,
                "Back on the library version. Undo brings your copy back.",
            );
            ed.close_saved();
        });
        row.append(&back);
    }
    bar.append(&row);
    bar.upcast()
}

/// Put the key's copy in the library, asking first when other keys follow that macro.
fn update_library(ed: &Ed, save: &OnChange) {
    if !ed.in_place() {
        return;
    }
    let apply: Rc<dyn Fn()> = {
        let (ed, save) = (ed.clone(), save.clone());
        Rc::new(move || {
            ed.end_recording();
            let m = ed.m.borrow().clone();
            let id = m.id;
            let mut saved = true;
            ed.state.edit(|e| {
                saved = e.save_macro(m);
                e.merge_next();
            });
            if !saved {
                crate::ui::toast(&ed.state, "A macro can't run itself");
                return;
            }
            write_action(&save, id, None);
            ed.close_saved();
        })
    };
    let others = ed.users.get();
    if others == 0 {
        return apply();
    }
    let keys = if others == 1 { "key" } else { "keys" };
    let alert = adw::AlertDialog::builder()
        .heading("Update the Library Macro?")
        .body(format!("This changes {others} other {keys}"))
        .build();
    alert.add_response("cancel", "Cancel");
    alert.add_response("update", "Update");
    alert.set_close_response("cancel");
    alert.set_response_appearance("update", adw::ResponseAppearance::Suggested);
    alert.connect_response(Some("update"), move |_, _| apply());
    alert.present(Some(&ed.window));
}

fn save(ed: &Ed, saving: &Saving) {
    ed.end_recording();
    let m = ed.m.borrow().clone();
    let id = m.id;
    match saving {
        Saving::Own(save) => {
            if !ed.in_place() {
                return;
            }
            write_action(save, id, Some(m));
        }
        Saving::Library(link) => {
            if link.is_some() && !ed.in_place() {
                return;
            }
            let mut saved = true;
            ed.state.edit(|e| {
                saved = e.save_macro(m);
                e.merge_next();
            });
            if !saved {
                crate::ui::toast(&ed.state, "A macro can't run itself");
                return;
            }
            if let Some(save) = link {
                write_action(save, id, None);
            }
        }
    }
    ed.close_saved();
}

fn stop_trying(ed: &Ed) {
    ed.trying.set(false);
    ed.try_run.set(ed.try_run.get() + 1);
    if let Some(c) = ed.state.client.borrow().clone() {
        runtime::run(async move { c.stop_macros().await }, |_| {});
    }
}

/// Try It: a countdown on the button, then one pass. The button stops it at any point.
fn try_it(ed: &Ed, button: &gtk::Button) {
    if ed.trying.get() {
        stop_trying(ed);
        button.set_label("Try It");
        return;
    }
    let Some(c) = ed.state.client.borrow().clone() else {
        return;
    };
    ed.end_recording();
    let m = ed.m.borrow().clone();
    let playing: u32 = m.steps.iter().map(|s| m.effective_delay(s)).sum();
    runtime::run(async move { c.try_macro(&m, TRY_COUNTDOWN * 1000).await }, |_| {});
    ed.trying.set(true);
    let run = ed.try_run.get() + 1;
    ed.try_run.set(run);
    let counting = |left: u32| format!("Stop ({left})");
    button.set_label(&counting(TRY_COUNTDOWN));
    let (b, ed) = (button.downgrade(), ed.clone());
    let left = Cell::new(TRY_COUNTDOWN);
    glib::timeout_add_seconds_local(1, move || {
        let Some(button) = b.upgrade().filter(|_| ed.try_run.get() == run) else {
            return glib::ControlFlow::Break;
        };
        left.set(left.get() - 1);
        if left.get() > 0 {
            button.set_label(&counting(left.get()));
            return glib::ControlFlow::Continue;
        }
        button.set_label("Stop");
        let (b, ed) = (button.downgrade(), ed.clone());
        glib::timeout_add_local_once(Duration::from_millis(playing as u64), move || {
            if ed.try_run.get() == run {
                ed.trying.set(false);
                if let Some(b) = b.upgrade() {
                    b.set_label("Try It");
                }
            }
        });
        glib::ControlFlow::Break
    });
}

/// Open the editor.
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>, target: MacroTarget) {
    let (m, saving) = load(state, target);
    let window = adw::Window::builder()
        .title("Macro")
        .default_width(760)
        .default_height(680)
        .modal(true)
        .build();
    if let Some(root) = parent.root().and_downcast::<gtk::Window>() {
        window.set_transient_for(Some(&root));
    }
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(18)
        .margin_end(18)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    let original = m.clone();
    let ed = Ed {
        state: state.clone(),
        m: Rc::new(RefCell::new(m)),
        window: window.clone(),
        steps: gtk::Box::new(gtk::Orientation::Vertical, 6),
        warning: gtk::Label::builder()
            .css_classes(["warning", "caption"])
            .xalign(0.0)
            .visible(false)
            .build(),
        redraw: Rc::default(),
        original: Rc::new(RefCell::new(original)),
        stop_recording: Rc::default(),
        users: Rc::default(),
        trying: Rc::default(),
        try_run: Rc::default(),
        selected: Rc::default(),
        anchor: Rc::default(),
        opened_on: (state.profile_id(), state.editor.borrow().page()),
    };
    {
        let ed2 = ed.clone();
        *ed.redraw.borrow_mut() = Some(Rc::new(move || draw_steps(&ed2)));
    }

    let name = gtk::Entry::builder()
        .text(ed.m.borrow().name.clone())
        .placeholder_text("Macro name")
        .hexpand(true)
        .build();
    {
        let ed = ed.clone();
        name.connect_changed(move |e| ed.m.borrow_mut().name = e.text().to_string());
    }
    let (record, record_panel) = recorder(&ed);
    let keys = gtk::EventControllerKey::new();
    {
        let ed = ed.clone();
        keys.connect_key_pressed(move |_, key, _, held| {
            let any = !ed.selected.borrow().is_empty();
            match key {
                gdk::Key::a | gdk::Key::A if held.contains(gdk::ModifierType::CONTROL_MASK) => {
                    ed.select_all();
                }
                gdk::Key::Delete if any => ed.delete_selected(),
                gdk::Key::Escape if any => ed.deselect(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keys);
    let try_button = gtk::Button::with_label("Try It");
    {
        let ed = ed.clone();
        try_button.connect_clicked(move |b| try_it(&ed, b));
    }
    let top = gtk::Box::builder().spacing(6).build();
    top.append(&name);
    top.append(&try_button);
    top.append(&record);
    body.append(&top);
    body.append(&record_panel);

    let id = ed.m.borrow().id;
    let is_own = matches!(saving, Saving::Own(_));
    let in_library = state.editor.borrow().library().get(id).is_some();
    if in_library && !is_own {
        let usage = gtk::Label::builder()
            .css_classes(["dim-label", "caption"])
            .build();
        let row = gtk::Box::builder().spacing(8).build();
        row.append(&usage);
        if let Some(c) = state.client.borrow().clone() {
            let (users, usage) = (ed.users.clone(), usage.clone());
            runtime::run(async move { c.macro_usage(id).await }, move |r| {
                let total: u32 = r.iter().flatten().map(|(_, n)| n).sum();
                users.set(total);
                let keys = if total == 1 { "key" } else { "keys" };
                usage.set_label(&format!("Used on {total} {keys}"));
            });
        }
        if let Some(old) = ed.m.borrow().previous.clone() {
            let restore = gtk::Button::builder()
                .label("Restore Previous Version")
                .css_classes(["flat"])
                .build();
            let (ed2, parent2) = (ed.clone(), parent.clone().upcast::<gtk::Widget>());
            restore.connect_clicked(move |_| {
                let alert = adw::AlertDialog::builder()
                    .heading("Restore the Previous Version?")
                    .body(
                        "This replaces the macro with how it was before the last save. Undo brings it back.",
                    )
                    .close_response("cancel")
                    .build();
                alert.add_response("cancel", "Cancel");
                alert.add_response("restore", "Restore");
                alert.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
                let (ed3, parent3, old) = (ed2.clone(), parent2.clone(), (*old).clone());
                alert.connect_response(Some("restore"), move |_, _| {
                    let (old, id) = (old.clone(), old.id);
                    ed3.end_recording();
                    ed3.state.edit(|e| {
                        e.save_macro(old);
                    });
                    ed3.close_saved();
                    open(&ed3.state, &parent3, MacroTarget::Library(id));
                });
                alert.present(Some(&ed2.window));
            });
            row.append(&restore);
        }
        if let Saving::Library(Some(save)) = &saving {
            let copy = gtk::Button::builder()
                .label("Edit for This Key Only")
                .css_classes(["flat"])
                .build();
            let (ed2, save, parent2) = (ed.clone(), save.clone(), parent.clone().upcast::<gtk::Widget>());
            copy.connect_clicked(move |_| {
                if !ed2.in_place() {
                    return;
                }
                ed2.end_recording();
                let m = ed2.m.borrow().clone();
                write_action(&save, m.id, Some(m.clone()));
                let target = MacroTarget::Action {
                    action: Action::Macro {
                        id: m.id,
                        own: Some(Box::new(m)),
                    },
                    save: save.clone(),
                };
                ed2.close_saved();
                open(&ed2.state, &parent2, target);
            });
            row.append(&copy);
        }
        body.append(&row);
    }
    if let Saving::Own(save) = &saving {
        let library_name = state.editor.borrow().library().get(id).map(|m| m.name.clone());
        body.append(&own_bar(&ed, save, library_name));
    }

    let (cards, sentence) = trigger_cards(&ed);
    body.append(&cards);
    body.append(&sentence);
    body.append(&trigger_option(&ed));
    body.append(&delays_row(&ed));
    body.append(&steps_heading(&ed));
    body.append(&ed.steps);
    body.append(&ed.warning);
    body.append(
        &gtk::Label::builder()
            .label("Key Feedback")
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .build(),
    );
    body.append(&feedback_row(&ed));
    ed.changed();

    let header = adw::HeaderBar::new();
    let done = gtk::Button::builder()
        .label("Done")
        .css_classes(["suggested-action"])
        .build();
    {
        let ed = ed.clone();
        done.connect_clicked(move |_| save(&ed, &saving));
    }
    header.pack_end(&done);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&body)
            .build(),
    ));
    window.set_content(Some(&view));
    {
        // Given up on closing: a handler on the window that kept this would keep the window.
        let held = RefCell::new(Some(ed.clone()));
        window.connect_close_request(move |w| {
            let Some(ed) = held.borrow().clone() else {
                return glib::Propagation::Proceed;
            };
            ed.end_recording();
            if ed.trying.get() {
                stop_trying(&ed);
            }
            if *ed.m.borrow() == *ed.original.borrow() {
                ed.let_go();
                held.take();
                return glib::Propagation::Proceed;
            }
            let alert = adw::AlertDialog::builder()
                .heading("Discard Your Changes?")
                .body("This macro has changes that aren't saved.")
                .close_response("keep")
                .build();
            alert.add_response("keep", "Keep Editing");
            alert.add_response("discard", "Discard");
            alert.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
            let (ed2, window) = (ed.clone(), w.clone());
            alert.connect_response(Some("discard"), move |_, _| {
                *ed2.original.borrow_mut() = ed2.m.borrow().clone();
                window.close();
            });
            alert.present(Some(w));
            glib::Propagation::Stop
        });
    }
    window.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_drop_downs(w: &gtk::Widget, out: &mut Vec<gtk::DropDown>) {
        if let Some(d) = w.downcast_ref::<gtk::DropDown>() {
            out.push(d.clone());
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            all_drop_downs(&c, out);
            child = c.next_sibling();
        }
    }

    #[test]
    fn new_macro_on_a_key_opens_the_editor_and_the_list_stays_put() {
        crate::ui::with_gtk(|| {
            let state = crate::state::AppState::new(ulanzi_core::model::Profile::new_default(
                "Default",
                ulanzi_core::model::Model::D200X,
            ));
            let mut library_macro = Macro::new("First");
            library_macro.id = Uuid::new_v4();
            let id = library_macro.id;
            state.edit(|e| {
                e.save_macro(library_macro);
            });
            let action = Action::Macro { id, own: None };
            let on: OnChange = Rc::new(|_, _| {});
            let rows = rows_for(&state, "test", &action, on);
            let combo = rows[0].clone().downcast::<adw::ComboRow>().expect("list");
            assert_eq!(combo.selected(), 1);
            let window = gtk::Window::new();
            let list = gtk::ListBox::new();
            list.append(&combo);
            window.set_child(Some(&list));
            window.present();
            let before = gtk::Window::list_toplevels().len();
            combo.set_selected(0);
            let context = glib::MainContext::default();
            while context.pending() {
                context.iteration(false);
            }
            assert_eq!(combo.selected(), 1);
            assert_eq!(gtk::Window::list_toplevels().len(), before + 1);
            for w in gtk::Window::list_toplevels() {
                w.downcast::<gtk::Window>().expect("window").destroy();
            }
        });
    }

    #[test]
    fn add_action_builds_its_list_when_opened() {
        crate::ui::with_gtk(|| {
            let state = crate::state::AppState::new(ulanzi_core::model::Profile::new_default(
                "Default",
                ulanzi_core::model::Model::D200X,
            ));
            let button = picker_button(&state, "Add Action", false, false, |_| {});
            assert!(button.popover().is_none());
            let window = gtk::Window::new();
            window.set_child(Some(&button));
            window.present();
            button.popup();
            assert!(button.popover().is_some());
            window.destroy();
        });
    }

    #[test]
    /// Run by hand: `cargo test -- --ignored opening_and_closing --nocapture` prints the growth.
    #[ignore]
    fn opening_and_closing_costs_little() {
        crate::ui::with_gtk(|| {
            let state = crate::state::AppState::new(ulanzi_core::model::Profile::new_default(
                "Default",
                ulanzi_core::model::Model::D200X,
            ));
            let parent = gtk::Window::new();
            let rss = || {
                std::fs::read_to_string("/proc/self/statm")
                    .ok()
                    .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
                    .unwrap_or(0)
                    * 4
            };
            let context = glib::MainContext::default();
            let cycle = || {
                open(&state, &parent, MacroTarget::New);
                while context.pending() {
                    context.iteration(false);
                }
                for window in gtk::Window::list_toplevels() {
                    if let Ok(w) = window.downcast::<adw::Window>() {
                        w.close();
                    }
                }
                while context.pending() {
                    context.iteration(false);
                }
            };
            for _ in 0..50 {
                cycle();
            }
            let before = rss();
            for _ in 0..200 {
                cycle();
            }
            eprintln!("{} KiB per open", (rss() - before) / 200);
        });
    }

    #[test]
    fn every_press_again_choice_settles() {
        crate::ui::with_gtk(|| {
            let state = crate::state::AppState::new(ulanzi_core::model::Profile::new_default(
                "Default",
                ulanzi_core::model::Model::D200X,
            ));
            let window = gtk::Window::new();
            open(&state, &window, MacroTarget::New);
            let editor = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<adw::Window>().ok())
                .next()
                .expect("editor");
            let mut drops = Vec::new();
            all_drop_downs(editor.upcast_ref(), &mut drops);
            let drop = drops.first().expect("drop down");
            assert_eq!(drop.model().expect("model").n_items(), AGAIN.len() as u32);
            for choice in [3, 1, 2, 3, 0] {
                drop.set_selected(choice);
                assert_eq!(drop.selected(), choice);
            }
            editor.destroy();
        });
    }
}
