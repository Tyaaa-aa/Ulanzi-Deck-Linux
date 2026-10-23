//! What the wide screen shows in widget mode: picks the widget (rotation), gathers its data
//! (timer, weather and command output fetched in the background), draws it, and lays the volume
//! overlay on top for a moment after a volume change.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;
use ulanzi_core::model::{Action, Profile, Rotation, ScreenConfig, TimerOp, Widget};
use ulanzi_core::render::Fonts;
use ulanzi_core::render::screen::{ScreenData, ScreenStyle, WeatherData, render_widget, volume_overlay};

use super::push::DynamicState;
use crate::widgets::rotation::Rotator;
use crate::widgets::timer::TimerState;
use crate::widgets::{command, weather};

/// How long the volume overlay stays after the last change.
pub const OVERLAY_FOR: Duration = Duration::from_millis(1500);
const FINISHED_SOUND: &str = "/usr/share/sounds/freedesktop/stereo/complete.oga";

/// Background results, keyed by what was asked.
#[derive(Default)]
struct Fetched {
    weather: HashMap<(String, bool), (WeatherData, Instant)>,
    command: HashMap<String, (String, Instant)>,
    in_flight: HashSet<String>,
}

pub struct ScreenRuntime {
    rot: Rotator,
    timer: TimerState,
    timer_cfg: Option<(u32, bool)>,
    fetched: Arc<Mutex<Fetched>>,
    /// Woken when a background fetch finished, so the screen redraws without waiting a tick.
    pub wake: Arc<Notify>,
    fonts: Option<Fonts>,
    /// The widget drawn last (for the screen's default tap).
    shown: Option<Widget>,
    overlay: Option<(u8, bool, String, Instant)>,
    /// The last widget picture, for the overlay to lie on.
    base: Option<Vec<u8>>,
}

fn widgets(p: &Profile) -> Option<(&[Widget], Rotation)> {
    match &p.screen {
        ScreenConfig::Custom { widgets, rotate, .. } => Some((widgets.as_slice(), *rotate)),
        _ => None,
    }
}

fn png_hash(png: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    png.hash(&mut h);
    h.finish()
}

impl ScreenRuntime {
    pub fn new(now: Instant) -> ScreenRuntime {
        ScreenRuntime {
            rot: Rotator::new(now.into_std()),
            timer: TimerState::new(25, false),
            timer_cfg: None,
            fetched: Arc::default(),
            wake: Arc::new(Notify::new()),
            fonts: None,
            shown: None,
            overlay: None,
            base: None,
        }
    }

    /// The overlay is up.
    pub fn overlay_active(&self, now: Instant) -> bool {
        self.overlay.as_ref().is_some_and(|(_, _, _, until)| now < *until)
    }

    /// Show (or update) the volume overlay.
    pub fn show_overlay(&mut self, percent: u8, muted: bool, label: String, now: Instant) {
        self.overlay = Some((percent, muted, label, now + OVERLAY_FOR));
    }

    /// The widget drawn last.
    #[cfg(test)]
    pub fn shown(&self) -> Option<&Widget> {
        self.shown.as_ref()
    }

    /// Default action for a screen tap when the profile sets none.
    pub fn default_tap(&self, p: &Profile) -> Option<Action> {
        let (list, rotate) = widgets(p)?;
        if rotate == Rotation::OnPress && list.len() > 1 {
            return Some(Action::ScreenNext);
        }
        self.shown
            .as_ref()
            .filter(|w| list.contains(w))
            .or(list.first())?
            .default_tap()
    }

    /// Next Widget.
    pub fn next(&mut self, now: Instant) {
        self.rot.next(now.into_std());
    }

    /// A timer action. The timer follows the first Timer widget's settings.
    pub fn timer(&mut self, op: TimerOp, now: Instant) {
        let now = now.into_std();
        match op {
            TimerOp::StartPause => self.timer.start_pause(now),
            TimerOp::Reset => self.timer.reset(),
            TimerOp::Add { minutes } => self.timer.add(minutes, now),
        }
    }

    /// The timer now (plays a sound when it runs out).
    pub fn timer_now(&mut self, now: Instant) -> ulanzi_core::render::screen::TimerData {
        let (t, finished) = self.timer.tick(now.into_std());
        if finished {
            play_finished();
        }
        t
    }

