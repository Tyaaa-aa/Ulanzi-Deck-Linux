//! The macro editor: name, type, steps, recording and key feedback.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::catalog::describe;
use ulanzi_core::edit::Slot;
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
use crate::ui::macro_steps::{
    Chip, Direction, RECORD_HINT, chips, held_warning, move_chip, remove_chip, trigger_sentence,
};

/// Seconds before Try It plays, to click into the target app.
const TRY_COUNTDOWN: u32 = 3;
const DEFAULT_STANDARD_DELAY: u32 = 50;

/// What the editor opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroTarget {
    New,
    Library(Uuid),
    /// The macro on a key: its own copy, its library macro, or a new one.
    Key(Slot),
}

/// What Done does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Saving {
    /// Into the library, linking `slot` to it.
    Library(Option<Slot>),
    /// Into the key's own copy.
    Own(Slot),
}

fn action_at(state: &State, slot: Slot) -> Option<Action> {
    let e = state.editor.borrow();
    match slot {
        Slot::Key(k) => e.key(k).and_then(|k| k.tap.clone()),
        Slot::Dial(d) => e.dials().get(d as usize).and_then(|d| d.press.clone()),
        Slot::Small(0) => e.profile().small_buttons.left.clone(),
        Slot::Small(_) => e.profile().small_buttons.right.clone(),
        Slot::Screen => None,
    }
}

fn load(state: &State, target: MacroTarget) -> (Macro, Saving) {
    let library = |id: Uuid| state.editor.borrow().library().get(id).cloned();
    match target {
        MacroTarget::New => (Macro::new("New Macro"), Saving::Library(None)),
        MacroTarget::Library(id) => (
            library(id).unwrap_or_else(|| Macro::new("New Macro")),
            Saving::Library(None),
        ),
        MacroTarget::Key(slot) => match action_at(state, slot) {
            Some(Action::Macro { own: Some(m), .. }) => (*m, Saving::Own(slot)),
            Some(Action::Macro { id, own: None }) => match library(id) {
                Some(m) => (m, Saving::Library(Some(slot))),
                None => (Macro::new("New Macro"), Saving::Library(Some(slot))),
            },
            _ => (Macro::new("New Macro"), Saving::Library(Some(slot))),
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
}

impl Ed {
    fn changed(&self) {
        let draw = self.redraw.borrow().clone();
        if let Some(draw) = draw {
            draw();
        }
    }

    fn edit(&self, f: impl FnOnce(&mut Macro)) {
        f(&mut self.m.borrow_mut());
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
        let action = picker_button(&ed.state.clone(), "Add Action", false, move |action| {
            ed.edit(|m| m.steps.push(new_step(MacroEvent::Action { action }, part)));
        });
        holder.append(&action);
    }
    holder.upcast()
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
    let first = chip.steps.first().copied();
    let m = ed.m.borrow().clone();
    if let Some(i) = first {
        match m.steps.get(i).map(|s| s.event.clone()) {
            Some(MacroEvent::TypeText { text }) => {
                let entry = gtk::Entry::builder().text(text).build();
                let ed2 = ed.clone();
                entry.connect_changed(move |e| {
                    let text = e.text().to_string();
                    if let Some(s) = ed2.m.borrow_mut().steps.get_mut(i) {
                        s.event = MacroEvent::TypeText { text };
                    }
                });
                let ed2 = ed.clone();
                popover.connect_closed(move |_| ed2.changed());
                body.append(&entry);
            }
            Some(MacroEvent::Wait { ms }) => {
                let spin = gtk::SpinButton::with_range(0.0, 600_000.0, 10.0);
                spin.set_value(ms as f64);
                let ed2 = ed.clone();
                spin.connect_value_changed(move |s| {
                    let ms = s.value() as u32;
                    if let Some(step) = ed2.m.borrow_mut().steps.get_mut(i) {
                        step.event = MacroEvent::Wait { ms };
                    }
                });
                let ed2 = ed.clone();
                popover.connect_closed(move |_| ed2.changed());
                body.append(&spin);
            }
            Some(MacroEvent::Action { action }) => {
                let ed2 = ed.clone();
                let on: OnChange = Rc::new(move |action: Action, _: How| {
                    if let Some(step) = ed2.m.borrow_mut().steps.get_mut(i) {
                        step.event = MacroEvent::Action { action };
                    }
                });
                let path = format!("macro/{}/{i}", m.id);
                let group = gtk::ListBox::builder().css_classes(["boxed-list"]).build();
                for row in rows_for(&ed.state, &path, &action, on) {
                    group.append(&row);
                }
                body.append(&gtk::Label::new(Some(&describe(&action))));
                body.append(&group);
                let ed2 = ed.clone();
                popover.connect_closed(move |_| ed2.changed());
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
    let ed2 = ed.clone();
    spin.connect_value_changed(move |s| {
        if let Some(step) = ed2.m.borrow_mut().steps.get_mut(step) {
            step.delay_ms = s.value() as u32;
        }
    });
    let ed2 = ed.clone();
    popover.connect_closed(move |_| ed2.changed());
    popover.popup();
}

/// One step chip: click to edit, drag to reorder.
fn chip_widget(ed: &Ed, chip: &Chip, index: usize, part: SequencePart) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name(chip.icon));
    let arrow = match chip.direction {
        Direction::Down => Some("go-down-symbolic"),
        Direction::Up => Some("go-up-symbolic"),
        Direction::Both => None,
    };
    if let Some(arrow) = arrow {
        content.append(&gtk::Image::from_icon_name(arrow));
    }
    content.append(&gtk::Label::new(Some(&chip.label)));
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["macro-step"])
        .build();
    {
        let (ed, chip) = (ed.clone(), chip.clone());
        button.connect_clicked(move |b| step_popover(&ed, b, &chip, index, part));
    }
    let drag = gtk::DragSource::builder().actions(gdk::DragAction::MOVE).build();
    drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&(index as u32).to_value())));
    button.add_controller(drag);
    let drop = gtk::DropTarget::new(u32::static_type(), gdk::DragAction::MOVE);
    {
        let ed = ed.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Ok(from) = value.get::<u32>() else {
                return false;
            };
            ed.edit(|m| move_chip(m, part, from as usize, index));
            true
        });
    }
    button.add_controller(drop);
    button.upcast()
}

