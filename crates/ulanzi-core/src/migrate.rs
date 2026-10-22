//! One-time import of the Python version's `~/.config/ulanzi/config.yaml` (spec §11).
//!
//! The old file is only read, never written.

use std::path::{Path, PathBuf};

use serde_norway::{Mapping, Value};

use crate::model::*;
use crate::store::{Store, write_atomic};
use crate::{Color, CoreError};

const MARKER: &str = "migrated-from-legacy";
const SCREEN_INDEX: u64 = 13;
const SMALL_INDICES: [u64; 2] = [15, 16];
const DIAL_INDICES: [u64; 3] = [17, 18, 19];

#[derive(Debug, Clone, PartialEq)]
pub struct MigrationReport {
    pub profile_id: uuid::Uuid,
    pub keys: usize,
    pub warnings: Vec<String>,
}

/// `~/.config/ulanzi/config.yaml`.
pub fn legacy_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("ulanzi").join("config.yaml"))
}

/// True when a previous import already ran (see [`migrate_legacy`]'s `force`).
pub fn already_migrated(store: &Store) -> bool {
    store.state_dir().join(MARKER).exists()
}

/// Import the legacy config into a new profile "Default" and merge its settings.
/// Refuses to run twice unless `force` is set.
pub fn migrate_legacy(yaml: &Path, store: &Store, force: bool) -> Result<MigrationReport, CoreError> {
    if already_migrated(store) && !force {
        return Err(CoreError::AlreadyMigrated);
    }
    let text = std::fs::read_to_string(yaml).map_err(|e| CoreError::io(yaml, e))?;
    let root: Value = serde_norway::from_str(&text).map_err(|e| CoreError::Parse {
        path: yaml.to_path_buf(),
        message: e.to_string(),
    })?;
    let base_dir = yaml.parent().unwrap_or(Path::new("."));

    let mut m = Migrator { warnings: Vec::new() };
    let mut profile = Profile::new_default("Default", Model::D200X);
    let icons_dir = store.profile_dir(profile.id).join("icons");

    let buttons = root
        .get("buttons")
        .and_then(Value::as_sequence)
        .cloned()
        .unwrap_or_default();
    let mut screen_tap = None;
    for (i, b) in buttons.iter().enumerate() {
        let i = i as u64;
        if b.is_null() {
            continue;
        }
        match i {
            0..=12 => {
                let key = m.key(i, b, base_dir, &icons_dir);
                profile.pages[0].keys.insert(i as u8, key);
            }
            SCREEN_INDEX => screen_tap = m.action_of(b, &format!("key {i}")),
            i if SMALL_INDICES.contains(&i) => {
                if let Some(a) = m.action_of(b, &format!("small button {i}")) {
                    let slot = if i == SMALL_INDICES[0] {
                        &mut profile.small_buttons.left
                    } else {
                        &mut profile.small_buttons.right
                    };
                    *slot = Some(a);
                }
            }
            other => m.warn(format!("button {other}: not a key on this device, skipped")),
        }
    }

    if let Some(dials) = root.get("dials").and_then(Value::as_mapping) {
        for (k, v) in dials {
            let Some(idx) = k.as_u64().or_else(|| k.as_str().and_then(|s| s.parse().ok())) else {
                continue;
            };
            let Some(pos) = DIAL_INDICES.iter().position(|d| *d == idx) else {
                m.warn(format!("dial {idx}: unknown dial index, skipped"));
                continue;
            };
            let ctx = format!("dial {}", pos + 1);
            let dial = &mut profile.pages[0].dials[pos];
            dial.press = v.get("click").and_then(|a| m.action_of(a, &ctx));
            dial.left = v.get("left").and_then(|a| m.action_of(a, &ctx));
            dial.right = v.get("right").and_then(|a| m.action_of(a, &ctx));
        }
    }

    let clock_mode = root.get("clock_mode").and_then(Value::as_u64).unwrap_or(1);
    profile.screen = match clock_mode {
        0 => ScreenConfig::FirmwareStats,
        2 => ScreenConfig::Custom {
            widgets: vec![Widget::Blank],
            tap: screen_tap.take(),
            hold: None,
        },
        3 => ScreenConfig::Custom {
            widgets: vec![Widget::NowPlaying],
            tap: screen_tap.take().or(Some(Action::Media {
                control: MediaControl::PlayPause,
                player: None,
            })),
            hold: None,
        },
        _ => ScreenConfig::default(),
    };
    if screen_tap.is_some() {
        m.warn(
            "the clock button's action was dropped: the firmware clock/stats screen has no tap action".into(),
        );
    }

    let mut settings = store.load_settings()?;
    m.settings(&root, &mut settings);
    settings.default_profile = Some(profile.id);

    let keys = profile.pages[0].keys.len();
    store.save_profile(&profile)?;
    store.save_settings(&settings)?;
    let marker = store.state_dir().join(MARKER);
    write_atomic(&marker, yaml.to_string_lossy().as_bytes()).map_err(|e| CoreError::io(&marker, e))?;
    Ok(MigrationReport {
        profile_id: profile.id,
        keys,
        warnings: m.warnings,
    })
}

