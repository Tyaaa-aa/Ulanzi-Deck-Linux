//! One short story of what a macro type does: the key on the left, the macro playing on the right,
//! and a caption saying what is happening. It stays still until someone points at the type cards,
//! so nothing moves while they work on the steps. Same drawing style as [`how_it_works`].
//!
//! [`how_it_works`]: crate::ui::how_it_works

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{cairo, glib};
use ulanzi_core::model::{PressAgain, Release, Trigger};

use crate::ui::how_it_works::{BLUE, GREEN, ease, phase, play_glyph, rounded};

const W: f64 = 460.0;
const H: f64 = 88.0;
const KEY: f64 = 48.0;
const KEY_AT: (f64, f64) = (32.0, 20.0);
const TRACK: (f64, f64, f64) = (116.0, 50.0, 300.0);
const TRACK_H: f64 = 12.0;
const PART_GAP: f64 = 6.0;
const RED: (f64, f64, f64) = (0.88, 0.27, 0.27);

/// How the macro's part of the picture ends up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    None,
    /// It is repeating.
    Repeat,
    /// One more play is waiting.
    Queued,
    /// It stopped before the end.
    Stopped,
    /// It reached the end.
    Done,
}

/// The whole picture at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// How far the key is pushed in, 0 to 1.
    pub down: f64,
    /// Seconds since the last press, while its ripple shows.
    pub ripple: Option<f64>,
    /// How full each part of the track is. One part, except for Sequence.
    pub fill: [f64; 3],
    pub parts: usize,
    pub mark: Mark,
    pub caption: &'static str,
    /// Fades the track away at the end of the loop.
    pub alpha: f64,
}

/// How long the story lasts.
pub fn period(t: &Trigger) -> f64 {
    match t {
        Trigger::Once {
            again: PressAgain::Queue,
        } => 6.4,
        Trigger::Once { .. } => 5.4,
        _ => 6.2,
    }
}

/// The moment shown when nothing is playing.
pub fn rest(t: &Trigger) -> f64 {
    match t {
        Trigger::Once { .. } => 1.3,
        Trigger::WhileHeld { .. } => 2.4,
        Trigger::Toggle => 2.4,
        Trigger::Sequence => 2.4,
    }
}

const TAP: f64 = 0.28;
const PRESS: f64 = 0.5;
const AGAIN: f64 = 2.0;
const LET_GO: f64 = 3.7;
const PASS: f64 = 1.2;
const PLAY: f64 = 2.4;

/// 0 to 1 and back: a key tapped at `at`.
fn tap(t: f64, at: f64) -> f64 {
    if (at..at + TAP).contains(&t) {
        ((t - at) / TAP * PI).sin()
    } else {
        0.0
    }
}

/// The key going in at `from` and coming back at `to`.
fn hold(t: f64, from: f64, to: f64) -> f64 {
    phase(t, from, from + 0.12) * (1.0 - phase(t, to, to + 0.12))
}

fn sweep(t: f64, from: f64, length: f64) -> f64 {
    ((t - from) / length).clamp(0.0, 1.0)
}

fn ripple(t: f64, presses: &[f64]) -> Option<f64> {
    presses
        .iter()
        .rev()
        .find(|at| t >= **at && t < **at + 0.8)
        .map(|at| t - at)
}

/// A pass that loops from `from`, as far as `t`.
fn looping(t: f64, from: f64) -> f64 {
    if t < from {
        0.0
    } else {
        ((t - from) / PASS).fract()
    }
}

