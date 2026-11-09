//! Dragging a dial left or right to swap it: which spot it lands on and how the others make room.
//! The row draws its dials shifted by eased offsets; nothing moves in the layout until the drop.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;

/// How far the pointer moves before a press on a dial becomes a drag.
const START_PX: f64 = 8.0;
/// How long the others take to slide aside, and the dragged dial to settle.
const SLIDE_MS: u32 = 200;

/// The dial that dial `from` lands on when dragged `dx` px; `pitch` is the distance between dial
/// centres.
pub fn target(from: u8, dx: f64, pitch: f64) -> u8 {
    if pitch <= 0.0 {
        return from;
    }
    (from as f64 + dx / pitch).round().clamp(0.0, 2.0) as u8
}

/// How far each dial sits from its place while `from` is held over `to`: the dial at `to` slides
/// into `from`'s place, the others stay. The dragged dial follows the pointer instead, so its
/// entry is 0.
pub fn offsets(from: u8, to: u8, pitch: f64) -> [f64; 3] {
    let mut o = [0.0; 3];
    if from != to
        && let Some(x) = o.get_mut(to as usize)
    {
        *x = (from as f64 - to as f64) * pitch;
    }
    o
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct DialRow {
        /// How far each dial is drawn from its place.
        pub offsets: Cell<[f64; 3]>,
        /// The dial being dragged, drawn over the others.
        pub lifted: Cell<Option<u8>>,
        /// The spot the dragged dial would land on, outlined.
        pub drop: Cell<Option<u8>>,
        /// A spot marked from outside (the Library tray's drop spot), outlined the same way.
        pub marked: Cell<Option<u8>>,
        /// The dial buttons, for where to draw that outline.
        pub dials: std::cell::RefCell<Vec<gtk::Button>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DialRow {
        const NAME: &'static str = "UlanziDialRow";
        type Type = super::DialRow;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BoxLayout>();
        }
    }

    impl ObjectImpl for DialRow {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for DialRow {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let row = self.obj();
            let offsets = self.offsets.get();
            let lifted = self.lifted.get().map(usize::from);
            let draw = |i: usize, c: &gtk::Widget| {
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(
                    offsets.get(i).copied().unwrap_or(0.0) as f32,
                    0.0,
                ));
                row.snapshot_child(c, snapshot);
                snapshot.restore();
            };
            for spot in [
                self.drop.get(),
                self.marked.get().filter(|m| self.drop.get() != Some(*m)),
            ] {
                if let Some(b) = spot
                    .and_then(|i| self.dials.borrow().get(i as usize).cloned())
                    .and_then(|b| b.compute_bounds(&*row))
                {
                    draw_drop_spot(snapshot, &b);
                }
            }
            let mut top = None;
            let mut child = row.first_child();
            let mut i = 0;
            while let Some(c) = child {
                if Some(i) == lifted {
                    top = Some((i, c.clone()));
                } else {
                    draw(i, &c);
                }
                child = c.next_sibling();
                i += 1;
            }
            if let Some((i, c)) = top {
                draw(i, &c);
            }
        }
    }
}

/// A dashed ring in the accent colour around the dial at `b` (where a dragged dial will land).
fn draw_drop_spot(snapshot: &gtk::Snapshot, b: &gtk::graphene::Rect) {
    let pad = 4.0;
    let area = gtk::graphene::Rect::new(
        b.x() - pad,
        b.y() - pad,
        b.width() + 2.0 * pad,
        b.height() + 2.0 * pad,
    );
    let cr = snapshot.append_cairo(&area);
    let c = adw::StyleManager::default().accent_color_rgba();
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 0.9);
    cr.set_line_width(2.0);
    cr.set_dash(&[6.0, 5.0], 0.0);
    let (cx, cy) = (
        (b.x() + b.width() / 2.0) as f64,
        (b.y() + b.height() / 2.0) as f64,
    );
    let r = (b.width().min(b.height()) as f64) / 2.0 + 2.0;
    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
}

