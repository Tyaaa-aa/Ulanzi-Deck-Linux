//! Parameter rows for each kind of action, shown under the action card in the inspector, and the
//! action picker popover.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ulanzi_core::apps::{self, AppEntry};
use ulanzi_core::catalog::{self, Category};
use ulanzi_core::model::*;
use uuid::Uuid;

use crate::state::State;

/// How an edit should be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// Typing or dragging: merged into one undo step per field, no rebuild.
    Typing(&'static str),
    /// A single change that keeps the same rows.
    Set,
    /// A change that adds or removes rows (rebuild the inspector).
    Structure,
}

pub type OnChange = Rc<dyn Fn(Action, How)>;

/// The action being edited and where changes go.
#[derive(Clone)]
struct Ed {
    cur: Rc<RefCell<Action>>,
    on: OnChange,
}

impl Ed {
    fn apply(&self, how: How, f: impl FnOnce(&mut Action)) {
        f(&mut self.cur.borrow_mut());
        let a = self.cur.borrow().clone();
        (self.on)(a, how);
    }
}

fn entry_row(
    ed: &Ed,
    title: &str,
    value: &str,
    key: &'static str,
    set: impl Fn(&mut Action, String) + 'static,
) -> gtk::Widget {
    let row = adw::EntryRow::builder().title(title).text(value).build();
    let ed = ed.clone();
    row.connect_changed(move |r| {
        let t = r.text().to_string();
        ed.apply(How::Typing(key), |a| set(a, t));
    });
    row.upcast()
}

fn switch_row(
    ed: &Ed,
    title: &str,
    subtitle: &str,
    value: bool,
    set: impl Fn(&mut Action, bool) + 'static,
) -> gtk::Widget {
    let row = adw::SwitchRow::builder()
        .title(title)
        .subtitle(subtitle)
        .active(value)
        .build();
    let ed = ed.clone();
    row.connect_active_notify(move |r| {
        let v = r.is_active();
        ed.apply(How::Set, |a| set(a, v));
    });
    row.upcast()
}

fn combo_row(
    ed: &Ed,
    title: &str,
    items: &[&str],
    selected: usize,
    how: How,
    set: impl Fn(&mut Action, usize) + 'static,
) -> adw::ComboRow {
    let model = gtk::StringList::new(items);
    let row = adw::ComboRow::builder()
        .title(title)
        .model(&model)
        .selected(selected as u32)
        .build();
    let ed = ed.clone();
    row.connect_selected_notify(move |r| {
        let i = r.selected() as usize;
        ed.apply(how, |a| set(a, i));
    });
    row
}

#[allow(clippy::too_many_arguments)]
fn spin_row(
    ed: &Ed,
    title: &str,
    subtitle: &str,
    (min, max, step): (f64, f64, f64),
    value: f64,
    key: &'static str,
    set: impl Fn(&mut Action, f64) + 'static,
) -> gtk::Widget {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(title);
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row.set_value(value);
    let ed = ed.clone();
    row.connect_value_notify(move |r| {
        let v = r.value();
        ed.apply(How::Typing(key), |a| set(a, v));
    });
    row.upcast()
}

thread_local! {
    static APPS: RefCell<Option<Rc<Vec<AppEntry>>>> = const { RefCell::new(None) };
}

/// Installed apps, sorted by name (scanned once).
pub fn installed_apps() -> Rc<Vec<AppEntry>> {
    APPS.with(|a| {
        a.borrow_mut()
            .get_or_insert_with(|| {
                let mut list = apps::scan(&apps::application_dirs());
                list.sort_by_key(|e| e.name.to_lowercase());
                Rc::new(list)
            })
            .clone()
    })
}

/// Output devices (name, description) from PipeWire / PulseAudio.
pub fn output_devices() -> Vec<(String, String)> {
    std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sinks"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| sinks_from_pactl_json(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

fn keycaps(keys: &str) -> gtk::Box {
    let b = gtk::Box::builder().spacing(4).valign(gtk::Align::Center).build();
    fill_keycaps(&b, keys);
    b
}

fn fill_keycaps(b: &gtk::Box, keys: &str) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    let chips = shortcut_chips(keys);
    if chips.is_empty() {
        b.append(
            &gtk::Label::builder()
                .label("Not set")
                .css_classes(["dim-label"])
                .build(),
        );
    }
    for c in chips {
        b.append(&gtk::Label::builder().label(c).css_classes(["keycap"]).build());
    }
}

