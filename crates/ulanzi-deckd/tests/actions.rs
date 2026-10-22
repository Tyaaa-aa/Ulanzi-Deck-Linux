use std::sync::{Arc, Mutex};
use std::time::Duration;

use evdev::KeyCode;
use ulanzi_core::model::{Action, AudioTarget, Step, VolumeOp};
use ulanzi_deckd::actions::audio::{app_volume_argv, match_sink_inputs, play_sound_argv, volume_argv};
use ulanzi_deckd::actions::keys::parse_combo;
use ulanzi_deckd::actions::launch::exec_from_desktop;
use ulanzi_deckd::actions::media::{PlaybackStatus, pick_player};
use ulanzi_deckd::actions::{ActionError, ActionRunner, run_multi};

fn argv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn combo_super_f12() {
    assert_eq!(
        parse_combo("super+F12").expect("ok"),
        vec![KeyCode::KEY_LEFTMETA, KeyCode::KEY_F12]
    );
}

#[test]
fn combo_ctrl_alt_t() {
    assert_eq!(
        parse_combo("ctrl+alt+t").expect("ok"),
        vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTALT, KeyCode::KEY_T]
    );
    assert_eq!(
        parse_combo("Control + Shift + Return").expect("ok"),
        vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_ENTER]
    );
    assert_eq!(
        parse_combo("XF86AudioPlay").expect("ok"),
        vec![KeyCode::KEY_PLAYPAUSE]
    );
}

#[test]
fn combo_unknown_errors() {
    let err = parse_combo("ctrl+bogus").expect_err("unknown key");
    assert!(err.to_string().contains("bogus"), "{err}");
    assert!(parse_combo("").is_err());
}

#[test]
fn desktop_exec_strips_field_codes() {
    assert_eq!(
        exec_from_desktop("[Desktop Entry]\nName=F\nExec=firefox %u\n"),
        Some(argv(&["firefox"]))
    );
    assert_eq!(
        exec_from_desktop("[Desktop Entry]\nExec=\"/opt/a b/app\" --x %F\n"),
        Some(argv(&["/opt/a b/app", "--x"]))
    );
    assert_eq!(
        exec_from_desktop("[Desktop Action new]\nExec=wrong\n[Desktop Entry]\nExec=right --flag\n"),
        Some(argv(&["right", "--flag"]))
    );
    assert_eq!(exec_from_desktop("[Desktop Entry]\nName=no exec\n"), None);
}

fn players() -> Vec<(String, PlaybackStatus)> {
    vec![
        (
            "org.mpris.MediaPlayer2.firefox.instance1".into(),
            PlaybackStatus::Paused,
        ),
        ("org.mpris.MediaPlayer2.spotify".into(), PlaybackStatus::Playing),
        ("org.mpris.MediaPlayer2.vlc".into(), PlaybackStatus::Stopped),
    ]
}

#[test]
fn pick_player_prefers_playing() {
    assert_eq!(
        pick_player(&players(), None).as_deref(),
        Some("org.mpris.MediaPlayer2.spotify")
    );
    let no_playing = vec![players()[0].clone(), players()[2].clone()];
    assert_eq!(
        pick_player(&no_playing, None).as_deref(),
        Some("org.mpris.MediaPlayer2.firefox.instance1")
    );
    assert_eq!(pick_player(&[], None), None);
}

#[test]
fn pick_player_wanted_substring() {
    assert_eq!(
        pick_player(&players(), Some("VLC")).as_deref(),
        Some("org.mpris.MediaPlayer2.vlc")
    );
    assert_eq!(pick_player(&players(), Some("mpv")), None);
}

