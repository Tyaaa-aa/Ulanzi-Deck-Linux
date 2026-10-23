//! Media players over MPRIS (D-Bus).

use std::collections::HashMap;

use ulanzi_core::model::MediaControl;
use zbus::Connection;
use zbus::zvariant::OwnedValue;

use super::ActionError;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

impl PlaybackStatus {
    fn parse(s: &str) -> PlaybackStatus {
        match s {
            "Playing" => PlaybackStatus::Playing,
            "Paused" => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        }
    }
}

/// What is playing, for labels and the Now Playing screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInfo {
    pub player: String,
    pub title: String,
    pub artist: String,
    pub status: PlaybackStatus,
    pub art_url: Option<String>,
    pub length_us: Option<u64>,
    pub position_us: Option<u64>,
}

/// Choose a player: a `wanted` substring match first, else the playing one, else a paused one, else any.
pub fn pick_player(players: &[(String, PlaybackStatus)], wanted: Option<&str>) -> Option<String> {
    if let Some(w) = wanted.filter(|w| !w.is_empty()) {
        let w = w.to_lowercase();
        return players
            .iter()
            .find(|(n, _)| n.to_lowercase().contains(&w))
            .map(|(n, _)| n.clone());
    }
    let with = |st| players.iter().find(|(_, s)| *s == st);
    with(PlaybackStatus::Playing)
        .or_else(|| with(PlaybackStatus::Paused))
        .or_else(|| players.first())
        .map(|(n, _)| n.clone())
}

