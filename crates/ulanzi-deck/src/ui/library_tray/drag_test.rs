//! A scripted drag through the Library tray, for checking how carrying tiles looks and behaves
//! without a hand on the mouse. Run in a sandbox with a pretend deck:
//!
//! ```text
//! scripts/sandbox.sh --demo --showcase -- --screenshot DIR/final.png --open library-tray \
//!     --drag-test DIR
//! ```
//!
//! It calls the same steps the tray's drag handlers call (pick up, hover, drop, the window's
//! leave as a drop ends, the drag's end), takes pictures of the window as it goes and writes
//! `DIR/log.txt`: what was aimed at, where the tray was in every picture, and PASS or FAIL checks.
//! GTK's own event delivery is not part of it; the handlers are thin over these steps.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use uuid::Uuid;

use super::{LAST, PER_PAGE, Tab, Tray};
use crate::ui::tile_drop::{Aim, Slot};

type Step = Box<dyn FnOnce(&Ctx)>;

struct Ctx {
    tray: Rc<Tray>,
    dir: PathBuf,
    snap: Rc<dyn Fn(&Path)>,
    log: RefCell<Vec<String>>,
    failed: RefCell<usize>,
    /// The tray's place when the test started; it must never move.
    frame_at_rest: RefCell<Option<(f32, f32, f32, f32)>>,
    order_before: RefCell<Vec<Uuid>>,
}

impl Ctx {
    fn say(&self, line: impl Into<String>) {
        let line = line.into();
        eprintln!("drag-test: {line}");
        self.log.borrow_mut().push(line);
    }

    fn check(&self, what: &str, ok: bool) {
        if !ok {
            *self.failed.borrow_mut() += 1;
        }
        self.say(format!("{} {what}", if ok { "PASS" } else { "FAIL" }));
    }

    fn slots(&self) -> Vec<Slot> {
        self.tray.slots()
    }

    fn slot(&self, k: usize) -> Slot {
        let page = self.tray.state.tray_page.get();
        self.slots()
            .into_iter()
            .find(|s| s.index == page * PER_PAGE + k)
            .expect("slot on the page")
    }

    /// The middle of shown tile `k`'s picture (k counted on the current page).
    fn picture(&self, k: usize) -> (f64, f64) {
        let s = self.slot(k);
        ((s.pic_left + s.pic_right) / 2.0, (s.pic_top + s.pic_bottom) / 2.0)
    }

    /// Half way between the pictures of tiles `a` and `a + 1`, at picture height.
    fn between(&self, a: usize) -> (f64, f64) {
        let (l, r) = (self.slot(a), self.slot(a + 1));
        ((l.pic_right + r.pic_left) / 2.0, (l.pic_top + l.pic_bottom) / 2.0)
    }

    fn ids(&self) -> Vec<Uuid> {
        self.tray.items.borrow().iter().map(|i| i.id).collect()
    }

    fn library(&self) -> Vec<Uuid> {
        self.tray
            .state
            .editor
            .borrow()
            .profile()
            .key_library
            .iter()
            .map(|l| l.id)
            .collect()
    }

    fn frame(&self) -> (f32, f32, f32, f32) {
        let b = self
            .tray
            .frame
            .compute_bounds(&self.tray.stage)
            .unwrap_or_else(|| gtk::graphene::Rect::new(0.0, 0.0, 0.0, 0.0));
        (b.x(), b.y(), b.width(), b.height())
    }

    /// Classes on the tiles that say what is moved aside or aimed at.
    fn marks(&self) -> String {
        let mut out = Vec::new();
        for (k, i) in self.tray.items.borrow().iter().enumerate() {
            for c in ["scooch-left", "scooch-right", "swap-target", "carried"] {
                if i.button.has_css_class(c) {
                    out.push(format!("{k}:{c}"));
                }
            }
        }
        out.join(" ")
    }

