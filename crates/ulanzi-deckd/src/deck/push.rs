use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use ulanzi_core::model::{Action, ImageSource, KeyLook, MediaControl, Profile};
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
    /// The wide-screen picture (widget mode, or the volume overlay). Set by the deck, not the sampler.
    pub screen_png: Option<Vec<u8>>,
    /// Values plugins set, by `<plugin>.<key>` (label variables like `{hello.count}`). Set by the
    /// deck, not the sampler.
    pub plugin_vars: BTreeMap<String, String>,
    /// The step each cycling key runs next, by [`cycle_key`]. Set by the deck.
    pub cycles: BTreeMap<String, usize>,
}

/// Where a cycling control's next step is kept: its page and the control (`key:3`, `dial:0`, with
/// `:hold` for its hold action).
pub fn cycle_key(page: uuid::Uuid, control: &str) -> String {
    format!("{page}/{control}")
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
pub const DECK_VARS: [&str; 2] = ["timer", "timer_running"];

impl DynamicState {
    /// Take over what the deck owns from `old`: the screen picture, OBS state, plugin values and the
    /// deck's variables.
    pub fn keep_deck_fields(&mut self, old: &DynamicState) {
        self.screen_png = old.screen_png.clone();
        self.plugin_vars = old.plugin_vars.clone();
        self.cycles = old.cycles.clone();
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

/// The parameters `$<param>` in labels and states refer to: a plugin action's.
fn params_of(tap: &Option<Action>) -> Option<&toml::Table> {
    match tap {
        Some(Action::Plugin { params, .. }) => Some(params),
        _ => None,
    }
}

/// The key as it looks now: a cycling key's step, play/pause swaps, OBS state, the timer's
/// time left, then the first of its own live states that holds.
fn live_look(
    look: &KeyLook,
    tap: &Option<Action>,
    dynamic: &DynamicState,
    vars: &BTreeMap<String, String>,
    cycle: Option<&str>,
) -> KeyLook {
    let mut look = look.clone();
    if let (Some(Action::Cycle { steps, show }), Some(at)) = (tap, cycle)
        && !steps.is_empty()
    {
        let next = dynamic.cycles.get(at).copied().unwrap_or(0);
        steps[show.step(next, steps.len())].look.apply(&mut look);
    }
    if !look.dynamic {
        return look;
    }
    // The key's own looks, else those its plugin action declares.
    let declared;
    let states = match tap {
        Some(Action::Plugin { plugin, action, .. }) if look.states.is_empty() => {
            declared = ulanzi_core::catalog::plugin_states(plugin, action);
            &declared
        }
        _ => &look.states,
    };
    if !states.is_empty() {
        let empty = toml::Table::new();
        look = look.with_states(states, params_of(tap).unwrap_or(&empty), |t| {
            // A value nobody published yet counts as empty.
            let v = resolve_vars(t, vars);
            if v.contains('{') { String::new() } else { v }
        });
    }
    if dynamic.playing == Some(true) && is_play_pause(tap) {
        swap_icon(&mut look, "play", "pause");
    }
    match tap {
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
    let page_id = profile.pages.get(page).map(|p| p.id);
    let vars = dynamic.label_vars();
    let mut used = HashSet::new();

    for i in 0..SCREEN_INDEX {
        let mut entry = ButtonEntry::default();
        if let Some(key) = keys.and_then(|k| k.get(&i)) {
            let at = page_id.map(|id| cycle_key(id, &format!("key:{i}")));
            let mut look = live_look(&key.look, &key.tap, dynamic, &vars, at.as_deref());
            if let Some(params) = params_of(&key.tap) {
                look.label.text = ulanzi_core::model::fill_params(&look.label.text, params);
            }
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
    fn plugin_keys_follow_their_live_state_and_cycles_their_step() {
        use ulanzi_core::model::{CycleStep, LookOverride, LookState, When};
        let blue = Color([0x4f, 0xc3, 0xf7, 0xff]);
        let mut params = toml::Table::new();
        params.insert("entity_id".into(), "climate.ac".into());
        let tap = Some(Action::Plugin {
            plugin: "ha".into(),
            action: "climate".into(),
            params,
        });
        let look = KeyLook {
            states: vec![LookState {
                when: When {
                    value: "{ha.$entity_id}".into(),
                    is: vec!["cool".into()],
                    not: vec![],
                },
                look: LookOverride {
                    foreground: Some(blue),
                    ..LookOverride::default()
                },
            }],
            ..KeyLook::default()
        };
        let mut d = DynamicState::default();
        assert_eq!(
            live_look(&look, &tap, &d, &d.vars, None),
            look,
            "nothing published yet"
        );
        d.plugin_vars.insert("ha.climate.ac".into(), "cool".into());
        let vars = d.label_vars().into_owned();
        assert_eq!(live_look(&look, &tap, &d, &vars, None).foreground, blue);
        d.plugin_vars.insert("ha.climate.ac".into(), "off".into());
        let vars = d.label_vars().into_owned();
        assert_eq!(
            live_look(&look, &tap, &d, &vars, None).foreground,
            look.foreground
        );

        // A cycle shows the step its next press runs.
        let step = |label: &str| CycleStep {
            action: Action::NextPage,
            look: LookOverride {
                label: Some(label.into()),
                ..LookOverride::default()
            },
        };
        let cycle = Some(Action::Cycle {
            steps: vec![step("One"), step("Two")],
            show: ulanzi_core::model::CycleShows::Next,
        });
        let mut d = DynamicState::default();
        let at = cycle_key(uuid::Uuid::nil(), "key:0");
        let shown = |d: &DynamicState| {
            live_look(&KeyLook::default(), &cycle, d, &d.vars, Some(&at))
                .label
                .text
        };
        assert_eq!(shown(&d), "One");
        d.cycles.insert(at.clone(), 1);
        assert_eq!(shown(&d), "Two");
        d.cycles.insert(at.clone(), 5);
        assert_eq!(shown(&d), "Two", "a step count that shrank wraps around");
    }
}