struct Migrator {
    warnings: Vec<String>,
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.is_empty()).map(str::to_string)
}

/// The old icon picker saved Lucide icons as `<name>_bg_<rrggbb>_fg_<rrggbb>.png`; turn those back into
/// icons (sharper, and play/pause can swap live).
fn generated_icon(path: &str) -> Option<(String, Color, Color)> {
    let stem = Path::new(path).file_stem()?.to_str()?;
    let (rest, fg) = stem.rsplit_once("_fg_")?;
    let (name, bg) = rest.rsplit_once("_bg_")?;
    let bg: Color = format!("#{bg}").parse().ok()?;
    let fg: Color = format!("#{fg}").parse().ok()?;
    crate::render::IconLibrary::bundled().svg(IconSet::Lucide, name)?;
    Some((name.to_string(), bg, fg))
}

/// `pw-play '/path'` and friends, as written by the old Soundboard picker.
fn sound_path(cmd: &str) -> Option<String> {
    let cmd = cmd.trim();
    let rest = ["pw-play ", "paplay ", "aplay ", "play "]
        .iter()
        .find_map(|p| cmd.strip_prefix(p))?;
    let inner = rest.trim().strip_prefix('\'')?.strip_suffix('\'')?;
    // The picker escaped quotes as '\''; undo that.
    Some(inner.replace("'\\''", "'"))
}

impl Migrator {
    fn warn(&mut self, w: String) {
        tracing::warn!("migration: {w}");
        self.warnings.push(w);
    }

    fn key(&mut self, i: u64, b: &Value, base: &Path, icons_dir: &Path) -> Key {
        let ctx = format!("key {i}");
        let mut look = KeyLook::default();
        if let Some(label) = str_of(b, "label") {
            look.label.text = label.to_string();
        }
        if let Some(spec) = b.get("icon_spec").filter(|s| s.is_mapping()) {
            self.icon_spec(&ctx, spec, &mut look);
        } else if let Some(img) = non_empty(str_of(b, "image")) {
            match generated_icon(&img) {
                Some((name, bg, fg)) => {
                    look.source = ImageSource::Icon {
                        set: IconSet::Lucide,
                        name,
                    };
                    look.background = bg;
                    look.foreground = fg;
                }
                None => look.source = self.copy_image(&ctx, &img, base, icons_dir),
            }
        }
        Key {
            look,
            tap: self.action_of(b, &ctx),
            hold: None,
        }
    }

