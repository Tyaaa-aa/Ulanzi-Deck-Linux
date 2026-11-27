//! Draw what a deck shows for each profile's first page, as the service sends it: the 13 key
//! images and the wide screen, with key labels drawn in (on the device its firmware draws them).
//! For pictures of the deck, such as a 3D model's screens.
//!
//! `cargo run -p ulanzi-core --example deck_faces -- <sandbox> <out-dir>` writes
//! `<out-dir>/<profile name>/key-<n>.png` (196 px, n from 0) and `screen.png` (458x196).

use ulanzi_core::model::*;
use ulanzi_core::render::screen::*;
use ulanzi_core::render::{Fonts, IconLibrary, KEY_PX, RenderCtx, key_image};
use ulanzi_core::store::{ProfileLoad, Store};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(sandbox), Some(out)) = (args.next(), args.next()) else {
        eprintln!("usage: deck_faces <sandbox-dir> <out-dir>");
        std::process::exit(2);
    };
    let sandbox = std::path::PathBuf::from(sandbox);
    let out = std::path::PathBuf::from(out);
    let store = Store::at(
        sandbox.join("config/ulanzi-deck"),
        sandbox.join("state/ulanzi-deck"),
        sandbox.join("cache/ulanzi-deck"),
    );
    let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 26)
        .and_then(|d| d.and_hms_opt(20, 42, 0))
        .expect("date");
    let data = ScreenData {
        now,
        stats: Some(StatsData {
            cpu: (0..60)
                .map(|i| (30.0 + 12.0 * (i as f32 * 0.35).sin()) as u8)
                .collect(),
            ram: (0..60)
                .map(|i| (56.0 + 3.0 * (i as f32 * 0.1).sin()) as u8)
                .collect(),
            gpu: (0..60)
                .map(|i| (24.0 + 10.0 * (i as f32 * 0.5).sin()) as u8)
                .collect(),
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
            temp: 9.0,
            code: 2,
            place: "Oslo".into(),
            high: 11.0,
            low: 4.0,
        }),
        ..ScreenData::default()
    };
    let mut fonts = Fonts::system();
    for load in store.list_profiles() {
        let ProfileLoad::Ok(p) = load else { continue };
        let dir = out.join(&p.name);
        std::fs::create_dir_all(&dir).expect("out dir");
        let profile_dir = store.profile_dir(p.id);
        let ctx = RenderCtx {
            profile_dir: &profile_dir,
            icons: IconLibrary::bundled(),
        };
        let page = p.shown_page(0).expect("a first page");
        for n in 0..13u8 {
            let mut look = page.keys.get(&n).map(|k| k.shown_look()).unwrap_or_default();
            // The firmware draws labels under the picture; draw them the same way here.
            if look.label.position.is_none() {
                look.label.position = Some(LabelPosition::Bottom);
            }
            let img = key_image(&look, &ctx, None);
            assert_eq!(img.width(), KEY_PX);
            img.save(dir.join(format!("key-{n}.png"))).expect("save key");
        }
        if let ScreenConfig::Custom {
            widgets,
            background,
            accent,
            ..
        } = &p.screen
        {
            let style = ScreenStyle {
                background: background.clone(),
                accent: *accent,
                profile_dir: profile_dir.clone(),
            };
            if let Some(w) = widgets.first() {
                let png = render_widget(w, &style, &data, &mut fonts);
                std::fs::write(dir.join("screen.png"), png).expect("save screen");
            }
        }
        println!("{}", dir.display());
    }
}
