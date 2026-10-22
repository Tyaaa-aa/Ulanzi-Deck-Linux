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

/// MPRIS client on the session bus.
#[derive(Clone)]
pub struct MediaClient {
    conn: Connection,
}

impl MediaClient {
    pub fn new(conn: Connection) -> MediaClient {
        MediaClient { conn }
    }

    async fn proxy(&self, name: &str) -> zbus::Result<zbus::Proxy<'static>> {
        zbus::Proxy::new(&self.conn, name.to_string(), PATH, PLAYER).await
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
        let players = self.list().await;
        let Some(name) = pick_player(&players, wanted) else {
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

    pub async fn now_playing(&self) -> Option<MediaInfo> {
        let players = self.list().await;
        let name = pick_player(&players, None)?;
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
        let position_us = p
            .get_property::<i64>("Position")
            .await
            .ok()
            .map(|n| n.max(0) as u64);
        Some(MediaInfo {
            player: name.trim_start_matches(PREFIX).to_string(),
            title: text("xesam:title").unwrap_or_default(),
            artist,
            status,
            art_url: text("mpris:artUrl").filter(|u| !u.is_empty()),
            length_us,
            position_us,
        })
    }
}