fn shortcut_row(ed: &Ed, keys: &str) -> gtk::Widget {
    let row = adw::ActionRow::builder()
        .title("Shortcut")
        .subtitle("Click Record, then press the keys")
        .build();
    let caps = keycaps(keys);
    let record = gtk::Button::builder()
        .label("Record")
        .valign(gtk::Align::Center)
        .focusable(true)
        .build();
    row.add_suffix(&caps);
    row.add_suffix(&record);
    let recording = Rc::new(std::cell::Cell::new(false));
    let stop = {
        let (record, recording) = (record.clone(), recording.clone());
        move || {
            recording.set(false);
            record.set_label("Record");
            record.remove_css_class("suggested-action");
        }
    };
    {
        let recording = recording.clone();
        record.connect_clicked(move |b| {
            recording.set(true);
            b.set_label("Press keys…");
            b.add_css_class("suggested-action");
            b.grab_focus();
        });
    }
    {
        let stop = stop.clone();
        record.connect_has_focus_notify(move |b| {
            if !b.has_focus() {
                stop();
            }
        });
    }
    let keys_ctl = gtk::EventControllerKey::new();
    keys_ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (ed, caps, record) = (ed.clone(), caps.clone(), record.clone());
        keys_ctl.connect_key_pressed(move |_, keyval, code, m| {
            if !recording.get() {
                return glib::Propagation::Proceed;
            }
            let none = !m.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK
                    | gdk::ModifierType::SHIFT_MASK,
            );
            if keyval == gdk::Key::Escape && none {
                stop();
                return glib::Propagation::Stop;
            }
            // The unshifted key, so Shift+1 records as shift+1 rather than "exclam".
            let base = record
                .display()
                .translate_key(code, gdk::ModifierType::empty(), 0)
                .map(|(k, ..)| k)
                .unwrap_or(keyval);
            let name = base.name().map(|n| n.to_string()).unwrap_or_default();
            let mods = Mods {
                ctrl: m.contains(gdk::ModifierType::CONTROL_MASK),
                alt: m.contains(gdk::ModifierType::ALT_MASK),
                shift: m.contains(gdk::ModifierType::SHIFT_MASK),
                super_: m.intersects(gdk::ModifierType::SUPER_MASK | gdk::ModifierType::META_MASK),
            };
            if let Some(combo) = combo_from_recording(mods, &name) {
                fill_keycaps(&caps, &combo);
                ed.apply(How::Set, |a| {
                    if let Action::Shortcut { keys } = a {
                        *keys = combo;
                    }
                });
                stop();
            }
            glib::Propagation::Stop
        });
    }
    record.add_controller(keys_ctl);
    row.upcast()
}

fn sound_file_row(ed: &Ed, path: &str) -> gtk::Widget {
    let shown = if path.is_empty() {
        "No file chosen".to_string()
    } else {
        std::path::Path::new(path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string())
    };
    let row = adw::ActionRow::builder()
        .title("Sound File")
        .subtitle(shown)
        .build();
    let choose = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&choose);
    let ed = ed.clone();
    let r = row.clone();
    choose.connect_clicked(move |b| {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Sounds"));
        filter.add_mime_type("audio/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title("Choose a Sound")
            .filters(&filters)
            .modal(true)
            .build();
        let window = b.root().and_downcast::<gtk::Window>();
        let (ed, r) = (ed.clone(), r.clone());
        dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |res| {
            let Some(path) = res.ok().and_then(|f| f.path()) else {
                return;
            };
            if let Some(name) = path.file_name() {
                r.set_subtitle(&name.to_string_lossy());
            }
            let p = path.to_string_lossy().into_owned();
            ed.apply(How::Set, |a| {
                if let Action::PlaySound { path, .. } = a {
                    *path = p;
                }
            });
        });
    });
    row.upcast()
}

