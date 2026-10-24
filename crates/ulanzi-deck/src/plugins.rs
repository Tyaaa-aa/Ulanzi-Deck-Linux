//! What the app knows about the daemon's plugins, and the form model for plugin settings and action
//! parameters. Pure Rust (no GTK) so it can be unit-tested.

use std::collections::BTreeMap;

use ulanzi_core::model::Action;
use ulanzi_core::plugin::{Field, FieldKind, Manifest, ManifestAction};

/// One installed plugin (or plugin folder that could not be loaded), as `PluginList` reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginRow {
    pub id: String,
    pub name: String,
    pub version: String,
    /// `off`, `ready`, `starting`, `running`, `problem`, `failed` or `incompatible`.
    pub status: String,
    /// The reason or message for `problem`, `failed` and `incompatible`.
    pub detail: String,
}

impl PluginRow {
    /// The plugin's name, or its id when it has none (a folder that could not be read).
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

/// The daemon's plugins as last fetched.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plugins {
    /// The list has arrived since the service (re)connected. Until then no key shows a plugin
    /// badge: the service or its plugin host may still be coming up.
    pub loaded: bool,
    pub list: Vec<PluginRow>,
    pub manifests: BTreeMap<String, Manifest>,
}

impl Plugins {
    pub fn row(&self, id: &str) -> Option<&PluginRow> {
        self.list.iter().find(|r| r.id == id)
    }

    /// Why the plugin's action can't run, in words for a key badge; `None` when it looks fine or
    /// the list is not known yet.
    pub fn action_problem(&self, plugin: &str, action: &str) -> Option<String> {
        if !self.loaded {
            return None;
        }
        let Some(row) = self.row(plugin) else {
            return Some(format!("Needs the “{plugin}” plugin"));
        };
        let name = row.display_name();
        match row.status.as_str() {
            "off" => Some(format!("The {name} plugin is turned off")),
            "failed" => Some(format!("{name} stopped working")),
            "incompatible" if row.detail.is_empty() => Some(format!("{name} can’t be loaded")),
            "incompatible" => Some(row.detail.clone()),
            _ => self
                .manifests
                .get(plugin)
                .filter(|m| m.action(action).is_none())
                .map(|_| format!("{name} has no action “{action}”")),
        }
    }

    /// The action's name from the manifest, else `<action> (<plugin>)`.
    pub fn action_title(&self, plugin: &str, action: &str) -> String {
        self.manifests
            .get(plugin)
            .and_then(|m| m.action(action))
            .map(|a| a.name.clone())
            .unwrap_or_else(|| format!("{action} ({plugin})"))
    }

    /// Whether going from `self` to `now` changes anything the key editor shows: the manifests, a
    /// plugin's name or a key badge. A status flip between running, ready, starting and problem
    /// does not: a plugin that retries its connection flips like that every few seconds, and a
    /// rebuild would close an open action picker and drop the typing focus.
    pub fn editor_differs(&self, now: &Plugins) -> bool {
        fn view(p: &Plugins) -> Vec<(&str, &str, &str, &str)> {
            p.list
                .iter()
                .map(|r| {
                    let (badge, detail) = match r.status.as_str() {
                        s @ ("off" | "failed") => (s, ""),
                        "incompatible" => ("incompatible", r.detail.as_str()),
                        _ => ("", ""),
                    };
                    (r.id.as_str(), r.display_name(), badge, detail)
                })
                .collect()
        }
        self.loaded != now.loaded || self.manifests != now.manifests || view(self) != view(now)
    }

    /// The plugin's name: from the list, else its manifest, else its id.
    pub fn plugin_name(&self, plugin: &str) -> String {
        self.row(plugin)
            .map(|r| r.display_name().to_string())
            .or_else(|| self.manifests.get(plugin).map(|m| m.name.clone()))
            .unwrap_or_else(|| plugin.to_string())
    }

    /// One-line summary of an action for tooltips and collapsed rows: a plugin action's name, else
    /// [`ulanzi_core::catalog::describe`].
    pub fn describe(&self, a: &Action) -> String {
        match a {
            Action::Plugin { plugin, action, .. } => self.action_title(plugin, action),
            _ => ulanzi_core::catalog::describe(a),
        }
    }