fn draw_steps(ed: &Ed) {
    while let Some(child) = ed.steps.first_child() {
        ed.steps.remove(&child);
    }
    let m = ed.m.borrow().clone();
    let parts: &[SequencePart] = if m.trigger == Trigger::Sequence {
        &[SequencePart::Press, SequencePart::Held, SequencePart::ReleaseKey]
    } else {
        &[SequencePart::Press]
    };
    for part in parts {
        if m.trigger == Trigger::Sequence {
            ed.steps.append(
                &gtk::Label::builder()
                    .label(part_title(*part))
                    .css_classes(["dim-label", "caption"])
                    .xalign(0.0)
                    .build(),
            );
        }
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(30)
            .column_spacing(4)
            .row_spacing(4)
            .build();
        for (i, chip) in chips(&m, *part).iter().enumerate() {
            flow.insert(&chip_widget(ed, chip, i, *part), -1);
            if let (Some(ms), Some(step)) = (chip.delay, chip.steps.last().copied()) {
                let b = gtk::Button::builder()
                    .label(format!("{ms} ms"))
                    .css_classes(["flat", "macro-delay"])
                    .build();
                let ed2 = ed.clone();
                b.connect_clicked(move |b| delay_popover(&ed2, b, step, ms));
                flow.insert(&b, -1);
            }
        }
        flow.insert(&add_menu(ed, *part), -1);
        ed.steps.append(&flow);
    }
    match held_warning(&m) {
        Some(w) => {
            ed.warning.set_label(&w);
            ed.warning.set_visible(true);
        }
        None => ed.warning.set_visible(false),
    }
}

