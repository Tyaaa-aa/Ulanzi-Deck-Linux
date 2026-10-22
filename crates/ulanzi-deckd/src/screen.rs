//! Samples live values (stats, media, wide-screen image) every couple of seconds for the deck.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ulanzi_core::render::{Fonts, NowPlaying, render_now_playing};

use crate::actions::media::{MediaClient, MediaInfo, PlaybackStatus};
use crate::deck::DynamicState;
use crate::sysinfo::SysSampler;

/// Output size of the wide-screen image (confirmed on hardware in M1's hardware session).
pub const SCREEN_OUT: (u32, u32) = (196, 196);
const ART_TIMEOUT_SECS: u64 = 3;
const ART_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Where a remote album-art URL is cached: `<cache>/art/<fnv64>.img`.
pub fn art_cache_path(cache_dir: &Path, url: &str) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    cache_dir.join("art").join(format!("{h:016x}.img"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Album art: local files directly, remote URLs downloaded once in the background.
#[derive(Clone)]
pub struct ArtCache {
    dir: PathBuf,
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl ArtCache {
    pub fn new(cache_dir: PathBuf) -> ArtCache {
        ArtCache {
            dir: cache_dir,
            in_flight: Arc::default(),
        }
    }

    /// A local path for `url` if available now; starts a download otherwise.
    pub fn get(&self, url: &str) -> Option<PathBuf> {
        if let Some(p) = url.strip_prefix("file://") {
            let p = PathBuf::from(percent_decode(p));
            return p.is_file().then_some(p);
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            let p = PathBuf::from(url);
            return p.is_file().then_some(p);
        }
        let path = art_cache_path(&self.dir, url);
        if path.is_file() {
            return Some(path);
        }
        let fresh = self
            .in_flight
            .lock()
            .map(|mut s| s.insert(url.to_string()))
            .unwrap_or(false);
        if fresh {
            let (url, in_flight) = (url.to_string(), self.in_flight.clone());
            tokio::task::spawn_blocking(move || {
                if let Err(e) = download(&url, &path) {
                    tracing::debug!("album art {url}: {e}");
                }
                if let Ok(mut s) = in_flight.lock() {
                    s.remove(&url);
                }
            });
        }
        None
    }
}

fn download(url: &str, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(ART_TIMEOUT_SECS)))
        .build()
        .into();
    let bytes = agent
        .get(url)
        .call()?
        .body_mut()
        .with_config()
        .limit(ART_MAX_BYTES)
        .read_to_vec()?;
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    ulanzi_core::store::write_atomic(to, &bytes)?;
    Ok(())
}

/// Produces a [`DynamicState`] on demand.
pub struct Sampler {
    sys: SysSampler,
    media: Option<MediaClient>,
    art: ArtCache,
    fonts: Option<Fonts>,
    last_screen: Option<(NowPlaying, Option<PathBuf>, Vec<u8>)>,
}

impl Sampler {
    pub fn new(media: Option<MediaClient>, cache_dir: PathBuf) -> Sampler {
        Sampler {
            sys: SysSampler::new("/"),
            media,
            art: ArtCache::new(cache_dir),
            fonts: None,
            last_screen: None,
        }
    }

    pub async fn sample(&mut self) -> DynamicState {
        let stats = self.sys.sample();
        let now = chrono::Local::now();
        let mut vars = BTreeMap::new();
        vars.insert("cpu".into(), stats.cpu.to_string());
        vars.insert("ram".into(), stats.ram.to_string());
        vars.insert("cpu_temp".into(), stats.cpu_temp.to_string());
        vars.insert("time".into(), now.format("%H:%M").to_string());
        vars.insert("date".into(), now.format("%Y-%m-%d").to_string());

        let media: Option<MediaInfo> = match &self.media {
            Some(m) => m.now_playing().await,
            None => None,
        };
        let status = match media.as_ref().map(|m| m.status) {
            Some(PlaybackStatus::Playing) => "Playing",
            Some(PlaybackStatus::Paused) => "Paused",
            _ => "Stopped",
        };
        vars.insert(
            "media_title".into(),
            media
                .as_ref()
                .map(|m| m.title.clone())
                .unwrap_or_else(|| "No Media".into()),
        );
        vars.insert(
            "media_artist".into(),
            media.as_ref().map(|m| m.artist.clone()).unwrap_or_default(),
        );
        vars.insert("media_status".into(), status.into());

        let np = NowPlaying {
            title: media.as_ref().map(|m| m.title.clone()).unwrap_or_default(),
            artist: media.as_ref().map(|m| m.artist.clone()).unwrap_or_default(),
        };
        let art = media
            .as_ref()
            .and_then(|m| m.art_url.as_deref())
            .and_then(|u| self.art.get(u));
        let screen_png = match &self.last_screen {
            Some((n, a, png)) if *n == np && *a == art => png.clone(),
            _ => {
                let fonts = self.fonts.get_or_insert_with(Fonts::system);
                let png = tokio::task::block_in_place(|| {
                    render_now_playing(&np, art.as_deref(), fonts, SCREEN_OUT)
                });
                self.last_screen = Some((np, art, png.clone()));
                png
            }
        };

        DynamicState {
            vars,
            playing: media.as_ref().map(|m| m.status == PlaybackStatus::Playing),
            screen_png: Some(screen_png),
        }
    }
}
