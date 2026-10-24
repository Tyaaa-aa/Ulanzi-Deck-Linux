//! Parameter rows for each kind of action, shown under the action card in the inspector, and the
//! action picker popover.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ulanzi_core::apps::{self, AppEntry};
use ulanzi_core::catalog;
use ulanzi_core::model::*;
use uuid::Uuid;

use crate::runtime;
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

/// The recorder button's state while it listens for a shortcut.
#[derive(Default)]
struct Recording {
    active: std::cell::Cell<bool>,
    recorder: RefCell<Recorder>,
    /// The toplevel whose system shortcuts are inhibited, restored when recording ends.
    inhibited: RefCell<Option<glib::WeakRef<gdk::Toplevel>>>,
    /// The app's accelerators, off while recording so any combination can be recorded.
    accels: RefCell<Option<crate::ui::key_routing::AccelGuard>>,
    inhibit: std::cell::Cell<InhibitState>,
    /// Handlers on the window and its surface while recording; disconnected when it ends.
    handlers: RefCell<Vec<(glib::WeakRef<glib::Object>, glib::SignalHandlerId)>>,
}

impl Recording {
    fn start(self: &Rc<Self>, b: &gtk::Button) {
        self.release();
        self.active.set(true);
        *self.recorder.borrow_mut() = Recorder::default();
        b.set_label("Press keys…");
        b.add_css_class("suggested-action");
        // The window's own keys (Escape closes the editor) step aside while recording.
        b.add_css_class(crate::ui::key_routing::RECORDING);
        b.grab_focus();
        // Another application taking focus ends recording (unless it is the inhibit prompt).
        if let Some(win) = b.root().and_downcast::<gtk::Window>() {
            let (rec, weak_b) = (Rc::downgrade(self), b.downgrade());
            let id = win.connect_is_active_notify(move |w| {
                let (Some(rec), Some(b)) = (rec.upgrade(), weak_b.upgrade()) else {
                    return;
                };
                if !rec.active.get() {
                    return;
                }
                let (state, stop) = on_window_active(rec.inhibit.get(), w.is_active());
                rec.inhibit.set(state);
                if stop {
                    rec.stop(&b);
                }
            });
            self.handlers
                .borrow_mut()
                .push((win.upcast_ref::<glib::Object>().downgrade(), id));
        }
        // Ctrl+L, Ctrl+O, Ctrl+comma and every other app shortcut: off until recording ends.
        if let Some(app) = b
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|w| w.application())
        {
            *self.accels.borrow_mut() = Some(crate::ui::key_routing::AccelGuard::park(&app));
        }
        // Let Super, Alt+Tab and friends reach the recorder instead of the desktop.
        if let Some(top) = b
            .native()
            .and_then(|n| n.surface())
            .and_downcast::<gdk::Toplevel>()
        {
            let rec = Rc::downgrade(self);
            let id = top.connect_shortcuts_inhibited_notify(move |_| {
                if let Some(rec) = rec.upgrade()
                    && rec.inhibit.get() == InhibitState::Waiting
                {
                    rec.inhibit.set(InhibitState::Answered);
                }
            });
            self.handlers
                .borrow_mut()
                .push((top.upcast_ref::<glib::Object>().downgrade(), id));
            self.inhibit.set(InhibitState::Waiting);
            top.inhibit_system_shortcuts(None::<&gdk::Event>);
            // X11 grabs the keyboard at once.
            if top.is_shortcuts_inhibited() {
                self.inhibit.set(InhibitState::Answered);
            }
            *self.inhibited.borrow_mut() = Some(top.downgrade());
        }
    }

    /// Ends recording for any reason; always gives the desktop its shortcuts back.
    fn stop(&self, b: &gtk::Button) {
        self.active.set(false);
        *self.recorder.borrow_mut() = Recorder::default();
        b.set_label("Record");
        b.remove_css_class("suggested-action");
        b.remove_css_class(crate::ui::key_routing::RECORDING);
        self.release();
    }

    /// Disconnects the recording-time handlers and restores the desktop's shortcuts.
    fn release(&self) {
        for (obj, id) in self.handlers.take() {
            if let Some(obj) = obj.upgrade() {
                obj.disconnect(id);
            }
        }
        self.inhibit.set(InhibitState::Off);
        self.accels.take();
        if let Some(top) = self.inhibited.take().and_then(|w| w.upgrade()) {
            top.restore_system_shortcuts();
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.release();
    }
}