    /// Plugins with actions, by name: (id, name, actions). Plugins that can't be loaded are left
    /// out.
    pub fn actions_by_plugin(&self) -> Vec<(String, String, Vec<ManifestAction>)> {
        let mut out: Vec<(String, String, Vec<ManifestAction>)> = self
            .manifests
            .iter()
            .filter(|(id, m)| {
                !m.actions.is_empty() && self.row(id).is_none_or(|r| r.status != "incompatible")
            })
            .map(|(id, m)| (id.clone(), m.name.clone(), m.actions.clone()))
            .collect();
        out.sort_by(|a, b| {
            a.1.to_lowercase()
                .cmp(&b.1.to_lowercase())
                .then_with(|| a.0.cmp(&b.0))
        });
        out
    }
}

/// One plugin's section in the action picker.
#[derive(Debug, Clone, PartialEq)]
pub struct PickerGroup {
    /// The plugin's name, the section heading.
    pub name: String,
    pub items: Vec<PickerItem>,
}

/// One plugin action in the action picker.
#[derive(Debug, Clone, PartialEq)]
pub struct PickerItem {
    pub title: String,
    /// What the search box matches: `<plugin name> <action name>`, lower case.
    pub search: String,
    /// The action a pick adds, with the parameters' defaults.
    pub action: Action,
}

impl Plugins {
    /// The action picker's plugin sections, in [`Plugins::actions_by_plugin`] order.
    pub fn picker_groups(&self) -> Vec<PickerGroup> {
        self.actions_by_plugin()
            .into_iter()
            .map(|(id, name, actions)| PickerGroup {
                items: actions
                    .iter()
                    .map(|a| PickerItem {
                        title: a.name.clone(),
                        search: format!("{name} {}", a.name).to_lowercase(),
                        action: Action::Plugin {
                            plugin: id.clone(),
                            action: a.id.clone(),
                            params: defaults(a),
                        },
                    })
                    .collect(),
                name,
            })
            .collect()
    }
}

/// A plugin's status in words, with the icon and style class shown next to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusWords {
    pub title: String,
    pub subtitle: String,
    pub icon: &'static str,
    pub css: &'static str,
}

pub fn status_words(row: &PluginRow) -> StatusWords {
    const OK: (&str, &str) = ("emblem-ok-symbolic", "success");
    const PAUSED: (&str, &str) = ("media-playback-pause-symbolic", "dim-label");
    const LOADING: (&str, &str) = ("content-loading-symbolic", "dim-label");
    const WARNING: (&str, &str) = ("dialog-warning-symbolic", "warning");
    let (title, detail, (icon, css)) = match row.status.as_str() {
        "running" => ("Running", false, OK),
        "ready" => ("Ready: starts when a key uses it", false, PAUSED),
        "starting" => ("Starting…", false, LOADING),
        "off" => ("Turned off", false, PAUSED),
        "failed" => ("Stopped working", true, WARNING),
        "incompatible" => ("Can’t be loaded", true, WARNING),
        _ => ("Needs attention", true, WARNING),
    };
    StatusWords {
        title: title.into(),
        subtitle: if detail { row.detail.clone() } else { String::new() },
        icon,
        css,
    }
}

/// The banner on the plugin's settings page: (text, button label).
pub fn banner(row: &PluginRow) -> Option<(String, Option<&'static str>)> {
    let detail = |fallback: String| {
        if row.detail.trim().is_empty() {
            fallback
        } else {
            row.detail.clone()
        }
    };
    match row.status.as_str() {
        "failed" => Some((format!("{} stopped working", row.display_name()), Some("Restart"))),
        "problem" => Some((detail("Needs attention".into()), Some("Restart"))),
        "incompatible" => Some((detail(format!("{} can’t be loaded", row.display_name())), None)),
        _ => None,
    }
}

/// The row that edits a field.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    Entry,
    Password,
    Spin {
        min: f64,
        max: f64,
        step: f64,
        digits: u32,
    },
    Switch,
    Combo {
        labels: Vec<String>,
        values: Vec<String>,
    },
    Color,
    Json,
    /// The value has a type the app can't edit; it is shown and kept as it is.
    ReadOnly,
}

