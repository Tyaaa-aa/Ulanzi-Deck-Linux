//! Samples live values (stats, media, wide-screen image) every couple of seconds for the deck.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ulanzi_core::render::screen::{MediaData, StatsData};

use crate::actions::media::{MediaClient, MediaInfo, PlaybackStatus};
use crate::deck::DynamicState;
use crate::sysinfo::SysSampler;

/// Output size of the wide-screen image: 458×196 was the only test pattern the deck showed undistorted.
pub const SCREEN_OUT: (u32, u32) = (458, 196);
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

/// How many samples the System widget's graphs keep (one a second).
pub const HISTORY: usize = 60;

/// Appends `v`, keeping the newest [`HISTORY`] values.
pub fn push_history(h: &mut Vec<u8>, v: u8) {
    h.push(v);
    if h.len() > HISTORY {
        h.drain(..h.len() - HISTORY);
    }
}

/// Bytes per second between two counter readings `secs` apart (0 after a counter reset).
pub fn rate(prev: u64, now: u64, secs: f64) -> u64 {
    if secs <= 0.0 {
        return 0;
    }
    (now.saturating_sub(prev) as f64 / secs) as u64
}

/// Produces a [`DynamicState`] on demand: values only; the deck draws the wide screen.
pub struct Sampler {
    sys: SysSampler,
    media: Option<MediaClient>,
    art: ArtCache,
    stats: StatsData,
    prev_net: Option<(u64, u64, std::time::Instant)>,
}

impl Sampler {
    pub fn new(media: Option<MediaClient>, cache_dir: PathBuf) -> Sampler {
        Sampler {
            sys: SysSampler::new("/"),
            media,
            art: ArtCache::new(cache_dir),
            stats: StatsData::default(),
            prev_net: None,
        }
    }

    pub async fn sample(&mut self) -> DynamicState {
        let sys = self.sys.sample();
        let now = chrono::Local::now();
        let mut vars = BTreeMap::new();
        vars.insert("cpu".into(), sys.cpu.to_string());
        vars.insert("ram".into(), sys.ram.to_string());
        vars.insert("gpu".into(), sys.gpu.to_string());
        vars.insert("cpu_temp".into(), sys.cpu_temp.to_string());
        vars.insert("time".into(), now.format("%H:%M").to_string());
        vars.insert("date".into(), now.format("%Y-%m-%d").to_string());

        push_history(&mut self.stats.cpu, sys.cpu);
        push_history(&mut self.stats.ram, sys.ram);
        push_history(&mut self.stats.gpu, sys.gpu);
        self.stats.temp = (sys.cpu_temp > 0).then_some(sys.cpu_temp as i32);
        let t = std::time::Instant::now();
        if let Some((rx, tx, at)) = self.prev_net {
            let secs = t.duration_since(at).as_secs_f64();
            self.stats.net_down = rate(rx, sys.net_rx, secs);
            self.stats.net_up = rate(tx, sys.net_tx, secs);
        }
        self.prev_net = Some((sys.net_rx, sys.net_tx, t));

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

        let media_data = media.as_ref().map(|m| MediaData {
            title: m.title.clone(),
            artist: m.artist.clone(),
            art: m.art_url.as_deref().and_then(|u| self.art.get(u)),
            position_s: m.position_us.map(|u| (u / 1_000_000) as u32),
            length_s: m.length_us.map(|u| (u / 1_000_000) as u32),
            playing: m.status == PlaybackStatus::Playing,
        });

        DynamicState {
            vars,
            playing: media.as_ref().map(|m| m.status == PlaybackStatus::Playing),
            stats: Some(self.stats.clone()),
            media: media_data,
            obs: None,
            screen_png: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_newest_and_rates_survive_resets() {
        let mut h = Vec::new();
        for v in 0..70u8 {
            push_history(&mut h, v);
        }
        assert_eq!(h.len(), HISTORY);
        assert_eq!(h.first(), Some(&10));
        assert_eq!(h.last(), Some(&69));
        assert_eq!(rate(1000, 3000, 2.0), 1000);
        assert_eq!(rate(5000, 10, 1.0), 0);
        assert_eq!(rate(0, 10, 0.0), 0);
    }
}
