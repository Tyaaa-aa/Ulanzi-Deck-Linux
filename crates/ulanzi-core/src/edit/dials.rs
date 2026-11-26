//! Dials: the library, pins, swapping, names and copy/paste. Each operation is one undo step and
//! leaves a dial from a newer version alone.

use uuid::Uuid;

use super::{Clip, DIALS, Editor, Slot, dial_mut, dial_spot};
use crate::model::*;

fn linked(id: Uuid) -> Dial {
    Dial {
        library: Some(id),
        ..Dial::default()
    }
}

impl Editor {
    /// Dial `n` as it works on the current page.
    pub fn dial_info(&self, n: u8) -> DialInfo {
        self.profile.dial_info(self.page, n)
    }

    /// Whether dial `n` can be edited here.
    fn dial_free(&self, n: u8) -> bool {
        n < DIALS && !self.is_foreign(Slot::Dial(n))
    }

    /// Move what shows at `n` into the library and link it there. Returns the library dial, also
    /// when it was linked already.
    pub fn add_dial_to_library(&mut self, n: u8) -> Option<Uuid> {
        if !self.dial_free(n) {
            return None;
        }
        let info = self.dial_info(n);
        if let Some(id) = info.library {
            return self.profile.library_dial(id).map(|l| l.id);
        }
        let id = Uuid::new_v4();
        self.edit(|p, page| {
            let Some(spot) = dial_spot(p, page, n) else {
                return;
            };
            let mut dial = std::mem::replace(spot, linked(id));
            if dial.name.trim().is_empty() {
                dial.name = format!("Dial {}", n + 1);
            }
            p.dial_library.push(LibraryDial { id, dial });
        });
        Some(id)
    }

    /// Link library dial `id` at `n`. A pin there stays a pin.
    pub fn place_dial(&mut self, n: u8, id: Uuid) {
        if !self.dial_free(n) || self.profile.library_dial(id).is_none() {
            return;
        }
        self.edit(|p, page| {
            if let Some(spot) = dial_spot(p, page, n) {
                *spot = linked(id);
            }
        });
    }

    /// Make a Copy: a linked dial becomes its own copy of the library dial, named "<name> (copy)".
    pub fn own_copy_dial(&mut self, n: u8) {
        if !self.dial_free(n) {
            return;
        }
        let info = self.dial_info(n);
        if info.library.is_none() {
            return;
        }
        self.edit(|p, page| {
            if let Some(spot) = dial_spot(p, page, n) {
                let name = match info.dial.name.trim() {
                    "" => format!("Dial {} (copy)", n + 1),
                    name => format!("{name} (copy)"),
                };
                *spot = Dial { name, ..info.dial };
            }
        });
    }

    /// Swap what shows at `a` and `b`: a pin moves with its dial, the page's own dials swap
    /// underneath. With a pin at either, every page swaps, so no page shows a different dial.
    pub fn swap_dials(&mut self, a: u8, b: u8) {
        if a == b || !self.dial_free(a) || !self.dial_free(b) {
            return;
        }
        self.edit(|p, page| {
            let pins = &mut p.pinned.dials;
            if pins.contains_key(&a) || pins.contains_key(&b) {
                for pg in &mut p.pages {
                    pg.dials.swap(a as usize, b as usize);
                }
                // A dial from a newer version on another page moves with its page's dials.
                for f in p.foreign.iter_mut().filter(|f| f.page.is_some()) {
                    if f.slot == Slot::Dial(a) {
                        f.slot = Slot::Dial(b);
                    } else if f.slot == Slot::Dial(b) {
                        f.slot = Slot::Dial(a);
                    }
                }
            } else {
                p.pages[page].dials.swap(a as usize, b as usize);
            }
            let pins = &mut p.pinned.dials;
            let (pa, pb) = (pins.remove(&a), pins.remove(&b));
            if let Some(d) = pa {
                pins.insert(b, d);
            }
            if let Some(d) = pb {
                pins.insert(a, d);
            }
        });
    }

    /// Show dial `n` as it is now on every page. Each page keeps its own dial underneath. Nothing
    /// to pin on an empty dial.
    pub fn pin_dial(&mut self, n: u8) {
        if !self.dial_free(n) || self.profile.pinned.dials.contains_key(&n) {
            return;
        }
        if self.profile.pages[self.page].dials[n as usize].is_empty() {
            return;
        }
        self.edit(|p, page| {
            let dial = p.pages[page].dials[n as usize].clone();
            p.pinned.dials.insert(n, dial);
        });
    }

