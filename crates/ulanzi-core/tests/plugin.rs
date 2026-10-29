use std::net::IpAddr;

use ulanzi_core::model::{Action, Key, Model, Profile, Settings, table_to_json};
use ulanzi_core::plugin::{
    CONTRACT, Manifest, ManifestError, Permissions, PickItem, PickerStyle, host_allowed, is_lan,
};
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
    let one_three = manifest("obs", "1.0.0", "1.3", "");
    assert!(matches!(
        Manifest::parse(&one_three),
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
            verified: true,
        };
        let back: Inspected = serde_json::from_str(&serde_json::to_string(&i).expect("json")).expect("back");
        assert_eq!(back.installed_version.as_deref(), Some("0.9.0"));
        assert!(back.verified);
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
    fn unset_settings_take_their_manifest_default() {
        let s = "[[settings]]\nkey = \"url\"\ndefault = \"host.local:1\"\n\
                 [[settings]]\nkey = \"port\"\nkind = \"number\"\ndefault = 4455\n\
                 [[settings]]\nkey = \"token\"\nkind = \"secret\"\ndefault = \"nope\"\n\
                 [[settings]]\nkey = \"other\"\n";
        let m = Manifest::parse(&manifest("x1", "1.0.0", "1.0", s)).expect("parses");
        let mut set = toml::Table::new();
        set.insert("port".into(), toml::Value::Integer(1));
        let out = m.settings_with_defaults(&set);
        assert_eq!(out["url"].as_str(), Some("host.local:1"));
        assert_eq!(out["port"].as_integer(), Some(1), "a set value wins");
        assert!(!out.contains_key("token"), "secrets never get a default");
        assert!(!out.contains_key("other"), "no default, no key");
        assert!(!set.contains_key("url"), "the input is untouched");
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
        // The OBS and Home Assistant plugins test their own manifests in their repo.
        parse(include_str!("../../../examples/plugins/hello/plugin.toml"));
        let probe = parse(include_str!("../../ulanzi-deckd/tests/plugins/probe/plugin.toml"));
        assert!(probe.secret_keys().iter().all(|k| !k.is_empty()));
    }
}

// ---- Contract 1.1: pickers ----

#[test]
fn pickers_parse_with_defaults() {
    let text = manifest(
        "obs",
        "1.0.0",
        "1.1",
        "[[pickers]]\nid = \"scenes\"\n\n[[pickers]]\nid = \"sources\"\nstyle = \"grid\"\ngroups = true\nmulti = true\n",
    );
    let m = Manifest::parse(&text).expect("parses");
    assert_eq!(m.pickers.len(), 2);
    let scenes = m.picker("scenes").expect("scenes");
    assert_eq!(scenes.style, PickerStyle::List);
    assert!(!scenes.groups);
    assert!(!scenes.multi);
    let sources = m.picker("sources").expect("sources");
    assert_eq!(sources.style, PickerStyle::Grid);
    assert!(sources.groups);
    assert!(sources.multi);
    assert!(m.picker("nope").is_none());
    let bad = manifest(
        "obs",
        "1.0.0",
        "1.1",
        "[[pickers]]\nid = \"x\"\nstyle = \"carousel\"\n",
    );
    assert!(matches!(Manifest::parse(&bad), Err(ManifestError::Parse(_))));
}

#[test]
fn item_field_needs_a_declared_picker() {
    let action = manifest(
        "obs",
        "1.0.0",
        "1.1",
        "[[actions]]\nid = \"go\"\nname = \"Go\"\nparams = [{ key = \"scene\", kind = \"item\", picker = \"scenes\" }]\n",
    );
    match Manifest::parse(&action) {
        Err(e @ ManifestError::UnknownPicker { .. }) => {
            assert_eq!(
                e,
                ManifestError::UnknownPicker {
                    scope: "action \"go\"".into(),
                    key: "scene".into(),
                    picker: "scenes".into(),
                }
            );
            assert!(e.to_string().contains("scenes"), "{e}");
        }
        other => panic!("accepted: {other:?}"),
    }
    let setting = manifest(
        "obs",
        "1.0.0",
        "1.1",
        "[[settings]]\nkey = \"room\"\nkind = \"item\"\npicker = \"rooms\"\n",
    );
    assert!(matches!(
        Manifest::parse(&setting),
        Err(ManifestError::UnknownPicker { scope, key, picker })
            if scope == "the settings" && key == "room" && picker == "rooms"
    ));
}

#[test]
fn item_without_picker_is_still_allowed() {
    let text = manifest(
        "obs",
        "1.0.0",
        "1.0",
        "[[actions]]\nid = \"go\"\nname = \"Go\"\nparams = [{ key = \"scene\", kind = \"item\" }]\n",
    );
    let m = Manifest::parse(&text).expect("a plain text field");
    assert!(m.pickers.is_empty());
    assert_eq!(m.actions[0].params[0].picker, None);
}

