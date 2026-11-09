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

    /// Remove the pin at `n`; each page shows its own dial again. This page keeps the pinned dial
    /// when it has none of its own, so unpinning never empties the dial you are looking at.
    pub fn unpin_dial(&mut self, n: u8) {
        if !self.dial_free(n) || !self.profile.pinned.dials.contains_key(&n) {
            return;
        }
        self.edit(|p, page| {
            let Some(pin) = p.pinned.dials.remove(&n) else {
                return;
            };
            let own = &mut p.pages[page].dials[n as usize];
            if own.is_empty() {
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

    /// Remove the pin on `side`; each page shows its own small button again. This page keeps the
    /// pinned button when it has none of its own.
    pub fn unpin_small(&mut self, side: u8) {
        if side > 1 || !self.profile.small_pinned(side) || self.is_foreign(Slot::Small(side)) {
            return;
        }
        self.edit(|p, page| {
            let (pin, own) = if side == 0 {
                (p.small_buttons.left.take(), &mut p.pages[page].small_buttons.left)
            } else {
                (
                    p.small_buttons.right.take(),
                    &mut p.pages[page].small_buttons.right,
                )
            };
            if own.is_none() {
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

    /// Delete library dial `id`; the page dials linked to it become blank, and a pin linked to it
    /// is removed so each page shows its own dial again.
    pub fn delete_library_dial(&mut self, id: Uuid) {
        self.edit(|p, _| {
            p.dial_library.retain(|l| l.id != id);
            p.pinned.dials.retain(|_, d| d.library != Some(id));
            for d in p.pages.iter_mut().flat_map(|pg| pg.dials.iter_mut()) {
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

    /// Copy library dial `id` (from the Library tray); it pastes as a link in this profile.
    pub fn copy_library_dial(&mut self, id: Uuid) {
        let Some(lib) = self.profile.library_dial(id) else {
            return;
        };
        self.clipboard = Some(Clip::Dial {
            dial: Dial {
                library: Some(id),
                ..lib.dial.clone()
            },
            profile: self.profile.id,
        });
    }

    /// Paste over library dial `id`: it takes the copied dial's contents and keeps its id, so its
    /// links show the pasted dial. A nameless dial keeps the old name.
    pub fn paste_library_dial(&mut self, id: Uuid) {
        let Some(Clip::Dial { dial, .. }) = self.clipboard.clone() else {
            return;
        };
        let Some(old) = self.profile.library_dial(id) else {
            return;
        };
        if dial.library == Some(id) {
            return;
        }
        let name = match dial.name.trim() {
            "" => old.dial.name.clone(),
            n => n.to_string(),
        };
        self.set_library_dial(id, |d| {
            *d = Dial {
                library: None,
                name,
                ..dial
            }
        });
    }

    pub fn has_dial_clip(&self) -> bool {
        matches!(self.clipboard, Some(Clip::Dial { .. }))
    }
}