/// What the story shows at time `t`.
pub fn frame(trigger: &Trigger, t: f64) -> Frame {
    let end = period(trigger);
    let mut f = Frame {
        down: 0.0,
        ripple: None,
        fill: [0.0; 3],
        parts: 1,
        mark: Mark::None,
        caption: "",
        alpha: 1.0 - phase(t, end - 0.5, end - 0.1),
    };
    match *trigger {
        Trigger::Once { again } => {
            f.down = tap(t, PRESS) + tap(t, AGAIN);
            f.ripple = ripple(t, &[PRESS, AGAIN]);
            f.caption = if t < PRESS {
                "Press the key"
            } else if t < AGAIN {
                "It plays"
            } else {
                match again {
                    PressAgain::Stop => "Press again: it stops",
                    PressAgain::Restart => "Press again: it starts over",
                    PressAgain::Ignore => "Press again: nothing changes",
                    PressAgain::Queue => "Press again: it plays once more after",
                }
            };
            let first = sweep(t, PRESS, PLAY);
            match again {
                PressAgain::Stop => {
                    f.fill[0] = first.min(sweep(AGAIN, PRESS, PLAY));
                    if t >= AGAIN {
                        f.mark = Mark::Stopped;
                    }
                }
                PressAgain::Restart => {
                    f.fill[0] = if t < AGAIN { first } else { sweep(t, AGAIN, PLAY) };
                    if t >= AGAIN + PLAY {
                        f.mark = Mark::Done;
                    }
                }
                PressAgain::Ignore => {
                    f.fill[0] = first;
                    if t >= PRESS + PLAY {
                        f.mark = Mark::Done;
                    }
                }
                PressAgain::Queue => {
                    let second = PRESS + PLAY;
                    f.fill[0] = if t < second { first } else { sweep(t, second, PLAY) };
                    if t >= second + PLAY {
                        f.mark = Mark::Done;
                    } else if (AGAIN..second).contains(&t) {
                        f.mark = Mark::Queued;
                    }
                }
            }
        }
        Trigger::WhileHeld { on_release } => {
            f.down = hold(t, PRESS, LET_GO);
            f.ripple = ripple(t, &[PRESS]);
            let stopped = looping(LET_GO, PRESS);
            match on_release {
                Release::FinishPass => {
                    let end_of_pass = LET_GO + (1.0 - stopped) * PASS;
                    f.fill[0] = if t < end_of_pass { looping(t, PRESS) } else { 1.0 };
                    f.mark = match t {
                        t if t >= end_of_pass => Mark::Done,
                        t if t >= PRESS => Mark::Repeat,
                        _ => Mark::None,
                    };
                }
                Release::StopNow => {
                    f.fill[0] = if t < LET_GO { looping(t, PRESS) } else { stopped };
                    f.mark = match t {
                        t if t >= LET_GO => Mark::Stopped,
                        t if t >= PRESS => Mark::Repeat,
                        _ => Mark::None,
                    };
                }
            }
            f.caption = if t < PRESS {
                "Hold the key"
            } else if t < LET_GO {
                "It repeats while you hold"
            } else if on_release == Release::FinishPass {
                "Let go: it finishes this pass"
            } else {
                "Let go: it stops at once"
            };
        }
        Trigger::Toggle => {
            f.down = tap(t, PRESS) + tap(t, LET_GO);
            f.ripple = ripple(t, &[PRESS, LET_GO]);
            f.fill[0] = if t < LET_GO {
                looping(t, PRESS)
            } else {
                looping(LET_GO, PRESS)
            };
            f.mark = match t {
                t if t >= LET_GO => Mark::Stopped,
                t if t >= PRESS => Mark::Repeat,
                _ => Mark::None,
            };
            f.caption = if t < PRESS {
                "Press the key"
            } else if t < LET_GO {
                "It keeps repeating, hands off"
            } else {
                "Press again: it stops"
            };
        }
        Trigger::Sequence => {
            const FIRST: f64 = 0.9;
            let held_from = PRESS + FIRST;
            f.parts = 3;
            f.down = hold(t, PRESS, LET_GO);
            f.ripple = ripple(t, &[PRESS]);
            f.fill[0] = sweep(t, PRESS, FIRST);
            f.fill[1] = if t >= LET_GO { 1.0 } else { looping(t, held_from) };
            f.fill[2] = sweep(t, LET_GO, FIRST);
            f.mark = match t {
                t if t >= LET_GO + FIRST => Mark::Done,
                t if t >= held_from && t < LET_GO => Mark::Repeat,
                _ => Mark::None,
            };
            f.caption = if t < PRESS {
                "Press and hold the key"
            } else if t < held_from {
                "Press: the first steps play"
            } else if t < LET_GO {
                "Hold: the middle steps repeat"
            } else {
                "Let go: the last steps play"
            };
        }
    }
    f
}