    /// Remove the pin at `n`; each page shows its own dial again, an empty one too. Only a pin no
    /// page has a copy of (an old profile-wide dial, or one edited while pinned) stays on this page
    /// when it has nothing of its own, so unpinning never loses it.
    pub fn unpin_dial(&mut self, n: u8) {
        if !self.dial_free(n) || !self.profile.pinned.dials.contains_key(&n) {
            return;
        }
        self.edit(|p, page| {
            let Some(pin) = p.pinned.dials.remove(&n) else {
                return;
            };
            let copied = p.pages.iter().any(|pg| pg.dials[n as usize] == pin);
            let own = &mut p.pages[page].dials[n as usize];
            if own.is_empty() && !copied {
                *own = pin;
            }
        });
    }

    /// Show the small button on `side` as it is now on every page. Each page keeps its own.
    pub fn pin_small(&mut self, side: u8) {
        if side > 1 || self.profile.small_pinned(side) || self.is_foreign(Slot::Small(side)) {
            return;
        }
        let Some(action) = self.profile.small_at(self.page, side) else {
            return;
        };
        self.edit(|p, _| {
            if side == 0 {
                p.small_buttons.left = Some(action);
            } else {
                p.small_buttons.right = Some(action);
            }
        });
    }

    /// Remove the pin on `side`; each page shows its own small button again. As for dials, only a
    /// pin no page has a copy of stays on this page.
    pub fn unpin_small(&mut self, side: u8) {
        if side > 1 || !self.profile.small_pinned(side) || self.is_foreign(Slot::Small(side)) {
            return;
        }
        self.edit(|p, page| {
            fn pick(b: &mut SmallButtons, side: u8) -> &mut Option<Action> {
                if side == 0 { &mut b.left } else { &mut b.right }
            }
            let pin = pick(&mut p.small_buttons, side).take();
            let copied = p
                .pages
                .iter_mut()
                .any(|pg| *pick(&mut pg.small_buttons, side) == pin);
            let own = pick(&mut p.pages[page].small_buttons, side);
            if own.is_none() && !copied {
                *own = pin;
            }
        });
    }

    /// Name dial `n` (a linked dial names the library dial, which keeps a name).
    pub fn rename_dial(&mut self, n: u8, name: &str) {
        let name = name.trim();
        if !self.dial_free(n) || (name.is_empty() && self.dial_info(n).library.is_some()) {
            return;
        }
        self.edit(|p, page| {
            if let Some(d) = dial_mut(p, page, n) {
                d.name = name.to_string();
            }
        });
    }

    /// A blank library dial named "New Dial".
    pub fn new_library_dial(&mut self) -> Uuid {
        let id = Uuid::new_v4();
        self.edit(|p, _| {
            p.dial_library.push(LibraryDial {
                id,
                dial: Dial {
                    name: "New Dial".into(),
                    ..Dial::default()
                },
            });
        });
        id
    }

    /// Change library dial `id` itself (picked in the Library tray).
    pub fn set_library_dial(&mut self, id: Uuid, f: impl FnOnce(&mut Dial)) {
        self.edit(|p, _| {
            if let Some(l) = p.dial_library.iter_mut().find(|l| l.id == id) {
                f(&mut l.dial);
            }
        });
    }

    /// Name library dial `id`; it keeps a name, so an empty one is ignored.
    pub fn rename_library_dial(&mut self, id: Uuid, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.set_library_dial(id, |d| d.name = name.to_string());
    }

    /// Move library dial `id` to position `to` in the library (the tray's order); the others
    /// shift to make room.
    pub fn move_library_dial(&mut self, id: Uuid, to: usize) {
        let lib = &self.profile.dial_library;
        let Some(from) = lib.iter().position(|l| l.id == id) else {
            return;
        };
        let to = to.min(lib.len() - 1);
        if from == to {
            return;
        }
        self.edit(|p, _| {
            let d = p.dial_library.remove(from);
            p.dial_library.insert(to, d);
        });
    }

    /// Library dials `a` and `b` trade places; the ones between stay where they are.
    pub fn swap_library_dial(&mut self, a: Uuid, b: Uuid) {
        let lib = &self.profile.dial_library;
        let (Some(i), Some(j)) = (
            lib.iter().position(|l| l.id == a),
            lib.iter().position(|l| l.id == b),
        ) else {
            return;
        };
        if i == j {
            return;
        }
        self.edit(|p, _| p.dial_library.swap(i, j));
    }

    /// Take library dial `id` out of the library but keep every dial that uses it, each as its own
    /// copy, a pin too.
    pub fn delete_library_dial_keep_uses(&mut self, id: Uuid) {
        self.delete_library_dials_keep_uses(&[id]);
    }

    /// [`Editor::delete_library_dial_keep_uses`] for each of `ids`, as one undo step.
    pub fn delete_library_dials_keep_uses(&mut self, ids: &[Uuid]) {
        let libs: Vec<(Uuid, Dial)> = ids
            .iter()
            .filter_map(|id| self.profile.library_dial(*id).map(|l| (*id, l.dial.clone())))
            .collect();
        if libs.is_empty() {
            return;
        }
        self.edit(|p, _| {
            p.dial_library.retain(|l| !ids.contains(&l.id));
            let dials = p
                .pinned
                .dials
                .values_mut()
                .chain(p.pages.iter_mut().flat_map(|pg| pg.dials.iter_mut()));
            for d in dials {
                if let Some((_, lib)) = libs.iter().find(|(id, _)| d.library == Some(*id)) {
                    *d = lib.clone();
                }
            }
        });
    }