/// Players without the browser's own entry when Plasma's browser integration already shows the
/// same browser (its `kde:pid` matches the entry's `instance<pid>`): one video, one player.
/// Each player comes with the `kde:pid` from its metadata, when it has one.
pub fn drop_duplicates(players: &[(String, Option<i32>)]) -> Vec<String> {
    let pids: Vec<i32> = players.iter().filter_map(|(_, pid)| *pid).collect();
    players
        .iter()
        .filter(|(name, _)| {
            let instance = name
                .rsplit_once(".instance")
                .and_then(|(_, n)| n.parse::<i32>().ok());
            !instance.is_some_and(|n| pids.contains(&n))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// Like [`pick_player`] without a wanted name, but stays on `last` while it plays (players change
/// state one after another at a track change) or while nothing else plays.
pub fn pick_sticky(players: &[(String, PlaybackStatus)], last: Option<&str>) -> Option<String> {
    let status = |n: &str| players.iter().find(|(p, _)| p == n).map(|(_, s)| *s);
    if let Some(l) = last {
        match status(l) {
            Some(PlaybackStatus::Playing) => return Some(l.to_string()),
            Some(_) if !players.iter().any(|(_, s)| *s == PlaybackStatus::Playing) => {
                return Some(l.to_string());
            }
            _ => {}
        }
    }
    pick_player(players, None)
}

/// Positions per track: some players keep reporting the old track's time after a change until
/// they are paused and played; the time then counts from the change instead.
#[derive(Debug, Clone, Default)]
pub struct TrackClock {
    track: Option<String>,
    /// The position the player reported when the track changed, if it hadn't reset.
    base: Option<u64>,
}

impl TrackClock {
    pub fn position(&mut self, track: &str, reported: u64) -> u64 {
        const SETTLED: u64 = 2_000_000;
        if self.track.as_deref() != Some(track) {
            let first = self.track.is_none();
            self.track = Some(track.to_string());
            self.base = (!first && reported > SETTLED).then_some(reported);
        }
        match self.base {
            Some(b) if reported >= b => reported - b,
            Some(_) => {
                self.base = None;
                reported
            }
            None => reported,
        }
    }
}

#[derive(Default)]
struct Shown {
    player: Option<String>,
    clock: TrackClock,
}

/// MPRIS client on the session bus.
#[derive(Clone)]
pub struct MediaClient {
    conn: Connection,
    shown: std::sync::Arc<std::sync::Mutex<Shown>>,
}

impl MediaClient {
    pub fn new(conn: Connection) -> MediaClient {
        MediaClient {
            conn,
            shown: Default::default(),
        }
    }

    /// A proxy that always asks the player (no property cache: values must be current).
    async fn proxy(&self, name: &str) -> zbus::Result<zbus::Proxy<'static>> {
        zbus::proxy::Builder::new(&self.conn)
            .destination(name.to_string())?
            .path(PATH)?
            .interface(PLAYER)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
    }

    async fn metadata(&self, name: &str) -> HashMap<String, OwnedValue> {
        match self.proxy(name).await {
            Ok(p) => p.get_property("Metadata").await.unwrap_or_default(),
            Err(_) => HashMap::new(),
        }
    }

    /// Players with their state, browser duplicates removed.
    async fn players(&self) -> Vec<(String, PlaybackStatus)> {
        let all = self.list().await;
        let mut pids = Vec::new();
        for (n, _) in &all {
            let pid = if n.contains("plasma-browser-integration") {
                self.metadata(n)
                    .await
                    .get("kde:pid")
                    .and_then(|v| i32::try_from(v.clone()).ok())
            } else {
                None
            };
            pids.push((n.clone(), pid));
        }
        let keep = drop_duplicates(&pids);
        all.into_iter().filter(|(n, _)| keep.contains(n)).collect()
    }

    /// The player the deck shows and controls.
    async fn choose(&self, wanted: Option<&str>) -> Option<String> {
        let players = self.players().await;
        if wanted.is_some_and(|w| !w.is_empty()) {
            return pick_player(&players, wanted);
        }
        let last = self.shown.lock().ok().and_then(|s| s.player.clone());
        pick_sticky(&players, last.as_deref())
    }

    pub async fn list(&self) -> Vec<(String, PlaybackStatus)> {
        let Ok(dbus) = zbus::fdo::DBusProxy::new(&self.conn).await else {
            return Vec::new();
        };
        let names = dbus.list_names().await.unwrap_or_default();
        let mut out = Vec::new();
        for n in names
            .iter()
            .map(|n| n.to_string())
            .filter(|n| n.starts_with(PREFIX))
        {
            let status = match self.proxy(&n).await {
                Ok(p) => p
                    .get_property::<String>("PlaybackStatus")
                    .await
                    .map(|s| PlaybackStatus::parse(&s)),
                Err(e) => Err(e),
            };
            out.push((n, status.unwrap_or(PlaybackStatus::Stopped)));
        }
        out
    }

    pub async fn control(&self, control: MediaControl, wanted: Option<&str>) -> Result<(), ActionError> {
        let Some(name) = self.choose(wanted).await else {
            return Err(ActionError::Unavailable(match wanted {
                Some(w) => format!("media player \"{w}\" is not running"),
                None => "no media player is running".into(),
            }));
        };
        let method = match control {
            MediaControl::PlayPause => "PlayPause",
            MediaControl::Next => "Next",
            MediaControl::Previous => "Previous",
            MediaControl::Stop => "Stop",
        };
        let p = self
            .proxy(&name)
            .await
            .map_err(|e| ActionError::Failed(e.to_string()))?;
        p.call_method(method, &())
            .await
            .map_err(|e| ActionError::Failed(format!("{name}: {e}")))?;
        Ok(())
    }

    /// Jump `seconds` forward (or back, when negative) in the chosen player's track.
    pub async fn seek(&self, seconds: i32, wanted: Option<&str>) -> Result<(), ActionError> {
        let Some(name) = self.choose(wanted).await else {
            return Err(ActionError::Unavailable("no media player is running".into()));
        };
        let p = self
            .proxy(&name)
            .await
            .map_err(|e| ActionError::Failed(e.to_string()))?;
        p.call_method("Seek", &(seconds as i64 * 1_000_000,))
            .await
            .map_err(|e| ActionError::Failed(format!("{name}: {e}")))?;
        Ok(())
    }

    pub async fn now_playing(&self) -> Option<MediaInfo> {
        let players = self.players().await;
        let last = self.shown.lock().ok().and_then(|s| s.player.clone());
        let name = pick_sticky(&players, last.as_deref())?;
        let status = players.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)?;
        let p = self.proxy(&name).await.ok()?;
        let meta: HashMap<String, OwnedValue> = p.get_property("Metadata").await.unwrap_or_default();
        let text = |k: &str| meta.get(k).and_then(|v| String::try_from(v.clone()).ok());
        let artist = meta
            .get("xesam:artist")
            .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
            .map(|a| a.join(", "))
            .or_else(|| text("xesam:artist"))
            .unwrap_or_default();
        let length_us = meta.get("mpris:length").and_then(|v| {
            i64::try_from(v.clone())
                .ok()
                .map(|n| n.max(0) as u64)
                .or_else(|| u64::try_from(v.clone()).ok())
        });
        let title = text("xesam:title").unwrap_or_default();
        let track = format!(
            "{}|{}",
            meta.get("mpris:trackid")
                .and_then(|v| zbus::zvariant::OwnedObjectPath::try_from(v.clone()).ok())
                .map(|o| o.to_string())
                .unwrap_or_default(),
            title
        );
        let reported = p
            .get_property::<i64>("Position")
            .await
            .ok()
            .map(|n| n.max(0) as u64);
        let position_us = {
            let mut shown = self.shown.lock().ok()?;
            if shown.player.as_deref() != Some(name.as_str()) {
                shown.player = Some(name.clone());
                shown.clock = TrackClock::default();
            }
            reported.map(|r| shown.clock.position(&track, r))
        };
        let position_us = match (position_us, length_us) {
            (Some(p), Some(l)) if l > 0 => Some(p.min(l)),
            (p, _) => p,
        };
        Some(MediaInfo {
            player: name.trim_start_matches(PREFIX).to_string(),
            title,
            artist,
            status,
            art_url: text("mpris:artUrl").filter(|u| !u.is_empty()),
            length_us,
            position_us,
        })
    }
}