/// The GDK name of the key itself, ignoring modifiers, so Shift+1 records as shift+1, not "exclam".
fn base_key_name(w: &impl IsA<gtk::Widget>, keyval: gdk::Key, code: u32) -> String {
    let base = w
        .display()
        .translate_key(code, gdk::ModifierType::empty(), 0)
        .map(|(k, ..)| k)
        .unwrap_or(keyval);
    base.name().map(|n| n.to_string()).unwrap_or_default()
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
    // Handlers on `record` get the button as an argument; none holds it, so nothing leaks.
    let rec = Rc::new(Recording::default());
    {
        let rec = rec.clone();
        record.connect_clicked(move |b| {
            if rec.active.get() {
                rec.stop(b);
            } else {
                rec.start(b);
            }
        });
    }
    {
        let rec = rec.clone();
        record.connect_has_focus_notify(move |b| {
            // Focus moving to another widget ends recording. The window going inactive is
            // handled by `Recording::start`'s is-active handler (GNOME's inhibit prompt).
            let window_active = b
                .root()
                .and_downcast::<gtk::Window>()
                .is_none_or(|w| w.is_active());
            if !b.has_focus() && window_active && rec.active.get() {
                rec.stop(b);
            }
        });
    }
    {
        let rec = rec.clone();
        record.connect_unmap(move |b| {
            if rec.active.get() {
                rec.stop(b);
            }
        });
    }
    let finish = {
        let (ed, caps) = (ed.clone(), caps.clone());
        move |rec: &Recording, b: &gtk::Button, step: RecordStep| match step {
            RecordStep::Wait => {}
            RecordStep::Cancel => rec.stop(b),
            RecordStep::Record(combo) => {
                fill_keycaps(&caps, &combo);
                ed.apply(How::Set, |a| {
                    if let Action::Shortcut { keys } = a {
                        *keys = combo;
                    }
                });
                rec.stop(b);
            }
        }
    };
    let finish = Rc::new(finish);
    let keys_ctl = gtk::EventControllerKey::new();
    keys_ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (rec, finish) = (rec.clone(), finish.clone());
        keys_ctl.connect_key_pressed(move |ctl, keyval, code, m| {
            let Some(b) = ctl.widget().and_downcast::<gtk::Button>() else {
                return glib::Propagation::Proceed;
            };
            if !rec.active.get() {
                return glib::Propagation::Proceed;
            }
            let name = base_key_name(&b, keyval, code);
            let step = rec.recorder.borrow_mut().press(mods_from_state(m), &name);
            finish(&rec, &b, step);
            glib::Propagation::Stop
        });
    }
    keys_ctl.connect_key_released(move |ctl, keyval, code, _| {
        let Some(b) = ctl.widget().and_downcast::<gtk::Button>() else {
            return;
        };
        if !rec.active.get() {
            return;
        }
        let name = base_key_name(&b, keyval, code);
        let step = rec.recorder.borrow_mut().release(&name);
        finish(&rec, &b, step);
    });
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
    let app = desktop_id.and_then(|d| apps.iter().find(|a| a.desktop_id == d));
    let row = adw::ActionRow::builder().title("App").build();
    let face: gtk::Widget = match (app, desktop_id) {
        (Some(a), _) => crate::ui::app_picker::app_label(Some(&a.desktop_id), &a.name, 20).upcast(),
        (None, Some(d)) => gtk::Label::new(Some(&format!(
            "{} (not installed)",
            d.trim_end_matches(".desktop")
        )))
        .upcast(),
        (None, None) => gtk::Label::new(Some("Choose an App…")).upcast(),
    };
    let ed2 = ed.clone();
    let picker = crate::ui::app_picker::button(&face, desktop_id.map(String::from), Vec::new(), move |a| {
        ed2.apply(How::Structure, |act| {
            if let Action::LaunchApp { desktop_id, exec } = act {
                *desktop_id = Some(a.desktop_id.clone());
                *exec = None;
            }
        })
    });
    row.add_suffix(&picker);
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
        AudioTarget::App { name } => rows.push(app_volume_row(ed, name)),
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
        let title = match &step.action {
            Action::Plugin { plugin, action, .. } => {
                glib::markup_escape_text(&state.plugins.borrow().action_title(plugin, action)).to_string()
            }
            a => catalog::spec_for(a)
                .map(|s| s.title)
                .unwrap_or("Action")
                .to_string(),
        };
        let exp = adw::ExpanderRow::builder()
            .title(format!("{}. {title}", i + 1))
            .subtitle(step_subtitle(state, step))
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
            let (ed, state) = (ed.clone(), state.clone());
            let exp = exp.clone();
            Rc::new(move |new: Action, how: How| {
                ed.apply(how, |a| {
                    if let Action::Multi { steps } = a
                        && let Some(s) = steps.get_mut(i)
                    {
                        s.action = new;
                        exp.set_subtitle(&step_subtitle(&state, s));
                    }
                })
            })
        };
        for r in rows_for(state, &step_path, &step.action, inner) {
            exp.add_row(&r);
        }
        let delay = {
            let (exp, state) = (exp.clone(), state.clone());
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
                        exp.set_subtitle(&step_subtitle(&state, s));
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

fn step_subtitle(state: &State, s: &Step) -> String {
    let d = match &s.action {
        // The title is the action's name; say which plugin it comes from.
        Action::Plugin { plugin, .. } => {
            glib::markup_escape_text(&state.plugins.borrow().plugin_name(plugin)).to_string()
        }
        a => catalog::describe(a),
    };
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
        Action::Seek { seconds, player } => vec![
            spin_row(
                &ed,
                "Seconds",
                if *seconds < 0 {
                    "How far back"
                } else {
                    "How far forward"
                },
                (1.0, 600.0, 5.0),
                seconds.unsigned_abs() as f64,
                "seek",
                |a, v| {
                    if let Action::Seek { seconds, .. } = a {
                        let sign = if *seconds < 0 { -1 } else { 1 };
                        *seconds = sign * v as i32;
                    }
                },
            ),
            entry_row(
                &ed,
                "Player (empty: the one playing)",
                player.as_deref().unwrap_or(""),
                "player",
                |a, t| {
                    if let Action::Seek { player, .. } = a {
                        *player = (!t.trim().is_empty()).then_some(t);
                    }
                },
            ),
        ],
        Action::SwitchOutput { devices } => output_rows(&ed, devices),
        Action::Brightness { op } => brightness_rows(&ed, *op),
        Action::GoToPage { page } | Action::OpenFolder { page } => {
            vec![page_combo(&ed, state, *page)]
        }
        Action::SwitchProfile { profile } => vec![profile_combo(&ed, state, *profile)],
        Action::Multi { steps } => multi_rows(&ed, state, path, steps),
        Action::Obs(o) => obs_rows(state, &ed, o),
        Action::Timer { op } => timer_rows(&ed, *op),
        Action::NextPage
        | Action::PrevPage
        | Action::Back
        | Action::AutoProfile
        | Action::SleepNow
        | Action::ScreenNext => Vec::new(),
        Action::Plugin {
            plugin,
            action,
            params,
        } => plugin_rows(state, &ed, plugin, action, params),
    }
}

