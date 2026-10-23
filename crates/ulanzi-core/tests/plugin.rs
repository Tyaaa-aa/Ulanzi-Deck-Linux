use std::net::IpAddr;

use ulanzi_core::model::{Action, Key, Model, Profile, Settings, table_to_json};
use ulanzi_core::plugin::{Manifest, ManifestError, Permissions, host_allowed};
use ulanzi_core::store::parse_profile;

const OBS: &str = r##"
id = "obs"
name = "OBS Studio"
version = "1.0.0"
contract = "1.0"
icon = "icon.svg"
description = "Scenes, recording, streaming and audio, with live state on keys."
categories = ["streaming"]
tested_on_hardware = true          # set by the maintainer at merge; see section 6

[permissions]
network = ["127.0.0.1", "localhost"]   # or lan = true
browser = false
dbus = []
programs = []

[detect]                           # for suggested integrations (section 6)
programs = ["obs"]
desktop_ids = ["com.obsproject.Studio"]

[auth]
kind = "password"                  # none | password | token | oauth | device_code | lan_pairing

[[settings]]                       # rendered by the GUI as the plugin's settings page
key = "host"
kind = "text"
label = "Address"
default = "localhost:4455"

[[actions]]
id = "set_scene"
name = "Switch scene"
icon = "mdi:filmstrip"
params = [{ key = "scene", kind = "item", picker = "scenes", label = "Scene" }]
face = "scene_active"              # key-face mapping, below

[[pickers]]
id = "scenes"
style = "grid"                     # grid | list
groups = true

[[faces]]
id = "scene_active"
when = { state = "obs.program", equals = "$scene" }
on = { color = "#e5484d", label = "LIVE" }
off = {}

[[widgets]]
id = "obs_status"
name = "OBS status"
template = "stats_row"             # or "canvas"

[[variables]]
id = "obs.rec_time"
name = "Recording time"
"##;

fn manifest(id: &str, version: &str, contract: &str, extra: &str) -> String {
    format!("id = \"{id}\"\nname = \"X\"\nversion = \"{version}\"\ncontract = \"{contract}\"\n{extra}")
}

#[test]
fn manifest_parses_the_spec_example() {
    let m = Manifest::parse(OBS).expect("parses");
    assert_eq!(m.id, "obs");
    assert_eq!(m.permissions.network, ["127.0.0.1", "localhost"]);
    assert!(m.has_network());
    assert!(!m.background);
    assert_eq!(m.actions.len(), 1);
    assert_eq!(m.actions[0].id, "set_scene");
    assert_eq!(m.actions[0].params.len(), 1);
}

#[test]
fn hello_example_manifest_parses() {
    let text = include_str!("../../../examples/plugins/hello/plugin.toml");
    let m = Manifest::parse(text).expect("hello parses");
    assert_eq!(m.id, "hello");
    let greet = m.actions.iter().find(|a| a.id == "greet").expect("greet");
    assert_eq!(greet.params.len(), 1);
}

#[test]
fn manifest_rejects() {
    let bad_id = manifest("OBS", "1.0.0", "1.0", "");
    assert!(matches!(Manifest::parse(&bad_id), Err(ManifestError::BadId(_))));
    let bad_ver = manifest("obs", "one", "1.0", "");
    assert!(matches!(
        Manifest::parse(&bad_ver),
        Err(ManifestError::BadVersion(_))
    ));
    let two = manifest("obs", "1.0.0", "2.0", "");
    assert!(matches!(
        Manifest::parse(&two),
        Err(ManifestError::Incompatible { needs }) if needs == "2.0"
    ));
    let one_one = manifest("obs", "1.0.0", "1.1", "");
    assert!(matches!(
        Manifest::parse(&one_one),
        Err(ManifestError::Incompatible { .. })
    ));
    let dup = manifest(
        "obs",
        "1.0.0",
        "1.0",
        "[[actions]]\nid = \"a\"\nname = \"A\"\n[[actions]]\nid = \"a\"\nname = \"B\"\n",
    );
    assert!(matches!(
        Manifest::parse(&dup),
        Err(ManifestError::DuplicateAction(a)) if a == "a"
    ));
    assert!(matches!(
        Manifest::parse("not = [toml"),
        Err(ManifestError::Parse(_))
    ));
}

fn ips(list: &[&str]) -> Vec<IpAddr> {
    list.iter().map(|s| s.parse().expect("ip")).collect()
}

#[test]
fn hosts() {
    let p = Permissions {
        network: vec!["127.0.0.1".into(), "*.twitch.tv".into()],
        ..Permissions::default()
    };
    assert!(host_allowed(&p, "127.0.0.1", &[]));
    assert!(host_allowed(&p, "api.twitch.tv", &[]));
    assert!(!host_allowed(&p, "twitch.tv.evil.com", &[]));
    assert!(!host_allowed(&p, "example.com", &[]));

    let lan = Permissions {
        lan: true,
        ..Permissions::default()
    };
    assert!(lan.lan);
    assert!(host_allowed(&lan, "homeassistant.local", &[]));
    assert!(host_allowed(&lan, "ha.example.com", &ips(&["192.168.1.5"])));
    assert!(!host_allowed(
        &lan,
        "ha.example.com",
        &ips(&["192.168.1.5", "8.8.8.8"])
    ));
    assert!(!host_allowed(&lan, "ha.example.com", &[]));
    assert!(host_allowed(&lan, "localhost", &[]));
}

#[test]
fn plugin_action_round_trips() {
    // The action as it appears in a profile file: `tap = { type = "plugin", ... }`.
    let key: Key = toml::from_str(
        "tap = { type = \"plugin\", plugin = \"hello\", action = \"greet\", params = { name = \"Ana\", when = 1979-05-27T07:32:00Z } }",
    )
    .expect("key parses");
    assert!(matches!(&key.tap, Some(Action::Plugin { plugin, .. }) if plugin == "hello"));
    let mut p = Profile::new_default("Default", Model::D200X);
    p.pages[0].keys.insert(0, key);
    let text = toml::to_string_pretty(&p).expect("serializes");
    assert!(text.contains("hello"));
    let q = parse_profile(&text).expect("reparses");
    assert_eq!(p, q);
    assert_eq!(toml::to_string_pretty(&q).expect("serializes"), text);
}

#[test]
fn params_to_json() {
    let t: toml::Table = toml::from_str("name = \"Ana\"\nwhen = 1979-05-27T07:32:00Z\n").expect("toml");
    assert_eq!(
        table_to_json(&t),
        serde_json::json!({"name": "Ana", "when": "1979-05-27T07:32:00Z"})
    );
    assert_eq!(table_to_json(&toml::Table::new()), serde_json::json!({}));
}

#[test]
fn settings_plugins_default() {
    let s: Settings = toml::from_str("version = 1\n").expect("old settings");
    assert!(!s.developer_mode);
    assert!(s.plugins.is_empty());
    let s: Settings = toml::from_str(
        "[plugins.hello]\n[plugins.obs]\nenabled = false\n[plugins.obs.settings]\nhost = \"x\"\n",
    )
    .expect("with plugins");
    assert!(s.plugins["hello"].enabled);
    assert!(!s.plugins["obs"].enabled);
    assert_eq!(s.plugins["obs"].settings["host"].as_str(), Some("x"));
    let back = toml::to_string_pretty(&Settings::default()).expect("ser");
    assert!(!back.contains("plugins"));
}

#[test]
fn plugin_action_variant_matches() {
    let a = Action::Plugin {
        plugin: "p".into(),
        action: "a".into(),
        params: toml::Table::new(),
    };
    assert!(matches!(a, Action::Plugin { .. }));
}