/// Output device choice: "Default" plus the current devices (the saved one kept if unplugged).
fn device_combo(
    ed: &Ed,
    title: &str,
    current: Option<&str>,
    with_default: bool,
    set: impl Fn(&mut Action, Option<String>) + 'static,
) -> gtk::Widget {
    let mut devices = output_devices();
    if let Some(c) = current
        && !c.is_empty()
        && !devices.iter().any(|(n, _)| n == c)
    {
        devices.push((c.to_string(), format!("{c} (not connected)")));
    }
    let mut names: Vec<Option<String>> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if with_default {
        names.push(None);
        labels.push("Default Output".into());
    }
    for (n, d) in devices {
        names.push(Some(n));
        labels.push(d);
    }
    if labels.is_empty() {
        names.push(None);
        labels.push("No devices found".into());
    }
    let selected = names
        .iter()
        .position(|n| n.as_deref() == current.filter(|c| !c.is_empty()))
        .unwrap_or(0);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let row = combo_row(ed, title, &refs, selected, How::Set, move |a, i| {
        set(a, names.get(i).cloned().flatten())
    });
    row.upcast()
}

fn page_combo(ed: &Ed, state: &State, current: Uuid) -> gtk::Widget {
    let pages: Vec<(Uuid, String)> = state
        .editor
        .borrow()
        .profile()
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id, format!("{} · {}", i + 1, p.name)))
        .collect();
    let mut ids: Vec<Option<Uuid>> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if !pages.iter().any(|(id, _)| *id == current) {
        ids.push(None);
        labels.push("Choose a page…".into());
    }
    for (id, l) in pages {
        ids.push(Some(id));
        labels.push(l);
    }
    let selected = ids.iter().position(|i| *i == Some(current)).unwrap_or(0);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    combo_row(ed, "Page", &refs, selected, How::Set, move |a, i| {
        if let Some(Some(id)) = ids.get(i)
            && let Action::GoToPage { page } | Action::OpenFolder { page } = a
        {
            *page = *id;
        }
    })
    .upcast()
}

fn profile_combo(ed: &Ed, state: &State, current: Uuid) -> gtk::Widget {
    let profiles = state.profiles.borrow().clone();
    let mut ids: Vec<Option<Uuid>> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if !profiles.iter().any(|(id, _)| *id == current) {
        ids.push(None);
        labels.push("Choose a profile…".into());
    }
    for (id, name) in profiles {
        ids.push(Some(id));
        labels.push(name);
    }
    let selected = ids.iter().position(|i| *i == Some(current)).unwrap_or(0);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    combo_row(ed, "Profile", &refs, selected, How::Set, move |a, i| {
        if let Some(Some(id)) = ids.get(i)
            && let Action::SwitchProfile { profile } = a
        {
            *profile = *id;
        }
    })
    .upcast()
}

fn app_combo(ed: &Ed, desktop_id: Option<&str>, exec: Option<&str>) -> Vec<gtk::Widget> {
    let apps = installed_apps();
    let mut ids: Vec<Option<String>> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let known = desktop_id.is_some_and(|d| apps.iter().any(|a| a.desktop_id == d));
    if !known {
        ids.push(None);
        labels.push(match (desktop_id, exec) {
            (Some(d), _) => format!("{d} (not installed)"),
            (None, Some(_)) => "Custom command".into(),
            _ => "Choose an app…".into(),
        });
    }
    for a in apps.iter() {
        ids.push(Some(a.desktop_id.clone()));
        labels.push(a.name.clone());
    }
    let selected = ids.iter().position(|i| i.as_deref() == desktop_id).unwrap_or(0);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let row = combo_row(ed, "App", &refs, selected, How::Structure, move |a, i| {
        if let Some(Some(id)) = ids.get(i)
            && let Action::LaunchApp { desktop_id, exec } = a
        {
            *desktop_id = Some(id.clone());
            *exec = None;
        }
    });
    row.set_enable_search(true);
    let mut rows = vec![row.upcast()];
    if desktop_id.is_none() {
        rows.push(entry_row(
            ed,
            "Or Run This Command",
            exec.unwrap_or(""),
            "exec",
            |a, t| {
                if let Action::LaunchApp { exec, .. } = a {
                    *exec = (!t.trim().is_empty()).then_some(t);
                }
            },
        ));
    }
    rows
}

const MEDIA_CONTROLS: [(MediaControl, &str); 4] = [
    (MediaControl::PlayPause, "Play / Pause"),
    (MediaControl::Next, "Next Track"),
    (MediaControl::Previous, "Previous Track"),
    (MediaControl::Stop, "Stop"),
];