/// The undo tag of a plugin parameter, so edits to different parameters never merge. Tags are
/// `'static`; each distinct key is leaked once.
fn undo_tag(key: &str) -> &'static str {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static TAGS: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new());
    }
    TAGS.with(|t| {
        *t.borrow_mut()
            .entry(key.to_string())
            .or_insert_with(|| Box::leak(format!("plugin:{key}").into_boxed_str()))
    })
}

/// One row per parameter the plugin's manifest declares for the action. Nothing when the plugin or
/// the action is unknown (the inspector's banner says why). Parameters the manifest doesn't name
/// are kept as they are.
fn plugin_rows(state: &State, ed: &Ed, plugin: &str, action: &str, params: &toml::Table) -> Vec<gtk::Widget> {
    use crate::plugins::{Control, control_for, current};
    // Cloned: the rows are built without holding the plugins borrowed.
    let Some(m) = state.plugins.borrow().manifests.get(plugin).cloned() else {
        return Vec::new();
    };
    let Some(a) = m.action(action) else {
        return Vec::new();
    };
    a.params
        .iter()
        .map(|f| {
            let value = current(f, params);
            let how = match control_for(f, &m.pickers, value.as_ref()) {
                Control::Entry | Control::Json | Control::Spin { .. } => How::Typing(undo_tag(&f.key)),
                _ => How::Set,
            };
            let (ed, key) = (ed.clone(), f.key.clone());
            let on: crate::ui::plugin_fields::OnValue = Rc::new(move |v| {
                // An error is shown on the row itself; nothing is written.
                let Ok(v) = v else { return };
                ed.apply(how, |a| {
                    if let Action::Plugin { params, .. } = a {
                        match v {
                            Some(v) => {
                                params.insert(key.clone(), v);
                            }
                            None => {
                                params.remove(&key);
                            }
                        }
                    }
                });
            });
            crate::ui::plugin_fields::row(
                state,
                &m,
                f,
                value.as_ref(),
                crate::ui::plugin_fields::Save::Live,
                on,
            )
        })
        .collect()
}

fn timer_rows(ed: &Ed, op: TimerOp) -> Vec<gtk::Widget> {
    let i = match op {
        TimerOp::StartPause => 0,
        TimerOp::Reset => 1,
        TimerOp::Add { .. } => 2,
    };
    let mut rows = vec![
        combo_row(
            ed,
            "Do",
            &["Start / Pause", "Reset", "Add Time"],
            i,
            How::Structure,
            |a, i| {
                if let Action::Timer { op } = a {
                    *op = match i {
                        0 => TimerOp::StartPause,
                        1 => TimerOp::Reset,
                        _ => TimerOp::Add { minutes: 5 },
                    };
                }
            },
        )
        .upcast(),
    ];
    if let TimerOp::Add { minutes } = op {
        rows.push(spin_row(
            ed,
            "Minutes",
            "",
            (1.0, 120.0, 1.0),
            minutes as f64,
            "timer-add",
            |a, v| {
                if let Action::Timer { op } = a {
                    *op = TimerOp::Add { minutes: v as u32 };
                }
            },
        ));
    }
    rows
}

/// A name from OBS: type it, or pick it from what OBS has now (`kind` as in the service's ObsList;
/// `arg` gives the scene or source the list belongs to).
#[allow(clippy::too_many_arguments)]
fn obs_name_row(
    state: &State,
    ed: &Ed,
    title: &str,
    value: &str,
    kind: &'static str,
    arg: impl Fn(&Action) -> String + 'static,
    key: &'static str,
    set: impl Fn(&mut Action, String) + 'static,
) -> gtk::Widget {
    let row = adw::EntryRow::builder().title(title).text(value).build();
    {
        let ed = ed.clone();
        row.connect_changed(move |r| {
            let t = r.text().to_string();
            ed.apply(How::Typing(key), |a| set(a, t));
        });
    }
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .propagate_natural_height(true)
        .max_content_height(320)
        .min_content_width(220)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let popover = gtk::Popover::builder().child(&scroller).build();
    {
        // Picking a name puts it in the field (the row's label is the name).
        let (entry, popover) = (row.downgrade(), popover.downgrade());
        list.connect_row_activated(move |_, r| {
            let name = r.child().and_downcast::<gtk::Label>().map(|l| l.label());
            if let (Some(name), Some(entry)) = (name, entry.upgrade()) {
                entry.set_text(&name);
            }
            if let Some(p) = popover.upgrade() {
                p.popdown();
            }
        });
    }
    let button = gtk::MenuButton::builder()
        .icon_name("view-list-symbolic")
        .tooltip_text("Choose from OBS")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .popover(&popover)
        .build();
    {
        let (state, ed) = (state.clone(), ed.clone());
        let arg = Rc::new(arg);
        popover.connect_show(move |_| {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let note = |text: &str| {
                gtk::Label::builder()
                    .label(text)
                    .wrap(true)
                    .max_width_chars(28)
                    .margin_top(8)
                    .margin_bottom(8)
                    .margin_start(8)
                    .margin_end(8)
                    .css_classes(["dim-label"])
                    .build()
            };
            let Some(c) = state.client.borrow().clone() else {
                list.append(&note("The service isn’t running"));
                return;
            };
            if !state.obs.borrow().connected() {
                let t = state.obs.borrow().title.clone();
                list.append(&note(&format!(
                    "OBS isn’t connected ({t}). Type the name instead."
                )));
                return;
            }
            list.append(&note("Loading…"));
            let a = arg(&ed.cur.borrow());
            let list = list.clone();
            runtime::run(async move { c.obs_list(kind, &a).await }, move |r| {
                while let Some(c) = list.first_child() {
                    list.remove(&c);
                }
                match r {
                    Ok(names) if names.is_empty() => list.append(&note("Nothing here in OBS")),
                    Ok(names) => {
                        for n in names.into_iter() {
                            let n: String = n;
                            let item = gtk::ListBoxRow::builder()
                                .child(&gtk::Label::builder().label(&n).xalign(0.0).build())
                                .build();
                            list.append(&item);
                        }
                    }
                    Err(e) => list.append(&note(&e.to_string())),
                }
            });
        });
    }
    row.add_suffix(&button);
    row.upcast()
}

