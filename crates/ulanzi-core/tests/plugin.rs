use std::net::IpAddr;

use ulanzi_core::model::{Action, Key, Model, Profile, Settings, table_to_json};
use ulanzi_core::plugin::{Manifest, ManifestError, Permissions, host_allowed, is_lan};
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
fn is_lan_boundaries() {
    let yes = [
        "10.1.2.3",
        "172.16.0.1",
        "172.31.255.255",
        "169.254.1.1",
        "127.0.0.1",
        "::1",
        "fc00::1",
        "fd00::1",
        "fe80::1",
        "::ffff:192.168.1.1",
        "192.168.0.1",
    ];
    let no = [
        "172.15.0.1",
        "172.32.0.1",
        "0.0.0.0",
        "8.8.8.8",
        "fec0::1",
        "::ffff:8.8.8.8",
    ];
    for ip in yes {
        assert!(is_lan(ip.parse().expect("ip")), "{ip} should be local");
    }
    for ip in no {
        assert!(!is_lan(ip.parse().expect("ip")), "{ip} should not be local");
    }
}

#[test]
fn host_matching_ignores_case_and_trailing_dot_but_not_the_apex() {
    let p = Permissions {
        network: vec!["*.twitch.tv".into(), "Example.COM".into(), "obs.test.".into()],
        ..Permissions::default()
    };
    assert!(host_allowed(&p, "API.Twitch.TV", &[]));
    assert!(host_allowed(&p, "api.twitch.tv.", &[]));
    assert!(!host_allowed(&p, "twitch.tv", &[]));
    assert!(host_allowed(&p, "example.com", &[]));
    assert!(host_allowed(&p, "EXAMPLE.com.", &[]));
    assert!(host_allowed(&p, "obs.test", &[]));
    let lan = Permissions {
        lan: true,
        ..Permissions::default()
    };
    assert!(host_allowed(&lan, "localhost.", &[]));
}

#[test]
fn manifest_rejects_unusable_network_entries() {
    for entry in ["", "*.", "*", "*.com", "*.com."] {
        let text = manifest(
            "obs",
            "1.0.0",
            "1.0",
            &format!("[permissions]\nnetwork = [\"{entry}\"]\n"),
        );
        match Manifest::parse(&text) {
            Err(ManifestError::Parse(m)) => assert!(m.starts_with(&format!("{entry}: ")), "{m}"),
            other => panic!("{entry:?} accepted: {other:?}"),
        }
    }
    let ok = manifest(
        "obs",
        "1.0.0",
        "1.0",
        "[permissions]\nnetwork = [\"*.example.com\", \"localhost\"]\n",
    );
    assert!(Manifest::parse(&ok).is_ok());
}

#[test]
fn plugins_used_finds_every_place() {
    use ulanzi_core::model::{Dial, Step};
    let plugin = |id: &str| Action::Plugin {
        plugin: id.into(),
        action: "go".into(),
        params: toml::Table::new(),
    };
    let mut p = Profile::new_default("Default", Model::D200X);
    assert!(p.plugins_used().is_empty());
    p.pages[0].keys.insert(
        0,
        Key {
            hold: Some(plugin("on_hold")),
            ..Key::default()
        },
    );
    p.pages[0].keys.insert(
        1,
        Key {
            tap: Some(Action::Multi {
                steps: vec![Step {
                    delay_ms: 0,
                    action: Action::Multi {
                        steps: vec![Step {
                            delay_ms: 10,
                            action: plugin("nested"),
                        }],
                    },
                }],
            }),
            ..Key::default()
        },
    );
    p.pages[0].dials[2] = Dial {
        left: Some(plugin("dial_left")),
        press: Some(plugin("dial_press")),
        ..Dial::default()
    };
    p.small_buttons.right = Some(plugin("small"));
    // Profile-wide dials from before dials were per page.
    p.dials[0].hold = Some(plugin("legacy"));
    let want: std::collections::BTreeSet<String> =
        ["dial_left", "dial_press", "legacy", "nested", "on_hold", "small"]
            .into_iter()
            .map(String::from)
            .collect();
    assert_eq!(p.plugins_used(), want);
}

mod typed_fields {
    use super::*;
    use ulanzi_core::plugin::{Field, FieldKind, Inspected, permission};

    fn field(extra: &str) -> Field {
        toml::from_str(&format!("key = \"k\"\n{extra}")).expect("field parses")
    }

    #[test]
    fn settings_and_params_parse() {
        let m = Manifest::parse(OBS).expect("parses");
        assert_eq!(m.settings[0].key, "host");
        assert_eq!(m.settings[0].kind, FieldKind::Text);
        assert_eq!(
            m.settings[0].default,
            Some(toml::Value::String("localhost:4455".into()))
        );
        let p = &m.action("set_scene").expect("action").params[0];
        assert_eq!(p.kind, FieldKind::Item);
        assert_eq!(p.picker, Some("scenes".into()));
    }

    #[test]
    fn unknown_kinds_are_kept() {
        let m = Manifest::parse(&manifest(
            "x1",
            "1.0.0",
            "1.0",
            "[[actions]]\nid = \"a\"\nname = \"A\"\nparams = [{ key = \"p\", kind = \"slider3d\" }]\n",
        ))
        .expect("parses");
        assert_eq!(m.actions[0].params[0].kind, FieldKind::Other("slider3d".into()));
        let json = serde_json::to_string(&m).expect("json");
        assert_eq!(serde_json::from_str::<Manifest>(&json).expect("back"), m);
        let i = Inspected {
            manifest: m,
            installed_version: Some("0.9.0".into()),
        };
        let back: Inspected = serde_json::from_str(&serde_json::to_string(&i).expect("json")).expect("back");
        assert_eq!(back.installed_version.as_deref(), Some("0.9.0"));
    }