fn draw_mark(cr: &cairo::Context, mark: Mark, x: f64, y: f64) {
    match mark {
        Mark::None => {}
        Mark::Repeat => {
            cr.set_source_rgb(BLUE.0, BLUE.1, BLUE.2);
            cr.set_line_width(2.2);
            cr.new_sub_path();
            cr.arc(x, y, 7.0, -0.3 * PI, 1.4 * PI);
            let _ = cr.stroke();
            cr.move_to(x + 2.0, y - 10.5);
            cr.line_to(x + 7.5, y - 7.0);
            cr.line_to(x + 1.5, y - 3.5);
            cr.close_path();
            let _ = cr.fill();
        }
        Mark::Queued => {
            cr.arc(x, y, 9.0, 0.0, 2.0 * PI);
            cr.set_source_rgb(BLUE.0, BLUE.1, BLUE.2);
            let _ = cr.fill();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.2);
            cr.move_to(x - 4.5, y);
            cr.line_to(x + 4.5, y);
            cr.move_to(x, y - 4.5);
            cr.line_to(x, y + 4.5);
            let _ = cr.stroke();
        }
        Mark::Stopped => {
            rounded(cr, x - 6.5, y - 6.5, 13.0, 13.0, 3.0);
            cr.set_source_rgb(RED.0, RED.1, RED.2);
            let _ = cr.fill();
        }
        Mark::Done => {
            cr.arc(x, y, 9.0, 0.0, 2.0 * PI);
            cr.set_source_rgb(GREEN.0, GREEN.1, GREEN.2);
            let _ = cr.fill();
            cr.set_source_rgb(0.05, 0.2, 0.12);
            cr.set_line_width(2.2);
            cr.move_to(x - 4.5, y + 0.5);
            cr.line_to(x - 1.2, y + 3.8);
            cr.line_to(x + 4.8, y - 3.2);
            let _ = cr.stroke();
        }
    }
}

/// Draw one frame. `fg` is the theme's text colour.
fn draw(cr: &cairo::Context, f: &Frame, fg: (f64, f64, f64)) {
    // A piece of the deck, physically dark in both themes.
    let (kx, ky) = KEY_AT;
    rounded(cr, kx - 12.0, ky - 12.0, KEY + 24.0, KEY + 24.0, 16.0);
    let grad = cairo::LinearGradient::new(0.0, ky - 12.0, 0.0, ky + KEY + 12.0);
    grad.add_color_stop_rgb(0.0, 0.17, 0.17, 0.19);
    grad.add_color_stop_rgb(1.0, 0.086, 0.086, 0.1);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();

    // The key stands proud of the deck and sinks flush when pushed, dimming as it goes.
    let (cx, cy) = (kx + KEY / 2.0, ky + KEY / 2.0);
    let pushed = ease(f.down);
    rounded(cr, kx, ky, KEY, KEY, 9.0);
    cr.set_source_rgb(0.03, 0.03, 0.035);
    let _ = cr.fill();
    let cap = KEY - 6.0 - 2.0 * pushed;
    let lift = 3.5 * (1.0 - pushed);
    let (capx, capy) = (cx - cap / 2.0, cy - cap / 2.0 - lift);
    if lift > 0.0 {
        rounded(cr, capx, capy + lift, cap, cap, 7.0);
        cr.set_source_rgb(BLUE.0 * 0.45, BLUE.1 * 0.45, BLUE.2 * 0.45);
        let _ = cr.fill();
    }
    let dim = 1.0 - 0.3 * pushed;
    rounded(cr, capx, capy, cap, cap, 7.0);
    cr.set_source_rgb(BLUE.0 * dim, BLUE.1 * dim, BLUE.2 * dim);
    let _ = cr.fill();
    play_glyph(cr, cx, cy - lift, cap * 0.42);

    if let Some(age) = f.ripple {
        let p = age / 0.8;
        cr.save().ok();
        rounded(cr, kx - 12.0, ky - 12.0, KEY + 24.0, KEY + 24.0, 16.0);
        cr.clip();
        cr.arc(cx, cy, KEY * 0.5 + 18.0 * p, 0.0, 2.0 * PI);
        cr.set_source_rgba(BLUE.0, BLUE.1, BLUE.2, 0.7 * (1.0 - p));
        cr.set_line_width(3.0);
        let _ = cr.stroke();
        cr.restore().ok();
    }

    // The macro: a track that fills as it plays.
    let (x, y, width) = TRACK;
    let parts = f.parts as f64;
    let part_w = (width - PART_GAP * (parts - 1.0)) / parts;
    for i in 0..f.parts {
        let px = x + i as f64 * (part_w + PART_GAP);
        rounded(cr, px, y, part_w, TRACK_H, TRACK_H / 2.0);
        cr.set_source_rgba(fg.0, fg.1, fg.2, 0.13);
        let _ = cr.fill();
        let fill = f.fill[i] * part_w;
        if fill > 0.5 {
            rounded(cr, px, y, fill.max(TRACK_H), TRACK_H, TRACK_H / 2.0);
            cr.set_source_rgba(BLUE.0, BLUE.1, BLUE.2, f.alpha);
            let _ = cr.fill();
        }
    }
    if f.alpha > 0.0 {
        cr.push_group();
        draw_mark(cr, f.mark, x + width + 20.0, y + TRACK_H / 2.0);
        let _ = cr.pop_group_to_source();
        let _ = cr.paint_with_alpha(f.alpha);
    }
}