fn obs_rows(state: &State, ed: &Ed, o: &ObsAction) -> Vec<gtk::Widget> {
    fn obs(a: &mut Action) -> Option<&mut ObsAction> {
        match a {
            Action::Obs(o) => Some(o),
            _ => None,
        }
    }
    fn obs_ref(a: &Action) -> Option<&ObsAction> {
        match a {
            Action::Obs(o) => Some(o),
            _ => None,
        }
    }
    let none = |_: &Action| String::new();
    let mut rows: Vec<gtk::Widget> = Vec::new();
    let info = state.obs.borrow().clone();
    if !info.connected() {
        let banner = adw::ActionRow::builder()
            .title("OBS isn’t connected")
            .subtitle(if info.title.is_empty() {
                "Connecting…".to_string()
            } else {
                info.title.clone()
            })
            .build();
        banner.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
        let setup = gtk::Button::builder()
            .label("Set Up…")
            .valign(gtk::Align::Center)
            .build();
        {
            let state = state.clone();
            setup.connect_clicked(move |b| crate::ui::obs_setup::open_dialog(&state, b));
        }
        banner.add_suffix(&setup);
        rows.push(banner.upcast());
    }
    let missing_scene = |scene: &str| -> Option<gtk::Widget> {
        (info.connected()
            && !scene.is_empty()
            && !info.scenes.is_empty()
            && !info.scenes.iter().any(|s| s == scene))
        .then(|| {
            let r = adw::ActionRow::builder()
                .title(format!("OBS has no scene “{scene}”"))
                .subtitle("Pick one from the list, or check the scene collection in OBS")
                .build();
            r.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            r.add_css_class("warning");
            r.upcast()
        })
    };
    match o {
        ObsAction::SetScene { scene, preview } => {
            rows.extend(missing_scene(scene));
            rows.push(obs_name_row(
                state,
                ed,
                "Scene",
                scene,
                "scenes",
                none,
                "obs-scene",
                |a, t| {
                    if let Some(ObsAction::SetScene { scene, .. }) = obs(a) {
                        *scene = t;
                    }
                },
            ));
            rows.push(switch_row(
                ed,
                "Preview Only",
                "In studio mode, put the scene on the preview instead of on air",
                *preview,
                |a, v| {
                    if let Some(ObsAction::SetScene { preview, .. }) = obs(a) {
                        *preview = v;
                    }
                },
            ));
        }
        ObsAction::ToggleScenes { a: first, b: second } => {
            rows.extend(missing_scene(first));
            rows.extend(missing_scene(second));
            rows.push(obs_name_row(
                state,
                ed,
                "First Scene",
                first,
                "scenes",
                none,
                "obs-a",
                |a, t| {
                    if let Some(ObsAction::ToggleScenes { a, .. }) = obs(a) {
                        *a = t;
                    }
                },
            ));
            rows.push(obs_name_row(
                state,
                ed,
                "Second Scene",
                second,
                "scenes",
                none,
                "obs-b",
                |a, t| {
                    if let Some(ObsAction::ToggleScenes { b, .. }) = obs(a) {
                        *b = t;
                    }
                },
            ));
        }
        ObsAction::ToggleSource { scene, source } => {
            rows.extend(missing_scene(scene));
            rows.push(obs_name_row(
                state,
                ed,
                "Scene",
                scene,
                "scenes",
                none,
                "obs-scene",
                |a, t| {
                    if let Some(ObsAction::ToggleSource { scene, .. }) = obs(a) {
                        *scene = t;
                    }
                },
            ));
            rows.push(obs_name_row(
                state,
                ed,
                "Source",
                source,
                "sources",
                |a| match obs_ref(a) {
                    Some(ObsAction::ToggleSource { scene, .. }) => scene.clone(),
                    _ => String::new(),
                },
                "obs-source",
                |a, t| {
                    if let Some(ObsAction::ToggleSource { source, .. }) = obs(a) {
                        *source = t;
                    }
                },
            ));
        }
        ObsAction::ToggleMute { input } => {
            rows.push(obs_name_row(
                state,
                ed,
                "Audio Input",
                input,
                "inputs",
                none,
                "obs-input",
                |a, t| {
                    if let Some(ObsAction::ToggleMute { input }) = obs(a) {
                        *input = t;
                    }
                },
            ));
        }
        ObsAction::InputVolume { input, op, step } => {
            rows.push(obs_name_row(
                state,
                ed,
                "Audio Input",
                input,
                "inputs",
                none,
                "obs-input",
                |a, t| {
                    if let Some(ObsAction::InputVolume { input, .. }) = obs(a) {
                        *input = t;
                    }
                },
            ));
            match op {
                VolumeOp::Set { percent } => rows.push(spin_row(
                    ed,
                    "Volume",
                    "Percent of full volume",
                    (0.0, 100.0, 5.0),
                    *percent as f64,
                    "obs-vol",
                    |a, v| {
                        if let Some(ObsAction::InputVolume { op, .. }) = obs(a) {
                            *op = VolumeOp::Set { percent: v as u8 };
                        }
                    },
                )),
                VolumeOp::Up | VolumeOp::Down => rows.push(spin_row(
                    ed,
                    "Step",
                    "Decibels per press or dial click",
                    (1.0, 20.0, 1.0),
                    *step as f64,
                    "obs-step",
                    |a, v| {
                        if let Some(ObsAction::InputVolume { step, .. }) = obs(a) {
                            *step = v as u8;
                        }
                    },
                )),
                VolumeOp::Mute => {}
            }
        }
        ObsAction::ToggleFilter { source, filter } => {
            rows.push(obs_name_row(
                state,
                ed,
                "Source",
                source,
                "inputs",
                none,
                "obs-fsource",
                |a, t| {
                    if let Some(ObsAction::ToggleFilter { source, .. }) = obs(a) {
                        *source = t;
                    }
                },
            ));
            rows.push(obs_name_row(
                state,
                ed,
                "Filter",
                filter,
                "filters",
                |a| match obs_ref(a) {
                    Some(ObsAction::ToggleFilter { source, .. }) => source.clone(),
                    _ => String::new(),
                },
                "obs-filter",
                |a, t| {
                    if let Some(ObsAction::ToggleFilter { filter, .. }) = obs(a) {
                        *filter = t;
                    }
                },
            ));
        }
        ObsAction::SetCollection { name } => {
            rows.push(obs_name_row(
                state,
                ed,
                "Scene Collection",
                name,
                "collections",
                none,
                "obs-coll",
                |a, t| {
                    if let Some(ObsAction::SetCollection { name }) = obs(a) {
                        *name = t;
                    }
                },
            ));
        }
        ObsAction::SetProfile { name } => {
            rows.push(obs_name_row(
                state,
                ed,
                "OBS Profile",
                name,
                "profiles",
                none,
                "obs-prof",
                |a, t| {
                    if let Some(ObsAction::SetProfile { name }) = obs(a) {
                        *name = t;
                    }
                },
            ));
        }
        ObsAction::Screenshot => rows.push(
            adw::ActionRow::builder()
                .subtitle("Saved where OBS saves screenshots (Settings → Advanced → Recording)")
                .css_classes(["dim-label"])
                .build()
                .upcast(),
        ),
        _ => {}
    }
    rows
}