    /// Delete library dial `id`; the page dials linked to it become blank, and a pin linked to it
    /// is removed so each page shows its own dial again.
    pub fn delete_library_dial(&mut self, id: Uuid) {
        self.delete_library_dials(&[id]);
    }

    /// [`Editor::delete_library_dial`] for each of `ids`, as one undo step.
    pub fn delete_library_dials(&mut self, ids: &[Uuid]) {
        let gone = |d: &Dial| d.library.is_some_and(|id| ids.contains(&id));
        self.edit(|p, _| {
            p.dial_library.retain(|l| !ids.contains(&l.id));
            p.pinned.dials.retain(|_, d| !gone(d));
            for d in p.pages.iter_mut().flat_map(|pg| pg.dials.iter_mut()) {
                if gone(d) {
                    *d = Dial::default();
                }
            }
        });
    }

    /// Remove from This Page: clear the page's own dial at `n`. A pin there is left alone.
    pub fn remove_dial(&mut self, n: u8) {
        if !self.dial_free(n) {
            return;
        }
        self.edit(|p, page| p.pages[page].dials[n as usize] = Dial::default());
    }

    pub fn copy_dial(&mut self, n: u8) {
        if !self.dial_free(n) {
            return;
        }
        let info = self.dial_info(n);
        self.clipboard = Some(Clip::Dials {
            dials: vec![Dial {
                library: info.library,
                ..info.dial
            }],
            profile: self.profile.id,
            library: false,
        });
    }

    /// Paste at `n` and the dials after it (over a pin when pinned); dials past the last are
    /// dropped. A link stays a link in the profile it was copied from while its library dial is
    /// there; anywhere else it pastes as an own copy.
    pub fn paste_dial(&mut self, n: u8) {
        let Some(Clip::Dials { dials, profile, .. }) = self.clipboard.clone() else {
            return;
        };
        let same = profile == self.profile.id;
        let dials: Vec<(u8, Dial)> = dials
            .into_iter()
            .zip(n..DIALS)
            .filter(|(_, at)| self.dial_free(*at))
            .map(|(dial, at)| {
                let dial = match dial.library {
                    Some(id) if same && self.profile.library_dial(id).is_some() => linked(id),
                    _ => Dial {
                        library: None,
                        ..dial
                    },
                };
                (at, dial)
            })
            .collect();
        if dials.is_empty() {
            return;
        }
        self.edit(|p, page| {
            for (at, dial) in dials {
                if let Some(spot) = dial_spot(p, page, at) {
                    *spot = dial;
                }
            }
        });
    }

    /// Copy library dials `ids` (from the Library tray), in library order; they paste as links in
    /// this profile.
    pub fn copy_library_dials(&mut self, ids: &[Uuid]) {
        let dials: Vec<Dial> = self
            .profile
            .dial_library
            .iter()
            .filter(|l| ids.contains(&l.id))
            .map(|l| Dial {
                library: Some(l.id),
                ..l.dial.clone()
            })
            .collect();
        if !dials.is_empty() {
            self.clipboard = Some(Clip::Dials {
                dials,
                profile: self.profile.id,
                library: true,
            });
        }
    }

    /// Paste the copied dials into the library as new library dials, after library dial `after`
    /// (else at the end). A name the library has already gets a number. Returns the new dials.
    pub fn paste_into_dial_library(&mut self, after: Option<Uuid>) -> Vec<Uuid> {
        let Some(Clip::Dials { dials, .. }) = self.clipboard.clone() else {
            return Vec::new();
        };
        let at = after
            .and_then(|id| self.profile.dial_library.iter().position(|l| l.id == id))
            .map_or(self.profile.dial_library.len(), |i| i + 1);
        let mut ids = Vec::new();
        self.edit(|p, _| {
            for (i, dial) in dials.into_iter().enumerate() {
                let base = match dial.name.trim() {
                    "" => "Dial".to_string(),
                    n => n.to_string(),
                };
                let name = super::unique_name(&base, p.dial_library.iter().map(|l| l.dial.name.as_str()));
                let id = Uuid::new_v4();
                p.dial_library.insert(
                    at + i,
                    LibraryDial {
                        id,
                        dial: Dial {
                            library: None,
                            name,
                            ..dial
                        },
                    },
                );
                ids.push(id);
            }
        });
        ids
    }

    pub fn has_dial_clip(&self) -> bool {
        matches!(self.clipboard, Some(Clip::Dials { .. }))
    }
}
