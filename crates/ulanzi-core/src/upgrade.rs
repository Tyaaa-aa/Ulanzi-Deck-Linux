//! Reading files written by older versions. Version 2 (Ulanzi Deck 2.1) moved OBS Studio from the
//! app into the OBS plugin: actions `{ type = "obs", obs = "<action>", … }` become plugin actions
//! `{ type = "plugin", plugin = "obs", action = "<action>", params = { … } }`, and the `[obs]`
//! settings become `[plugins.obs.settings]`. Done on the TOML before it is read, so nothing older
//! reaches the app.

/// Upgrade a version-1 profile or settings table to version 2 in place.
pub fn v1_to_v2(table: &mut toml::Table) {
    obs_actions(table);
    if let Some(toml::Value::Table(obs)) = table.remove("obs") {
        obs_settings(table, obs);
    }
    table.insert("version".into(), toml::Value::Integer(2));
}

/// Every OBS action anywhere in `table` (keys, dials, multi and cycle steps) as a plugin action.
fn obs_actions(table: &mut toml::Table) {
    if table.get("type").and_then(toml::Value::as_str) == Some("obs") {
        *table = obs_action(table);
        return;
    }
    for (_, v) in table.iter_mut() {
        match v {
            toml::Value::Table(t) => obs_actions(t),
            toml::Value::Array(items) => {
                for i in items.iter_mut() {
                    if let toml::Value::Table(t) = i {
                        obs_actions(t);
                    }
                }
            }
            _ => {}
        }
    }
}

/// One OBS action as the OBS plugin's action with the same parameters.
fn obs_action(old: &toml::Table) -> toml::Table {
    let action = old
        .get("obs")
        .and_then(toml::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut params = toml::Table::new();
    for (k, v) in old {
        match (k.as_str(), v) {
            ("type" | "obs", _) => {}
            // Input volume: `op = { kind = "set", percent = 50 }` → `op = "set"`, `percent = 50`.
            ("op", toml::Value::Table(op)) => {
                if let Some(kind) = op.get("kind") {
                    params.insert("op".into(), kind.clone());
                }
                if let Some(p) = op.get("percent") {
                    params.insert("percent".into(), p.clone());
                }
            }
            _ => {
                params.insert(k.clone(), v.clone());
            }
        }
    }
    let mut t = toml::Table::new();
    t.insert("type".into(), "plugin".into());
    t.insert("plugin".into(), "obs".into());
    t.insert("action".into(), action.into());
    if !params.is_empty() {
        t.insert("params".into(), toml::Value::Table(params));
    }
    t
}

/// `[obs]` (host, port, password) into `[plugins.obs.settings]`, without overwriting what the
/// plugin's settings already have. The service moves the password into the plugin's secret store.
fn obs_settings(table: &mut toml::Table, obs: toml::Table) {
    let plugins = table
        .entry("plugins")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(plugins) = plugins.as_table_mut() else {
        return;
    };
    let entry = plugins
        .entry("obs")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(entry) = entry.as_table_mut() else {
        return;
    };
    let settings = entry
        .entry("settings")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(settings) = settings.as_table_mut() else {
        return;
    };
    for key in ["host", "port", "password"] {
        match obs.get(key) {
            Some(toml::Value::String(s)) if s.is_empty() => {}
            Some(v) => {
                settings.entry(key).or_insert_with(|| v.clone());
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upgraded(text: &str) -> toml::Table {
        let mut t: toml::Table = toml::from_str(text).expect("toml");
        v1_to_v2(&mut t);
        t
    }

    #[test]
    fn obs_actions_become_plugin_actions_everywhere() {
        let t = upgraded(
            r#"
version = 1
[[pages]]
[pages.keys.0.tap]
type = "obs"
obs = "set_scene"
scene = "Intro"
[pages.keys.1.tap]
type = "obs"
obs = "input_volume"
input = "Mic"
op = { kind = "set", percent = 40 }
step = 2
[pages.keys.2.tap]
type = "multi"
steps = [{ action = { type = "obs", obs = "toggle_record" }, delay_ms = 0 }]
[pages.keys.3.tap]
type = "next_page"
"#,
        );
        assert_eq!(t["version"].as_integer(), Some(2));
        let keys = &t["pages"][0]["keys"];
        let scene = &keys["0"]["tap"];
        assert_eq!(scene["type"].as_str(), Some("plugin"));
        assert_eq!(scene["plugin"].as_str(), Some("obs"));
        assert_eq!(scene["action"].as_str(), Some("set_scene"));
        assert_eq!(scene["params"]["scene"].as_str(), Some("Intro"));
        let vol = &keys["1"]["tap"]["params"];
        assert_eq!(vol["op"].as_str(), Some("set"));
        assert_eq!(vol["percent"].as_integer(), Some(40));
        assert_eq!(vol["step"].as_integer(), Some(2));
        let step = &keys["2"]["tap"]["steps"][0]["action"];
        assert_eq!(step["action"].as_str(), Some("toggle_record"));
        assert!(step.get("params").is_none(), "no parameters, no table");
        assert_eq!(keys["3"]["tap"]["type"].as_str(), Some("next_page"));
    }

    #[test]
    fn obs_settings_move_to_the_plugin() {
        let t = upgraded(
            "version = 1\n[obs]\nhost = \"localhost\"\nport = 4456\npassword = \"pw\"\nauto_detect = true\n\
             [plugins.obs.settings]\nhost = \"studio.local\"\n",
        );
        assert!(t.get("obs").is_none());
        let s = &t["plugins"]["obs"]["settings"];
        assert_eq!(
            s["host"].as_str(),
            Some("studio.local"),
            "what the plugin had stays"
        );
        assert_eq!(s["port"].as_integer(), Some(4456));
        assert_eq!(s["password"].as_str(), Some("pw"));
        assert!(s.get("auto_detect").is_none());
    }
}