/// Apps playing sound right now (their names as the sound system knows them), newest first.
fn apps_with_sound() -> Vec<String> {
    let out = std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sink-inputs"])
        .output()
        .ok();
    let v: serde_json::Value = out
        .and_then(|o| serde_json::from_slice(&o.stdout).ok())
        .unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for it in v.as_array().into_iter().flatten().rev() {
        let props = it.get("properties");
        let n = ["application.name", "application.process.binary"]
            .iter()
            .find_map(|k| props.and_then(|p| p.get(*k)).and_then(|v| v.as_str()));
        if let Some(n) = n {
            let n = n.split(" (").next().unwrap_or(n).trim().to_string();
            if !n.is_empty() && !names.contains(&n) {
                names.push(n);
            }
        }
    }
    names
}

/// The installed app whose name matches, for its icon.
fn installed_app_named(name: &str) -> Option<AppEntry> {
    let n = name.to_lowercase();
    installed_apps()
        .iter()
        .find(|a| a.name.to_lowercase() == n || a.desktop_id.trim_end_matches(".desktop").to_lowercase() == n)
        .cloned()
}

/// Which app's volume: pick an installed app, or one that is playing sound right now.
fn app_volume_row(ed: &Ed, name: &str) -> gtk::Widget {
    let row = adw::ActionRow::builder()
        .title("App")
        .subtitle(if name.is_empty() {
            "Choose the app whose volume this changes".to_string()
        } else {
            "Matched by name while the app plays sound".to_string()
        })
        .build();
    let found = installed_app_named(name);
    let shown = crate::ui::app_picker::app_label(
        found.as_ref().map(|a| a.desktop_id.as_str()),
        if name.is_empty() { "Choose App…" } else { name },
        20,
    );
    let set = {
        let ed = ed.clone();
        Rc::new(move |n: String| {
            ed.apply(How::Structure, |a| {
                if let Action::Volume {
                    target: AudioTarget::App { name },
                    ..
                } = a
                {
                    *name = n;
                }
            })
        })
    };
    let picker = {
        let set = set.clone();
        crate::ui::app_picker::button(&shown, found.map(|a| a.desktop_id), Vec::new(), move |app| {
            set(app.name)
        })
    };
    // Apps making sound now, as the sound system names them (always matches).
    let now = gtk::MenuButton::builder()
        .icon_name("audio-volume-high-symbolic")
        .tooltip_text("Apps playing sound now")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let popover = gtk::Popover::builder().child(&list).build();
    now.set_popover(Some(&popover));
    {
        let list = list.clone();
        popover.connect_show(move |_| {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let names = apps_with_sound();
            if names.is_empty() {
                list.append(
                    &gtk::Label::builder()
                        .label("No app is playing sound right now")
                        .margin_top(8)
                        .margin_bottom(8)
                        .margin_start(8)
                        .margin_end(8)
                        .css_classes(["dim-label"])
                        .build(),
                );
            }
            for n in names {
                let app = installed_app_named(&n);
                let item =
                    crate::ui::app_picker::app_label(app.as_ref().map(|a| a.desktop_id.as_str()), &n, 20);
                item.set_margin_top(4);
                item.set_margin_bottom(4);
                list.append(&gtk::ListBoxRow::builder().child(&item).name(n.as_str()).build());
            }
        });
    }
    {
        let popover = popover.downgrade();
        list.connect_row_activated(move |_, r| {
            if let Some(p) = popover.upgrade() {
                p.popdown();
            }
            set(r.widget_name().to_string());
        });
    }
    row.add_suffix(&now);
    row.add_suffix(&picker);
    row.upcast()
}

