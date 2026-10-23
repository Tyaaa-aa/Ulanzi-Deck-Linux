use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use ulanzi_core::Color;
use ulanzi_core::model::{Action, ImageSource, KeyLook, MediaControl, ObsAction, Profile};
use ulanzi_core::render::{RenderCtx, look_hash, render_key};
use ulanzi_proto::{ButtonEntry, ButtonImage};

use crate::vars::resolve_vars;

/// Index of the wide screen in the manifest.
pub const SCREEN_INDEX: u8 = 13;

/// Live values that change what the page shows, refreshed every 2 s by the sampler.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DynamicState {
    /// Label variables, e.g. `cpu` → `"12"`.
    pub vars: BTreeMap<String, String>,
    /// Whether the active media player is playing (`None` = no player).
    pub playing: Option<bool>,
    /// System stats with history, for the System widget.
    pub stats: Option<ulanzi_core::render::screen::StatsData>,
    /// The playing track, for the Now Playing widget.
    pub media: Option<ulanzi_core::render::screen::MediaData>,
    /// OBS Studio's state, once the OBS connection has reported.
    pub obs: Option<ulanzi_core::obs::ObsLive>,
    /// The wide-screen picture (widget mode, or the volume overlay). Set by the deck, not the sampler.
    pub screen_png: Option<Vec<u8>>,
    /// Values plugins set, by `<plugin>.<key>` (label variables like `{hello.count}`). Set by the
    /// deck, not the sampler.
    pub plugin_vars: BTreeMap<String, String>,
}

/// Rendered key images by look hash, plus which warnings were already logged. Holds only what
/// the last built page uses: a label drawn into the picture with a changing value would otherwise
/// keep one picture per value forever.
pub struct RenderCache {
    images: HashMap<u64, Option<Vec<u8>>>,
    warned: HashSet<u64>,
    /// Part of every image name. Seeded randomly per process and bumped on every (re)connect: the
    /// deck caches images by file name, so a new connection or daemon must never reuse names from an
    /// earlier one. Within a connection names are stable, so unchanged keys are not re-sent.
    pub generation: u32,
}

impl RenderCache {
    pub fn new() -> RenderCache {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
            .unwrap_or(0);
        let seed = nanos ^ std::process::id().rotate_left(16);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        RenderCache {
            images: HashMap::new(),
            warned: HashSet::new(),
            generation: seed.wrapping_add(n.wrapping_mul(0x9e37_79b9)),
        }
    }
}

impl Default for RenderCache {
    fn default() -> Self {
        RenderCache::new()
    }
}

/// Variables the deck itself fills (the sampler doesn't know them).
pub const DECK_VARS: [&str; 5] = [
    "obs_scene",
    "obs_rec_time",
    "obs_stream_time",
    "timer",
    "timer_running",
];

impl DynamicState {
    /// Take over what the deck owns from `old`: the screen picture, OBS state, plugin values and the
    /// deck's variables.
    pub fn keep_deck_fields(&mut self, old: &DynamicState) {
        self.screen_png = old.screen_png.clone();
        self.obs = old.obs.clone();
        self.plugin_vars = old.plugin_vars.clone();
        for k in DECK_VARS {
            if let Some(v) = old.vars.get(k) {
                self.vars.insert(k.to_string(), v.clone());
            }
        }
    }

    /// A value a plugin set (`{<plugin>.<key>}` in labels). True if it changed.
    pub fn set_plugin_var(&mut self, plugin: &str, key: &str, value: String) -> bool {
        let name = format!("{plugin}.{key}");
        if self.plugin_vars.get(&name) == Some(&value) {
            return false;
        }
        self.plugin_vars.insert(name, value);
        true
    }

    /// Forget every value `plugin` set. True if there were any.
    pub fn clear_plugin(&mut self, plugin: &str) -> bool {
        let prefix = format!("{plugin}.");
        let before = self.plugin_vars.len();
        self.plugin_vars.retain(|k, _| !k.starts_with(&prefix));
        self.plugin_vars.len() != before
    }