/// How far through a loop of the diagram each of its four parts of a key press has come.
fn diagram_state(t: &Trigger, phase: f64) -> (bool, usize) {
    let lit = |from: f64, to: f64| ((phase - from) / (to - from) * 4.0).floor().clamp(-1.0, 3.0) as i32;
    match t {
        Trigger::Once { .. } => (phase < 0.08, lit(0.08, 0.6).max(-1) as usize % 5),
        Trigger::WhileHeld { .. } => (phase < 0.75, (phase * 16.0) as usize % 4),
        Trigger::Toggle => (
            (0.0..0.06).contains(&phase) || (0.75..0.81).contains(&phase),
            (phase * 16.0) as usize % 4,
        ),
        Trigger::Sequence => (phase < 0.7, (phase * 4.0) as usize % 4),
    }
}

/// A small looping picture of a key press and what the steps do.
fn diagram(trigger: Trigger) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(96)
        .content_height(34)
        .build();
    let phase = Rc::new(Cell::new(0.0f64));
    {
        let phase = phase.clone();
        area.set_draw_func(move |w, cr, width, _| {
            let color = w.color();
            let (down, active) = diagram_state(&trigger, phase.get());
            cr.set_source_rgba(color.red().into(), color.green().into(), color.blue().into(), 0.9);
            let key = if down { 14.0 } else { 18.0 };
            cr.rectangle(2.0, (34.0 - key) / 2.0, key, key);
            let _ = if down { cr.fill() } else { cr.stroke() };
            for i in 0..4 {
                let alpha = if i == active { 0.9 } else { 0.25 };
                cr.set_source_rgba(
                    color.red().into(),
                    color.green().into(),
                    color.blue().into(),
                    alpha,
                );
                let x = 28.0 + i as f64 * (width as f64 - 30.0) / 4.0;
                cr.rectangle(x, 12.0, 11.0, 10.0);
                let _ = cr.fill();
            }
        });
    }
    area.add_tick_callback(move |w, clock| {
        phase.set((clock.frame_time() as f64 / 2_400_000.0).fract());
        w.queue_draw();
        glib::ControlFlow::Continue
    });
    area
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
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.append(&diagram(trigger));
        content.append(&gtk::Label::new(Some(name)));
        let card = gtk::ToggleButton::builder()
            .child(&content)
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
        cards.append(&card);
    }
    (cards.upcast(), sentence)
}

