use ulanzi_core::autoswitch::{Rule, Window, aliases, pick_profile};
use uuid::Uuid;

fn win(class: &str, desktop: &str) -> Window {
    Window {
        class: class.into(),
        desktop_id: desktop.into(),
        title: String::new(),
    }
}

#[test]
fn aliases_cover_desktop_id_and_window_class() {
    let a = aliases("org.kde.spectacle.desktop", None);
    assert!(a.contains(&"org.kde.spectacle".to_string()));
    let steam = aliases("steam:1145360", None);
    assert_eq!(steam, vec!["steam_app_1145360".to_string()]);
}

#[test]
fn picks_the_matching_profile_or_the_default() {
    let (def, art, game) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
    let rules = vec![
        Rule {
            profile: art,
            apps: vec![
                aliases("org.blender.Blender.desktop", None),
                vec!["blender".into()],
            ],
            priority: 0,
        },
        Rule {
            profile: game,
            apps: vec![aliases("steam:1145360", None)],
            priority: 0,
        },
    ];
    // Matched by window class, case-insensitively.
    assert_eq!(pick_profile(&rules, &win("Blender", ""), def), Some(art));
    // Matched by the app's desktop file.
    assert_eq!(
        pick_profile(&rules, &win("", "org.blender.Blender"), def),
        Some(art)
    );
    // Steam games by app id.
    assert_eq!(
        pick_profile(&rules, &win("steam_app_1145360", ""), def),
        Some(game)
    );
    // Anything else goes back to the default profile.
    assert_eq!(pick_profile(&rules, &win("firefox", "firefox"), def), Some(def));
    // The desktop shell, launchers and this app itself never switch.
    for shell in [
        "plasmashell",
        "org.kde.plasmashell",
        "krunner",
        "",
        "io.github.tyaaa_aa.ulanzideck",
    ] {
        assert_eq!(pick_profile(&rules, &win(shell, shell), def), None, "{shell}");
    }
}

#[test]
fn higher_priority_wins() {
    let (def, a, b) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
    let rules = vec![
        Rule {
            profile: a,
            apps: vec![vec!["code".into()]],
            priority: 0,
        },
        Rule {
            profile: b,
            apps: vec![vec!["code".into()]],
            priority: 5,
        },
    ];
    assert_eq!(pick_profile(&rules, &win("code", ""), def), Some(b));
}

#[test]
fn steam_shortcuts_match_the_game_window() {
    let app = ulanzi_core::apps::AppEntry {
        desktop_id: "Hades.desktop".into(),
        name: "Hades".into(),
        icon: None,
        exec: vec!["steam".into(), "steam://rungameid/1145360".into()],
        keywords: vec![],
        categories: vec!["Game".into()],
        wm_class: None,
    };
    assert!(aliases("Hades.desktop", Some(&app)).contains(&"steam_app_1145360".to_string()));
}

#[test]
fn auto_switch_is_on_by_default() {
    let s: ulanzi_core::model::Settings = toml::from_str("").expect("defaults");
    assert!(s.auto_switch);
}