#[test]
fn duplicate_picker_is_refused() {
    let text = manifest(
        "obs",
        "1.0.0",
        "1.1",
        "[[pickers]]\nid = \"scenes\"\n[[pickers]]\nid = \"scenes\"\nstyle = \"grid\"\n",
    );
    assert!(matches!(
        Manifest::parse(&text),
        Err(ManifestError::DuplicatePicker(id)) if id == "scenes"
    ));
}

#[test]
fn params_cannot_use_a_multi_picker() {
    let pickers = "[[pickers]]\nid = \"sources\"\nmulti = true\n";
    let param = manifest(
        "obs",
        "1.0.0",
        "1.1",
        &format!(
            "{pickers}[[actions]]\nid = \"go\"\nname = \"Go\"\nparams = [{{ key = \"src\", kind = \"item\", picker = \"sources\" }}]\n"
        ),
    );
    assert!(matches!(
        Manifest::parse(&param),
        Err(ManifestError::MultiParam { action, key }) if action == "go" && key == "src"
    ));
    // A setting may use one: it stores a list of ids.
    let setting = manifest(
        "obs",
        "1.0.0",
        "1.1",
        &format!("{pickers}[[settings]]\nkey = \"watched\"\nkind = \"item\"\npicker = \"sources\"\n"),
    );
    assert!(Manifest::parse(&setting).is_ok());
}

#[test]
fn contracts_up_to_1_2_load_and_1_3_does_not() {
    assert_eq!(CONTRACT, (1, 2));
    for ok in ["1.0", "1.1", "1.2"] {
        assert!(Manifest::parse(&manifest("obs", "1.0.0", ok, "")).is_ok(), "{ok}");
    }
    for bad in ["1.3", "2.0", "0.9"] {
        assert!(
            matches!(
                Manifest::parse(&manifest("obs", "1.0.0", bad, "")),
                Err(ManifestError::Incompatible { needs }) if needs == bad
            ),
            "{bad}"
        );
    }
}

#[test]
fn pick_items_skip_empty_fields_in_json() {
    let item = PickItem {
        id: "a".into(),
        label: "A".into(),
        subtitle: None,
        group: Some("g".into()),
        icon: None,
        state: None,
    };
    let json = serde_json::to_string(&item).expect("json");
    assert_eq!(json, r#"{"id":"a","label":"A","group":"g"}"#);
    let back: PickItem = serde_json::from_str(&json).expect("parse");
    assert_eq!(back, item);
}

// ---- Contract 1.0 keeps its promise: [[pickers]] accepted and ignored ----

#[test]
fn contract_1_0_ignores_pickers_and_their_checks() {
    // A picker that is not declared, a multi picker on a param, a duplicate and an odd-shaped
    // table: all refused for 1.1 (above), all accepted and ignored for 1.0.
    let text = manifest(
        "obs",
        "1.0.0",
        "1.0",
        "[[pickers]]\nid = \"sources\"\nmulti = true\n\
         [[pickers]]\nid = \"sources\"\n\
         [[pickers]]\nstyle = \"carousel\"\nwhatever = 3\n\
         [[settings]]\nkey = \"room\"\nkind = \"item\"\npicker = \"rooms\"\n\
         [[actions]]\nid = \"go\"\nname = \"Go\"\nparams = [{ key = \"src\", kind = \"item\", picker = \"sources\" }]\n",
    );
    let m = Manifest::parse(&text).expect("a 1.0 manifest keeps loading");
    assert!(m.pickers.is_empty(), "{:?}", m.pickers);
    assert!(m.picker("sources").is_none());
    assert_eq!(m.settings[0].picker, Some("rooms".into()));
}

#[test]
fn contract_1_0_accepts_a_pickers_value_of_any_shape() {
    for extra in ["pickers = 5\n", "pickers = [\"a\", 2]\n", "[pickers]\nid = 1\n"] {
        let text = manifest("obs", "1.0.0", "1.0", extra);
        let m = Manifest::parse(&text).unwrap_or_else(|e| panic!("{extra}: {e}"));
        assert!(m.pickers.is_empty());
    }
}

#[test]
fn contract_1_1_still_checks_pickers() {
    let text = manifest("obs", "1.0.0", "1.1", "[[pickers]]\nstyle = \"list\"\n");
    assert!(matches!(Manifest::parse(&text), Err(ManifestError::Parse(_))));
    let text = manifest(
        "obs",
        "1.0.0",
        "1.1",
        "[[settings]]\nkey = \"room\"\nkind = \"item\"\npicker = \"rooms\"\n",
    );
    assert!(matches!(
        Manifest::parse(&text),
        Err(ManifestError::UnknownPicker { .. })
    ));
}

#[test]
fn short_names_drop_a_shared_leading_phrase() {
    use ulanzi_core::plugin::key_set::short_names;
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        short_names(&names(&[
            "Satya Bedroom Floor Lamp",
            "Satya Bedroom Fan",
            "Satya Bedroom Lights",
            "Privacy Satya",
            "Satya Fan Speed",
        ])),
        ["Floor Lamp", "Fan", "Lights", "Privacy Satya", "Fan Speed"]
    );
    // Alone, or the whole name shared: kept.
    assert_eq!(short_names(&names(&["Kitchen Lamp"])), ["Kitchen Lamp"]);
    assert_eq!(short_names(&names(&["Lamp", "Lamp"])), ["Lamp", "Lamp"]);
}