fn volume_rows(ed: &Ed, target: &AudioTarget, op: VolumeOp, step: u8, limit_100: bool) -> Vec<gtk::Widget> {
    let mut rows = Vec::new();
    let t = match target {
        AudioTarget::Default => 0,
        AudioTarget::Input => 1,
        AudioTarget::Device { .. } => 2,
        AudioTarget::App { .. } => 3,
    };
    rows.push(
        combo_row(
            ed,
            "Control",
            &["Default Output", "Microphone", "Output Device", "App"],
            t,
            How::Structure,
            |a, i| {
                if let Action::Volume { target, .. } = a {
                    let keep = match target {
                        AudioTarget::Device { name } | AudioTarget::App { name } => name.clone(),
                        _ => String::new(),
                    };
                    *target = match i {
                        0 => AudioTarget::Default,
                        1 => AudioTarget::Input,
                        2 => AudioTarget::Device { name: keep },
                        _ => AudioTarget::App { name: keep },
                    };
                }
            },
        )
        .upcast(),
    );
    match target {
        AudioTarget::Device { name } => rows.push(device_combo(ed, "Device", Some(name), false, |a, n| {
            if let Action::Volume {
                target: AudioTarget::Device { name },
                ..
            } = a
            {
                *name = n.unwrap_or_default();
            }
        })),
        AudioTarget::App { name } => rows.push(entry_row(ed, "App Name", name, "vol-app", |a, t| {
            if let Action::Volume {
                target: AudioTarget::App { name },
                ..
            } = a
            {
                *name = t;
            }
        })),
        _ => {}
    }
    let o = match op {
        VolumeOp::Up => 0,
        VolumeOp::Down => 1,
        VolumeOp::Mute => 2,
        VolumeOp::Set { .. } => 3,
    };
    rows.push(
        combo_row(
            ed,
            "Do",
            &["Turn Up", "Turn Down", "Mute / Unmute", "Set To"],
            o,
            How::Structure,
            |a, i| {
                if let Action::Volume { op, .. } = a {
                    *op = match i {
                        0 => VolumeOp::Up,
                        1 => VolumeOp::Down,
                        2 => VolumeOp::Mute,
                        _ => VolumeOp::Set { percent: 50 },
                    };
                }
            },
        )
        .upcast(),
    );
    match op {
        VolumeOp::Up | VolumeOp::Down => {
            rows.push(spin_row(
                ed,
                "Step",
                "Percent per press or dial click",
                (1.0, 25.0, 1.0),
                step as f64,
                "vol-step",
                |a, v| {
                    if let Action::Volume { step, .. } = a {
                        *step = v as u8;
                    }
                },
            ));
            if op == VolumeOp::Up {
                rows.push(switch_row(
                    ed,
                    "Stop at 100%",
                    "Never boost above full volume",
                    limit_100,
                    |a, v| {
                        if let Action::Volume { limit_100, .. } = a {
                            *limit_100 = v;
                        }
                    },
                ));
            }
        }
        VolumeOp::Set { percent } => rows.push(spin_row(
            ed,
            "Volume",
            "Percent",
            (0.0, 100.0, 5.0),
            percent as f64,
            "vol-set",
            |a, v| {
                if let Action::Volume { op, .. } = a {
                    *op = VolumeOp::Set { percent: v as u8 };
                }
            },
        )),
        VolumeOp::Mute => {}
    }
    rows
}

fn brightness_rows(ed: &Ed, op: BrightnessOp) -> Vec<gtk::Widget> {
    let (i, v) = match op {
        BrightnessOp::Up { step } => (0, step),
        BrightnessOp::Down { step } => (1, step),
        BrightnessOp::Set { percent } => (2, percent),
    };
    let combo = combo_row(
        ed,
        "Do",
        &["Brighter", "Dimmer", "Set To"],
        i,
        How::Structure,
        |a, i| {
            if let Action::Brightness { op } = a {
                *op = match i {
                    0 => BrightnessOp::Up { step: 10 },
                    1 => BrightnessOp::Down { step: 10 },
                    _ => BrightnessOp::Set { percent: 50 },
                };
            }
        },
    );
    let (title, range) = if i == 2 {
        ("Brightness", (0.0, 100.0, 5.0))
    } else {
        ("Step", (1.0, 50.0, 1.0))
    };
    let spin = spin_row(ed, title, "Percent", range, v as f64, "bri", |a, v| {
        if let Action::Brightness { op } = a {
            let v = v as u8;
            *op = match op {
                BrightnessOp::Up { .. } => BrightnessOp::Up { step: v },
                BrightnessOp::Down { .. } => BrightnessOp::Down { step: v },
                BrightnessOp::Set { .. } => BrightnessOp::Set { percent: v },
            };
        }
    });
    vec![combo.upcast(), spin]
}

