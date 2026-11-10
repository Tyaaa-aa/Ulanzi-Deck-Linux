//! Keys and the key library: linking, pins and the library itself. Each operation is one undo
//! step and leaves a key from a newer version alone. The everyday key edits (look, actions,
//! copy/paste) live in the editor itself and reach a pinned or linked key the same way.

use uuid::Uuid;

use super::{Clip, Editor, KEYS, LIBRARY_KEY, Slot, key_spot};
use crate::model::*;

/// Remove the pin at `n` (see [`Editor::unpin_key`]).
fn unpin(p: &mut Profile, page: usize, n: u8) {
    let Some(pin) = p.pinned.keys.remove(&n) else {
        return;
    };
    let copied = p.pages.iter().any(|pg| pg.keys.get(&n) == Some(&pin));
    if !copied {
        p.pages[page].keys.entry(n).or_insert(pin);
    }
}

fn linked(id: Uuid) -> Key {
    Key {
        library: Some(id),
        ..Key::default()
    }
}

impl Editor {
    /// Key `n` as it works on the current page.
    pub fn key_info(&self, n: u8) -> KeyInfo {
        if n == LIBRARY_KEY {
            return KeyInfo {
                key: self.key(n).cloned(),
                pinned: false,
                library: self.library_key.filter(|_| self.key(n).is_some()),
            };
        }
        self.profile.key_info(self.page, n)
    }

    /// Edit library key `id` at [`LIBRARY_KEY`] (picked in the Library tray), or nothing there.
    pub fn edit_library_key(&mut self, id: Option<Uuid>) {
        self.library_key = id;
    }

    /// The library key edited at [`LIBRARY_KEY`], while it exists.
    pub fn edited_library_key(&self) -> Option<Uuid> {
        self.library_key
            .filter(|id| self.profile.library_key(*id).is_some())
    }

    /// Whether key `n` can be edited here.
    fn key_free(&self, n: u8) -> bool {
        n < KEYS && !self.is_foreign(Slot::Key(n))
    }

    /// Move what shows at `n` into the library, named after its label or action, and link it
    /// there. Returns the library key, also when it was linked already; `None` for no key.
    pub fn add_key_to_library(&mut self, n: u8) -> Option<Uuid> {
        if !self.key_free(n) {
            return None;
        }
        let info = self.key_info(n);
        if let Some(id) = info.library {
            return self.profile.library_key(id).map(|l| l.id);
        }
        info.key.as_ref()?;
        let id = Uuid::new_v4();
        self.edit(|p, page| {
            let Some(spot) = key_spot(p, page, n) else {
                return;
            };
            let key = std::mem::replace(spot, linked(id));
            let name = key.suggested_name();
            p.key_library.push(LibraryKey { id, name, key });
        });
        Some(id)
    }

    /// Link library key `id` at `n`. A pin there stays a pin.
    pub fn place_key(&mut self, n: u8, id: Uuid) {
        if !self.key_free(n) || self.profile.library_key(id).is_none() {
            return;
        }
        self.edit(|p, page| {
            if let Some(spot) = key_spot(p, page, n) {
                *spot = linked(id);
            }
        });
    }

    /// Make a Copy: a linked key becomes its own copy of the library key.
    pub fn own_copy_key(&mut self, n: u8) {
        if !self.key_free(n) {
            return;
        }
        let info = self.key_info(n);
        let (Some(_), Some(key)) = (info.library, info.key) else {
            return;
        };
        self.edit(|p, page| {
            if let Some(spot) = key_spot(p, page, n) {
                *spot = key;
            }
        });
    }

    /// Show key `n` as it is now on every page, folders included. Each page keeps its own key
    /// underneath. Nothing to pin on an empty key.
    pub fn pin_key(&mut self, n: u8) {
        self.pin_keys(&[n]);
    }

    /// [`Editor::pin_key`] for each of `keys`, as one undo step.
    pub fn pin_keys(&mut self, keys: &[u8]) {
        let pins: Vec<(u8, Key)> = keys
            .iter()
            .filter(|n| self.key_free(**n) && !self.profile.pinned.keys.contains_key(n))
            .filter_map(|n| Some((*n, self.profile.pages[self.page].keys.get(n)?.clone())))
            .collect();
        if pins.is_empty() {
            return;
        }
        self.edit(|p, _| p.pinned.keys.extend(pins));
    }

