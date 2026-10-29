//! What the OBS widget shows, read from the OBS plugin's live values (`{obs.recording}`, …).

use std::collections::BTreeSet;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObsStats {
    /// Frames per second, rounded.
    pub fps: u32,
    /// OBS's CPU use, percent.
    pub cpu: u32,
    /// Frames skipped by the encoder or the network, per thousand frames.
    pub dropped_permille: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObsLive {
    pub connected: bool,
    /// The scene on the program output.
    pub program: String,
    /// The preview scene, while studio mode is on.
    pub preview: Option<String>,
    pub scenes: Vec<String>,
    pub recording: bool,
    pub rec_paused: bool,
    pub rec_secs: u64,
    pub streaming: bool,
    pub stream_secs: u64,
    pub replay: bool,
    pub vcam: bool,
    /// Inputs that are muted.
    pub muted: BTreeSet<String>,
    /// (scene, source) pairs that are hidden.
    pub hidden: BTreeSet<(String, String)>,
    pub stats: Option<ObsStats>,
}

impl ObsLive {
    /// From the OBS plugin's published values (`obs.<key>`); `None` while the plugin has not said
    /// whether it is connected (not installed, or not started).
    pub fn from_plugin(vars: &std::collections::BTreeMap<String, String>) -> Option<ObsLive> {
        let get = |k: &str| vars.get(&format!("obs.{k}")).map(String::as_str);
        let yes = |k: &str| get(k) == Some("yes");
        let secs = |k: &str| {
            get(k).map_or(0, |t| {
                t.split(':')
                    .filter_map(|p| p.parse::<u64>().ok())
                    .fold(0, |acc, n| acc * 60 + n)
            })
        };
        let num = |k: &str| get(k).and_then(|v| v.trim_end_matches('%').parse::<f64>().ok());
        get("connected")?;
        Some(ObsLive {
            connected: yes("connected"),
            program: get("scene").unwrap_or_default().to_string(),
            preview: get("preview").filter(|p| !p.is_empty()).map(str::to_string),
            scenes: Vec::new(),
            recording: yes("recording"),
            rec_paused: yes("rec_paused"),
            rec_secs: secs("rec_time"),
            streaming: yes("streaming"),
            stream_secs: secs("stream_time"),
            replay: yes("replay"),
            vcam: yes("vcam"),
            muted: Default::default(),
            hidden: Default::default(),
            stats: num("fps").map(|fps| ObsStats {
                fps: fps.round() as u32,
                cpu: num("cpu").unwrap_or(0.0).round() as u32,
                dropped_permille: (num("dropped").unwrap_or(0.0) * 10.0).round() as u32,
            }),
        })
    }
}

/// `h:mm:ss`, or `mm:ss` under an hour.
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(83), "01:23");
        assert_eq!(clock(3600 + 61), "1:01:01");
    }

    #[test]
    fn the_widget_reads_the_plugins_values() {
        let mut v = std::collections::BTreeMap::new();
        assert_eq!(ObsLive::from_plugin(&v), None, "no plugin");
        for (k, x) in [
            ("obs.connected", "yes"),
            ("obs.scene", "Intro"),
            ("obs.recording", "yes"),
            ("obs.rec_time", "1:02:03"),
            ("obs.fps", "60"),
            ("obs.dropped", "0.4%"),
        ] {
            v.insert(k.to_string(), x.to_string());
        }
        let l = ObsLive::from_plugin(&v).expect("live");
        assert!(l.connected && l.recording && !l.streaming);
        assert_eq!(l.program, "Intro");
        assert_eq!(l.rec_secs, 3723);
        assert_eq!(l.stats.expect("stats").dropped_permille, 4);
    }
}