/// What another press does, or what letting go does, for the types that ask.
fn trigger_option(ed: &Ed) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(8).build();
    let label = gtk::Label::new(None);
    row.append(&label);
    let drop = gtk::DropDown::from_strings(&[]);
    row.append(&drop);
    let holder = (row.clone(), label, drop);
    let draw: Rc<dyn Fn()> = {
        let (ed, (row, label, drop)) = (ed.clone(), holder);
        let updating = Rc::new(Cell::new(false));
        {
            let (ed, updating) = (ed.clone(), updating.clone());
            drop.connect_selected_notify(move |d| {
                if updating.get() {
                    return;
                }
                let i = d.selected();
                ed.edit(|m| match m.trigger {
                    Trigger::Once { .. } => {
                        m.trigger = Trigger::Once {
                            again: match i {
                                1 => PressAgain::Restart,
                                2 => PressAgain::Ignore,
                                3 => PressAgain::Queue,
                                _ => PressAgain::Stop,
                            },
                        }
                    }
                    Trigger::WhileHeld { .. } => {
                        m.trigger = Trigger::WhileHeld {
                            on_release: if i == 1 {
                                Release::StopNow
                            } else {
                                Release::FinishPass
                            },
                        }
                    }
                    _ => {}
                });
            });
        }
        Rc::new(move || {
            updating.set(true);
            let trigger = ed.m.borrow().trigger;
            let (text, names, selected): (&str, &[&str], u32) = match trigger {
                Trigger::Once { again } => (
                    "Pressing again",
                    &["Stops it", "Restarts it", "Does nothing", "Plays it again after"],
                    match again {
                        PressAgain::Stop => 0,
                        PressAgain::Restart => 1,
                        PressAgain::Ignore => 2,
                        PressAgain::Queue => 3,
                    },
                ),
                Trigger::WhileHeld { on_release } => (
                    "Letting go",
                    &["Finishes the pass", "Stops at once"],
                    (on_release == Release::StopNow) as u32,
                ),
                _ => ("", &[], 0),
            };
            row.set_visible(!names.is_empty());
            label.set_label(text);
            drop.set_model(Some(&gtk::StringList::new(names)));
            drop.set_selected(selected);
            updating.set(false);
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
        ("Off", Feedback::Off),
        ("Progress", Feedback::Progress),
        ("3/7", Feedback::Count { show_total: true }),
        ("3", Feedback::Count { show_total: false }),
        ("Custom per Step", Feedback::Custom),
    ];
    let mut first: Option<gtk::ToggleButton> = None;
    for (label, feedback) in choices {
        let b = gtk::ToggleButton::builder()
            .label(label)
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
            record.clone(),
        );
        Rc::new(move || {
            capture.end();
            panel.set_visible(false);
            record.set_label("Record");
            record.remove_css_class("destructive-action");
            if let Some(r) = live.borrow_mut().take() {
                let steps = r.finish();
                ed.edit(|m| m.steps.extend(steps));
            }
        })
    };
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

    let click = gtk::GestureClick::builder().button(0).build();
    let button_of = |n: u32| match n {
        2 => MouseButton::Middle,
        3 => MouseButton::Right,
        8 => MouseButton::Back,
        9 => MouseButton::Forward,
        _ => MouseButton::Left,
    };
    {
        let live = live.clone();
        click.connect_pressed(move |g, _, _, _| {
            if let Some(r) = live.borrow_mut().as_mut() {
                r.button(button_of(g.current_button()), true, g.current_event_time() as u64);
            }
        });
    }
    {
        let live = live.clone();
        click.connect_released(move |g, _, _, _| {
            if let Some(r) = live.borrow_mut().as_mut() {
                r.button(
                    button_of(g.current_button()),
                    false,
                    g.current_event_time() as u64,
                );
            }
        });
    }
    pad.add_controller(click);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    {
        let live = live.clone();
        scroll.connect_scroll(move |c, dx, dy| {
            if let Some(r) = live.borrow_mut().as_mut() {
                r.scroll(
                    dx.round() as i32,
                    dy.round() as i32,
                    c.current_event_time() as u64,
                );
            }
            glib::Propagation::Stop
        });
    }
    pad.add_controller(scroll);
    (record, panel.upcast())
}