#[test]
fn icons_and_key_sets_are_checked() {
    let base = "id = \"xy\"\nname = \"X\"\nversion = \"1.0.0\"\ncontract = \"1.2\"\n";
    let parse = |extra: &str| Manifest::parse(&format!("{base}{extra}"));
    parse("icon = \"assets/icon.png\"\nsymbolic_icon = \"assets/s.svg\"\ncolor = \"#18bcf2\"\n")
        .expect("valid");
    assert!(
        parse("icon = \"mdi:lightbulb\"\n").is_ok(),
        "a theme name still works"
    );
    assert!(parse("icon = \"assets/../x.svg\"\n").is_err());
    assert!(
        parse("symbolic_icon = \"assets/s.png\"\n").is_err(),
        "one-colour icons are SVG"
    );
    assert!(
        parse("symbolic_icon = \"s.svg\"\n").is_err(),
        "must be in assets/"
    );
    assert!(parse("color = \"blue\"\n").is_err());
    let action = "[[actions]]\nid = \"go\"\nname = \"Go\"\nicon = \"assets/go.svg\"\n[[pickers]]\nid = \"p\"\nmulti = true\n";
    let set = |keys: &str| {
        parse(&format!(
            "{action}[[key_sets]]\nid = \"keys\"\nname = \"S\"\npicker = \"p\"\n[[key_sets.variants]]\nkeys = [{keys}]\n"
        ))
    };
    let m = set("{ action = \"go\", params = { id = \"$id\" }, label = \"$name\" }").expect("valid");
    let item = ulanzi_core::plugin::key_set::Picked {
        id: "a.b".into(),
        name: "Bee".into(),
        group: None,
    };
    let keys = m.key_set("keys").expect("set").keys_for(&m, &item, true);
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].look.label.text, "Bee");
    assert_eq!(
        keys[0].look.source,
        ulanzi_core::model::ImageSource::Plugin {
            plugin: "xy".into(),
            icon: "assets/go.svg".into()
        },
        "the action's icon when the key has none"
    );
    match &keys[0].tap {
        Some(ulanzi_core::model::Action::Plugin { params, .. }) => {
            assert_eq!(params["id"].as_str(), Some("a.b"))
        }
        other => panic!("{other:?}"),
    }
    assert!(set("{ action = \"nope\" }").is_err(), "unknown action");
    assert!(set("").is_err(), "no keys");
}

#[test]
fn contract_1_2_fields_and_min_app() {
    let m = |contract: &str, extra: &str| {
        Manifest::parse(&format!(
            "id = \"xy\"\nname = \"X\"\nversion = \"1.0.0\"\ncontract = \"{contract}\"\n{extra}"
        ))
    };
    let newer = "color = \"#112233\"\n[[actions]]\nid = \"go\"\nname = \"Go\"\nlabels = [{ name = \"State\", text = \"{x.s}\" }]\n";
    // 1.1 keeps its promise: the 1.2 fields are accepted and ignored.
    let old = m("1.1", newer).expect("1.1");
    assert_eq!(old.color, None);
    assert!(old.actions[0].labels.is_empty());
    let new = m("1.2", newer).expect("1.2");
    assert_eq!(new.color.as_deref(), Some("#112233"));
    assert_eq!(new.actions[0].labels.len(), 1);
    // min_app: this app or older installs; newer says which version it needs.
    assert!(m("1.2", "min_app = \"2.0.0\"\n").is_ok());
    assert!(m("1.2", &format!("min_app = \"{}\"\n", ulanzi_core::APP_VERSION)).is_ok());
    assert_eq!(
        m("1.2", "min_app = \"99.0.0\"\n"),
        Err(ManifestError::NeedsApp("99.0.0".into()))
    );
    assert!(m("1.2", "min_app = \"two\"\n").is_err());
    assert!(m("1.3", "").is_err(), "a newer contract than this app has");
}