fn small_icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build()
}

fn multi_rows(ed: &Ed, state: &State, path: &str, steps: &[Step]) -> Vec<gtk::Widget> {
    let mut rows: Vec<gtk::Widget> = Vec::new();
    let n = steps.len();
    for (i, step) in steps.iter().enumerate() {
        let title = catalog::spec_for(&step.action)
            .map(|s| s.title)
            .unwrap_or("Action");
        let exp = adw::ExpanderRow::builder()
            .title(format!("{}. {title}", i + 1))
            .subtitle(step_subtitle(step))
            .build();
        exp.add_prefix(&action_chip(state, Some(&step.action), 28));
        let step_path = format!("{path}/step{i}");
        state.remember_expanded(&exp, &step_path);
        let up = small_icon_button("go-up-symbolic", "Move up");
        let down = small_icon_button("go-down-symbolic", "Move down");
        let remove = small_icon_button("user-trash-symbolic", "Remove step");
        up.set_sensitive(i > 0);
        down.set_sensitive(i + 1 < n);
        for b in [&up, &down, &remove] {
            exp.add_suffix(b);
        }
        let reorder = |delta: isize| {
            let ed = ed.clone();
            let (state, path) = (state.clone(), path.to_string());
            move |_: &gtk::Button| {
                // The open/closed state moves with the step.
                let j = (i as isize + delta) as usize;
                let (a, b) = (format!("{path}/step{i}"), format!("{path}/step{j}"));
                {
                    let mut set = state.expanded.borrow_mut();
                    let (ha, hb) = (set.remove(&a), set.remove(&b));
                    if ha {
                        set.insert(b.clone());
                    }
                    if hb {
                        set.insert(a.clone());
                    }
                }
                ed.apply(How::Structure, |a| {
                    if let Action::Multi { steps } = a {
                        let j = (i as isize + delta) as usize;
                        if j < steps.len() {
                            steps.swap(i, j);
                        }
                    }
                })
            }
        };
        up.connect_clicked(reorder(-1));
        down.connect_clicked(reorder(1));
        {
            let ed = ed.clone();
            remove.connect_clicked(move |_| {
                ed.apply(How::Structure, |a| {
                    if let Action::Multi { steps } = a {
                        steps.remove(i);
                    }
                })
            });
        }
        // The step's own parameters, written back into the list.
        let inner: OnChange = {
            let ed = ed.clone();
            let exp = exp.clone();
            Rc::new(move |new: Action, how: How| {
                ed.apply(how, |a| {
                    if let Action::Multi { steps } = a
                        && let Some(s) = steps.get_mut(i)
                    {
                        s.action = new;
                        exp.set_subtitle(&step_subtitle(s));
                    }
                })
            })
        };
        for r in rows_for(state, &step_path, &step.action, inner) {
            exp.add_row(&r);
        }
        let delay = {
            let exp = exp.clone();
            spin_row(
                ed,
                "Then Wait",
                "Milliseconds before the next step",
                (0.0, 10_000.0, 50.0),
                step.delay_ms as f64,
                "delay",
                move |a, v| {
                    if let Action::Multi { steps } = a
                        && let Some(s) = steps.get_mut(i)
                    {
                        s.delay_ms = v as u32;
                        exp.set_subtitle(&step_subtitle(s));
                    }
                },
            )
        };
        exp.add_row(&delay);
        rows.push(exp.upcast());
    }
    let add_row = adw::ActionRow::builder()
        .title(if n == 0 { "No steps yet" } else { "Add a Step" })
        .build();
    let ed2 = ed.clone();
    let add = picker_button(state, "Add Step", true, move |action| {
        ed2.apply(How::Structure, |a| {
            if let Action::Multi { steps } = a {
                steps.push(Step { action, delay_ms: 0 });
            }
        })
    });
    add_row.add_suffix(&add);
    rows.push(add_row.upcast());
    rows
}

fn step_subtitle(s: &Step) -> String {
    let d = catalog::describe(&s.action);
    if s.delay_ms > 0 {
        format!("{d} · then wait {} ms", s.delay_ms)
    } else {
        d
    }
}