    /// Every label variable: the sampler's and the deck's, then the plugins'.
    fn label_vars(&self) -> std::borrow::Cow<'_, BTreeMap<String, String>> {
        if self.plugin_vars.is_empty() {
            return std::borrow::Cow::Borrowed(&self.vars);
        }
        let mut all = self.plugin_vars.clone();
        // Plugin names always contain a dot, so they never hide a built-in variable.
        all.extend(self.vars.iter().map(|(k, v)| (k.clone(), v.clone())));
        std::borrow::Cow::Owned(all)
    }

    /// Variables for OBS (`{obs_scene}` and the timers).
    pub fn set_obs(&mut self, live: ulanzi_core::obs::ObsLive) {
        use ulanzi_core::obs::clock;
        let rec = if live.recording {
            clock(live.rec_secs)
        } else {
            String::new()
        };
        let stream = if live.streaming {
            clock(live.stream_secs)
        } else {
            String::new()
        };
        self.vars.insert("obs_scene".into(), live.program.clone());
        self.vars.insert("obs_rec_time".into(), rec);
        self.vars.insert("obs_stream_time".into(), stream);
        self.obs = Some(live);
    }
}

/// A page ready to upload, with a hash for "did anything change".
pub struct BuiltPage {
    pub entries: BTreeMap<u8, ButtonEntry>,
    pub images: Vec<ButtonImage>,
    pub hash: u64,
}

fn is_play_pause(a: &Option<Action>) -> bool {
    matches!(
        a,
        Some(Action::Media {
            control: MediaControl::PlayPause,
            ..
        })
    )
}

const ON_AIR: Color = Color([0x26, 0xa2, 0x69, 0xff]);
const RED: Color = Color([0xc0, 0x1c, 0x28, 0xff]);
const AMBER: Color = Color([0xc6, 0x46, 0x00, 0xff]);
const BLUE: Color = Color([0x1c, 0x71, 0xd8, 0xff]);
const DIMMED: Color = Color([0x5e, 0x5c, 0x64, 0xff]);

fn set_label(look: &mut KeyLook, text: String) {
    if look.label.visible && !look.label.text.is_empty() {
        look.label.text = text;
    }
}

fn swap_icon(look: &mut KeyLook, from: &str, to: &str) {
    if let ImageSource::Icon { name, .. } = &mut look.source
        && name == from
    {
        *name = to.into();
    }
}

/// OBS keys follow OBS: recording red with its time, the scene on air green, muted inputs red,
/// hidden sources dimmed, and every OBS key dimmed while OBS is not connected.
fn obs_look(look: &mut KeyLook, o: &ObsAction, l: &ulanzi_core::obs::ObsLive) {
    use ulanzi_core::obs::clock;
    if !l.connected {
        look.foreground = DIMMED;
        return;
    }
    match o {
        ObsAction::ToggleRecord | ObsAction::StartRecord | ObsAction::StopRecord if l.recording => {
            if l.rec_paused {
                look.background = AMBER;
                set_label(look, "PAUSED".into());
            } else {
                look.background = RED;
                set_label(look, format!("REC {}", clock(l.rec_secs)));
            }
        }
        ObsAction::PauseRecord if l.rec_paused => look.background = AMBER,
        ObsAction::ToggleStream | ObsAction::StartStream | ObsAction::StopStream if l.streaming => {
            look.background = RED;
            set_label(look, format!("LIVE {}", clock(l.stream_secs)));
        }
        ObsAction::ToggleReplay if l.replay => look.background = ON_AIR,
        ObsAction::ToggleVirtualCam if l.vcam => look.background = BLUE,
        ObsAction::SetScene { scene, .. } if *scene == l.program => look.background = ON_AIR,
        ObsAction::SetScene { scene, .. } if l.preview.as_deref() == Some(scene.as_str()) => {
            look.background = AMBER
        }
        ObsAction::ToggleScenes { a, b } if l.program == *a || l.program == *b => {
            set_label(look, l.program.clone())
        }
        ObsAction::ToggleSource { scene, source } if l.hidden.contains(&(scene.clone(), source.clone())) => {
            look.foreground = DIMMED;
            swap_icon(look, "eye", "eye-off");
        }
        ObsAction::ToggleMute { input } if l.muted.contains(input) => {
            look.background = RED;
            swap_icon(look, "mic", "mic-off");
        }
        _ => {}
    }
}

/// The key as it looks now: play/pause swaps, OBS state, the timer's time left.
fn live_look(look: &KeyLook, tap: &Option<Action>, dynamic: &DynamicState) -> KeyLook {
    let mut look = look.clone();
    if !look.dynamic {
        return look;
    }
    if dynamic.playing == Some(true) && is_play_pause(tap) {
        swap_icon(&mut look, "play", "pause");
    }
    match tap {
        Some(Action::Obs(o)) => {
            if let Some(l) = &dynamic.obs {
                obs_look(&mut look, o, l);
            }
        }
        Some(Action::Timer {
            op: ulanzi_core::model::TimerOp::StartPause,
        }) if dynamic.vars.get("timer_running").is_some_and(|v| v == "1") => {
            if let Some(t) = dynamic.vars.get("timer") {
                set_label(&mut look, t.clone());
            }
        }
        _ => {}
    }
    look
}