    /// The picture for the wide screen now: the widget (in widget mode) with the overlay on top
    /// while it is up; `None` when a firmware mode shows and no overlay is up.
    pub fn picture(
        &mut self,
        p: &Profile,
        dir: &Path,
        dynamic: &DynamicState,
        now: Instant,
    ) -> Option<Vec<u8>> {
        let style = match &p.screen {
            ScreenConfig::Custom {
                background, accent, ..
            } => Some(ScreenStyle {
                background: background.clone(),
                accent: *accent,
                profile_dir: dir.to_path_buf(),
            }),
            _ => None,
        };
        let base = match (&style, widgets(p)) {
            (Some(style), Some((list, rotate))) if !list.is_empty() => {
                let i = self.rot.current(rotate, list.len(), now.into_std());
                let w = list[i].clone();
                let data = self.data(&w, list, dir, dynamic, now);
                let fonts = self.fonts.get_or_insert_with(Fonts::system);
                let png = render_widget(&w, style, &data, fonts);
                self.shown = Some(w);
                Some(png)
            }
            _ => {
                self.shown = None;
                None
            }
        };
        self.base = base.clone();
        match self.overlay.clone() {
            Some((pct, muted, label, until)) if now < until => {
                let accent = style
                    .map(|s| s.accent)
                    .unwrap_or_else(ulanzi_core::model::default_accent);
                let fonts = self.fonts.get_or_insert_with(Fonts::system);
                Some(volume_overlay(base.as_deref(), pct, muted, &label, accent, fonts))
            }
            _ => {
                self.overlay = None;
                base
            }
        }
    }

    fn data(
        &mut self,
        w: &Widget,
        list: &[Widget],
        dir: &Path,
        dynamic: &DynamicState,
        now: Instant,
    ) -> ScreenData {
        let mut data = ScreenData {
            now: chrono::Local::now().naive_local(),
            stats: dynamic.stats.clone(),
            media: dynamic.media.clone(),
            obs: dynamic.obs.clone(),
            ..ScreenData::default()
        };
        // The timer follows the first Timer widget's settings and keeps running while another shows.
        if let Some(Widget::Timer { minutes, pomodoro }) =
            list.iter().find(|w| matches!(w, Widget::Timer { .. }))
            && self.timer_cfg != Some((*minutes, *pomodoro))
        {
            self.timer.configure(*minutes, *pomodoro);
            self.timer_cfg = Some((*minutes, *pomodoro));
        }
        data.timer = Some(self.timer_now(now));
        match w {
            Widget::Weather { place, fahrenheit } => {
                data.weather = Some(self.weather(place, *fahrenheit, now))
            }
            Widget::Command { cmd, every_s, .. } => data.command = Some(self.command(cmd, *every_s, now)),
            Widget::Pictures { paths, every_s } if !paths.is_empty() => {
                let secs = chrono::Local::now().timestamp().max(0) as u64;
                let i = (secs / (*every_s).max(1) as u64) as usize % paths.len();
                data.picture = Some(dir.join(&paths[i]));
            }
            _ => {}
        }
        data
    }

    fn weather(&self, place: &str, fahrenheit: bool, now: Instant) -> WeatherData {
        let key = (place.to_string(), fahrenheit);
        let Ok(mut f) = self.fetched.lock() else {
            return WeatherData::Err("unavailable".into());
        };
        let cached = f.weather.get(&key).cloned();
        let stale = cached
            .as_ref()
            .is_none_or(|(_, at)| now.duration_since(*at) >= Duration::from_secs(weather::REFRESH_SECS));
        let flight = format!("w:{place}:{fahrenheit}");
        if stale && f.in_flight.insert(flight.clone()) {
            let (fetched, wake) = (self.fetched.clone(), self.wake.clone());
            tokio::task::spawn_blocking(move || {
                let w = weather::fetch(&key.0, key.1);
                if let Ok(mut f) = fetched.lock() {
                    f.weather.insert(key, (w, Instant::now()));
                    f.in_flight.remove(&flight);
                }
                wake.notify_one();
            });
        }
        cached
            .map(|(w, _)| w)
            .unwrap_or_else(|| WeatherData::Err("Loading…".into()))
    }