#[test]
fn volume_argv_default_up() {
    assert_eq!(
        volume_argv(&AudioTarget::Default, &VolumeOp::Up, 2),
        vec![argv(&["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "2%+"])]
    );
    assert_eq!(
        volume_argv(
            &AudioTarget::Device {
                name: "alsa_output.usb".into()
            },
            &VolumeOp::Set { percent: 30 },
            0
        ),
        vec![argv(&["pactl", "set-sink-volume", "alsa_output.usb", "30%"])]
    );
}

#[test]
fn volume_argv_input_mute() {
    assert_eq!(
        volume_argv(&AudioTarget::Input, &VolumeOp::Mute, 5),
        vec![argv(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "toggle"])]
    );
}

#[test]
fn match_sink_inputs_discord() {
    let json = include_str!("fixtures/sink-inputs.json");
    assert_eq!(match_sink_inputs(json, "discord"), vec![(530, 100), (2050, 30)]);
    assert_eq!(match_sink_inputs(json, "Vivaldi"), vec![(2049, 50)]);
    assert!(match_sink_inputs(json, "spotify").is_empty());
    assert!(match_sink_inputs("not json", "x").is_empty());
}

#[test]
fn app_up_respects_limit_100() {
    let inputs = [(530, 100), (2050, 30)];
    assert_eq!(
        app_volume_argv(&inputs, &VolumeOp::Up, 4, true),
        vec![argv(&["pactl", "set-sink-input-volume", "2050", "+4%"])]
    );
    assert_eq!(app_volume_argv(&inputs, &VolumeOp::Up, 4, false).len(), 2);
    assert_eq!(
        app_volume_argv(&inputs, &VolumeOp::Mute, 4, true),
        vec![
            argv(&["pactl", "set-sink-input-mute", "530", "toggle"]),
            argv(&["pactl", "set-sink-input-mute", "2050", "toggle"])
        ]
    );
}

#[test]
fn play_sound_argv_target() {
    assert_eq!(play_sound_argv("/x/y.mp3", None), argv(&["pw-play", "/x/y.mp3"]));
    assert_eq!(
        play_sound_argv("/x/y.mp3", Some("speakers")),
        argv(&["pw-play", "--target", "speakers", "/x/y.mp3"])
    );
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(Action, tokio::time::Instant)>>);

#[async_trait::async_trait]
impl ActionRunner for Recorder {
    async fn run(&self, a: &Action) -> Result<(), ActionError> {
        self.0
            .lock()
            .expect("lock")
            .push((a.clone(), tokio::time::Instant::now()));
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn multi_runs_in_order_with_delays() {
    let r = Arc::new(Recorder::default());
    let start = tokio::time::Instant::now();
    let steps = vec![
        Step {
            action: Action::Shortcut { keys: "a".into() },
            delay_ms: 0,
        },
        Step {
            action: Action::Shortcut { keys: "b".into() },
            delay_ms: 150,
        },
    ];
    run_multi(r.as_ref(), &steps).await.expect("ok");
    let ran = r.0.lock().expect("lock").clone();
    assert_eq!(ran.len(), 2);
    assert_eq!(ran[0].0, Action::Shortcut { keys: "a".into() });
    assert!(ran[1].1 - start >= Duration::from_millis(150));
}

#[test]
fn launched_apps_get_their_own_scope_under_systemd() {
    use ulanzi_deckd::actions::command::wrap_in_scope;
    let argv = vec!["firefox".to_string(), "--new-window".to_string()];
    assert_eq!(
        wrap_in_scope(&argv, false, true),
        argv,
        "not running under systemd"
    );
    assert_eq!(
        wrap_in_scope(&argv, true, false),
        argv,
        "no systemd-run available"
    );
    let wrapped = wrap_in_scope(&argv, true, true);
    assert_eq!(&wrapped[..4], &["systemd-run", "--user", "--scope", "--quiet"]);
    assert_eq!(&wrapped[wrapped.len() - 2..], &argv[..]);
}

#[tokio::test]
async fn run_capture_times_out() {
    use ulanzi_deckd::actions::command::run_capture_timeout;
    let t = std::time::Instant::now();
    let r = run_capture_timeout(
        &["sleep".to_string(), "5".to_string()],
        Duration::from_millis(200),
    )
    .await;
    assert!(r.is_err());
    assert!(t.elapsed() < Duration::from_secs(2));
}