    fn color(&mut self, ctx: &str, v: Option<&str>, fallback: Color) -> Color {
        match v.map(str::parse::<Color>) {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                self.warn(format!("{ctx}: {e}"));
                fallback
            }
            None => fallback,
        }
    }

    fn icon_spec(&mut self, ctx: &str, spec: &Value, look: &mut KeyLook) {
        match str_of(spec, "type").unwrap_or("solid") {
            "icon" => {
                look.source = ImageSource::Icon {
                    set: IconSet::Lucide,
                    name: str_of(spec, "name").unwrap_or_default().to_string(),
                };
                look.background = self.color(ctx, str_of(spec, "bg_color"), Color::BLACK);
                look.foreground = self.color(ctx, str_of(spec, "fg_color"), Color::WHITE);
                look.icon_scale = 0.70;
            }
            "text" => {
                look.source = ImageSource::Text {
                    content: str_of(spec, "text").unwrap_or_default().into(),
                };
                look.background = self.color(ctx, str_of(spec, "color"), Color::BLACK);
                look.foreground = self.color(ctx, str_of(spec, "text_color"), Color::WHITE);
            }
            "solid" => look.background = self.color(ctx, str_of(spec, "color"), Color::BLACK),
            other => {
                look.background = self.color(ctx, str_of(spec, "color"), Color::BLACK);
                self.warn(format!("{ctx}: icon type \"{other}\" became a solid background"));
            }
        }
    }

    fn copy_image(&mut self, ctx: &str, img: &str, base: &Path, icons_dir: &Path) -> ImageSource {
        let src = if Path::new(img).is_absolute() {
            PathBuf::from(img)
        } else {
            base.join(img)
        };
        let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            self.warn(format!("{ctx}: image path {img} has no file name"));
            return ImageSource::None;
        };
        if !src.is_file() {
            self.warn(format!(
                "{ctx}: image {} is missing; the key keeps its action but has no picture",
                src.display()
            ));
            return ImageSource::None;
        }
        let copy =
            std::fs::create_dir_all(icons_dir).and_then(|_| std::fs::copy(&src, icons_dir.join(&name)));
        match copy {
            Ok(_) => ImageSource::Image {
                path: format!("icons/{name}"),
            },
            Err(e) => {
                self.warn(format!("{ctx}: could not copy {}: {e}", src.display()));
                ImageSource::None
            }
        }
    }

    /// Map `{action, params}` to an [`Action`]. Empty params mean "no action", as in the old daemon.
    fn action_of(&mut self, v: &Value, ctx: &str) -> Option<Action> {
        let kind = str_of(v, "action")?;
        let empty = Mapping::new();
        let p = v.get("params").and_then(Value::as_mapping).unwrap_or(&empty);
        let get = |k: &str| p.get(k).and_then(Value::as_str);
        let action = match kind {
            "command" => {
                let cmd = non_empty(get("cmd"))?;
                match sound_path(&cmd) {
                    Some(path) => Action::PlaySound { path, device: None },
                    None => Action::Command { cmd, terminal: false },
                }
            }
            "app" => Action::LaunchApp {
                desktop_id: None,
                exec: Some(non_empty(get("name"))?),
            },
            "key" => Action::Shortcut {
                keys: non_empty(get("keys"))?,
            },
            "media" => {
                let control = match get("control")? {
                    "play_pause" => MediaControl::PlayPause,
                    "next" => MediaControl::Next,
                    "previous" => MediaControl::Previous,
                    "stop" => MediaControl::Stop,
                    other => {
                        self.warn(format!("{ctx}: unknown media control \"{other}\""));
                        return None;
                    }
                };
                Action::Media {
                    control,
                    player: non_empty(get("player")),
                }
            }
            "volume" => {
                let name = non_empty(get("name"));
                let target = match (get("target").unwrap_or("default"), name) {
                    ("app", Some(name)) => AudioTarget::App { name },
                    ("device", Some(name)) => AudioTarget::Device { name },
                    ("default", _) => AudioTarget::Default,
                    (t, None) => {
                        self.warn(format!(
                            "{ctx}: volume target \"{t}\" without a name; using the default output"
                        ));
                        AudioTarget::Default
                    }
                    (other, Some(_)) => {
                        self.warn(format!("{ctx}: unknown volume target \"{other}\""));
                        return None;
                    }
                };
                let op = match get("operation")? {
                    "up" => VolumeOp::Up,
                    "down" => VolumeOp::Down,
                    "mute" => VolumeOp::Mute,
                    other => {
                        self.warn(format!("{ctx}: unknown volume operation \"{other}\""));
                        return None;
                    }
                };
                let step = p.get("step").and_then(Value::as_u64).unwrap_or(5).min(100) as u8;
                let limit_100 = p.get("limit_100").and_then(Value::as_bool).unwrap_or(true);
                Action::Volume {
                    target,
                    op,
                    step,
                    limit_100,
                }
            }
            "obs" => {
                let s = |k: &str| get(k).unwrap_or_default().to_string();
                let obs = match get("action")? {
                    "toggle_scene" => ObsAction::ToggleScenes {
                        a: s("scene1"),
                        b: s("scene2"),
                    },
                    "set_scene" => ObsAction::SetScene { scene: s("scene") },
                    "toggle_source" => ObsAction::ToggleSource {
                        scene: s("scene"),
                        source: s("source"),
                    },
                    "toggle_recording" => ObsAction::ToggleRecord,
                    "toggle_streaming" => ObsAction::ToggleStream,
                    other => {
                        self.warn(format!("{ctx}: unknown OBS action \"{other}\""));
                        return None;
                    }
                };
                Action::Obs(obs)
            }
            other => {
                self.warn(format!("{ctx}: unknown action type \"{other}\""));
                return None;
            }
        };
        Some(action)
    }

    fn settings(&mut self, root: &Value, s: &mut Settings) {
        let num = |k: &str| root.get(k).and_then(Value::as_u64);
        if let Some(b) = num("brightness") {
            s.brightness = b.min(100) as u8;
        }
        if let Some(t) = num("sleep_timeout") {
            s.sleep_after_min = t as u32;
        }
        if let Some(b) = num("sleep_brightness") {
            s.sleep_brightness = b.min(100) as u8;
        }
        if let Some(h) = root.get("hide_labels").and_then(Value::as_bool) {
            s.hide_labels = h;
        }
        if let Some(ls) = root.get("label_style").and_then(Value::as_mapping) {
            let st = &mut s.label_style;
            for (k, v) in ls {
                match (k.as_str().unwrap_or_default(), v) {
                    ("Align", v) => st.align = v.as_str().unwrap_or(&st.align).to_string(),
                    ("Color", v) => st.color = v.as_u64().map(|c| c as u32).unwrap_or(st.color),
                    ("FontName", v) => st.font_name = v.as_str().unwrap_or(&st.font_name).to_string(),
                    ("Size", v) => st.size = v.as_u64().map(|c| c as u32).unwrap_or(st.size),
                    ("Weight", v) => st.weight = v.as_u64().map(|c| c as u32).unwrap_or(st.weight),
                    ("ShowTitle", v) => {
                        if v.as_bool() == Some(false) {
                            s.hide_labels = true;
                        }
                    }
                    (other, _) => self.warn(format!("label_style.{other} is not supported and was skipped")),
                }
            }
        }
        if let Some(obs) = root.get("obs") {
            if let Some(h) = obs.get("host").and_then(Value::as_str) {
                s.obs.host = h.to_string();
            }
            if let Some(p) = obs.get("port").and_then(Value::as_u64) {
                s.obs.port = p.min(u16::MAX as u64) as u16;
                if p == 4444 {
                    self.warn(
                        "OBS port 4444 kept from the old config; OBS 28 and newer listen on 4455 by default"
                            .into(),
                    );
                }
            }
            s.obs.password = obs.get("password").and_then(Value::as_str).map(str::to_string);
        }
    }
}