/// The diagram: `show` plays a type once, `hover` keeps it looping while the pointer is on the
/// cards, and `settle` goes back to the type in use.
#[derive(Clone)]
pub struct Diagram {
    pub root: gtk::Widget,
    area: gtk::DrawingArea,
    caption: gtk::Label,
    shown: Rc<Cell<Trigger>>,
    chosen: Rc<Cell<Trigger>>,
    time: Rc<Cell<f64>>,
    hovering: Rc<Cell<bool>>,
    playing: Rc<Cell<bool>>,
    /// Goes up with each start, so an older tick callback knows to give up.
    run: Rc<Cell<u32>>,
    last: Rc<Cell<Option<i64>>>,
}

impl Diagram {
    pub fn new(trigger: Trigger) -> Self {
        let area = gtk::DrawingArea::builder()
            .content_width(W as i32)
            .content_height(H as i32)
            .halign(gtk::Align::Start)
            .build();
        let caption = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start(TRACK.0 as i32)
            .margin_top(20)
            .css_classes(["heading"])
            .build();
        let root = gtk::Overlay::builder().child(&area).build();
        root.add_overlay(&caption);
        root.set_measure_overlay(&caption, false);
        let d = Self {
            root: root.upcast(),
            area,
            caption,
            shown: Rc::new(Cell::new(trigger)),
            chosen: Rc::new(Cell::new(trigger)),
            time: Rc::new(Cell::new(rest(&trigger))),
            hovering: Rc::default(),
            playing: Rc::default(),
            run: Rc::default(),
            last: Rc::default(),
        };
        let (shown, time) = (d.shown.clone(), d.time.clone());
        d.area.set_draw_func(move |a, cr, _, _| {
            let c = a.color();
            let fg = (c.red() as f64, c.green() as f64, c.blue() as f64);
            draw(cr, &frame(&shown.get(), time.get()), fg);
        });
        d.refresh();
        d
    }

    fn refresh(&self) {
        let f = frame(&self.shown.get(), self.time.get());
        if self.caption.label() != f.caption {
            self.caption.set_label(f.caption);
        }
        self.area.queue_draw();
    }

    fn start(&self) {
        let animate = gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations());
        if !animate {
            self.time.set(rest(&self.shown.get()));
            self.refresh();
            return;
        }
        self.time.set(0.0);
        self.last.set(None);
        self.refresh();
        self.playing.set(true);
        let run = self.run.get() + 1;
        self.run.set(run);
        let d = self.clone();
        self.area.add_tick_callback(move |_, clock| {
            if d.run.get() != run {
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let step = d.last.replace(Some(now)).map_or(0.0, |l| (now - l) as f64 / 1e6);
            let t = d.time.get() + step;
            let shown = d.shown.get();
            if t < period(&shown) {
                d.time.set(t);
            } else if d.hovering.get() {
                d.time.set(0.0);
            } else {
                d.time.set(rest(&shown));
                d.playing.set(false);
                d.refresh();
                return glib::ControlFlow::Break;
            }
            d.refresh();
            glib::ControlFlow::Continue
        });
    }

    /// Play `trigger` from the start.
    pub fn show(&self, trigger: Trigger) {
        self.shown.set(trigger);
        self.start();
    }

    /// The type in use changed: it plays once.
    pub fn choose(&self, trigger: Trigger) {
        if self.chosen.replace(trigger) != trigger {
            self.show(trigger);
        }
    }