/// Editor rows for `action`'s parameters (empty for actions without any).
/// `path` names where these rows are (for remembering which rows are open).
pub fn rows_for(state: &State, path: &str, action: &Action, on: OnChange) -> Vec<gtk::Widget> {
    let ed = Ed {
        cur: Rc::new(RefCell::new(action.clone())),
        on,
    };
    match action {
        Action::Command { cmd, terminal } => vec![
            entry_row(&ed, "Command", cmd, "cmd", |a, t| {
                if let Action::Command { cmd, .. } = a {
                    *cmd = t;
                }
            }),
            switch_row(
                &ed,
                "Run in a Terminal",
                "Opens a terminal window that shows the output",
                *terminal,
                |a, v| {
                    if let Action::Command { terminal, .. } = a {
                        *terminal = v;
                    }
                },
            ),
        ],
        Action::LaunchApp { desktop_id, exec } => app_combo(&ed, desktop_id.as_deref(), exec.as_deref()),
        Action::OpenUrl { url } => vec![entry_row(&ed, "Address", url, "url", |a, t| {
            if let Action::OpenUrl { url } = a {
                *url = t;
            }
        })],
        Action::Shortcut { keys } => vec![shortcut_row(&ed, keys)],
        Action::TypeText { text } => vec![entry_row(&ed, "Text to Type", text, "type", |a, t| {
            if let Action::TypeText { text } = a {
                *text = t;
            }
        })],
        Action::PlaySound { path, device } => vec![
            sound_file_row(&ed, path),
            device_combo(&ed, "Play On", device.as_deref(), true, |a, n| {
                if let Action::PlaySound { device, .. } = a {
                    *device = n;
                }
            }),
        ],
        Action::Media { control, player } => {
            let i = MEDIA_CONTROLS.iter().position(|(c, _)| c == control).unwrap_or(0);
            let labels: Vec<&str> = MEDIA_CONTROLS.iter().map(|(_, l)| *l).collect();
            vec![
                combo_row(&ed, "Control", &labels, i, How::Set, |a, i| {
                    if let Action::Media { control, .. } = a {
                        *control = MEDIA_CONTROLS[i].0;
                    }
                })
                .upcast(),
                entry_row(
                    &ed,
                    "Player (empty: the one playing)",
                    player.as_deref().unwrap_or(""),
                    "player",
                    |a, t| {
                        if let Action::Media { player, .. } = a {
                            *player = (!t.trim().is_empty()).then_some(t);
                        }
                    },
                ),
            ]
        }
        Action::Volume {
            target,
            op,
            step,
            limit_100,
        } => volume_rows(&ed, target, *op, *step, *limit_100),
        Action::Brightness { op } => brightness_rows(&ed, *op),
        Action::GoToPage { page } | Action::OpenFolder { page } => {
            vec![page_combo(&ed, state, *page)]
        }
        Action::SwitchProfile { profile } => vec![profile_combo(&ed, state, *profile)],
        Action::Multi { steps } => multi_rows(&ed, state, path, steps),
        Action::Obs(o) => obs_rows(&ed, o),
        Action::NextPage | Action::PrevPage | Action::Back | Action::AutoProfile | Action::SleepNow => {
            Vec::new()
        }
    }
}

fn obs_rows(ed: &Ed, o: &ObsAction) -> Vec<gtk::Widget> {
    fn obs(a: &mut Action) -> Option<&mut ObsAction> {
        match a {
            Action::Obs(o) => Some(o),
            _ => None,
        }
    }
    match o {
        ObsAction::SetScene { scene } => vec![entry_row(ed, "Scene", scene, "obs-scene", |a, t| {
            if let Some(ObsAction::SetScene { scene }) = obs(a) {
                *scene = t;
            }
        })],
        ObsAction::ToggleScenes { a: first, b: second } => vec![
            entry_row(ed, "First Scene", first, "obs-a", |a, t| {
                if let Some(ObsAction::ToggleScenes { a, .. }) = obs(a) {
                    *a = t;
                }
            }),
            entry_row(ed, "Second Scene", second, "obs-b", |a, t| {
                if let Some(ObsAction::ToggleScenes { b, .. }) = obs(a) {
                    *b = t;
                }
            }),
        ],
        ObsAction::ToggleSource { scene, source } => vec![
            entry_row(ed, "Scene", scene, "obs-scene", |a, t| {
                if let Some(ObsAction::ToggleSource { scene, .. }) = obs(a) {
                    *scene = t;
                }
            }),
            entry_row(ed, "Source", source, "obs-source", |a, t| {
                if let Some(ObsAction::ToggleSource { source, .. }) = obs(a) {
                    *source = t;
                }
            }),
        ],
        ObsAction::ToggleRecord | ObsAction::ToggleStream => Vec::new(),
    }
}