/// The bar for a key's own copy: back to the library, one way or another.
fn own_bar(ed: &Ed, slot: Slot, library_name: Option<String>) -> gtk::Widget {
    let bar = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let name = ed.m.borrow().name.clone();
    bar.append(
        &gtk::Label::builder()
            .label(format!("This key's own copy of {name}"))
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build(),
    );
    let row = gtk::Box::builder().spacing(6).build();
    let keep = move |ed: &Ed| {
        let m = ed.m.borrow().clone();
        let id = m.id;
        ed.state.edit(|e| {
            e.set_slot_action(
                slot,
                Action::Macro {
                    id,
                    own: Some(Box::new(m)),
                },
            );
        });
    };
    let new = gtk::Button::with_label("Save as New Macro");
    {
        let ed = ed.clone();
        new.connect_clicked(move |_| {
            let ed2 = ed.clone();
            let name = ed.m.borrow().name.clone();
            ask_text(&ed, "Name the New Macro", &name, move |name| {
                keep(&ed2);
                ed2.state.edit(|e| {
                    e.own_to_new(slot, &name);
                });
                ed2.window.close();
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
        let ed = ed.clone();
        update.connect_clicked(move |_| update_library(&ed, slot));
    }
    row.append(&update);
    if library_name.is_some() {
        let back = gtk::Button::with_label("Use Library Version");
        let ed = ed.clone();
        back.connect_clicked(move |_| {
            ed.state.edit(|e| e.use_library(slot));
            crate::ui::toast(
                &ed.state,
                "Back on the library version. Undo brings your copy back.",
            );
            ed.window.close();
        });
        row.append(&back);
    }
    bar.append(&row);
    bar.upcast()
}

/// Put the key's copy in the library, asking first when other keys follow that macro.
fn update_library(ed: &Ed, slot: Slot) {
    let apply: Rc<dyn Fn()> = {
        let ed = ed.clone();
        Rc::new(move || {
            let m = ed.m.borrow().clone();
            let id = m.id;
            ed.state.edit(|e| {
                e.set_slot_action(
                    slot,
                    Action::Macro {
                        id,
                        own: Some(Box::new(m)),
                    },
                );
                e.own_to_library(slot);
            });
            ed.window.close();
        })
    };
    let others = ed.state.editor.borrow().macro_users(ed.m.borrow().id).len();
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

fn save(ed: &Ed, saving: Saving) {
    let m = ed.m.borrow().clone();
    let id = m.id;
    match saving {
        Saving::Own(slot) => ed.state.edit(|e| {
            e.set_slot_action(
                slot,
                Action::Macro {
                    id,
                    own: Some(Box::new(m)),
                },
            );
        }),
        Saving::Library(slot) => {
            let mut saved = true;
            ed.state.edit(|e| {
                saved = e.save_macro(m);
                if let (true, Some(slot)) = (saved, slot) {
                    e.set_slot_action(slot, Action::Macro { id, own: None });
                }
            });
            if !saved {
                crate::ui::toast(&ed.state, "A macro can't run itself");
                return;
            }
        }
    }
    ed.window.close();
}

fn try_it(ed: &Ed, button: &gtk::Button) {
    let Some(c) = ed.state.client.borrow().clone() else {
        return;
    };
    let m = ed.m.borrow().clone();
    runtime::run(async move { c.try_macro(&m, TRY_COUNTDOWN * 1000).await }, |_| {});
    let left = Rc::new(Cell::new(TRY_COUNTDOWN));
    button.set_sensitive(false);
    button.set_label(&format!("Playing in {TRY_COUNTDOWN}…"));
    let b = button.downgrade();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let Some(b) = b.upgrade() else {
            return glib::ControlFlow::Break;
        };
        left.set(left.get() - 1);
        if left.get() == 0 {
            b.set_label("Try It");
            b.set_sensitive(true);
            return glib::ControlFlow::Break;
        }
        b.set_label(&format!("Playing in {}…", left.get()));
        glib::ControlFlow::Continue
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

    match saving {
        Saving::Own(slot) => {
            let id = ed.m.borrow().id;
            let library_name = state.editor.borrow().library().get(id).map(|m| m.name.clone());
            body.append(&own_bar(&ed, slot, library_name));
        }
        Saving::Library(slot) => {
            let id = ed.m.borrow().id;
            let users = state.editor.borrow().macro_users(id).len();
            if users > 0 || slot.is_some() {
                let row = gtk::Box::builder().spacing(8).build();
                let keys = if users == 1 { "key" } else { "keys" };
                row.append(
                    &gtk::Label::builder()
                        .label(format!("Used on {users} {keys}"))
                        .css_classes(["dim-label", "caption"])
                        .build(),
                );
                if let (Some(slot), true) = (slot, state.editor.borrow().library().get(id).is_some()) {
                    let copy = gtk::Button::builder()
                        .label("Edit for This Key Only")
                        .css_classes(["flat"])
                        .build();
                    let (state2, window2, parent2) = (
                        state.clone(),
                        window.clone(),
                        parent.clone().upcast::<gtk::Widget>(),
                    );
                    copy.connect_clicked(move |_| {
                        state2.edit(|e| e.edit_for_this_key(slot));
                        window2.close();
                        open(&state2, &parent2, MacroTarget::Key(slot));
                    });
                    row.append(&copy);
                }
                body.append(&row);
            }
        }
    }

    let (cards, sentence) = trigger_cards(&ed);
    body.append(&cards);
    body.append(&sentence);
    body.append(&trigger_option(&ed));
    body.append(&delays_row(&ed));
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
        done.connect_clicked(move |_| save(&ed, saving));
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
    window.present();
}