    /// A picture of the window, and where things were in it: the tiles sliding and hidden.
    fn shot(&self, name: &str) -> (usize, usize) {
        let frame = self.frame();
        let ghosts = {
            let mut n = 0;
            let mut c = self.tray.ghosts.first_child();
            while let Some(w) = c {
                if w != self.tray.gap_mark && w != self.tray.swap_mark {
                    n += 1;
                }
                c = w.next_sibling();
            }
            n
        };
        let hidden = self
            .tray
            .items
            .borrow()
            .iter()
            .filter(|i| i.column.opacity() < 1.0)
            .count();
        // How far each of the first tiles' contents is moved aside.
        let aside: Vec<String> = self
            .tray
            .items
            .borrow()
            .iter()
            .take(8)
            .map(|i| {
                let c = i.column.compute_bounds(&i.button).map_or(0.0, |c| c.x());
                format!("{c:.1}")
            })
            .collect();
        self.say(format!(
            "shot {name}: frame {frame:?} page {} aim {:?} ghosts {ghosts} hidden {hidden} aside [{}] marks [{}]",
            self.tray.state.tray_page.get(),
            self.tray.aim.get(),
            aside.join(" "),
            self.marks()
        ));
        // A shorter last page makes a shorter tray; the tray must not move on its own page.
        if let Some(rest) = *self.frame_at_rest.borrow()
            && self.tray.state.tray_page.get() == 0
        {
            self.check(
                &format!("{name}: the tray has not moved"),
                (frame.1 - rest.1).abs() < 0.5 && (frame.3 - rest.3).abs() < 0.5,
            );
        }
        (self.snap)(&self.dir.join(format!("{name}.png")));
        (ghosts, hidden)
    }

    /// The pointer at `(x, y)` in the page area, as the window and the page area see it.
    fn hover(&self, (x, y): (f64, f64)) -> Aim {
        let t = &self.tray;
        t.edge_hover(t.edge_step(&t.pages, x, y));
        t.hover_at(x, y)
    }

    /// Let go at `(x, y)` as GTK does: the drop, the window's leave, then the drag's end.
    fn drop(&self, (x, y): (f64, f64)) -> bool {
        let t = &self.tray;
        let ok = t.drop_at(x, y);
        t.edge_hover(None);
        t.carry_end();
        ok
    }
}