/// The widest range a number row allows when the field gives no bounds.
const UNBOUNDED: f64 = 1e9;

pub fn control_for(f: &Field, current: Option<&toml::Value>) -> Control {
    let textual = matches!(current, None | Some(toml::Value::String(_)));
    match &f.kind {
        FieldKind::Text => match current {
            Some(toml::Value::Table(_)) => Control::ReadOnly,
            _ => Control::Entry,
        },
        FieldKind::Secret => Control::Password,
        FieldKind::Number => {
            let int = f.is_integer();
            Control::Spin {
                min: f.min.unwrap_or(-UNBOUNDED),
                max: f.max.unwrap_or(UNBOUNDED),
                step: f.step.unwrap_or(if int { 1.0 } else { 0.1 }),
                digits: if int { 0 } else { 2 },
            }
        }
        FieldKind::Toggle => Control::Switch,
        FieldKind::Choice => Control::Combo {
            labels: f.choices.iter().map(|c| c.label()).collect(),
            values: f.choices.iter().map(|c| c.value().to_string()).collect(),
        },
        FieldKind::Color => Control::Color,
        FieldKind::Json => Control::Json,
        FieldKind::Item | FieldKind::KeyCombo | FieldKind::File | FieldKind::Other(_) => {
            if textual {
                Control::Entry
            } else {
                Control::ReadOnly
            }
        }
    }
}

/// The field's value in `table`, else its default.
pub fn current(f: &Field, table: &toml::Table) -> Option<toml::Value> {
    table.get(&f.key).cloned().or_else(|| f.default.clone())
}

/// A number without a trailing `.0` when it is whole.
fn number_text(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

/// A value as text for an entry or a read-only row; arrays are joined by `, `.
pub fn shown_text(v: Option<&toml::Value>) -> String {
    match v {
        None => String::new(),
        Some(toml::Value::String(s)) => s.clone(),
        Some(toml::Value::Integer(i)) => i.to_string(),
        Some(toml::Value::Float(f)) => number_text(*f),
        Some(toml::Value::Boolean(b)) => b.to_string(),
        Some(toml::Value::Datetime(d)) => d.to_string(),
        Some(toml::Value::Array(a)) => a
            .iter()
            .map(|x| shown_text(Some(x)))
            .collect::<Vec<_>>()
            .join(", "),
        Some(v @ toml::Value::Table(_)) => v.to_string(),
    }
}

/// What the user entered in a row.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Text(String),
    Number(f64),
    Bool(bool),
    /// Index into the field's choices.
    Choice(usize),
    /// `#rrggbb` (or `#rgb`, `#rrggbbaa`).
    Color(String),
}

fn is_colour(s: &str) -> bool {
    s.strip_prefix('#')
        .is_some_and(|hex| matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit()))
}

/// The value to store for the field; `Ok(None)` removes the key (blank text); `Err` is the
/// message shown under the row. Secret fields are refused: they go straight into the plugin's secret store
/// (`PluginSetSecret`), never into `settings.toml`.
pub fn value_from(f: &Field, input: Input) -> Result<Option<toml::Value>, String> {
    if f.kind == FieldKind::Secret {
        return Err("Secrets are saved separately".into());
    }
    match input {
        Input::Text(s) if s.trim().is_empty() => Ok(None),
        Input::Text(s) => {
            if f.kind == FieldKind::Json
                && let Err(e) = serde_json::from_str::<serde_json::Value>(&s)
            {
                return Err(format!("Not valid JSON: {e}"));
            }
            Ok(Some(toml::Value::String(s)))
        }
        Input::Number(n) => {
            let (min, max) = (f.min.unwrap_or(-UNBOUNDED), f.max.unwrap_or(UNBOUNDED));
            if !n.is_finite() || n < min || n > max {
                return Err(format!(
                    "Must be between {} and {}",
                    number_text(min),
                    number_text(max)
                ));
            }
            Ok(Some(if f.is_integer() {
                toml::Value::Integer(n.round() as i64)
            } else {
                toml::Value::Float(n)
            }))
        }
        Input::Bool(b) => Ok(Some(toml::Value::Boolean(b))),
        Input::Choice(i) => f
            .choices
            .get(i)
            .map(|c| Some(toml::Value::String(c.value().to_string())))
            .ok_or_else(|| "Not one of the choices".to_string()),
        Input::Color(s) if s.trim().is_empty() => Ok(None),
        Input::Color(s) if is_colour(s.trim()) => Ok(Some(toml::Value::String(s.trim().to_string()))),
        Input::Color(_) => Err("Not a colour".into()),
    }
}