/// A small rendered key showing the action's default look (or an empty slot).
pub fn action_chip(state: &State, action: Option<&Action>, size: i32) -> gtk::Widget {
    let texture = action.and_then(|a| {
        crate::ui::canvas::key_texture(&catalog::default_look(a), &crate::ui::canvas::profile_dir(state))
    });
    match texture {
        Some(t) => {
            let i = gtk::Image::from_paintable(Some(&t));
            i.set_pixel_size(size);
            i.set_valign(gtk::Align::Center);
            i.set_overflow(gtk::Overflow::Hidden);
            i.add_css_class("action-chip-img");
            i.upcast()
        }
        None => {
            let i = gtk::Image::from_icon_name("list-add-symbolic");
            i.set_size_request(size, size);
            i.set_valign(gtk::Align::Center);
            i.add_css_class("action-chip-empty");
            i.upcast()
        }
    }
}

/// A button that opens a searchable list of every action; `on_pick` gets a fresh action.
pub fn picker_button(
    state: &State,
    label: &str,
    allow_multi: bool,
    on_pick: impl Fn(Action) + 'static,
) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .build();
    let popover = gtk::Popover::new();
    popover.add_css_class("action-picker");
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search actions")
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(320)
        .max_content_height(420)
        .min_content_width(260)
        .propagate_natural_height(true)
        .child(&list)
        .build();
    column.append(&search);
    column.append(&scroller);
    popover.set_child(Some(&column));
    button.set_popover(Some(&popover));

    let specs: Vec<&'static catalog::ActionSpec> = catalog::catalog()
        .iter()
        .filter(|s| allow_multi || s.id != "multi")
        // Per-app switching arrives later; until then the action would do nothing useful.
        .filter(|s| s.id != "auto_profile")
        .collect();
    let mut index: Vec<Option<usize>> = Vec::new();
    for cat in Category::ALL {
        let in_cat: Vec<usize> = specs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.category == cat)
            .map(|(i, _)| i)
            .collect();
        if in_cat.is_empty() {
            continue;
        }
        let head = gtk::ListBoxRow::builder()
            .activatable(false)
            .selectable(false)
            .child(
                &gtk::Label::builder()
                    .label(cat.title())
                    .xalign(0.0)
                    .css_classes(["heading", "dim-label"])
                    .margin_top(8)
                    .build(),
            )
            .build();
        list.append(&head);
        index.push(None);
        for i in in_cat {
            let spec = specs[i];
            let b = gtk::Box::builder().spacing(10).build();
            b.append(&action_chip(state, Some(&(spec.make)()), 24));
            b.append(&gtk::Label::builder().label(spec.title).xalign(0.0).build());
            let row = gtk::ListBoxRow::builder().child(&b).build();
            row.set_widget_name(&spec.title.to_lowercase());
            list.append(&row);
            index.push(Some(i));
        }
    }
    {
        let search = search.clone();
        list.set_filter_func(move |row| {
            let q = search.text().to_lowercase();
            if q.is_empty() {
                return true;
            }
            let name = row.widget_name();
            !name.is_empty() && !name.starts_with("Gtk") && name.contains(q.trim())
        });
    }
    {
        let list = list.clone();
        search.connect_search_changed(move |_| list.invalidate_filter());
    }
    {
        let popover = popover.clone();
        list.connect_row_activated(move |_, row| {
            let Some(Some(i)) = index.get(row.index().max(0) as usize).copied() else {
                return;
            };
            popover.popdown();
            on_pick((specs[i].make)());
        });
    }
    {
        let list = list.clone();
        search.connect_activate(move |_| {
            // Enter picks the first match.
            let mut i = 0;
            while let Some(row) = list.row_at_index(i) {
                if row.is_child_visible() && row.is_activatable() {
                    row.emit_activate();
                    break;
                }
                i += 1;
            }
        });
    }
    {
        let search = search.clone();
        popover.connect_show(move |_| {
            search.set_text("");
            search.grab_focus();
        });
    }
    button
}