/// Build all 14 manifest entries (keys 0-12 and the wide screen) for `page`.
pub fn build_page(
    profile: &Profile,
    page: usize,
    ctx: &RenderCtx,
    dynamic: &DynamicState,
    cache: &mut RenderCache,
) -> BuiltPage {
    let mut entries = BTreeMap::new();
    let mut images = Vec::new();
    let mut h = DefaultHasher::new();
    let keys = profile.pages.get(page).map(|p| &p.keys);
    let vars = dynamic.label_vars();
    let mut used = HashSet::new();

    for i in 0..SCREEN_INDEX {
        let mut entry = ButtonEntry::default();
        if let Some(key) = keys.and_then(|k| k.get(&i)) {
            let mut look = live_look(&key.look, &key.tap, dynamic);
            // A key with its own label position gets the label drawn into its picture.
            let own_label = look.label.position.is_some();
            if own_label {
                look.label.text = resolve_vars(&look.label.text, &vars);
            }
            let lh = look_hash(&look, ctx.profile_dir);
            let png = cache.images.entry(lh).or_insert_with(|| {
                render_key(&look, ctx).map(|r| {
                    if !r.warnings.is_empty() && cache.warned.insert(lh) {
                        for w in &r.warnings {
                            tracing::warn!("key {i}: {w}");
                        }
                    }
                    r.png
                })
            });
            if let Some(png) = png {
                let name = format!("k{i}_{lh:016x}_{:08x}.png", cache.generation);
                entry.icon = Some(format!("icons/{name}"));
                images.push(ButtonImage {
                    name,
                    png: png.clone(),
                });
            }
            if !own_label && look.label.visible && !look.label.text.is_empty() {
                entry.text = Some(resolve_vars(&look.label.text, &vars));
            }
            lh.hash(&mut h);
            used.insert(lh);
        }
        (i, &entry.text, &entry.icon).hash(&mut h);
        entries.insert(i, entry);
    }

    let mut screen = ButtonEntry::default();
    if let Some(png) = &dynamic.screen_png {
        let mut sh = DefaultHasher::new();
        png.hash(&mut sh);
        let name = format!("screen_{:016x}_{:08x}.png", sh.finish(), cache.generation);
        screen.icon = Some(format!("icons/{name}"));
        name.hash(&mut h);
        images.push(ButtonImage {
            name,
            png: png.clone(),
        });
    }
    entries.insert(SCREEN_INDEX, screen);
    cache.images.retain(|lh, _| used.contains(lh));
    cache.warned.retain(|lh| used.contains(lh));
    BuiltPage {
        entries,
        images,
        hash: h.finish(),
    }
}

