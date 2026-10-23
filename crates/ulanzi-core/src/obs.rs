//! What the deck knows about OBS Studio right now, for live keys, variables and the OBS widget.

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
}