    /// The pointer came onto a card or the diagram, or left.
    pub fn hover(&self, on: bool) {
        self.hovering.set(on);
        if on && !self.playing.get() {
            self.start();
        } else if !on && self.shown.get() != self.chosen.get() {
            let chosen = self.chosen.get();
            self.shown.set(chosen);
            self.time.set(rest(&chosen));
            self.playing.set(false);
            self.run.set(self.run.get() + 1);
            self.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn once(again: PressAgain) -> Trigger {
        Trigger::Once { again }
    }

    fn held(on_release: Release) -> Trigger {
        Trigger::WhileHeld { on_release }
    }

    /// Render frames to PNG for a look: `DIAGRAM_FRAMES=dir cargo test -- --ignored diagram_frames`.
    #[test]
    #[ignore]
    fn diagram_frames() {
        let Some(dir) = std::env::var_os("DIAGRAM_FRAMES") else {
            return;
        };
        for (name, trigger) in [
            ("stop", once(PressAgain::Stop)),
            ("queue", once(PressAgain::Queue)),
            ("held", held(Release::FinishPass)),
            ("toggle", Trigger::Toggle),
            ("sequence", Trigger::Sequence),
        ] {
            for (i, t) in [0.2, rest(&trigger), 3.0, 4.4, 5.0].iter().enumerate() {
                let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, W as i32 * 2, H as i32 * 2)
                    .expect("surface");
                let cr = cairo::Context::new(&surf).expect("context");
                cr.set_source_rgb(0.14, 0.14, 0.15);
                cr.paint().expect("bg");
                cr.scale(2.0, 2.0);
                draw(&cr, &frame(&trigger, *t), (1.0, 1.0, 1.0));
                drop(cr);
                let (w, h, stride) = (surf.width() as u32, surf.height() as u32, surf.stride() as usize);
                let data = surf.take_data().expect("data");
                let img = image::RgbaImage::from_fn(w, h, |x, y| {
                    let o = y as usize * stride + x as usize * 4;
                    image::Rgba([data[o + 2], data[o + 1], data[o], 255])
                });
                img.save(std::path::Path::new(&dir).join(format!("{name}{i}.png")))
                    .expect("png");
            }
        }
    }

    #[test]
    fn each_press_again_choice_tells_its_own_ending() {
        let mid = 3.0;
        assert_eq!(frame(&once(PressAgain::Stop), mid).mark, Mark::Stopped);
        assert_eq!(frame(&once(PressAgain::Queue), 2.5).mark, Mark::Queued);
        assert_eq!(frame(&once(PressAgain::Ignore), 3.2).mark, Mark::Done);
        // Restart goes back: less full just after the second press than just before.
        let r = once(PressAgain::Restart);
        assert!(frame(&r, AGAIN + 0.1).fill[0] < frame(&r, AGAIN - 0.1).fill[0]);
        assert_eq!(frame(&r, AGAIN + PLAY + 0.1).mark, Mark::Done);
        // Stop freezes where it was.
        let s = once(PressAgain::Stop);
        assert_eq!(frame(&s, AGAIN + 0.1).fill[0], frame(&s, 4.0).fill[0]);
    }

    #[test]
    fn letting_go_shows_the_difference() {
        let finish = held(Release::FinishPass);
        let now = held(Release::StopNow);
        assert_eq!(frame(&now, LET_GO + 0.1).mark, Mark::Stopped);
        assert_eq!(frame(&finish, LET_GO + 0.05).mark, Mark::Repeat);
        assert_eq!(frame(&finish, 5.2).mark, Mark::Done);
        assert_eq!(frame(&finish, 5.2).fill[0], 1.0);
    }

    #[test]
    fn the_key_stays_down_only_when_held() {
        assert!(frame(&held(Release::StopNow), 2.5).down > 0.9);
        assert!(frame(&Trigger::Sequence, 2.5).down > 0.9);
        assert_eq!(frame(&Trigger::Toggle, 2.5).down, 0.0);
    }

    #[test]
    fn sequence_fills_its_parts_in_order() {
        let s = Trigger::Sequence;
        assert_eq!(frame(&s, 0.2).fill, [0.0, 0.0, 0.0]);
        assert_eq!(frame(&s, 1.5).fill[0], 1.0);
        assert_eq!(frame(&s, 1.5).fill[2], 0.0);
        assert_eq!(frame(&s, 5.0).fill, [1.0, 1.0, 1.0]);
        assert_eq!(frame(&s, 5.0).mark, Mark::Done);
    }

    #[test]
    fn every_moment_has_a_caption_and_the_rest_moment_is_inside_the_story() {
        for t in [
            once(PressAgain::Stop),
            once(PressAgain::Restart),
            once(PressAgain::Ignore),
            once(PressAgain::Queue),
            held(Release::FinishPass),
            held(Release::StopNow),
            Trigger::Toggle,
            Trigger::Sequence,
        ] {
            assert!(rest(&t) < period(&t));
            let mut at = 0.0;
            while at < period(&t) {
                assert!(!frame(&t, at).caption.is_empty());
                at += 0.1;
            }
        }
    }
}