/// The keys of a partial update with their images under new names (suffix `_u{seq}`), and only
/// those images. The deck removes a key's previous image after unpacking a partial update, so
/// re-sending an image under its old name (when only the label changed) left the key black.
pub fn with_fresh_names(
    changed: &BTreeMap<u8, ButtonEntry>,
    images: &[ButtonImage],
    seq: u32,
) -> (BTreeMap<u8, ButtonEntry>, Vec<ButtonImage>) {
    let mut out_images = Vec::new();
    let entries = changed
        .iter()
        .map(|(i, e)| {
            let mut e = e.clone();
            if let Some(icon) = e.icon.take() {
                let old = icon.strip_prefix("icons/").unwrap_or(&icon).to_string();
                let new = match old.strip_suffix(".png") {
                    Some(stem) => format!("{stem}_u{seq}.png"),
                    None => format!("{old}_u{seq}"),
                };
                if let Some(img) = images.iter().find(|img| img.name == old) {
                    out_images.push(ButtonImage {
                        name: new.clone(),
                        png: img.png.clone(),
                    });
                }
                e.icon = Some(format!("icons/{new}"));
            }
            (*i, e)
        })
        .collect();
    (entries, out_images)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ulanzi_core::Color;
    use ulanzi_core::model::{IconSet, Key, Model};
    use ulanzi_core::render::IconLibrary;

    use super::*;

    fn ctx() -> RenderCtx<'static> {
        RenderCtx {
            profile_dir: std::path::Path::new("/nonexistent"),
            icons: IconLibrary::bundled(),
        }
    }

    fn profile() -> Profile {
        let mut p = Profile::new_default("P", Model::D200X);
        let mut look = KeyLook {
            source: ImageSource::Icon {
                set: IconSet::Lucide,
                name: "play".into(),
            },
            background: Color::from_str("#000000").expect("c"),
            ..KeyLook::default()
        };
        look.label.text = "CPU {cpu}%".into();
        p.pages[0].keys.insert(
            6,
            Key {
                look,
                tap: Some(Action::Media {
                    control: MediaControl::PlayPause,
                    player: None,
                }),
                hold: None,
            },
        );
        p
    }

    #[test]
    fn always_fourteen_entries() {
        let b = build_page(
            &profile(),
            0,
            &ctx(),
            &DynamicState::default(),
            &mut RenderCache::default(),
        );
        assert_eq!(b.entries.len(), 14);
        assert_eq!(b.images.len(), 1);
        assert!(
            b.entries[&6]
                .icon
                .as_deref()
                .is_some_and(|i| i.starts_with("icons/k6_"))
        );
        assert_eq!(b.entries[&0], ButtonEntry::default());
    }

    #[test]
    fn labels_resolve_variables() {
        let mut d = DynamicState::default();
        d.vars.insert("cpu".into(), "42".into());
        let b = build_page(&profile(), 0, &ctx(), &d, &mut RenderCache::default());
        assert_eq!(b.entries[&6].text.as_deref(), Some("CPU 42%"));
    }

    #[test]
    fn plugin_vars_survive_refresh() {
        let mut p = profile();
        p.pages[0].keys.get_mut(&6).expect("key").look.label.text = "Count {hello.count}".into();
        let mut deck = DynamicState::default();
        deck.set_plugin_var("hello", "count", "3".into());
        // What the deck does with each sampler refresh.
        let mut fresh = DynamicState::default();
        fresh.vars.insert("cpu".into(), "5".into());
        fresh.keep_deck_fields(&deck);
        let b = build_page(&p, 0, &ctx(), &fresh, &mut RenderCache::default());
        assert_eq!(b.entries[&6].text.as_deref(), Some("Count 3"));
        assert!(fresh.clear_plugin("hello"));
        assert!(!fresh.clear_plugin("hello"));
        let b = build_page(&p, 0, &ctx(), &fresh, &mut RenderCache::default());
        assert_eq!(b.entries[&6].text.as_deref(), Some("Count {hello.count}"));
    }

    #[test]
    fn play_swaps_to_pause_and_changes_hash() {
        let mut cache = RenderCache::default();
        let stopped = build_page(&profile(), 0, &ctx(), &DynamicState::default(), &mut cache);
        let playing = DynamicState {
            playing: Some(true),
            ..DynamicState::default()
        };
        let b = build_page(&profile(), 0, &ctx(), &playing, &mut cache);
        assert_ne!(stopped.hash, b.hash);
        assert_ne!(stopped.entries[&6].icon, b.entries[&6].icon);
    }

    #[test]
    fn screen_image_goes_out_when_drawn() {
        // The deck draws the picture (widgets, or the volume overlay over a firmware mode).
        let p = profile();
        let b = build_page(
            &p,
            0,
            &ctx(),
            &DynamicState::default(),
            &mut RenderCache::default(),
        );
        assert_eq!(b.entries[&SCREEN_INDEX].icon, None);
        let d = DynamicState {
            screen_png: Some(vec![1, 2, 3]),
            ..DynamicState::default()
        };
        let b = build_page(&p, 0, &ctx(), &d, &mut RenderCache::default());
        assert!(b.entries[&SCREEN_INDEX].icon.is_some());
    }

    #[test]
    fn partial_updates_use_fresh_image_names() {
        let mut changed = BTreeMap::new();
        changed.insert(
            6u8,
            ButtonEntry {
                state: 0,
                text: Some("CPU 12%".into()),
                icon: Some("icons/k6_abc_01.png".into()),
            },
        );
        changed.insert(13u8, ButtonEntry::default());
        let images = vec![
            ButtonImage {
                name: "k6_abc_01.png".into(),
                png: vec![1, 2, 3],
            },
            ButtonImage {
                name: "k7_def_01.png".into(),
                png: vec![4],
            },
        ];
        let (entries, imgs) = with_fresh_names(&changed, &images, 7);
        assert_eq!(entries[&6].icon.as_deref(), Some("icons/k6_abc_01_u7.png"));
        assert_eq!(entries[&6].text.as_deref(), Some("CPU 12%"));
        assert_eq!(entries[&13].icon, None);
        // Only the images the changed keys use, renamed to match.
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].name, "k6_abc_01_u7.png");
        assert_eq!(imgs[0].png, vec![1, 2, 3]);
    }
    #[test]
    fn label_with_own_position_goes_into_the_image() {
        let mut p = profile();
        let key = p.pages[0].keys.get_mut(&6).expect("key");
        key.look.label.position = Some(ulanzi_core::model::LabelPosition::Top);
        let mut d = DynamicState::default();
        d.vars.insert("cpu".into(), "12".into());
        let mut cache = RenderCache::default();
        let a = build_page(&p, 0, &ctx(), &d, &mut cache);
        // No firmware text: the label is part of the picture.
        assert_eq!(a.entries[&6].text, None);
        d.vars.insert("cpu".into(), "13".into());
        let b = build_page(&p, 0, &ctx(), &d, &mut cache);
        // A new value means a new picture.
        assert_ne!(a.entries[&6].icon, b.entries[&6].icon);
    }

    #[test]
    fn cache_keeps_only_the_current_page() {
        // A label drawn into the picture with a changing value must not keep every old picture.
        let mut p = profile();
        let key = p.pages[0].keys.get_mut(&6).expect("key");
        key.look.label.position = Some(ulanzi_core::model::LabelPosition::Top);
        key.look.label.text = "N {hello.n}".into();
        let mut d = DynamicState::default();
        let mut cache = RenderCache::default();
        for n in 0..50 {
            d.set_plugin_var("hello", "n", n.to_string());
            build_page(&p, 0, &ctx(), &d, &mut cache);
        }
        assert_eq!(cache.images.len(), 1);
        assert!(cache.warned.len() <= 1);
    }

    #[test]
    fn obs_and_timer_keys_show_live_state() {
        use ulanzi_core::model::{ObsAction, TimerOp};
        use ulanzi_core::obs::ObsLive;
        let look = |label: &str| {
            let mut l = KeyLook::default();
            l.label.text = label.into();
            l.label.visible = true;
            l
        };
        let rec = Some(Action::Obs(ObsAction::ToggleRecord));
        let mut d = DynamicState::default();
        // Nothing known about OBS: unchanged.
        assert_eq!(live_look(&look("Record"), &rec, &d), look("Record"));
        d.obs = Some(ObsLive {
            connected: true,
            recording: true,
            rec_secs: 83,
            ..ObsLive::default()
        });
        let l = live_look(&look("Record"), &rec, &d);
        assert_eq!(l.label.text, "REC 01:23");
        assert_ne!(l.background, look("Record").background);
        // A key whose look is fixed stays as it is.
        let mut fixed = look("Record");
        fixed.dynamic = false;
        assert_eq!(live_look(&fixed, &rec, &d), fixed);
        // The scene on air is highlighted; offline keys are dimmed.
        let scene = Some(Action::Obs(ObsAction::SetScene {
            scene: "A".into(),
            preview: false,
        }));
        d.obs = Some(ObsLive {
            connected: true,
            program: "A".into(),
            ..ObsLive::default()
        });
        assert_ne!(live_look(&look("A"), &scene, &d).background, look("A").background);
        d.obs = Some(ObsLive::default());
        assert_ne!(live_look(&look("A"), &scene, &d).foreground, look("A").foreground);
        // A muted input.
        let mute = Some(Action::Obs(ObsAction::ToggleMute { input: "Mic".into() }));
        let mut muted = ObsLive {
            connected: true,
            ..ObsLive::default()
        };
        muted.muted.insert("Mic".into());
        d.obs = Some(muted);
        assert_ne!(
            live_look(&look("Mic"), &mute, &d).background,
            look("Mic").background
        );
        // The timer key shows the time left while it runs.
        let t = Some(Action::Timer {
            op: TimerOp::StartPause,
        });
        d.vars.insert("timer".into(), "18:42".into());
        assert_eq!(live_look(&look("Timer"), &t, &d).label.text, "Timer");
        d.vars.insert("timer_running".into(), "1".into());
        assert_eq!(live_look(&look("Timer"), &t, &d).label.text, "18:42");
    }
}