    fn command(&self, cmd: &str, every_s: u32, now: Instant) -> String {
        let Ok(mut f) = self.fetched.lock() else {
            return String::new();
        };
        let cached = f.command.get(cmd).cloned();
        let stale = cached
            .as_ref()
            .is_none_or(|(_, at)| now.duration_since(*at) >= Duration::from_secs(every_s.max(1) as u64));
        let flight = format!("c:{cmd}");
        if stale && f.in_flight.insert(flight.clone()) {
            let (fetched, wake, cmd) = (self.fetched.clone(), self.wake.clone(), cmd.to_string());
            tokio::spawn(async move {
                let out = command::run_bounded(&cmd, command::TIMEOUT).await;
                if let Ok(mut f) = fetched.lock() {
                    f.command.insert(cmd, (out, Instant::now()));
                    f.in_flight.remove(&flight);
                }
                wake.notify_one();
            });
        }
        cached.map(|(s, _)| s).unwrap_or_default()
    }
}

/// The picture changed since `last` (by hash); updates `last`.
pub fn changed(last: &mut Option<u64>, png: Option<&[u8]>) -> bool {
    let h = png.map(png_hash);
    if *last == h {
        return false;
    }
    *last = h;
    true
}

fn play_finished() {
    if Path::new(FINISHED_SOUND).is_file() {
        let _ = crate::actions::command::spawn_detached(&["pw-play".into(), FINISHED_SOUND.into()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulanzi_core::model::Model;

    fn custom(widgets: Vec<Widget>, rotate: Rotation) -> Profile {
        let mut p = Profile::new_default("T", Model::D200X);
        let mut s = ScreenConfig::custom(widgets, None, None);
        if let ScreenConfig::Custom { rotate: r, .. } = &mut s {
            *r = rotate;
        }
        p.screen = s;
        p
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unchanged_screen_is_not_resent() {
        let p = custom(vec![Widget::Blank], Rotation::Off);
        let mut rt = ScreenRuntime::new(Instant::now());
        let mut last = None;
        let dir = std::env::temp_dir();
        let a = rt.picture(&p, &dir, &DynamicState::default(), Instant::now());
        assert!(a.is_some());
        assert!(changed(&mut last, a.as_deref()));
        let b = rt.picture(&p, &dir, &DynamicState::default(), Instant::now());
        assert!(!changed(&mut last, b.as_deref()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn overlay_extends_and_restores_mode() {
        let mut p = Profile::new_default("T", Model::D200X);
        p.screen = ScreenConfig::FirmwareStats;
        let mut rt = ScreenRuntime::new(Instant::now());
        let dir = std::env::temp_dir();
        let t0 = Instant::now();
        assert!(rt.picture(&p, &dir, &DynamicState::default(), t0).is_none());
        rt.show_overlay(40, false, String::new(), t0);
        rt.show_overlay(45, false, "Spotify".into(), t0 + Duration::from_millis(1000));
        assert!(rt.overlay_active(t0 + Duration::from_millis(2000)));
        assert!(
            rt.picture(
                &p,
                &dir,
                &DynamicState::default(),
                t0 + Duration::from_millis(2000)
            )
            .is_some()
        );
        assert!(!rt.overlay_active(t0 + Duration::from_millis(2600)));
        assert!(
            rt.picture(
                &p,
                &dir,
                &DynamicState::default(),
                t0 + Duration::from_millis(2600)
            )
            .is_none()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn screen_next_and_timer_actions() {
        let p = custom(
            vec![
                Widget::Timer {
                    minutes: 1,
                    pomodoro: false,
                },
                Widget::Blank,
            ],
            Rotation::OnPress,
        );
        let t0 = Instant::now();
        let mut rt = ScreenRuntime::new(t0);
        let dir = std::env::temp_dir();
        rt.picture(&p, &dir, &DynamicState::default(), t0);
        assert!(matches!(rt.shown(), Some(Widget::Timer { .. })));
        assert_eq!(rt.default_tap(&p), Some(Action::ScreenNext));
        rt.next(t0);
        rt.picture(&p, &dir, &DynamicState::default(), t0);
        assert_eq!(rt.shown(), Some(&Widget::Blank));
        rt.timer(TimerOp::StartPause, t0);
        let data = rt.data(
            &Widget::Blank,
            widgets(&p).map(|w| w.0).unwrap_or(&[]),
            &dir,
            &DynamicState::default(),
            t0 + Duration::from_secs(10),
        );
        let t = data.timer.expect("timer data");
        assert!(t.running);
        assert_eq!(t.remaining_s, 50);
        rt.timer(TimerOp::Reset, t0);
        let single = custom(
            vec![Widget::Timer {
                minutes: 5,
                pomodoro: false,
            }],
            Rotation::Off,
        );
        assert_eq!(
            rt.default_tap(&single),
            Some(Action::Timer {
                op: TimerOp::StartPause
            })
        );
    }
}