    /// [`Editor::unpin_key`] for each of `keys`, as one undo step.
    pub fn unpin_keys(&mut self, keys: &[u8]) {
        let keys: Vec<u8> = keys
            .iter()
            .copied()
            .filter(|n| self.key_free(*n) && self.profile.pinned.keys.contains_key(n))
            .collect();
        if keys.is_empty() {
            return;
        }
        self.edit(|p, page| {
            for n in keys {
                unpin(p, page, n);
            }
        });
    }

    /// Remove the pin at `n`; each page shows its own key again, an empty one too. Only a pin no
    /// page has a copy of (one edited while pinned) stays on this page when it has nothing of its
    /// own, so unpinning never loses it.
    pub fn unpin_key(&mut self, n: u8) {
        if !self.key_free(n) || !self.profile.pinned.keys.contains_key(&n) {
            return;
        }
        self.edit(|p, page| unpin(p, page, n));
    }

    /// A blank library key named "New Key".
    pub fn new_library_key(&mut self) -> Uuid {
        let id = Uuid::new_v4();
        self.edit(|p, _| {
            p.key_library.push(LibraryKey {
                id,
                name: "New Key".into(),
                key: Key::default(),
            });
        });
        id
    }

    /// Change library key `id` itself (picked in the Library tray).
    pub fn set_library_key(&mut self, id: Uuid, f: impl FnOnce(&mut Key)) {
        self.edit(|p, _| {
            if let Some(l) = p.key_library.iter_mut().find(|l| l.id == id) {
                f(&mut l.key);
            }
        });
    }

    /// Name library key `id`; it keeps a name, so an empty one is ignored.
    pub fn rename_library_key(&mut self, id: Uuid, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.edit(|p, _| {
            if let Some(l) = p.key_library.iter_mut().find(|l| l.id == id) {
                l.name = name.to_string();
            }
        });
    }

    /// Move library key `id` to position `to` in the library (the tray's order).
    pub fn move_library_key(&mut self, id: Uuid, to: usize) {
        let lib = &self.profile.key_library;
        let Some(from) = lib.iter().position(|l| l.id == id) else {
            return;
        };
        let to = to.min(lib.len() - 1);
        if from == to {
            return;
        }
        self.edit(|p, _| {
            let k = p.key_library.remove(from);
            p.key_library.insert(to, k);
        });
    }

    /// Delete library key `id`; keys linked to it are removed, and a pin linked to it too, so each
    /// page shows its own key again.
    /// Take library key `id` out of the library but keep every key that uses it, each as its own
    /// copy, a pin too.
    pub fn delete_library_key_keep_uses(&mut self, id: Uuid) {
        let Some(lib) = self.profile.library_key(id).map(|l| l.key.clone()) else {
            return;
        };
        self.edit(|p, _| {
            p.key_library.retain(|l| l.id != id);
            let keys = p
                .pinned
                .keys
                .values_mut()
                .chain(p.pages.iter_mut().flat_map(|pg| pg.keys.values_mut()));
            for k in keys.filter(|k| k.library == Some(id)) {
                *k = lib.clone();
            }
        });
    }

    pub fn delete_library_key(&mut self, id: Uuid) {
        self.edit(|p, _| {
            p.key_library.retain(|l| l.id != id);
            p.pinned.keys.retain(|_, k| k.library != Some(id));
            for pg in &mut p.pages {
                pg.keys.retain(|_, k| k.library != Some(id));
            }
        });
    }

    /// Copy library key `id` (from the Library tray); it pastes as a link in this profile.
    pub fn copy_library_key(&mut self, id: Uuid) {
        let Some(lib) = self.profile.library_key(id) else {
            return;
        };
        self.clipboard = Some(Clip::Keys {
            items: vec![(0, linked(id), lib.key.clone())],
            profile: self.profile.id,
        });
    }

    /// Paste over library key `id`: it takes the first copied key's look and actions and keeps its
    /// id and name, so its links show the pasted key.
    pub fn paste_library_key(&mut self, id: Uuid) {
        let Some(Clip::Keys { items, .. }) = self.clipboard.clone() else {
            return;
        };
        let Some((_, raw, key)) = items.into_iter().next() else {
            return;
        };
        if raw.library == Some(id) || self.profile.library_key(id).is_none() {
            return;
        }
        self.set_library_key(id, |k| *k = Key { library: None, ..key });
    }
}
