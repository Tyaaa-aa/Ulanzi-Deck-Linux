//! Draw every wide-screen widget with sample data into one picture (for docs and the README).
//!
//! `cargo run -p ulanzi-core --example widgets_sheet -- out.png`

use ulanzi_core::Color;
use ulanzi_core::model::*;
use ulanzi_core::obs::{ObsLive, ObsStats};
use ulanzi_core::render::Fonts;
use ulanzi_core::render::screen::*;

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "widgets-sheet.png".into());
    let now = chrono::NaiveDate::from_ymd_opt(2026, 10, 23)
        .and_then(|d| d.and_hms_opt(14, 5, 37))
        .expect("date");
    let wave = |base: f32, amp: f32, k: f32| -> Vec<u8> {
        (0..60)
            .map(|i| (base + amp * ((i as f32) * k).sin()).clamp(0.0, 100.0) as u8)
            .collect()
    };
    let data = ScreenData {
        now,
        stats: Some(StatsData {
            cpu: wave(32.0, 14.0, 0.35),
            ram: wave(58.0, 3.0, 0.1),
            gpu: wave(21.0, 12.0, 0.5),
            temp: Some(54),
            net_down: 2_400_000,
            net_up: 180_000,
        }),
        media: Some(MediaData {
            title: "Midnight Drive".into(),
            artist: "The Sample Band".into(),
            art: None,
            position_s: Some(83),
            length_s: Some(214),
            playing: true,
        }),
        timer: Some(TimerData {
            remaining_s: 18 * 60 + 42,
            total_s: 25 * 60,
            running: true,
            phase: Some("Focus".into()),
        }),
        weather: Some(WeatherData::Ok {
            temp: 21.0,
            code: 2,
            place: "Oslo".into(),
            high: 24.0,
            low: 14.0,
        }),
        command: Some("main ✓ 3 commits ahead\nlast build 2 min ago".into()),
        picture: None,
        obs: Some(ObsLive {
            connected: true,
            program: "Gameplay".into(),
            recording: true,
            rec_secs: 754,
            streaming: true,
            stream_secs: 3725,
            stats: Some(ObsStats {
                fps: 60,
                cpu: 4,
                dropped_permille: 1,
            }),
            ..ObsLive::default()
        }),
    };
    let widgets = [
        (
            Widget::Clock {
                face: ClockFace::Digital,
                hour24: true,
                seconds: true,
                date: true,
            },
            Background::Theme,
        ),
        (
            Widget::Clock {
                face: ClockFace::Analog,
                hour24: true,
                seconds: true,
                date: true,
            },
            Background::Gradient {
                from: Color::rgb(0x10, 0x1c, 0x2e),
                to: Color::rgb(0x1c, 0x3d, 0x5a),
            },
        ),
        (
            Widget::NowPlaying {
                progress: true,
                blur_background: false,
            },
            Background::Theme,
        ),
        (
            Widget::System {
                stats: vec![Stat::Cpu, Stat::Gpu, Stat::Ram, Stat::Net],
                graphs: true,
            },
            Background::Theme,
        ),
        (
            Widget::Timer {
                minutes: 25,
                pomodoro: true,
            },
            Background::Solid {
                color: Color::rgb(0x1e, 0x1e, 0x1e),
            },
        ),
        (
            Widget::Weather {
                place: "Oslo".into(),
                fahrenheit: false,
                lat: None,
                lon: None,
            },
            Background::Gradient {
                from: Color::rgb(0x12, 0x2a, 0x45),
                to: Color::rgb(0x2a, 0x5d, 0x8c),
            },
        ),
        (
            Widget::Command {
                cmd: "git status -sb".into(),
                every_s: 30,
                title: "Repository".into(),
            },
            Background::Theme,
        ),
        (
            Widget::Obs { stats: true },
            Background::Gradient {
                from: Color::rgb(0x1a, 0x10, 0x2e),
                to: Color::rgb(0x4a, 0x1d, 0x5c),
            },
        ),
    ];
    let mut fonts = Fonts::system();
    let (w, h, gap) = (458u32, 196u32, 24u32);
    let cols = 2u32;
    let rows = (widgets.len() as u32).div_ceil(cols);
    let mut sheet = image::RgbaImage::from_pixel(
        cols * w + (cols + 1) * gap,
        rows * h + (rows + 1) * gap,
        image::Rgba([0x24, 0x24, 0x24, 255]),
    );
    for (i, (wd, bg)) in widgets.iter().enumerate() {
        let style = ScreenStyle {
            background: bg.clone(),
            accent: match wd {
                Widget::Obs { .. } => Color::rgb(0xc0, 0x61, 0xcb),
                Widget::Timer { .. } => Color::rgb(0xff, 0x78, 0x00),
                _ => default_accent(),
            },
            profile_dir: std::env::temp_dir(),
        };
        let png = render_widget(wd, &style, &data, &mut fonts);
        let img = image::load_from_memory(&png).expect("png").to_rgba8();
        let (c, r) = (i as u32 % cols, i as u32 / cols);
        image::imageops::overlay(
            &mut sheet,
            &img,
            (gap + c * (w + gap)) as i64,
            (gap + r * (h + gap)) as i64,
        );
    }
    sheet.save(&out).expect("save");
    println!("wrote {out}");
}
