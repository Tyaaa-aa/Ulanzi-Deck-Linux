//! What the app knows about the daemon's plugins, and the form model for plugin settings and action
//! parameters. Pure Rust (no GTK) so it can be unit-tested.

use std::collections::BTreeMap;

use ulanzi_core::model::Action;
use ulanzi_core::plugin::{Field, FieldKind, Manifest, ManifestAction, PickItem, Picker};

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
    /// Picked from the plugin's list (an `item` field whose picker the manifest declares).
    Item {
        picker: Picker,
    },
    /// The value has a type the app can't edit; it is shown and kept as it is.
    ReadOnly,
}

/// The widest range a number row allows when the field gives no bounds.
const UNBOUNDED: f64 = 1e9;

pub fn control_for(f: &Field, pickers: &[Picker], current: Option<&toml::Value>) -> Control {
    let textual = matches!(current, None | Some(toml::Value::String(_)));
    if f.kind == FieldKind::Item
        && let Some(p) = f
            .picker
            .as_deref()
            .and_then(|id| pickers.iter().find(|p| p.id == id))
    {
        let list = |a: &[toml::Value]| a.iter().all(toml::Value::is_str);
        return match current {
            None | Some(toml::Value::String(_)) => Control::Item { picker: p.clone() },
            Some(toml::Value::Array(a)) if p.multi && list(a) => Control::Item { picker: p.clone() },
            _ => Control::ReadOnly,
        };
    }
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

/// The ids an `item` field holds: a string is one id (with `multi`, a legacy comma-separated
/// string is several), an array of strings is several. Blank ids are dropped; a stored id is kept
/// whether or not the plugin still lists it.
pub fn item_ids(v: Option<&toml::Value>, multi: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |id: &str| {
        if !id.trim().is_empty() && !out.iter().any(|x| x == id) {
            out.push(id.to_string());
        }
    };
    match v {
        Some(toml::Value::String(s)) if multi => s.split(',').for_each(|id| add(id.trim())),
        Some(toml::Value::String(s)) => add(s),
        Some(toml::Value::Array(a)) => a.iter().filter_map(toml::Value::as_str).for_each(&mut add),
        _ => {}
    }
    out
}

/// The value to store for the picked ids: nothing for none, a string, or an array with `multi`.
pub fn item_value(selected: &[String], multi: bool) -> Option<toml::Value> {
    match selected {
        [] => None,
        _ if multi => Some(toml::Value::Array(
            selected
                .iter()
                .map(|id| toml::Value::String(id.clone()))
                .collect(),
        )),
        [first, ..] => Some(toml::Value::String(first.clone())),
    }
}

/// The label of `id` among `known`, else the id itself.
fn label_of<'a>(id: &'a str, known: &'a [PickItem]) -> &'a str {
    known.iter().find(|i| i.id == id).map_or(id, |i| i.label.as_str())
}

/// What an item row shows: "None", one label, "A and B", "A, B and C", "A, B and 3 more". An id
/// the plugin no longer lists shows as itself.
pub fn item_summary(ids: &[String], known: &[PickItem]) -> String {
    let labels: Vec<&str> = ids.iter().map(|id| label_of(id, known)).collect();
    match labels.as_slice() {
        [] => "None".into(),
        [one] => (*one).to_string(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

/// The ids typed into the picker's manual entry (with `multi`, separated by commas).
pub fn manual_ids(text: &str, multi: bool) -> Vec<String> {
    item_ids(Some(&toml::Value::String(text.trim().to_string())), multi)
}

/// Entries by group, groups in first-seen order; entries without a group under "".
pub fn group_items(items: &[PickItem]) -> Vec<(String, Vec<PickItem>)> {
    let mut out: Vec<(String, Vec<PickItem>)> = Vec::new();
    for i in items {
        let g = i.group.clone().unwrap_or_default();
        match out.iter_mut().find(|(n, _)| *n == g) {
            Some((_, v)) => v.push(i.clone()),
            None => out.push((g, vec![i.clone()])),
        }
    }
    out
}

/// The groups seen in `items`, first-seen order, without the empty one (the group drop-down).
pub fn group_names(items: &[PickItem]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in items.iter().filter_map(|i| i.group.as_deref()) {
        if !g.is_empty() && !out.iter().any(|x| x == g) {
            out.push(g.to_string());
        }
    }
    out
}

/// Whether a plugin built for `contract` can list items (`list-items` came with 1.1).
pub fn can_list(contract: &str) -> bool {
    let mut parts = contract.trim().split('.').map(str::parse::<u32>);
    match (parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => (major, minor) >= (1, 1),
        _ => false,
    }
}

/// The themed icon name to look up for an entry's `icon` (`mdi:lightbulb` → `lightbulb`); `None`
/// when there is nothing to look up.
pub fn icon_lookup(icon: &str) -> Option<&str> {
    let name = icon.rsplit_once(':').map_or(icon, |(_, n)| n).trim();
    (!name.is_empty()).then_some(name)
}

/// Stored ids the list does not contain, as entries labelled with the id, so the current value
/// stays visible (and selected) instead of silently disappearing.
pub fn missing_current(items: &[PickItem], current: &[String]) -> Vec<PickItem> {
    current
        .iter()
        .filter(|id| !items.iter().any(|i| &i.id == *id))
        .map(|id| PickItem {
            id: id.clone(),
            label: id.clone(),
            subtitle: Some("Not in the list now".into()),
            group: None,
            icon: None,
            state: None,
        })
        .collect()
}

/// Add `id` to or remove it from the selection, keeping the order it was picked in.
pub fn toggle_id(selected: &mut Vec<String>, id: &str, on: bool) {
    let at = selected.iter().position(|x| x == id);
    match (on, at) {
        (true, None) => selected.push(id.to_string()),
        (false, Some(i)) => {
            selected.remove(i);
        }
        _ => {}
    }
}

/// At most this many entries are shown at once; searching narrows the rest.
pub const SHOW_MAX: usize = 300;

/// The entries to show and, when some are left out, a note saying so.
pub fn shown_items(items: &[PickItem], max: usize) -> (&[PickItem], Option<String>) {
    if items.len() <= max {
        return (items, None);
    }
    (
        &items[..max],
        Some(format!(
            "Showing {max} of {}. Search to find the rest.",
            items.len()
        )),
    )
}

/// What the picker says when the plugin answers an empty list.
pub fn empty_message(query: &str, group: &str) -> String {
    match (query.trim(), group) {
        ("", "") => "The list is empty".into(),
        (q, "") => format!("Nothing matches “{q}”"),
        ("", g) => format!("Nothing in {g}"),
        (q, g) => format!("Nothing in {g} matches “{q}”"),
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
        assert_eq!(control_for(&f, &[], v.as_ref()), Control::Entry);
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
        assert_eq!(
            control_for(&f, &[], current(&f, &table).as_ref()),
            Control::ReadOnly
        );
        assert_eq!(
            control_for(&f, &[], Some(&toml::Value::String("x".into()))),
            Control::Entry
        );
        assert_eq!(control_for(&f, &[], None), Control::Entry);
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
            control_for(&port, &[], None),
            Control::Spin {
                min: 1.0,
                max: 65535.0,
                step: 1.0,
                digits: 0
            }
        );
        assert_eq!(
            control_for(&field("x", FieldKind::Number), &[], None),
            Control::Spin {
                min: -1e9,
                max: 1e9,
                step: 0.1,
                digits: 2
            }
        );
        assert_eq!(
            control_for(&field("p", FieldKind::Secret), &[], None),
            Control::Password
        );
        assert_eq!(
            control_for(&field("t", FieldKind::Toggle), &[], None),
            Control::Switch
        );
        assert_eq!(
            control_for(&field("j", FieldKind::Json), &[], None),
            Control::Json
        );
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

    fn item(id: &str, label: &str, group: Option<&str>) -> PickItem {
        PickItem {
            id: id.into(),
            label: label.into(),
            subtitle: None,
            group: group.map(Into::into),
            icon: None,
            state: None,
        }
    }

    fn picker(id: &str, multi: bool) -> Picker {
        Picker {
            id: id.into(),
            style: ulanzi_core::plugin::PickerStyle::List,
            groups: true,
            multi,
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn item_fields_with_a_declared_picker_are_picked() {
        let scenes = picker("scenes", false);
        let entities = picker("entities", true);
        let pickers = vec![scenes.clone(), entities.clone()];
        let f = Field {
            picker: Some("scenes".into()),
            ..field("scene", FieldKind::Item)
        };
        let s = |v: &str| toml::Value::String(v.into());
        assert_eq!(
            control_for(&f, &pickers, None),
            Control::Item {
                picker: scenes.clone()
            }
        );
        assert_eq!(
            control_for(&f, &pickers, Some(&s("Gone"))),
            Control::Item { picker: scenes }
        );
        // A table can't be an id.
        let table: toml::Table = toml::from_str("scene = { a = 1 }").expect("toml");
        assert_eq!(control_for(&f, &pickers, table.get("scene")), Control::ReadOnly);
        // Undeclared picker or no picker: a text entry, as in contract 1.0.
        let undeclared = Field {
            picker: Some("nope".into()),
            ..field("scene", FieldKind::Item)
        };
        assert_eq!(control_for(&undeclared, &pickers, None), Control::Entry);
        assert_eq!(
            control_for(&field("x", FieldKind::Item), &pickers, None),
            Control::Entry
        );
        // A multi picker holds a list; a single one can't show one.
        let multi = Field {
            picker: Some("entities".into()),
            ..field("entities", FieldKind::Item)
        };
        let arr: toml::Table = toml::from_str(r#"entities = ["light.a", "light.b"]"#).expect("toml");
        assert_eq!(
            control_for(&multi, &pickers, arr.get("entities")),
            Control::Item {
                picker: entities.clone()
            }
        );
        assert_eq!(
            control_for(&multi, &pickers, Some(&s("light.a, light.b"))),
            Control::Item { picker: entities }
        );
        assert_eq!(control_for(&f, &pickers, arr.get("entities")), Control::ReadOnly);
        let mixed: toml::Table = toml::from_str(r#"entities = ["light.a", 3]"#).expect("toml");
        assert_eq!(
            control_for(&multi, &pickers, mixed.get("entities")),
            Control::ReadOnly
        );
    }

    #[test]
    fn item_ids_read_strings_lists_and_legacy_commas() {
        let s = |v: &str| toml::Value::String(v.into());
        assert_eq!(item_ids(None, false), Vec::<String>::new());
        assert_eq!(item_ids(Some(&s("")), false), Vec::<String>::new());
        assert_eq!(item_ids(Some(&s("Scene 1")), false), ids(&["Scene 1"]));
        // A single id is kept as stored, commas and all.
        assert_eq!(item_ids(Some(&s("a, b")), false), ids(&["a, b"]));
        assert_eq!(
            item_ids(Some(&s(" light.a, light.b ,,light.a")), true),
            ids(&["light.a", "light.b"])
        );
        let arr: toml::Table = toml::from_str(r#"e = ["light.a", "", "switch.b"]"#).expect("toml");
        assert_eq!(item_ids(arr.get("e"), true), ids(&["light.a", "switch.b"]));
    }

    #[test]
    fn item_values_round_trip() {
        assert_eq!(item_value(&[], false), None);
        assert_eq!(item_value(&[], true), None);
        assert_eq!(
            item_value(&ids(&["Scene 1"]), false),
            Some(toml::Value::String("Scene 1".into()))
        );
        let arr: toml::Table = toml::from_str(r#"e = ["light.a", "switch.b"]"#).expect("toml");
        let v = arr.get("e").cloned();
        assert_eq!(item_value(&item_ids(v.as_ref(), true), true), v);
        // The legacy comma string becomes a list on the next pick.
        let legacy = toml::Value::String("light.a,switch.b".into());
        assert_eq!(item_value(&item_ids(Some(&legacy), true), true), v);
    }

    #[test]
    fn item_summaries() {
        let known = vec![
            item("light.kitchen", "Kitchen", Some("Lights")),
            item("light.hall", "Hall", Some("Lights")),
            item("light.desk", "Desk", Some("Lights")),
        ];
        assert_eq!(item_summary(&[], &known), "None");
        assert_eq!(item_summary(&ids(&["light.kitchen"]), &known), "Kitchen");
        assert_eq!(item_summary(&ids(&["light.gone"]), &known), "light.gone");
        assert_eq!(
            item_summary(&ids(&["light.kitchen", "light.gone"]), &known),
            "Kitchen and light.gone"
        );
        assert_eq!(
            item_summary(&ids(&["light.kitchen", "light.hall", "light.desk"]), &known),
            "Kitchen, Hall and Desk"
        );
        assert_eq!(
            item_summary(&ids(&["light.kitchen", "light.hall", "a", "b", "c"]), &known),
            "Kitchen, Hall and 3 more"
        );
    }

    #[test]
    fn items_are_grouped_in_first_seen_order() {
        let items = vec![
            item("light.a", "A", Some("Lights")),
            item("x", "X", None),
            item("switch.b", "B", Some("Switches")),
            item("light.c", "C", Some("Lights")),
        ];
        let g = group_items(&items);
        let shape: Vec<(&str, Vec<&str>)> = g
            .iter()
            .map(|(n, v)| (n.as_str(), v.iter().map(|i| i.id.as_str()).collect()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("Lights", vec!["light.a", "light.c"]),
                ("", vec!["x"]),
                ("Switches", vec!["switch.b"]),
            ]
        );
        assert_eq!(group_names(&items), ids(&["Lights", "Switches"]));
        assert!(group_items(&[]).is_empty());
    }

    #[test]
    fn only_contract_1_1_and_later_lists() {
        assert!(!can_list("1.0"));
        assert!(can_list("1.1"));
        assert!(can_list("1.2"));
        assert!(can_list("2.0"));
        assert!(!can_list("nonsense"));
    }

    #[test]
    fn icons_are_looked_up_without_their_prefix() {
        assert_eq!(icon_lookup("mdi:lightbulb"), Some("lightbulb"));
        assert_eq!(
            icon_lookup("camera-video-symbolic"),
            Some("camera-video-symbolic")
        );
        assert_eq!(icon_lookup(""), None);
        assert_eq!(icon_lookup("mdi:"), None);
    }

    #[test]
    fn a_stored_id_the_list_lacks_stays_visible() {
        let items = vec![item("Scene 1", "Scene 1", None)];
        let missing = missing_current(&items, &ids(&["Scene 1", "Old scene"]));
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].id, "Old scene");
        assert_eq!(missing[0].label, "Old scene");
        assert_eq!(missing[0].subtitle.as_deref(), Some("Not in the list now"));
        assert!(missing_current(&items, &ids(&["Scene 1"])).is_empty());
    }

    #[test]
    fn toggling_keeps_the_pick_order() {
        let mut sel = ids(&["b"]);
        toggle_id(&mut sel, "a", true);
        toggle_id(&mut sel, "a", true);
        assert_eq!(sel, ids(&["b", "a"]));
        toggle_id(&mut sel, "b", false);
        toggle_id(&mut sel, "zz", false);
        assert_eq!(sel, ids(&["a"]));
    }

    #[test]
    fn long_lists_are_cut_with_a_note() {
        let items: Vec<PickItem> = (0..5).map(|i| item(&i.to_string(), "x", None)).collect();
        let (shown, note) = shown_items(&items, 10);
        assert_eq!((shown.len(), note), (5, None));
        let (shown, note) = shown_items(&items, 3);
        assert_eq!(shown.len(), 3);
        assert_eq!(note.as_deref(), Some("Showing 3 of 5. Search to find the rest."));
    }

    #[test]
    fn empty_lists_say_why() {
        assert_eq!(empty_message("", ""), "The list is empty");
        assert_eq!(empty_message("kit", ""), "Nothing matches “kit”");
        assert_eq!(empty_message("", "Lights"), "Nothing in Lights");
        assert_eq!(empty_message("kit", "Lights"), "Nothing in Lights matches “kit”");
    }

    #[test]
    fn item_params_start_empty() {
        let f = Field {
            picker: Some("scenes".into()),
            ..field("scene", FieldKind::Item)
        };
        assert_eq!(initial_value(&f), None);
    }
    #[test]
    fn manual_entry_is_trimmed_and_split_for_multi() {
        assert_eq!(manual_ids("  Scene 1 ", false), ids(&["Scene 1"]));
        assert_eq!(manual_ids("a, b", false), ids(&["a, b"]));
        assert_eq!(
            manual_ids("light.a, switch.b,", true),
            ids(&["light.a", "switch.b"])
        );
        assert!(manual_ids("   ", false).is_empty());
    }
}