/// What a parameter's row shows when nothing is stored: the manifest default, else the first
/// choice of a choice field, else the number a spin row falls back to (0 within the bounds).
/// A newly added action stores exactly this, so what the rows show is what the key does.
pub fn initial_value(f: &Field) -> Option<toml::Value> {
    if let Some(d) = &f.default {
        return Some(d.clone());
    }
    match &f.kind {
        FieldKind::Choice => f
            .choices
            .first()
            .map(|c| toml::Value::String(c.value().to_string())),
        FieldKind::Number => {
            let n = 0.0_f64
                .max(f.min.unwrap_or(-UNBOUNDED))
                .min(f.max.unwrap_or(UNBOUNDED));
            Some(if f.is_integer() {
                toml::Value::Integer(n.round() as i64)
            } else {
                toml::Value::Float(n)
            })
        }
        _ => None,
    }
}

/// The action's parameters as its rows show them, for a newly added action.
pub fn defaults(a: &ManifestAction) -> toml::Table {
    a.params
        .iter()
        .filter_map(|p| initial_value(p).map(|d| (p.key.clone(), d)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulanzi_core::plugin::Choice;

    fn field(key: &str, kind: FieldKind) -> Field {
        Field {
            key: key.into(),
            kind,
            label: String::new(),
            help: String::new(),
            default: None,
            min: None,
            max: None,
            step: None,
            choices: Vec::new(),
            picker: None,
            ask_first: false,
        }
    }

    fn obs_manifest() -> Manifest {
        Manifest::parse(include_str!("../../../plugins/obs/plugin.toml")).expect("obs manifest")
    }

    fn row(status: &str, detail: &str) -> PluginRow {
        PluginRow {
            id: "obs".into(),
            name: "OBS Studio".into(),
            version: "1.0.0".into(),
            status: status.into(),
            detail: detail.into(),
        }
    }

    fn loaded(status: &str, detail: &str) -> Plugins {
        Plugins {
            loaded: true,
            list: vec![row(status, detail)],
            manifests: [("obs".to_string(), obs_manifest())].into_iter().collect(),
        }
    }

    #[test]
    fn whole_numbers_stay_integers() {
        let mut port = Field {
            min: Some(1.0),
            max: Some(65535.0),
            default: Some(toml::Value::Integer(4455)),
            ..field("port", FieldKind::Number)
        };
        assert_eq!(
            value_from(&port, Input::Number(4455.0)),
            Ok(Some(toml::Value::Integer(4455)))
        );
        assert_eq!(
            value_from(&port, Input::Number(70000.0)),
            Err("Must be between 1 and 65535".to_string())
        );
        port.step = Some(0.5);
        assert_eq!(
            value_from(&port, Input::Number(2.5)),
            Ok(Some(toml::Value::Float(2.5)))
        );
    }

    #[test]
    fn array_in_text_field_is_shown_joined() {
        let f = field("entities", FieldKind::Text);
        let table: toml::Table = toml::from_str(r#"entities = ["light.a", "light.b"]"#).expect("toml");
        let v = current(&f, &table);
        assert_eq!(shown_text(v.as_ref()), "light.a, light.b");
        assert_eq!(control_for(&f, v.as_ref()), Control::Entry);
    }

    #[test]
    fn blank_text_removes_the_key() {
        let f = field("host", FieldKind::Text);
        assert_eq!(value_from(&f, Input::Text("  ".into())), Ok(None));
    }

    #[test]
    fn json_is_checked() {
        let f = field("data", FieldKind::Json);
        assert_eq!(
            value_from(&f, Input::Text("{\"a\":1}".into())),
            Ok(Some(toml::Value::String("{\"a\":1}".into())))
        );
        let e = value_from(&f, Input::Text("{a".into())).expect_err("bad json");
        assert!(e.starts_with("Not valid JSON: "), "{e}");
    }

    #[test]
    fn choice_and_color() {
        let f = Field {
            choices: vec![Choice::Plain("up".into()), Choice::Plain("down".into())],
            ..field("dir", FieldKind::Choice)
        };
        assert_eq!(
            value_from(&f, Input::Choice(1)),
            Ok(Some(toml::Value::String("down".into())))
        );
        let c = field("colour", FieldKind::Color);
        assert_eq!(
            value_from(&c, Input::Color("#ff0000".into())),
            Ok(Some(toml::Value::String("#ff0000".into())))
        );
        assert_eq!(
            value_from(&c, Input::Color("red".into())),
            Err("Not a colour".to_string())
        );
    }

    #[test]
    fn unknown_kind_with_a_table_is_read_only() {
        let f = field("m", FieldKind::Other("map".into()));
        let table: toml::Table = toml::from_str("m = { a = 1 }").expect("toml");
        assert_eq!(control_for(&f, current(&f, &table).as_ref()), Control::ReadOnly);
        assert_eq!(
            control_for(&f, Some(&toml::Value::String("x".into()))),
            Control::Entry
        );
        assert_eq!(control_for(&f, None), Control::Entry);
    }

    #[test]
    fn no_badges_before_the_list_arrives() {
        assert_eq!(Plugins::default().action_problem("obs", "x"), None);
        let p = Plugins {
            loaded: true,
            ..Plugins::default()
        };
        assert_eq!(
            p.action_problem("obs", "x"),
            Some("Needs the “obs” plugin".to_string())
        );
    }

    #[test]
    fn badges_follow_status() {
        assert_eq!(
            loaded("off", "").action_problem("obs", "record"),
            Some("The OBS Studio plugin is turned off".to_string())
        );
        assert_eq!(
            loaded("failed", "boom").action_problem("obs", "record"),
            Some("OBS Studio stopped working".to_string())
        );
        assert_eq!(
            loaded("incompatible", "OBS Studio needs a newer Ulanzi Deck").action_problem("obs", "record"),
            Some("OBS Studio needs a newer Ulanzi Deck".to_string())
        );
        assert_eq!(
            loaded("running", "").action_problem("obs", "zap"),
            Some("OBS Studio has no action “zap”".to_string())
        );
        for status in ["running", "ready", "starting", "problem"] {
            assert_eq!(
                loaded(status, "").action_problem("obs", "input_volume"),
                None,
                "{status}"
            );
        }
    }

    #[test]
    fn titles_and_groups() {
        let p = loaded("running", "");
        assert_eq!(p.action_title("obs", "input_volume"), "Change an input's volume");
        assert_eq!(p.action_title("ha", "toggle"), "toggle (ha)");
        let groups = p.actions_by_plugin();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "obs");
        assert_eq!(groups[0].1, "OBS Studio");
        assert!(!groups[0].2.is_empty());
        assert!(loaded("incompatible", "x").actions_by_plugin().is_empty());
        assert_eq!(p.row("obs").map(|r| r.status.as_str()), Some("running"));
    }

    #[test]
    fn status_words_and_banners() {
        let failed = row("failed", "it crashed");
        let w = status_words(&failed);
        assert_eq!(w.title, "Stopped working");
        assert_eq!(w.subtitle, "it crashed");
        assert_eq!((w.icon, w.css), ("dialog-warning-symbolic", "warning"));
        assert_eq!(
            banner(&failed),
            Some(("OBS Studio stopped working".to_string(), Some("Restart")))
        );
        let running = row("running", "");
        let w = status_words(&running);
        assert_eq!(w.title, "Running");
        assert_eq!((w.icon, w.css), ("emblem-ok-symbolic", "success"));
        assert_eq!(banner(&running), None);
        assert_eq!(
            status_words(&row("ready", "")).title,
            "Ready: starts when a key uses it"
        );
        assert_eq!(
            status_words(&row("off", "")).icon,
            "media-playback-pause-symbolic"
        );
        assert_eq!(status_words(&row("starting", "")).title, "Starting…");
        assert_eq!(
            banner(&row("problem", "Wrong password")),
            Some(("Wrong password".to_string(), Some("Restart")))
        );
        assert_eq!(
            banner(&row("incompatible", "Needs a newer app")),
            Some(("Needs a newer app".to_string(), None))
        );
    }

    #[test]
    fn controls_for_kinds() {
        let port = Field {
            min: Some(1.0),
            max: Some(65535.0),
            ..field("port", FieldKind::Number)
        };
        assert_eq!(
            control_for(&port, None),
            Control::Spin {
                min: 1.0,
                max: 65535.0,
                step: 1.0,
                digits: 0
            }
        );
        assert_eq!(
            control_for(&field("x", FieldKind::Number), None),
            Control::Spin {
                min: -1e9,
                max: 1e9,
                step: 0.1,
                digits: 2
            }
        );
        assert_eq!(
            control_for(&field("p", FieldKind::Secret), None),
            Control::Password
        );
        assert_eq!(control_for(&field("t", FieldKind::Toggle), None), Control::Switch);
        assert_eq!(control_for(&field("j", FieldKind::Json), None), Control::Json);
        assert_eq!(shown_text(Some(&toml::Value::Float(3.0))), "3");
        assert_eq!(shown_text(Some(&toml::Value::Integer(4455))), "4455");
    }

    #[test]
    fn secrets_never_go_through_the_form() {
        let f = field("password", FieldKind::Secret);
        assert_eq!(
            value_from(&f, Input::Text("hunter2".into())),
            Err("Secrets are saved separately".to_string())
        );
        assert_eq!(
            value_from(&f, Input::Text("••••".into())),
            Err("Secrets are saved separately".to_string())
        );
        assert_eq!(
            value_from(&f, Input::Text(String::new())),
            Err("Secrets are saved separately".to_string())
        );
    }

    #[test]
    fn banners_without_a_detail_still_say_something() {
        assert_eq!(
            banner(&row("problem", "")),
            Some(("Needs attention".to_string(), Some("Restart")))
        );
        assert_eq!(
            banner(&row("incompatible", "  ")),
            Some(("OBS Studio can’t be loaded".to_string(), None))
        );
    }

    fn manifest(id: &str, name: &str, actions: &[(&str, &str)]) -> Manifest {
        let mut text = format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1.0.0\"\ncontract = \"1.0\"\n");
        for (aid, aname) in actions {
            text.push_str(&format!("[[actions]]\nid = \"{aid}\"\nname = \"{aname}\"\n"));
        }
        Manifest::parse(&text).expect("manifest")
    }

    fn picker_plugins() -> Plugins {
        let plain = |id: &str, name: &str, status: &str| PluginRow {
            id: id.into(),
            name: name.into(),
            version: "1.0.0".into(),
            status: status.into(),
            detail: String::new(),
        };
        Plugins {
            loaded: true,
            list: vec![
                plain("obs", "OBS Studio", "running"),
                plain("ha", "Home Assistant", "problem"),
                plain("bad", "Bad", "incompatible"),
            ],
            manifests: [
                manifest("obs", "OBS Studio", &[("toggle_record", "Toggle recording")]),
                manifest(
                    "ha",
                    "Home Assistant",
                    &[("toggle", "Toggle"), ("scene", "Run a scene")],
                ),
                manifest("bad", "Bad", &[("x", "X")]),
            ]
            .into_iter()
            .map(|m| (m.id.clone(), m))
            .collect(),
        }
    }

    #[test]
    fn actions_by_plugin_groups_and_sorts() {
        let groups = picker_plugins().actions_by_plugin();
        let shape: Vec<(&str, &str, Vec<&str>)> = groups
            .iter()
            .map(|(id, name, acts)| {
                (
                    id.as_str(),
                    name.as_str(),
                    acts.iter().map(|a| a.id.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                ("ha", "Home Assistant", vec!["toggle", "scene"]),
                ("obs", "OBS Studio", vec!["toggle_record"]),
            ]
        );
    }

    #[test]
    fn picker_groups_name_rows_for_search_and_fill_defaults() {
        let mut p = picker_plugins();
        p.manifests.insert("obs".into(), obs_manifest());
        let groups = p.picker_groups();
        assert_eq!(
            groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Home Assistant", "OBS Studio"]
        );
        let ha = &groups[0].items;
        assert_eq!(ha[1].title, "Run a scene");
        assert_eq!(ha[1].search, "home assistant run a scene");
        assert_eq!(
            ha[1].action,
            Action::Plugin {
                plugin: "ha".into(),
                action: "scene".into(),
                params: toml::Table::new(),
            }
        );
        let vol = groups[1]
            .items
            .iter()
            .find(|i| i.title == "Change an input's volume")
            .expect("input_volume");
        assert_eq!(vol.search, "obs studio change an input's volume");
        let want: toml::Table = toml::from_str("op = \"up\"\npercent = 0\nstep = 2").expect("toml");
        assert_eq!(
            vol.action,
            Action::Plugin {
                plugin: "obs".into(),
                action: "input_volume".into(),
                params: want,
            }
        );
    }

    #[test]
    fn plugin_actions_are_named_from_the_manifest() {
        let p = picker_plugins();
        let a = |plugin: &str, action: &str| Action::Plugin {
            plugin: plugin.into(),
            action: action.into(),
            params: toml::Table::new(),
        };
        assert_eq!(p.describe(&a("ha", "scene")), "Run a scene");
        assert_eq!(p.describe(&a("zz", "go")), "go (zz)");
        assert_eq!(
            p.describe(&Action::NextPage),
            ulanzi_core::catalog::describe(&Action::NextPage)
        );
        assert_eq!(p.plugin_name("ha"), "Home Assistant");
        assert_eq!(p.plugin_name("zz"), "zz");
    }

    #[test]
    fn retry_flips_do_not_touch_the_editor() {
        let before = loaded("problem", "OBS is not running");
        assert!(!before.editor_differs(&loaded("starting", "")));
        assert!(!before.editor_differs(&loaded("running", "")));
        assert!(before.editor_differs(&loaded("off", "")));
        assert!(before.editor_differs(&loaded("failed", "boom")));
        assert!(loaded("incompatible", "a").editor_differs(&loaded("incompatible", "b")));
        assert!(before.editor_differs(&Plugins::default()));
        let mut renamed = before.clone();
        renamed.list[0].name = "OBS".into();
        assert!(before.editor_differs(&renamed));
        let mut no_manifest = before.clone();
        no_manifest.manifests.clear();
        assert!(before.editor_differs(&no_manifest));
    }

    #[test]
    fn defaults_fill_params() {
        let m = obs_manifest();
        let a = m.action("input_volume").expect("input_volume");
        let want: toml::Table = toml::from_str("op = \"up\"\npercent = 0\nstep = 2").expect("toml");
        assert_eq!(defaults(a), want);
    }

    /// Every shipped action, freshly added (text parameters filled in as the user would), is
    /// accepted by its plugin's own parser.
    #[test]
    fn shipped_actions_work_out_of_the_box() {
        for (name, manifest) in [
            ("obs", obs_manifest()),
            (
                "ha",
                Manifest::parse(include_str!("../../../plugins/ha/plugin.toml")).expect("ha manifest"),
            ),
        ] {
            for a in &manifest.actions {
                let mut t = defaults(a);
                for p in &a.params {
                    if matches!(p.kind, FieldKind::Text | FieldKind::Item) && !t.contains_key(&p.key) {
                        t.insert(
                            p.key.clone(),
                            toml::Value::String(
                                match (p.key.as_str(), a.id.as_str()) {
                                    ("entity_id", "run") => "scene.test",
                                    ("entity_id", _) => "light.test",
                                    _ => "test",
                                }
                                .into(),
                            ),
                        );
                    }
                }
                let params = serde_json::to_value(&t).expect("json");
                let r = match name {
                    "obs" => ulanzi_plugin_obs::actions::parse_action(&a.id, &params).map(|_| ()),
                    _ => ulanzi_plugin_ha::actions::plan(
                        &a.id,
                        &params,
                        &ulanzi_plugin_ha::protocol::Tracked::default(),
                    )
                    .map(|_| ()),
                };
                // Actions the plugin handles itself (reconnect, forget the password) are not
                // the parser's; any other failure is a gap in what a new action stores.
                if r.as_ref().is_err_and(|e| e.starts_with("unknown action")) && a.params.is_empty() {
                    continue;
                }
                assert!(r.is_ok(), "{name}/{}: {:?} with {t:?}", a.id, r);
            }
        }
    }
}
