use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use ulanzi_core::model::{Action, ImageSource, KeyLook, MediaControl, Profile, ScreenConfig};
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
    /// Pre-rendered wide-screen image when the screen is `Custom`.
    pub screen_png: Option<Vec<u8>>,
}

/// Rendered key images by look hash, plus which warnings were already logged.
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

/// Swap a `play` icon for `pause` on play/pause keys while media is playing.
fn live_look(look: &KeyLook, tap: &Option<Action>, dynamic: &DynamicState) -> KeyLook {
    let mut look = look.clone();
    if look.dynamic
        && dynamic.playing == Some(true)
        && is_play_pause(tap)
        && let ImageSource::Icon { name, .. } = &mut look.source
        && name == "play"
    {
        *name = "pause".into();
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

    for i in 0..SCREEN_INDEX {
        let mut entry = ButtonEntry::default();
        if let Some(key) = keys.and_then(|k| k.get(&i)) {
            let look = live_look(&key.look, &key.tap, dynamic);
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
            if look.label.visible && !look.label.text.is_empty() {
                entry.text = Some(resolve_vars(&look.label.text, &dynamic.vars));
            }
            lh.hash(&mut h);
        }
        (i, &entry.text, &entry.icon).hash(&mut h);
        entries.insert(i, entry);
    }

    let mut screen = ButtonEntry::default();
    if matches!(profile.screen, ScreenConfig::Custom { .. })
        && let Some(png) = &dynamic.screen_png
    {
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
    use ulanzi_core::model::{IconSet, Key, Model, Widget};
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
    fn screen_image_only_in_custom_mode() {
        let d = DynamicState {
            screen_png: Some(vec![1, 2, 3]),
            ..DynamicState::default()
        };
        let mut p = profile();
        let b = build_page(&p, 0, &ctx(), &d, &mut RenderCache::default());
        assert_eq!(b.entries[&SCREEN_INDEX].icon, None);
        p.screen = ScreenConfig::Custom {
            widgets: vec![Widget::NowPlaying],
            tap: None,
            hold: None,
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
}