    #[test]
    fn choices_plain_and_labelled() {
        let f = field("kind = \"choice\"\nchoices = [\"up\", { value = \"set\", label = \"Set to\" }]");
        let values: Vec<&str> = f.choices.iter().map(|c| c.value()).collect();
        let labels: Vec<String> = f.choices.iter().map(|c| c.label()).collect();
        assert_eq!(values, ["up", "set"]);
        assert_eq!(labels, ["Up", "Set to"]);
    }

    #[test]
    fn number_integer_rule() {
        assert!(field("min = 0\nmax = 100").is_integer());
        assert!(field("default = 4455").is_integer());
        assert!(!field("step = 0.5").is_integer());
        assert!(!field("default = 1.5").is_integer());
        assert!(!field("").is_integer());
    }

    #[test]
    fn duplicate_fields_refused() {
        let dup = "[[settings]]\nkey = \"host\"\n[[settings]]\nkey = \"host\"\n";
        assert_eq!(
            Manifest::parse(&manifest("x1", "1.0.0", "1.0", dup)),
            Err(ManifestError::DuplicateField {
                scope: "the settings".into(),
                key: "host".into()
            })
        );
        let dup = "[[actions]]\nid = \"x\"\nname = \"X\"\nparams = [{ key = \"a\" }, { key = \"a\" }]\n";
        let err = Manifest::parse(&manifest("x1", "1.0.0", "1.0", dup)).expect_err("refused");
        assert_eq!(
            err,
            ManifestError::DuplicateField {
                scope: "action \"x\"".into(),
                key: "a".into()
            }
        );
        assert_eq!(err.to_string(), "\"a\" is declared twice in action \"x\"");
    }

    #[test]
    fn kinds_match_case_insensitively() {
        assert_eq!(field("kind = \"Secret\"").kind, FieldKind::Secret);
        assert_eq!(field("kind = \"KEY_COMBO\"").kind, FieldKind::KeyCombo);
        assert_eq!(field("kind = \"Gizmo\"").kind, FieldKind::Other("Gizmo".into()));
        let s = "[[actions]]\nid = \"x\"\nname = \"X\"\nparams = [{ key = \"a\", kind = \"Secret\" }]\n";
        assert!(matches!(
            Manifest::parse(&manifest("x1", "1.0.0", "1.0", s)),
            Err(ManifestError::SecretParam { .. })
        ));
    }

    #[test]
    fn empty_field_keys_refused() {
        for key in ["", "  "] {
            let s = format!("[[settings]]\nkey = \"{key}\"\n");
            let err = Manifest::parse(&manifest("x1", "1.0.0", "1.0", &s)).expect_err("refused");
            assert_eq!(
                err,
                ManifestError::EmptyFieldKey {
                    scope: "the settings".into()
                }
            );
            assert_eq!(err.to_string(), "a field in the settings has an empty key");
        }
    }

    #[test]
    fn secret_param_refused() {
        let s = "[[actions]]\nid = \"x\"\nname = \"X\"\nparams = [{ key = \"a\", kind = \"secret\" }]\n";
        let err = Manifest::parse(&manifest("x1", "1.0.0", "1.0", s)).expect_err("refused");
        assert_eq!(
            err,
            ManifestError::SecretParam {
                action: "x".into(),
                key: "a".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "action \"x\": \"a\" cannot be a secret; secrets belong in [[settings]]"
        );
    }

    #[test]
    fn secret_keys_listed() {
        let s = "[[settings]]\nkey = \"password\"\nkind = \"secret\"\n[[settings]]\nkey = \"host\"\n";
        let m = Manifest::parse(&manifest("x1", "1.0.0", "1.0", s)).expect("parses");
        assert_eq!(m.secret_keys(), vec!["password"]);
    }

    #[test]
    fn permissions_in_plain_words() {
        let d = permission::describe;
        assert_eq!(
            d(&Permissions {
                network: vec!["127.0.0.1".into(), "localhost".into()],
                ..Default::default()
            }),
            ["Connect to 127.0.0.1, localhost"]
        );
        assert_eq!(
            d(&Permissions {
                programs: vec!["easyeffects".into()],
                ..Default::default()
            }),
            ["Run the program `easyeffects`"]
        );
        assert_eq!(
            d(&Permissions::default()),
            ["Nothing outside the app: no network, no programs"]
        );
        assert_eq!(
            d(&Permissions {
                lan: true,
                browser: true,
                dbus: vec!["org.a".into(), "org.b".into()],
                ..Default::default()
            }),
            [
                "Connect to devices on your local network",
                "Open your web browser to sign in",
                "Talk to org.a over D-Bus",
                "Talk to org.b over D-Bus"
            ]
        );
    }

    #[test]
    fn shipped_manifests_parse() {
        let parse = |text: &str| Manifest::parse(text).expect("shipped manifest parses");
        let obs = parse(include_str!("../../../plugins/obs/plugin.toml"));
        let ha = parse(include_str!("../../../plugins/ha/plugin.toml"));
        parse(include_str!("../../../examples/plugins/hello/plugin.toml"));
        parse(include_str!("../../ulanzi-deckd/tests/plugins/probe/plugin.toml"));
        assert_eq!(obs.secret_keys(), ["password"]);
        assert_eq!(ha.secret_keys(), ["token"]);
        assert!(obs.settings[1].is_integer());
    }
}
