use chrono::NaiveDate;
use ulanzi_core::Color;
use ulanzi_core::model::*;
use ulanzi_core::render::Fonts;
use ulanzi_core::render::screen::*;

fn at(h: u32, m: u32, s: u32) -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 23)
        .expect("date")
        .and_hms_opt(h, m, s)
        .expect("time")
}

fn decode(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory(png).expect("png").to_rgba8()
}

fn style() -> ScreenStyle {
    ScreenStyle {
        background: Background::Theme,
        accent: Color::rgb(255, 0, 0),
        profile_dir: std::path::PathBuf::from("/nonexistent"),
    }
}

fn data() -> ScreenData {
    ScreenData {
        now: at(9, 41, 7),
        ..ScreenData::default()
    }
}

/// How many distinct colours (a blank picture has one).
fn colours(img: &image::RgbaImage) -> usize {
    let mut v: Vec<[u8; 4]> = img.pixels().map(|p| p.0).collect();
    v.sort();
    v.dedup();
    v.len()
}

#[test]
fn twelve_hour_and_hands() {
    assert_eq!(
        time_text(at(0, 30, 0), false, false),
        ("12:30".to_string(), Some("AM".to_string()))
    );
    assert_eq!(
        time_text(at(12, 30, 0), false, false),
        ("12:30".to_string(), Some("PM".to_string()))
    );
    assert_eq!(
        time_text(at(13, 5, 9), true, true),
        ("13:05:09".to_string(), None)
    );
    let (h, m, s) = hand_angles(at(3, 0, 0));
    assert!(
        (h - 90.0).abs() < 0.01 && m.abs() < 0.01 && s.abs() < 0.01,
        "{h} {m} {s}"
    );
    let (h, m, _) = hand_angles(at(6, 30, 0));
    assert!((h - 195.0).abs() < 0.01 && (m - 180.0).abs() < 0.01);
}

#[test]
fn each_face_draws() {
    let mut fonts = Fonts::system();
    for face in ClockFace::ALL {
        let w = Widget::Clock {
            face,
            hour24: true,
            seconds: true,
            date: true,
        };
        let img = decode(&render_widget(&w, &style(), &data(), &mut fonts));
        assert_eq!(img.dimensions(), (458, 196));
        assert!(colours(&img) > 20, "{face:?} draws something");
        assert!(
            img.pixels().any(|p| p.0[0] > 200 && p.0[1] < 60 && p.0[2] < 60),
            "{face:?} uses the accent colour"
        );
    }
}

#[test]
fn background_variants() {
    let mut fonts = Fonts::system();
    let mut s = style();
    s.background = Background::Solid {
        color: Color::rgb(10, 200, 30),
    };
    let img = decode(&render_widget(&Widget::Blank, &s, &data(), &mut fonts));
    assert_eq!(img.get_pixel(0, 0).0, [10, 200, 30, 255]);
    s.background = Background::Gradient {
        from: Color::rgb(0, 0, 0),
        to: Color::rgb(255, 255, 255),
    };
    let img = decode(&render_widget(&Widget::Blank, &s, &data(), &mut fonts));
    assert_ne!(img.get_pixel(0, 0).0, img.get_pixel(457, 195).0);
}

#[test]
fn sparkline_scales() {
    let pts = sparkline(&[0, 50, 100], 100.0, 40.0);
    assert_eq!(pts.len(), 3);
    assert_eq!(pts[0], (0.0, 40.0));
    assert_eq!(pts[1], (50.0, 20.0));
    assert_eq!(pts[2], (100.0, 0.0));
    assert!(sparkline(&[], 100.0, 40.0).is_empty());
}

#[test]
fn weather_code_icons() {
    assert_eq!(weather_icon(0), "sun");
    assert_eq!(weather_icon(61), "cloud-rain");
    assert_eq!(weather_icon(95), "cloud-lightning");
    assert_eq!(weather_icon(71), "snowflake");
    assert_eq!(weather_icon(45), "cloud-fog");
}

#[test]
fn each_widget_draws_with_and_without_data() {
    let mut fonts = Fonts::system();
    let full = ScreenData {
        now: at(9, 41, 7),
        stats: Some(StatsData {
            cpu: vec![10, 40, 30],
            ram: vec![50, 51],
            gpu: vec![5],
            temp: Some(61),
            net_down: 1_200_000,
            net_up: 30_000,
        }),
        media: Some(MediaData {
            title: "Song".into(),
            artist: "Artist".into(),
            art: None,
            position_s: Some(30),
            length_s: Some(200),
            playing: true,
        }),
        timer: Some(TimerData {
            remaining_s: 754,
            total_s: 1500,
            running: true,
            phase: Some("Focus".into()),
        }),
        weather: Some(WeatherData::Ok {
            temp: 31.4,
            code: 2,
            place: "Singapore".into(),
            high: 33.0,
            low: 26.0,
        }),
        command: Some("up 3 hours".into()),
        picture: None,
    };
    for w in Widget::all_kinds() {
        for d in [&full, &data()] {
            let img = decode(&render_widget(&w, &style(), d, &mut fonts));
            assert_eq!(img.dimensions(), (458, 196), "{}", w.title());
            if w != Widget::Blank {
                assert!(colours(&img) > 10, "{} draws something", w.title());
            }
        }
    }
    let err = ScreenData {
        weather: Some(WeatherData::Err("offline".into())),
        ..data()
    };
    let w = Widget::Weather {
        place: "Nowhere".into(),
        fahrenheit: false,
    };
    assert!(colours(&decode(&render_widget(&w, &style(), &err, &mut fonts))) > 10);
}

#[test]
fn volume_overlay_draws_over_a_picture() {
    let mut fonts = Fonts::system();
    let base = render_widget(&Widget::Blank, &style(), &data(), &mut fonts);
    let img = decode(&volume_overlay(
        Some(&base),
        45,
        false,
        Color::rgb(255, 0, 0),
        &mut fonts,
    ));
    assert_eq!(img.dimensions(), (458, 196));
    assert!(img.pixels().any(|p| p.0[0] > 200 && p.0[1] < 60));
    let muted = decode(&volume_overlay(None, 45, true, Color::rgb(255, 0, 0), &mut fonts));
    assert!(colours(&muted) > 10);
}