glib::wrapper! {
    /// The three dials side by side, equal widths, each drawn at its drag offset.
    pub struct DialRow(ObjectSubclass<imp::DialRow>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DialRow {
    pub fn new(columns: &[gtk::Box], spacing: u32) -> DialRow {
        let row: DialRow = glib::Object::new();
        if let Some(l) = row.layout_manager().and_downcast::<gtk::BoxLayout>() {
            l.set_spacing(spacing);
            l.set_homogeneous(true);
        }
        for c in columns {
            c.set_parent(&row);
        }
        row
    }

    fn set_offsets(&self, o: [f64; 3]) {
        self.imp().offsets.set(o);
        self.queue_draw();
    }

    fn offsets(&self) -> [f64; 3] {
        self.imp().offsets.get()
    }

    fn set_drop(&self, n: Option<u8>) {
        self.imp().drop.set(n);
        self.queue_draw();
    }

    /// Outline dial `n` as where something will land, or none.
    pub fn set_marked(&self, n: Option<u8>) {
        self.imp().marked.set(n);
        self.queue_draw();
    }

    fn set_lifted(&self, n: Option<u8>) {
        self.imp().lifted.set(n);
        self.queue_draw();
    }

    /// Distance between neighbouring dial centres.
    fn pitch(&self) -> f64 {
        let centre = |c: Option<gtk::Widget>| {
            c.and_then(|c| c.compute_bounds(self))
                .map(|b| b.x() as f64 + b.width() as f64 / 2.0)
        };
        match (centre(self.first_child()), centre(self.last_child())) {
            (Some(a), Some(b)) => (b - a) / 2.0,
            _ => 0.0,
        }
    }
}

/// One drag of a dial.
#[derive(Default)]
struct Drag {
    from: Cell<Option<u8>>,
    /// Past [`START_PX`]: lifted and following the pointer.
    moving: Cell<bool>,
    pitch: Cell<f64>,
    to: Cell<u8>,
    /// Where the dragged dial is, following the pointer.
    x: Cell<f64>,
    /// Offsets the running animation goes from and to.
    start: Cell<[f64; 3]>,
    end: Cell<[f64; 3]>,
    /// The pointer is up: the animation settles the dial, then the swap commits.
    released: Cell<bool>,
}

fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// Whether a swap from `from` to `to` lands: another spot, and the dials still show what they
/// showed when the pointer went up (`at_release` is `place` then, `now` is `place` now).
pub fn lands<K: PartialEq>(from: u8, to: u8, at_release: Option<&K>, now: &K) -> bool {
    to != from && at_release == Some(now)
}

/// Let each of `dials` (the buttons in `row`'s columns) be dragged onto another spot. `place`
/// names what the dials show (a page); a move is dropped if it changes while the dial settles.
/// `allowed` says whether a dial may move from one spot to another (else it settles back home);
/// `commit` makes the move once the dial has settled.
pub fn attach<K: PartialEq + 'static>(
    row: &DialRow,
    dials: &[gtk::Button],
    place: impl Fn() -> K + 'static,
    allowed: impl Fn(u8, u8) -> bool + 'static,
    commit: impl Fn(u8, u8) + 'static,
) {
    let drag = Rc::new(Drag::default());
    let place = Rc::new(place);
    let released_on: Rc<RefCell<Option<K>>> = Rc::new(RefCell::new(None));
    let weak_row = row.downgrade();
    let moves = {
        let (drag, row) = (drag.clone(), weak_row.clone());
        adw::CallbackAnimationTarget::new(move |t| {
            let Some(row) = row.upgrade() else { return };
            let mut o = lerp(drag.start.get(), drag.end.get(), t);
            if let Some(f) = drag.from.get()
                && !drag.released.get()
            {
                o[f as usize] = drag.x.get();
            }
            row.set_offsets(o);
        })
    };
    let anim = adw::TimedAnimation::builder()
        .widget(row)
        .value_from(0.0)
        .value_to(1.0)
        .duration(SLIDE_MS)
        .easing(adw::Easing::EaseOutCubic)
        .target(&moves)
        .build();
    // Run the animation from where everything is now to `end`.
    let slide = {
        // Holds the animation: nothing else does.
        let (drag, row, anim) = (drag.clone(), weak_row.clone(), anim.clone());
        Rc::new(move |end: [f64; 3]| {
            let Some(row) = row.upgrade() else { return };
            drag.start.set(row.offsets());
            drag.end.set(end);
            anim.reset();
            anim.play();
        })
    };
    let dials: Rc<Vec<gtk::Button>> = Rc::new(dials.to_vec());
    *row.imp().dials.borrow_mut() = dials.to_vec();
    let allowed = Rc::new(allowed);
    {
        let (drag, row, dials) = (drag.clone(), weak_row.clone(), dials.clone());
        let (place, released_on) = (place.clone(), released_on.clone());
        anim.connect_done(move |_| {
            if !drag.released.get() {
                return;
            }
            let (Some(row), Some(from)) = (row.upgrade(), drag.from.take()) else {
                return;
            };
            drag.released.set(false);
            drag.moving.set(false);
            let to = drag.to.get();
            if lands(from, to, released_on.take().as_ref(), &place()) {
                commit(from, to);
            }
            // The dials now show their new contents in place, so nothing jumps.
            row.set_offsets([0.0; 3]);
            row.set_lifted(None);
            if let Some(d) = dials.get(from as usize) {
                d.remove_css_class("lifted");
            }
        });
    }
    for (n, dial) in dials.iter().enumerate() {
        let n = n as u8;
        let g = gtk::GestureDrag::new();
        // The left button only: a right-click opens the menu.
        g.set_button(gtk::gdk::BUTTON_PRIMARY);
        g.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let (drag, row) = (drag.clone(), weak_row.clone());
            g.connect_drag_begin(move |g, _, _| {
                // Still settling the last one.
                if drag.from.get().is_some() {
                    g.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                let Some(row) = row.upgrade() else { return };
                drag.from.set(Some(n));
                drag.to.set(n);
                drag.x.set(0.0);
                drag.moving.set(false);
                drag.pitch.set(row.pitch());
            });
        }
        {
            let (drag, row, dial, slide) = (drag.clone(), weak_row.clone(), dial.clone(), slide.clone());
            g.connect_drag_update(move |g, dx, dy| {
                let Some(row) = row.upgrade() else { return };
                if drag.from.get() != Some(n) || drag.released.get() {
                    return;
                }
                if !drag.moving.get() {
                    if dx.hypot(dy) < START_PX {
                        return;
                    }
                    // A drag now, not a click.
                    g.set_state(gtk::EventSequenceState::Claimed);
                    drag.moving.set(true);
                    dial.add_css_class("lifted");
                    row.set_lifted(Some(n));
                    row.set_drop(Some(n));
                }
                let pitch = drag.pitch.get();
                let x = dx.clamp(-(n as f64) * pitch, (2.0 - n as f64) * pitch);
                drag.x.set(x);
                let to = target(n, dx, pitch);
                if to != drag.to.get() {
                    drag.to.set(to);
                    row.set_drop(Some(to));
                    slide(offsets(n, to, pitch));
                } else {
                    let mut o = row.offsets();
                    o[n as usize] = x;
                    row.set_offsets(o);
                }
            });
        }
        // Pointer up, or the sequence went elsewhere: settle, and swap if over another spot.
        let finish = {
            let (drag, row, slide, allowed) =
                (drag.clone(), weak_row.clone(), slide.clone(), allowed.clone());
            let (place, released_on) = (place.clone(), released_on.clone());
            Rc::new(move |cancelled: bool| {
                if drag.from.get() != Some(n) || drag.released.get() {
                    return;
                }
                if !drag.moving.get() {
                    drag.from.set(None);
                    return;
                }
                let pitch = drag.pitch.get();
                let mut to = drag.to.get();
                // A spot that can't take it, or a cancelled drag: back home.
                if cancelled || !allowed(n, to) {
                    to = n;
                }
                drag.to.set(to);
                let mut end = offsets(n, to, pitch);
                end[n as usize] = (to as f64 - n as f64) * pitch;
                *released_on.borrow_mut() = Some(place());
                drag.released.set(true);
                if let Some(row) = row.upgrade() {
                    row.set_drop(None);
                }
                slide(end);
            })
        };
        {
            let finish = finish.clone();
            g.connect_drag_end(move |_, _, _| finish(false));
        }
        g.connect_cancel(move |_, _| finish(true));
        dial.add_controller(g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_rounds_to_the_nearest_dial() {
        let p = 100.0;
        assert_eq!(target(0, 0.6 * p, p), 1);
        assert_eq!(target(0, 1.6 * p, p), 2);
        assert_eq!(target(0, -3.0 * p, p), 0);
        assert_eq!(target(2, 9.0 * p, p), 2);
        assert_eq!(target(1, 0.4 * p, p), 1);
        assert_eq!(target(1, 0.0, 0.0), 1);
    }

    #[test]
    fn a_swap_lands_only_on_the_page_it_was_dropped_on() {
        assert!(lands(0, 2, Some(&1usize), &1));
        assert!(!lands(0, 0, Some(&1usize), &1));
        // The page changed during the settle.
        assert!(!lands(0, 2, Some(&1usize), &2));
        assert!(!lands(0, 2, None, &1usize));
    }

    #[test]
    fn offsets_make_room() {
        let p = 100.0;
        // A swap: only the dial being dropped on moves, into the dragged dial's place.
        assert_eq!(offsets(0, 2, p)[1..], [0.0, -2.0 * p]);
        assert_eq!(offsets(2, 0, p)[..2], [2.0 * p, 0.0]);
        assert_eq!(offsets(1, 2, p), [0.0, 0.0, -p]);
        assert_eq!(offsets(1, 1, p), [0.0; 3]);
    }
}