/// Outputs (name, description) from `pactl`, for the Switch Output checklist.
fn audio_outputs() -> Vec<(String, String)> {
    std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sinks"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
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

fn output_rows(ed: &Ed, devices: &[String]) -> Vec<gtk::Widget> {
    let mut rows: Vec<gtk::Widget> = vec![
        adw::ActionRow::builder()
            .subtitle("Each press makes the next ticked output the default. None ticked: every output.")
            .css_classes(["dim-label"])
            .build()
            .upcast(),
    ];
    let mut known = audio_outputs();
    // Chosen outputs that aren't plugged in now still show, so they can be unticked.
    for d in devices {
        if !known.iter().any(|(n, _)| n == d) {
            known.push((d.clone(), format!("{d} (not connected)")));
        }
    }
    if known.is_empty() {
        rows.push(
            adw::ActionRow::builder()
                .title("No outputs found")
                .subtitle("Switching outputs needs pactl (pipewire-pulse)")
                .build()
                .upcast(),
        );
    }
    for (name, desc) in known {
        let on = devices.contains(&name);
        rows.push(switch_row(ed, &desc, "", on, move |a, v| {
            if let Action::SwitchOutput { devices } = a {
                devices.retain(|d| *d != name);
                if v {
                    devices.push(name.clone());
                }
            }
        }));
    }
    rows
}

/// A small rendered key showing the action's default look (or an empty slot).
pub fn action_chip(state: &State, action: Option<&Action>, size: i32) -> gtk::Widget {
    // Plugin actions have no look of their own yet: every plugin uses the add-on icon.
    if let Some(Action::Plugin { .. }) = action {
        let i = gtk::Image::from_icon_name("application-x-addon-symbolic");
        i.set_pixel_size(size);
        i.set_size_request(size, size);
        i.set_valign(gtk::Align::Center);
        return i.upcast();
    }
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

/// A button that opens the action list (search, categories, sections); `on_pick` gets a fresh
/// action.
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
    // The list is owned by the popover; its pick handler holds the popover and button weakly.
    let pick = {
        let (popover, state, button) = (popover.downgrade(), state.clone(), button.downgrade());
        move |a: Action| {
            if let Some(p) = popover.upgrade() {
                p.popdown();
            }
            let is_obs = matches!(a, Action::Obs(_));
            on_pick(a);
            // The first OBS action while OBS is away: show how to connect.
            if is_obs && let Some(b) = button.upgrade() {
                crate::ui::obs_setup::offer_once(&state, &b);
            }
        }
    };
    let list = crate::ui::library::action_list(
        state,
        crate::ui::library::Use::Picker {
            include_multi: allow_multi,
            on_pick: Box::new(pick),
        },
    );
    list.scroller.set_vexpand(false);
    list.scroller.set_min_content_height(320);
    list.scroller.set_max_content_height(420);
    list.scroller.set_min_content_width(300);
    list.scroller.set_propagate_natural_height(true);
    list.root.set_width_request(320);
    popover.set_child(Some(&list.root));
    button.set_popover(Some(&popover));
    {
        // Plugins come and go: the list is read again each time the popover opens.
        let (search, reload) = (list.search.clone(), list.reload.clone());
        popover.connect_show(move |_| {
            if !search.text().is_empty() {
                search.set_text("");
            }
            reload();
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
            "super" | "meta" | "win" | "hyper" => "Super".into(),
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

impl Mods {
    fn is_empty(self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.super_)
    }

    fn with(self, o: Mods) -> Mods {
        Mods {
            ctrl: self.ctrl || o.ctrl,
            alt: self.alt || o.alt,
            shift: self.shift || o.shift,
            super_: self.super_ || o.super_,
        }
    }

    /// The combo naming only these modifiers (`ctrl+super`), or None when there are none.
    fn combo(self) -> Option<String> {
        let parts: Vec<&str> = [
            (self.ctrl, "ctrl"),
            (self.alt, "alt"),
            (self.shift, "shift"),
            (self.super_, "super"),
        ]
        .into_iter()
        .filter_map(|(on, n)| on.then_some(n))
        .collect();
        (!parts.is_empty()).then(|| parts.join("+"))
    }
}

/// The modifiers held in a GDK event state. Super, Meta and Hyper all record as `super`
/// (the daemon's Left Meta key). Meta together with Alt is Alt alone: some keymaps put the
/// virtual Meta modifier on the Alt key, which would otherwise turn every Alt+X into Alt+Super+X.
pub fn mods_from_state(m: gdk::ModifierType) -> Mods {
    use gdk::ModifierType as M;
    let alt = m.contains(M::ALT_MASK);
    Mods {
        ctrl: m.contains(M::CONTROL_MASK),
        alt,
        shift: m.contains(M::SHIFT_MASK),
        super_: m.intersects(M::SUPER_MASK | M::HYPER_MASK) || (m.contains(M::META_MASK) && !alt),
    }
}

/// For a modifier key (GDK key name), the modifier it adds; `Some(Mods::default())` for a
/// modifier the daemon has no plain name for (AltGr). None for any other key.
pub fn modifier_of(key: &str) -> Option<Mods> {
    let none = Mods::default();
    Some(match key.to_ascii_lowercase().as_str() {
        "control_l" | "control_r" => Mods { ctrl: true, ..none },
        "alt_l" | "alt_r" => Mods { alt: true, ..none },
        "shift_l" | "shift_r" => Mods { shift: true, ..none },
        "super_l" | "super_r" | "meta_l" | "meta_r" | "hyper_l" | "hyper_r" => Mods { super_: true, ..none },
        k if MODIFIER_KEYS.contains(&k) => none,
        _ => return None,
    })
}

/// What the recorder does after a key event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordStep {
    /// Keep listening.
    Wait,
    /// Stop without changing the shortcut (Escape with no modifiers).
    Cancel,
    /// Store this combo and stop.
    Record(String),
}

/// Where the request to pause the desktop's shortcuts stands while recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InhibitState {
    /// Not requested (no toplevel surface).
    #[default]
    Off,
    /// Requested, no answer yet: the compositor may be asking the user (GNOME's prompt).
    Waiting,
    /// Granted or denied.
    Answered,
}

/// What the window's activation changing means for recording: the new inhibit state, and
/// whether to stop. Losing activation stops recording unless the inhibit request is still
/// waiting (its prompt takes focus); coming back while waiting means the prompt was answered.
pub fn on_window_active(state: InhibitState, active: bool) -> (InhibitState, bool) {
    match (state, active) {
        (InhibitState::Waiting, true) => (InhibitState::Answered, false),
        (InhibitState::Waiting, false) => (state, false),
        (_, active) => (state, !active),
    }
}

/// Key-combo recorder logic. A modifier pressed and released with no other key in between
/// records that modifier (with any others held), so Super alone is a valid shortcut.
#[derive(Debug, Default)]
pub struct Recorder {
    /// The modifiers held since the last modifier press, while no other key has been pressed.
    pending: Option<Mods>,
    /// Modifier keys physically held (lowercase GDK names), seen pressed since recording began.
    held: Vec<String>,
}

impl Recorder {
    /// A key press; `mods` are the modifiers held before it (GDK leaves the pressed modifier out).
    pub fn press(&mut self, mods: Mods, key: &str) -> RecordStep {
        // Trust the keys held as well as the event state: some keymaps report Meta as Alt.
        let mods = self.held_mods().with(mods);
        if key == "Escape" && mods.is_empty() {
            self.pending = None;
            return RecordStep::Cancel;
        }
        if let Some(m) = modifier_of(key) {
            let k = key.to_ascii_lowercase();
            if !self.held.contains(&k) {
                self.held.push(k);
            }
            self.pending = Some(self.pending.unwrap_or_default().with(mods).with(m));
            return RecordStep::Wait;
        }
        self.pending = None;
        match combo_from_recording(mods, key) {
            Some(combo) => RecordStep::Record(combo),
            None => RecordStep::Wait,
        }
    }

    /// A key release.
    pub fn release(&mut self, key: &str) -> RecordStep {
        if modifier_of(key).is_none() {
            return RecordStep::Wait;
        }
        let k = key.to_ascii_lowercase();
        self.held.retain(|h| *h != k);
        match self.pending.take().and_then(Mods::combo) {
            Some(combo) => RecordStep::Record(combo),
            None => RecordStep::Wait,
        }
    }

    fn held_mods(&self) -> Mods {
        self.held
            .iter()
            .filter_map(|k| modifier_of(k))
            .fold(Mods::default(), Mods::with)
    }
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
    Some(match mods.combo() {
        Some(m) => format!("{m}+{key}"),
        None => key,
    })
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
    fn state_maps_super_meta_hyper_to_super() {
        use gdk::ModifierType as M;
        let sup = Mods {
            super_: true,
            ..Mods::default()
        };
        assert_eq!(mods_from_state(M::SUPER_MASK), sup);
        assert_eq!(mods_from_state(M::META_MASK), sup);
        assert_eq!(mods_from_state(M::HYPER_MASK), sup);
        assert_eq!(
            mods_from_state(M::CONTROL_MASK | M::SHIFT_MASK),
            Mods {
                ctrl: true,
                shift: true,
                ..Mods::default()
            }
        );
        // Meta riding on the Alt key is just Alt.
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        assert_eq!(mods_from_state(M::ALT_MASK | M::META_MASK), alt);
        assert_eq!(
            mods_from_state(M::ALT_MASK | M::META_MASK | M::SUPER_MASK),
            alt.with(sup)
        );
        assert_eq!(mods_from_state(M::BUTTON1_MASK | M::LOCK_MASK), Mods::default());
    }

    #[test]
    fn modifier_keys_are_known() {
        let sup = Some(Mods {
            super_: true,
            ..Mods::default()
        });
        for k in ["Super_L", "Super_R", "Meta_L", "Meta_R", "Hyper_L", "Hyper_R"] {
            assert_eq!(modifier_of(k), sup, "{k}");
        }
        assert_eq!(
            modifier_of("Control_R"),
            Some(Mods {
                ctrl: true,
                ..Mods::default()
            })
        );
        assert_eq!(
            modifier_of("Alt_L"),
            Some(Mods {
                alt: true,
                ..Mods::default()
            })
        );
        assert_eq!(
            modifier_of("Shift_R"),
            Some(Mods {
                shift: true,
                ..Mods::default()
            })
        );
        assert_eq!(modifier_of("ISO_Level3_Shift"), Some(Mods::default()));
        assert_eq!(modifier_of("a"), None);
        assert_eq!(modifier_of("Escape"), None);
    }

    #[test]
    fn super_alone_records_on_release() {
        let mut r = Recorder::default();
        assert_eq!(r.press(Mods::default(), "Super_L"), RecordStep::Wait);
        assert_eq!(r.release("Super_L"), RecordStep::Record("super".into()));
        for (k, want) in [
            ("Control_L", "ctrl"),
            ("Alt_R", "alt"),
            ("Shift_L", "shift"),
            ("Meta_R", "super"),
        ] {
            let mut r = Recorder::default();
            assert_eq!(r.press(Mods::default(), k), RecordStep::Wait);
            assert_eq!(r.release(k), RecordStep::Record(want.into()), "{k}");
        }
    }

    #[test]
    fn super_then_key_records_the_combo() {
        let sup = Mods {
            super_: true,
            ..Mods::default()
        };
        let mut r = Recorder::default();
        assert_eq!(r.press(Mods::default(), "Super_L"), RecordStep::Wait);
        assert_eq!(r.press(sup, "x"), RecordStep::Record("super+x".into()));
        // Releasing Super after a key was pressed records nothing more.
        assert_eq!(r.release("Super_L"), RecordStep::Wait);
        // Ctrl+Alt+Shift+Super+F5
        let all = Mods {
            ctrl: true,
            alt: true,
            shift: true,
            super_: true,
        };
        let mut r = Recorder::default();
        assert_eq!(
            r.press(all, "F5"),
            RecordStep::Record("ctrl+alt+shift+super+f5".into())
        );
    }

    #[test]
    fn several_modifiers_alone_record_together() {
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        let mut r = Recorder::default();
        assert_eq!(r.press(Mods::default(), "Control_L"), RecordStep::Wait);
        assert_eq!(r.press(ctrl, "Super_L"), RecordStep::Wait);
        assert_eq!(r.release("Super_L"), RecordStep::Record("ctrl+super".into()));
    }

    #[test]
    fn escape_cancels_unless_modified() {
        let mut r = Recorder::default();
        assert_eq!(r.press(Mods::default(), "Escape"), RecordStep::Cancel);
        let ctrl = Mods {
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(r.press(ctrl, "Escape"), RecordStep::Record("ctrl+escape".into()));
    }

    #[test]
    fn stray_releases_and_altgr_wait() {
        let mut r = Recorder::default();
        // A release with nothing pressed since recording began (the click's own keys).
        assert_eq!(r.release("Return"), RecordStep::Wait);
        assert_eq!(r.release("Super_L"), RecordStep::Wait);
        // AltGr alone has no daemon name: keep listening.
        assert_eq!(r.press(Mods::default(), "ISO_Level3_Shift"), RecordStep::Wait);
        assert_eq!(r.release("ISO_Level3_Shift"), RecordStep::Wait);
        // A key with no name keeps listening too.
        assert_eq!(r.press(Mods::default(), ""), RecordStep::Wait);
    }

    #[test]
    fn deactivation_stops_unless_inhibit_prompt_is_open() {
        use InhibitState::*;
        assert_eq!(on_window_active(Answered, false), (Answered, true));
        assert_eq!(on_window_active(Off, false), (Off, true));
        assert_eq!(on_window_active(Waiting, false), (Waiting, false));
        // Back from the prompt: answered, and the next deactivation stops.
        assert_eq!(on_window_active(Waiting, true), (Answered, false));
        assert_eq!(on_window_active(Answered, true), (Answered, false));
        assert_eq!(on_window_active(Off, true), (Off, false));
    }

    #[test]
    fn held_super_key_adds_super() {
        let alt = Mods {
            alt: true,
            ..Mods::default()
        };
        // Meta sharing Mod1 with Alt: the event state says only Alt, the key held says Super.
        let mut r = Recorder::default();
        assert_eq!(r.press(Mods::default(), "Meta_L"), RecordStep::Wait);
        assert_eq!(r.press(alt, "x"), RecordStep::Record("alt+super+x".into()));
        // A state that lost Super entirely.
        let mut r = Recorder::default();
        assert_eq!(r.press(Mods::default(), "Super_R"), RecordStep::Wait);
        assert_eq!(
            r.press(Mods::default(), "x"),
            RecordStep::Record("super+x".into())
        );
        // A held modifier also keeps Escape from cancelling.
        let mut r = Recorder::default();
        assert_eq!(r.press(Mods::default(), "Super_L"), RecordStep::Wait);
        assert_eq!(
            r.press(Mods::default(), "Escape"),
            RecordStep::Record("super+escape".into())
        );
    }

    #[test]
    fn released_modifiers_are_no_longer_held() {
        let mut r = Recorder::default();
        assert_eq!(r.press(Mods::default(), "Control_L"), RecordStep::Wait);
        assert_eq!(r.release("Control_L"), RecordStep::Record("ctrl".into()));
        assert_eq!(r.press(Mods::default(), "x"), RecordStep::Record("x".into()));
        // Left and right are tracked apart.
        let mut r = Recorder::default();
        r.press(Mods::default(), "Super_L");
        r.press(Mods::default(), "Super_R");
        r.release("Super_L");
        assert_eq!(
            r.press(Mods::default(), "x"),
            RecordStep::Record("super+x".into())
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