/// Run the script, then `done`.
pub fn run(dir: PathBuf, snap: Rc<dyn Fn(&Path)>, done: Box<dyn FnOnce()>) {
    let Some(tray) = LAST.with(|l| l.borrow().upgrade()) else {
        eprintln!("drag-test: no tray");
        done();
        return;
    };
    std::fs::create_dir_all(&dir).ok();
    let ctx = Rc::new(Ctx {
        tray,
        dir,
        snap,
        log: RefCell::default(),
        failed: RefCell::new(0),
        frame_at_rest: RefCell::default(),
        order_before: RefCell::default(),
    });
    let steps: Vec<(u64, Step)> = vec![
        // Enough tiles for three pages, on the Keys tab, page 1.
        (
            0,
            Box::new(|c: &Ctx| {
                let have = c.library().len();
                c.tray.state.edit(|e| {
                    for _ in have..(PER_PAGE * 2 + 4) {
                        e.new_library_key();
                    }
                    // Numbered by where they start, so the pictures show who went where.
                    let ids: Vec<Uuid> = e.profile().key_library.iter().map(|l| l.id).collect();
                    for (n, id) in ids.into_iter().enumerate() {
                        e.rename_library_key(id, &format!("Tile {n}"));
                    }
                });
                c.tray.set_tab(Tab::Keys);
                c.tray.go(0);
            }),
        ),
        (
            900,
            Box::new(|c: &Ctx| {
                *c.frame_at_rest.borrow_mut() = Some(c.frame());
                c.say(format!(
                    "{} tiles, slots on page: {}",
                    c.ids().len(),
                    c.slots().len()
                ));
                c.shot("01-rest");
                let id = c.ids()[1];
                c.tray.carry_begin(id, Tab::Keys);
            }),
        ),
        (
            250,
            Box::new(|c: &Ctx| {
                c.shot("02-carried");
            }),
        ),
        // Between tiles 3 and 4: they move apart, the mark shows.
        (
            0,
            Box::new(|c: &Ctx| {
                let aim = c.hover(c.between(3));
                c.check(
                    "between 3 and 4 aims at that gap",
                    aim == Aim::Gap { before: 4, row: 0 },
                );
            }),
        ),
        (
            40,
            Box::new(|c: &Ctx| {
                c.shot("03-gap-40ms");
            }),
        ),
        (
            260,
            Box::new(|c: &Ctx| {
                c.shot("03-gap-300ms");
                c.check(
                    "only tiles 3 and 4 move aside",
                    c.marks() == "1:carried 3:scooch-left 4:scooch-right",
                );
                // With them moved aside, the same place still means the same gap.
                let again = c.hover(c.between(3));
                c.check(
                    "moved aside, the same place keeps the gap",
                    again == Aim::Gap { before: 4, row: 0 },
                );
            }),
        ),
        // What a real drag adds: GTK's drop state on the tiles' area and hover on the tile under
        // the pointer. Neither may show while carrying.
        (
            0,
            Box::new(|c: &Ctx| {
                c.tray.pages.set_state_flags(gtk::StateFlags::DROP_ACTIVE, false);
                if let Some(i) = c.tray.items.borrow().get(4) {
                    i.button.set_state_flags(gtk::StateFlags::PRELIGHT, false);
                }
            }),
        ),
        (
            200,
            Box::new(|c: &Ctx| {
                c.shot("04-drop-states");
                c.tray.pages.unset_state_flags(gtk::StateFlags::DROP_ACTIVE);
                if let Some(i) = c.tray.items.borrow().get(4) {
                    i.button.unset_state_flags(gtk::StateFlags::PRELIGHT);
                }
            }),
        ),
        // Wobble across tile 4's picture edge: at most one change in and one out.
        (
            0,
            Box::new(|c: &Ctx| {
                let s = c.slot(4);
                let y = (s.pic_top + s.pic_bottom) / 2.0;
                let mut was = c.tray.aim.get();
                let mut changes = Vec::new();
                for d in [
                    -8.0, -5.0, -3.0, -1.0, 1.0, 3.0, 5.0, 7.0, 5.0, 2.0, -1.0, -4.0, -7.0, -10.0, -13.0,
                ] {
                    let now = c.hover((s.pic_left + d, y));
                    if now != was {
                        changes.push(format!("{d:+}:{now:?}"));
                    }
                    was = now;
                }
                c.say(format!("wobble at tile 4's edge: {}", changes.join(", ")));
                c.check("wobbling at an edge changes at most twice", changes.len() <= 2);
            }),
        ),
        // On tile 8's picture (second row): a swap, outlined.
        (
            0,
            Box::new(|c: &Ctx| {
                let aim = c.hover(c.picture(8));
                c.check("on tile 8's picture aims at a swap", aim == Aim::Swap(8));
            }),
        ),
        (
            250,
            Box::new(|c: &Ctx| {
                c.shot("05-swap-aim");
                c.check("only tile 8 is outlined", c.marks() == "1:carried 8:swap-target");
                c.check("the ring is showing", c.tray.swap_mark.has_css_class("shown"));
            }),
        ),
        // Drop between 3 and 4.
        (
            0,
            Box::new(|c: &Ctx| {
                c.hover(c.between(3));
            }),
        ),
        (
            250,
            Box::new(|c: &Ctx| {
                c.shot("06-before-insert");
                *c.order_before.borrow_mut() = c.library();
                let ok = c.drop(c.between(3));
                c.check("the drop between 3 and 4 was taken", ok);
                let (b, now) = (c.order_before.borrow().clone(), c.library());
                let mut want = b.clone();
                let moved = want.remove(1);
                want.insert(3, moved);
                c.check("tile 1 now sits between the old tiles 3 and 4", now == want);
            }),
        ),
        (
            20,
            Box::new(|c: &Ctx| {
                let (sliding, _) = c.shot("06-insert-20ms");
                // Tiles 2 and 3 move back one, tile 1 lands, tile 4 settles back from aside.
                c.check(
                    "only the tiles that changed place (and the one moved aside) slide",
                    sliding == 4,
                );
            }),
        ),
        (
            60,
            Box::new(|c: &Ctx| {
                c.shot("06-insert-80ms");
            }),
        ),
        (
            80,
            Box::new(|c: &Ctx| {
                c.shot("06-insert-160ms");
            }),
        ),
        (
            240,
            Box::new(|c: &Ctx| {
                let left = c.shot("06-insert-400ms");
                c.check("nothing left sliding or hidden", left == (0, 0));
            }),
        ),
        // Swap tile 0 with tile 5: only those two change.
        (
            0,
            Box::new(|c: &Ctx| {
                *c.order_before.borrow_mut() = c.library();
                let id = c.ids()[0];
                c.tray.carry_begin(id, Tab::Keys);
                c.hover(c.picture(5));
            }),
        ),
        (
            250,
            Box::new(|c: &Ctx| {
                c.shot("07-before-swap");
                let ok = c.drop(c.picture(5));
                c.check("the drop on tile 5 was taken", ok);
                let (b, now) = (c.order_before.borrow().clone(), c.library());
                let mut want = b.clone();
                want.swap(0, 5);
                c.check("tiles 0 and 5 traded places, the rest stayed", now == want);
            }),
        ),
        (
            40,
            Box::new(|c: &Ctx| {
                let (sliding, _) = c.shot("07-swap-40ms");
                c.check("only the two swapped tiles slide", sliding == 2);
            }),
        ),
        (
            100,
            Box::new(|c: &Ctx| {
                c.shot("07-swap-140ms");
            }),
        ),
        (
            260,
            Box::new(|c: &Ctx| {
                let left = c.shot("07-swap-400ms");
                c.check("nothing left sliding or hidden", left == (0, 0));
            }),
        ),
        // Held at the right side: one turn, a rest, then the next.
        (
            0,
            Box::new(|c: &Ctx| {
                let id = c.ids()[2];
                c.tray.carry_begin(id, Tab::Keys);
                // In the tray's right padding, past the tiles' area.
                let (_, y) = c.picture(0);
                let x = c.tray.pages.width() as f64 + 8.0;
                c.check(
                    "the tiles' area itself is not an edge",
                    c.tray.edge_zone(&c.tray.pages, x - 30.0, y).is_none(),
                );
                c.check(
                    "pushed past the tray's side is the edge",
                    c.tray.edge_zone(&c.tray.pages, x + 40.0, y) == Some(1),
                );
                c.hover((x, y));
                c.say("held at the right side");
            }),
        ),
        (
            100,
            Box::new(|c: &Ctx| {
                c.shot("08-edge-100ms");
                c.check("not turned yet at 0.1 s", c.tray.state.tray_page.get() == 0);
            }),
        ),
        (
            150,
            Box::new(|c: &Ctx| {
                c.shot("08-edge-250ms-flash");
                c.check("turned to page 2 by 0.25 s", c.tray.state.tray_page.get() == 1);
            }),
        ),
        (
            650,
            Box::new(|c: &Ctx| {
                c.check(
                    "resting at 0.9 s: still page 2",
                    c.tray.state.tray_page.get() == 1,
                );
            }),
        ),
        (
            450,
            Box::new(|c: &Ctx| {
                c.shot("08-edge-1350ms");
                c.check(
                    "turned again to page 3 by 1.35 s",
                    c.tray.state.tray_page.get() == 2,
                );
                c.hover(c.picture(0));
                c.tray.edge_hover(None);
                c.tray.carry_end();
            }),
        ),
        (
            400,
            Box::new(|c: &Ctx| {
                c.shot("09-end");
                let failed = *c.failed.borrow();
                c.say(format!("{} checks failed", failed));
            }),
        ),
    ];
    next(ctx, steps.into_iter().collect(), done);
}

fn next(ctx: Rc<Ctx>, mut steps: std::collections::VecDeque<(u64, Step)>, done: Box<dyn FnOnce()>) {
    let Some((wait, step)) = steps.pop_front() else {
        let log = ctx.log.borrow().join("\n");
        std::fs::write(ctx.dir.join("log.txt"), log + "\n").ok();
        done();
        return;
    };
    glib::timeout_add_local_once(std::time::Duration::from_millis(wait), move || {
        step(&ctx);
        next(ctx, steps, done);
    });
}
