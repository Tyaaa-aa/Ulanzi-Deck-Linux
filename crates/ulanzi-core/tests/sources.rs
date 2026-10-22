use std::path::{Path, PathBuf};

use ulanzi_core::Color;
use ulanzi_core::model::{IconSet, ImageSource, KeyLook};
use ulanzi_core::render::{IconLibrary, RenderCtx, render_key};
use ulanzi_core::{apps, steam};

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, text).expect("write");
}

fn png(p: &Path, rgb: [u8; 3]) {
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    image::RgbImage::from_pixel(64, 64, image::Rgb(rgb))
        .save(p)
        .expect("image");
}

fn decode(png: &[u8]) -> image::RgbaImage {
    image::load_from_memory(png).expect("png").to_rgba8()
}

fn ctx(dir: &Path) -> RenderCtx<'_> {
    RenderCtx {
        profile_dir: dir,
        icons: IconLibrary::bundled(),
    }
}

#[test]
fn scan_reads_desktop_entries() {
    let d = tempfile::tempdir().expect("tmp");
    let a = d.path().join("a/applications");
    let b = d.path().join("b/applications");
    write(
        &a.join("firefox.desktop"),
        "[Desktop Entry]\nType=Application\nName=Firefox\nIcon=firefox\nExec=firefox %u\nKeywords=web;browser;\n",
    );
    write(
        &a.join("hidden.desktop"),
        "[Desktop Entry]\nName=Hidden\nExec=x\nNoDisplay=true\n",
    );
    write(
        &b.join("firefox.desktop"),
        "[Desktop Entry]\nName=Old Firefox\nExec=old\n",
    );
    write(&b.join("code.desktop"), "[Desktop Entry]\nName=Code\nExec=code\n");
    let apps = apps::scan(&[a, b]);
    let names: Vec<&str> = apps.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(
        names,
        ["Code", "Firefox"],
        "sorted by name, hidden skipped, first dir wins"
    );
    let ff = apps
        .iter()
        .find(|x| x.desktop_id == "firefox.desktop")
        .expect("firefox");
    assert_eq!(ff.exec, vec!["firefox".to_string()]);
    assert_eq!(ff.icon.as_deref(), Some("firefox"));
    assert!(ff.keywords.contains(&"browser".to_string()));
}

#[test]
fn find_icon_prefers_scalable() {
    let d = tempfile::tempdir().expect("tmp");
    let root = d.path().join("icons");
    write(&root.join("hicolor/48x48/apps/app.png"), "x");
    write(&root.join("hicolor/scalable/apps/app.svg"), "<svg/>");
    write(&root.join("mytheme/64x64/apps/other.png"), "x");
    let found = apps::find_icon("app", std::slice::from_ref(&root), &["mytheme"]).expect("found");
    assert!(found.ends_with("scalable/apps/app.svg"), "{found:?}");
    assert!(apps::find_icon("other", std::slice::from_ref(&root), &["mytheme"]).is_some());
    assert!(apps::find_icon("missing", &[root], &["mytheme"]).is_none());
    assert_eq!(
        apps::find_icon("/abs/x.png", &[], &[]),
        Some(PathBuf::from("/abs/x.png"))
    );
}

#[test]
fn steam_scan_two_libraries() {
    let d = tempfile::tempdir().expect("tmp");
    let root = d.path().join("Steam");
    let other = d.path().join("Games");
    write(
        &root.join("steamapps/libraryfolders.vdf"),
        &format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            root.display(),
            other.display()
        ),
    );
    write(
        &root.join("steamapps/appmanifest_620.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"620\"\n\t\"name\"\t\t\"Portal 2\"\n}\n",
    );
    write(
        &other.join("steamapps/appmanifest_1145360.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"1145360\"\n\t\"name\"\t\t\"Hades\"\n}\n",
    );
    write(
        &root.join("steamapps/appmanifest_1070560.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"1070560\"\n\t\"name\"\t\t\"Steam Linux Runtime 1.0 (scout)\"\n}\n",
    );
    png(
        &root.join("appcache/librarycache/620/library_600x900.jpg"),
        [1, 2, 3],
    );
    let games = steam::scan(&root);
    let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["Hades", "Portal 2"], "tools and runtimes are skipped");
    assert!(
        games
            .iter()
            .find(|g| g.appid == 620)
            .and_then(|g| g.art.clone())
            .is_some()
    );
}

#[test]
fn render_text_draws_pixels() {
    let d = tempfile::tempdir().expect("tmp");
    let look = KeyLook {
        source: ImageSource::Text {
            content: "OBS".into(),
        },
        foreground: Color::WHITE,
        ..KeyLook::default()
    };
    let r = render_key(&look, &ctx(d.path())).expect("drawn");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let img = decode(&r.png);
    let white = img
        .pixels()
        .filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200)
        .count();
    assert!(white > 200, "only {white} text pixels");
}