/// One chip per key of an xdotool-style combo, for display: `ctrl+super+m` → Ctrl, Super, M.
pub fn shortcut_chips(keys: &str) -> Vec<String> {
    keys.split('+')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| match k.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl".into(),
            "alt" => "Alt".into(),
            "shift" => "Shift".into(),
            "super" | "meta" | "win" => "Super".into(),
            "return" | "enter" => "Enter".into(),
            "escape" | "esc" => "Esc".into(),
            _ if k.chars().count() == 1 => k.to_uppercase(),
            other => {
                // f12 → F12, page_down → Page Down
                if let Some(n) = other.strip_prefix('f').filter(|n| n.parse::<u8>().is_ok()) {
                    return format!("F{n}");
                }
                other
                    .split('_')
                    .map(|w| {
                        let mut c = w.chars();
                        c.next()
                            .map(|f| f.to_uppercase().chain(c).collect::<String>())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        })
        .collect()
}

/// Modifiers held while recording a shortcut.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_: bool,
}

const MODIFIER_KEYS: &[&str] = &[
    "control_l",
    "control_r",
    "alt_l",
    "alt_r",
    "shift_l",
    "shift_r",
    "super_l",
    "super_r",
    "meta_l",
    "meta_r",
    "iso_level3_shift",
    "hyper_l",
    "hyper_r",
];

/// The combo string the daemon understands for a recorded key press (`key` is the GDK key name),
/// or None while only modifiers are held.
pub fn combo_from_recording(mods: Mods, key: &str) -> Option<String> {
    let key = key.to_ascii_lowercase();
    if key.is_empty() || MODIFIER_KEYS.contains(&key.as_str()) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    if mods.ctrl {
        parts.push("ctrl");
    }
    if mods.alt {
        parts.push("alt");
    }
    if mods.shift {
        parts.push("shift");
    }
    if mods.super_ {
        parts.push("super");
    }
    parts.push(&key);
    Some(parts.join("+"))
}

/// Output devices from `pactl -f json list sinks`: (name, description).
pub fn sinks_from_pactl_json(json: &str) -> Vec<(String, String)> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|s| {
            let name = s.get("name")?.as_str()?.to_string();
            let desc = s
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or(&name)
                .to_string();
            Some((name, desc))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_are_pretty() {
        assert_eq!(shortcut_chips("ctrl+super+m"), ["Ctrl", "Super", "M"]);
        assert_eq!(shortcut_chips("super+F12"), ["Super", "F12"]);
        assert_eq!(shortcut_chips("alt+page_down"), ["Alt", "Page Down"]);
        assert!(shortcut_chips("").is_empty());
    }

    #[test]
    fn recorded_combo_uses_daemon_names() {
        let m = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        assert_eq!(combo_from_recording(m, "M").as_deref(), Some("ctrl+shift+m"));
        let s = Mods {
            super_: true,
            ..Mods::default()
        };
        assert_eq!(combo_from_recording(s, "F12").as_deref(), Some("super+f12"));
        assert_eq!(
            combo_from_recording(Mods::default(), "Return").as_deref(),
            Some("return")
        );
        // A modifier alone is not a shortcut yet.
        assert_eq!(combo_from_recording(m, "Control_L"), None);
        assert_eq!(combo_from_recording(s, "Super_L"), None);
        // Shifted symbols arrive as their own names.
        assert_eq!(
            combo_from_recording(Mods::default(), "Page_Down").as_deref(),
            Some("page_down")
        );
    }

    #[test]
    fn sinks_parse() {
        let json = r#"[{"index":55,"name":"alsa_output.usb-Focusrite","description":"Scarlett 2i2","state":"RUNNING"},
                      {"index":56,"name":"alsa_output.pci-hdmi","description":"HDMI"}]"#;
        assert_eq!(
            sinks_from_pactl_json(json),
            vec![
                (
                    "alsa_output.usb-Focusrite".to_string(),
                    "Scarlett 2i2".to_string()
                ),
                ("alsa_output.pci-hdmi".to_string(), "HDMI".to_string()),
            ]
        );
        assert!(sinks_from_pactl_json("not json").is_empty());
    }
}
