//! The light on a drawn dial when the real one moves: the whole ring glows and fades, and an arc
//! sweeps the way it turned (a press only glows).

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

use adw::prelude::*;

/// How a dial moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialMove {
    Press,
    Left,
    Right,
}

/// How long the light lasts.
const MS: u32 = 400;
/// How far the arc sweeps.
const SWEEP: f64 = PI / 2.0;

fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The ring's light at `t` (0..1 of the animation): full at the start, easing out to nothing.
pub fn glow_alpha(t: f64) -> f64 {
    1.0 - ease_out(t)
}

/// Where the arc starts: the top of the dial.
const TOP: f64 = -PI / 2.0;
/// How opaque the arc is at its start; it fades to nothing at its moving end.
const ARC_ALPHA: f64 = 0.55;

/// The direction arc at `t` as (start, end) angles in radians (Cairo's: clockwise from 3 o'clock),
/// growing from the top of the dial the way it turned; `None` for a press.
pub fn arc(m: DialMove, t: f64) -> Option<(f64, f64)> {
    let grown = SWEEP * ease_out(t);
    match m {
        DialMove::Press => None,
        DialMove::Right => Some((TOP, TOP + grown)),
        DialMove::Left => Some((TOP, TOP - grown)),
    }
}

/// A light drawn over a dial: a transparent area laid over the dial and the room around it, so
/// the arc can run just outside the rim.
pub struct Ring {
    pub area: gtk::DrawingArea,
    anim: adw::TimedAnimation,
    shown: Rc<Cell<Option<DialMove>>>,
}

impl Ring {
    /// A ring for `dial`, drawn on its bounds wherever the area sits around it.
    pub fn new(dial: &impl IsA<gtk::Widget>) -> Ring {
        let area = gtk::DrawingArea::builder().can_target(false).build();
        let shown: Rc<Cell<Option<DialMove>>> = Rc::default();
        // Linear here: the ease-out lives in glow_alpha and arc.
        let target = {
            let area = area.downgrade();
            adw::CallbackAnimationTarget::new(move |_| {
                if let Some(a) = area.upgrade() {
                    a.queue_draw();
                }
            })
        };
        let anim = adw::TimedAnimation::builder()
            .widget(&area)
            .value_from(0.0)
            .value_to(1.0)
            .duration(MS)
            .easing(adw::Easing::Linear)
            .target(&target)
            .build();
        {
            let (shown, anim, dial) = (shown.clone(), anim.downgrade(), dial.as_ref().downgrade());
            area.set_draw_func(move |area, cr, _, _| {
                let (Some(m), Some(anim), Some(dial)) = (shown.get(), anim.upgrade(), dial.upgrade()) else {
                    return;
                };
                if anim.state() != adw::AnimationState::Playing {
                    return;
                }
                let Some(b) = dial.compute_bounds(area) else {
                    return;
                };
                let (cx, cy) = (
                    b.x() as f64 + b.width() as f64 / 2.0,
                    b.y() as f64 + b.height() as f64 / 2.0,
                );
                let rim = (b.width().min(b.height()) as f64 / 2.0 - 1.0).max(1.0);
                draw(cr, cx, cy, rim, m, anim.value());
            });
        }
        Ring { area, anim, shown }
    }

    /// Light the ring for `m`, starting over if it is still lit.
    pub fn play(&self, m: DialMove) {
        self.shown.set(Some(m));
        self.anim.reset();
        self.anim.play();
    }
}

/// The light at `t` on a dial of radius `rim` centred on (`cx`, `cy`).
fn draw(cr: &gtk::cairo::Context, cx: f64, cy: f64, rim: f64, m: DialMove, t: f64) {
    let accent = adw::StyleManager::default().accent_color_rgba();
    let (r, g, b) = (accent.red() as f64, accent.green() as f64, accent.blue() as f64);
    let a = glow_alpha(t);
    // A press also lights the face a little.
    if m == DialMove::Press {
        cr.arc(cx, cy, rim, 0.0, 2.0 * PI);
        cr.set_source_rgba(r, g, b, 0.22 * a);
        let _ = cr.fill();
    }
    // Soft glow, then the ring itself.
    for (width, alpha) in [(7.0, 0.25), (2.0, 1.0)] {
        cr.arc(cx, cy, rim, 0.0, 2.0 * PI);
        cr.set_line_width(width);
        cr.set_source_rgba(r, g, b, alpha * a);
        let _ = cr.stroke();
    }
    let Some((start, end)) = arc(m, t) else { return };
    // Not grown yet.
    if (end - start).abs() < 0.01 {
        return;
    }
    let out = rim + 5.0;
    cr.set_line_width(2.5);
    // Lighter than the ring, fading more slowly so the grown arc can be seen, and from full at
    // its start to nothing at its moving end (Cairo has no gradient along an arc: short pieces).
    let (r, g, b) = (0.6 + r * 0.4, 0.6 + g * 0.4, 0.6 + b * 0.4);
    let fade = ARC_ALPHA * (1.0 - t.clamp(0.0, 1.0));
    const PIECES: usize = 32;
    for i in 0..PIECES {
        let (a0, a1) = (i as f64 / PIECES as f64, (i + 1) as f64 / PIECES as f64);
        let (s0, s1) = (start + (end - start) * a0, start + (end - start) * a1);
        cr.set_source_rgba(r, g, b, fade * (1.0 - a0));
        if s1 > s0 {
            cr.arc(cx, cy, out, s0, s1);
        } else {
            cr.arc_negative(cx, cy, out, s0, s1);
        }
        let _ = cr.stroke();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glow_fades_out() {
        assert_eq!(glow_alpha(0.0), 1.0);
        assert_eq!(glow_alpha(1.0), 0.0);
        let (a, b, c) = (glow_alpha(0.25), glow_alpha(0.5), glow_alpha(0.75));
        assert!(1.0 > a && a > b && b > c && c > 0.0, "{a} {b} {c}");
    }

    #[test]
    fn right_sweeps_clockwise_left_anticlockwise() {
        for t in [0.5, 1.0] {
            let (s, e) = arc(DialMove::Right, t).expect("right arc");
            assert!(e - s > 0.0, "right at {t}");
            let (s, e) = arc(DialMove::Left, t).expect("left arc");
            assert!(e - s < 0.0, "left at {t}");
        }
        for m in [DialMove::Right, DialMove::Left] {
            let (s, e) = arc(m, 1.0).expect("arc");
            assert!(((e - s).abs() - PI / 2.0).abs() < 1e-9, "{m:?}");
        }
    }

    #[test]
    fn arcs_start_at_the_top() {
        for m in [DialMove::Right, DialMove::Left] {
            let (s, _) = arc(m, 0.5).expect("arc");
            assert!((s + PI / 2.0).abs() < 1e-9, "{m:?}");
        }
    }

    #[test]
    fn press_has_no_arc() {
        for t in [0.0, 0.5, 1.0] {
            assert_eq!(arc(DialMove::Press, t), None);
        }
    }
}