#[test]
fn render_emoji_uses_color() {
    let d = tempfile::tempdir().expect("tmp");
    let look = KeyLook {
        source: ImageSource::Emoji { ch: "🔥".into() },
        ..KeyLook::default()
    };
    let r = render_key(&look, &ctx(d.path())).expect("drawn");
    if !r.warnings.is_empty() {
        eprintln!("no colour emoji font: {:?}", r.warnings);
        return;
    }
    let img = decode(&r.png);
    let coloured = img.pixels().filter(|p| p[0] > 150 && p[2] < 100).count();
    assert!(coloured > 100, "only {coloured} orange-ish pixels");
}

#[test]
fn render_app_icon_from_theme() {
    let d = tempfile::tempdir().expect("tmp");
    let data = d.path().join("data");
    write(
        &data.join("applications/tool.desktop"),
        "[Desktop Entry]\nName=Tool\nExec=tool\nIcon=tool\n",
    );
    png(&data.join("icons/hicolor/64x64/apps/tool.png"), [250, 10, 10]);
    temp_env(&data, || {
        let look = KeyLook {
            source: ImageSource::App {
                desktop_id: "tool.desktop".into(),
            },
            ..KeyLook::default()
        };
        let r = render_key(&look, &ctx(d.path())).expect("drawn");
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let p = decode(&r.png).get_pixel(98, 98).0;
        assert!(p[0] > 200 && p[1] < 60, "{p:?}");
    });
}

#[test]
fn brand_icon_tinted() {
    let d = tempfile::tempdir().expect("tmp");
    let look = KeyLook {
        source: ImageSource::Icon {
            set: IconSet::Brands,
            name: "steam".into(),
        },
        foreground: Color::rgb(0, 200, 0),
        ..KeyLook::default()
    };
    let r = render_key(&look, &ctx(d.path())).expect("drawn");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let green = decode(&r.png)
        .pixels()
        .filter(|p| p[1] > 150 && p[0] < 60)
        .count();
    assert!(green > 500, "{green}");
}

/// Point XDG_DATA_HOME/XDG_DATA_DIRS at `data` while `f` runs (tests in this file don't run it in
/// parallel with other env readers).
fn temp_env(data: &Path, f: impl FnOnce()) {
    apps::with_data_dirs_override(vec![data.to_path_buf()], f);
}

#[test]
fn apps_have_groups_and_window_class() {
    let d = tempfile::tempdir().expect("tmp");
    let a = d.path().join("applications");
    write(
        &a.join("org.blender.Blender.desktop"),
        "[Desktop Entry]\nType=Application\nName=Blender\nExec=blender\nCategories=Graphics;3DGraphics;\nStartupWMClass=Blender\n",
    );
    write(
        &a.join("hades.desktop"),
        "[Desktop Entry]\nType=Application\nName=Hades\nExec=steam steam://rungameid/1145360\nCategories=Game;\n",
    );
    write(
        &a.join("misc.desktop"),
        "[Desktop Entry]\nType=Application\nName=Misc\nExec=misc\n",
    );
    let list = apps::scan(&[a]);
    let get = |n: &str| list.iter().find(|x| x.name == n).expect(n).clone();
    let blender = get("Blender");
    assert_eq!(blender.wm_class.as_deref(), Some("Blender"));
    assert_eq!(blender.group(), apps::AppGroup::Graphics);
    assert_eq!(get("Hades").group(), apps::AppGroup::Games);
    assert_eq!(get("Misc").group(), apps::AppGroup::Other);
    // Groups have names and a fixed order for section headers.
    assert_eq!(apps::AppGroup::Games.title(), "Games");
    assert!(apps::AppGroup::ALL.contains(&apps::AppGroup::Other));
}
