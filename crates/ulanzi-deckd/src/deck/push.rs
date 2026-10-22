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
#[derive(Default)]
pub struct RenderCache {
    images: HashMap<u64, Option<Vec<u8>>>,
    warned: HashSet<u64>,
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
                let name = format!("k{i}_{lh:016x}.png");
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
        let name = format!("screen_{:016x}.png", sh.finish());
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
}
