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

    /// Edit for This Page Only: a linked dial becomes its own copy of the library dial.
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
                *spot = info.dial;
            }
        });
    }

    /// Swap what shows at `a` and `b`: a pin moves with its dial, the page's own dials swap
    /// underneath.
    pub fn swap_dials(&mut self, a: u8, b: u8) {
        if a == b || !self.dial_free(a) || !self.dial_free(b) {
            return;
        }
        self.edit(|p, page| {
            p.pages[page].dials.swap(a as usize, b as usize);
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

    /// Show dial `n` as it is now on every page. Each page keeps its own dial underneath.
    pub fn pin_dial(&mut self, n: u8) {
        if !self.dial_free(n) || self.profile.pinned.dials.contains_key(&n) {
            return;
        }
        self.edit(|p, page| {
            let dial = p.pages[page].dials[n as usize].clone();
            p.pinned.dials.insert(n, dial);
        });
    }

    /// Remove the pin at `n`; each page shows its own dial again.
    pub fn unpin_dial(&mut self, n: u8) {
        if !self.dial_free(n) {
            return;
        }
        self.edit(|p, _| {
            p.pinned.dials.remove(&n);
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

    /// Remove the pin on `side`; each page shows its own small button again.
    pub fn unpin_small(&mut self, side: u8) {
        if side > 1 || self.is_foreign(Slot::Small(side)) {
            return;
        }
        self.edit(|p, _| {
            if side == 0 {
                p.small_buttons.left = None;
            } else {
                p.small_buttons.right = None;
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

    /// Delete library dial `id`; the dials linked to it, pins included, become blank.
    pub fn delete_library_dial(&mut self, id: Uuid) {
        self.edit(|p, _| {
            p.dial_library.retain(|l| l.id != id);
            let pages = p.pages.iter_mut().flat_map(|pg| pg.dials.iter_mut());
            for d in pages.chain(p.pinned.dials.values_mut()) {
                if d.library == Some(id) {
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
        self.clipboard = Some(Clip::Dial {
            dial: Dial {
                library: info.library,
                ..info.dial
            },
            profile: self.profile.id,
        });
    }

    /// Paste at `n` (over a pin when pinned). A link stays a link in the profile it was copied from
    /// while its library dial is there; anywhere else it pastes as an own copy.
    pub fn paste_dial(&mut self, n: u8) {
        let Some(Clip::Dial { dial, profile }) = self.clipboard.clone() else {
            return;
        };
        if !self.dial_free(n) {
            return;
        }
        let dial = match dial.library {
            Some(id) if profile == self.profile.id && self.profile.library_dial(id).is_some() => linked(id),
            _ => Dial {
                library: None,
                ..dial
            },
        };
        self.edit(|p, page| {
            if let Some(spot) = dial_spot(p, page, n) {
                *spot = dial;
            }
        });
    }

    pub fn has_dial_clip(&self) -> bool {
        matches!(self.clipboard, Some(Clip::Dial { .. }))
    }
}
