//! `ulanzi-deck`: the Ulanzi Deck app.

use std::path::PathBuf;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ulanzi_core::model::{Model, Profile};
use ulanzi_deck::state::AppState;
use ulanzi_deck::ui;

const APP_ID: &str = "io.github.tyaaa_aa.UlanziDeck";

/// `--screenshot` options: render the window to a PNG and exit (used to check the UI without a person
/// at the screen).
#[derive(Debug, Clone, Default)]
struct Shot {
    path: PathBuf,
    size: (i32, i32),
    dark: Option<bool>,
    select: Option<String>,
}

fn parse_size(s: &str) -> Option<(i32, i32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn parse_args() -> Option<Shot> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let path = get("--screenshot")?;
    Some(Shot {
        path: PathBuf::from(path),
        size: get("--size").and_then(|s| parse_size(&s)).unwrap_or((1280, 800)),
        dark: get("--theme").map(|t| t == "dark"),
        select: get("--select"),
    })
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn save_screenshot(window: &adw::ApplicationWindow, path: &std::path::Path) -> Result<(), String> {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let (w, h) = (window.width() as f64, window.height() as f64);
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w, h);
    let node = snapshot.to_node().ok_or("nothing drawn")?;
    let renderer = window.renderer().ok_or("no renderer")?;
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).map_err(|e| e.to_string())
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ULANZI_LOG").unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let shot = parse_args();
    let flags = if shot.is_some() {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        gio::ApplicationFlags::empty()
    };
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    app.connect_startup(|_| load_css());
    app.connect_activate(move |app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        let state = AppState::new(Profile::new_default("Default", Model::D200X));
        let win = ui::window::build(app, &state);
        if let Some(shot) = shot.clone() {
            if let Some(dark) = shot.dark {
                adw::StyleManager::default().set_color_scheme(if dark {
                    adw::ColorScheme::ForceDark
                } else {
                    adw::ColorScheme::ForceLight
                });
            }
            win.window.set_default_size(shot.size.0, shot.size.1);
        }
        win.window.present();
        let window = win.window.clone();
        let app = app.clone();
        let state2 = state.clone();
        let shot = shot.clone();
        ui::window::connect(&state, move || {
            let Some(shot) = shot else { return };
            if let Some(sel) = &shot.select {
                state2.select(|s| match sel.split_once(':') {
                    Some(("key", n)) => n
                        .split(',')
                        .filter_map(|k| k.parse().ok())
                        .for_each(|k| s.toggle(ulanzi_core::edit::Slot::Key(k))),
                    Some(("dial", n)) => s.click(ulanzi_core::edit::Slot::Dial(n.parse().unwrap_or(0))),
                    _ if sel == "screen" => s.click(ulanzi_core::edit::Slot::Screen),
                    _ => {}
                });
            }
            glib::timeout_add_local_once(Duration::from_millis(900), move || {
                match save_screenshot(&window, &shot.path) {
                    Ok(()) => println!("saved {}", shot.path.display()),
                    Err(e) => eprintln!("screenshot failed: {e}"),
                }
                app.quit();
            });
        });
    });
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("800x700"), Some((800, 700)));
        assert_eq!(parse_size("800"), None);
    }
}
