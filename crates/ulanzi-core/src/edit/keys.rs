//! Keys and the key library: linking, pins and the library itself. Each operation is one undo
//! step and leaves a key from a newer version alone. The everyday key edits (look, actions,
//! copy/paste) live in the editor itself and reach a pinned or linked key the same way.

use uuid::Uuid;

use super::{Clip, ClipKey, Editor, KEYS, LIBRARY_KEY, Slot, key_spot};
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

/// What [`Editor::add_keys_to_library`] did: keys added as new library keys, keys linked to a
/// library key just like them, and keys that were in the library already.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddedKeys {
    pub new: usize,
    pub matched: usize,
    pub already: usize,
}

impl AddedKeys {
    /// What a toast says about it; `None` when one key was simply added.
    pub fn message(&self) -> Option<String> {
        let keys = |n: usize| {
            if n == 1 {
                "1 key".to_string()
            } else {
                format!("{n} keys")
            }
        };
        let was = |n: usize| if n == 1 { "was" } else { "were" };
        let done = self.new + self.matched;
        if done == 1 && self.matched == 0 && self.already == 0 {
            return None;
        }
        let mut parts = Vec::new();
        if done > 0 {
            parts.push(format!("Added {} to your library.", keys(done)));
        }
        if self.matched > 0 {
            let links = if self.matched == 1 { "links" } else { "link" };
            parts.push(format!(
                "{} matched a library key and {links} to it.",
                self.matched
            ));
        }
        if self.already > 0 {
            parts.push(format!("{} {} in it already.", self.already, was(self.already)));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
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
        self.add_keys_to_library(&[n]);
        self.key_info(n).library
    }

    /// [`Editor::add_key_to_library`] for each of `keys`, as one undo step. Keys already linked
    /// and empty keys are left alone; a key just like one in the library links to that one.
    pub fn add_keys_to_library(&mut self, keys: &[u8]) -> AddedKeys {
        let mut added = AddedKeys::default();
        let mut todo: Vec<u8> = Vec::new();
        for &n in keys {
            if !self.key_free(n) || todo.contains(&n) {
                continue;
            }
            let info = self.key_info(n);
            if info.library.is_some() {
                added.already += 1;
            } else if info.key.is_some() {
                todo.push(n);
            }
        }
        if todo.is_empty() {
            return added;
        }
        self.edit(|p, page| {
            for n in todo {
                let Some(key) = key_spot(p, page, n).map(|s| s.clone()) else {
                    continue;
                };
                let id = match p.key_library.iter().find(|l| l.key == key) {
                    Some(l) => {
                        added.matched += 1;
                        l.id
                    }
                    None => {
                        added.new += 1;
                        let id = Uuid::new_v4();
                        let name = key.suggested_name();
                        p.key_library.push(LibraryKey { id, name, key });
                        id
                    }
                };
                if let Some(spot) = key_spot(p, page, n) {
                    *spot = linked(id);
                }
            }
        });
        added
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

    /// Take library key `id` out of the library but keep every key that uses it, each as its own
    /// copy, a pin too.
    pub fn delete_library_key_keep_uses(&mut self, id: Uuid) {
        self.delete_library_keys_keep_uses(&[id]);
    }

    /// [`Editor::delete_library_key_keep_uses`] for each of `ids`, as one undo step.
    pub fn delete_library_keys_keep_uses(&mut self, ids: &[Uuid]) {
        let libs: Vec<(Uuid, Key)> = ids
            .iter()
            .filter_map(|id| self.profile.library_key(*id).map(|l| (*id, l.key.clone())))
            .collect();
        if libs.is_empty() {
            return;
        }
        self.edit(|p, _| {
            p.key_library.retain(|l| !ids.contains(&l.id));
            let keys = p
                .pinned
                .keys
                .values_mut()
                .chain(p.pages.iter_mut().flat_map(|pg| pg.keys.values_mut()));
            for k in keys {
                if let Some((_, lib)) = libs.iter().find(|(id, _)| k.library == Some(*id)) {
                    *k = lib.clone();
                }
            }
        });
    }

    /// Delete library key `id`; keys linked to it are removed, and a pin linked to it too, so each
    /// page shows its own key again.
    pub fn delete_library_key(&mut self, id: Uuid) {
        self.delete_library_keys(&[id]);
    }

    /// [`Editor::delete_library_key`] for each of `ids`, as one undo step: the keys linked to them
    /// go too.
    pub fn delete_library_keys(&mut self, ids: &[Uuid]) {
        let gone = |k: &Key| k.library.is_some_and(|id| ids.contains(&id));
        self.edit(|p, _| {
            p.key_library.retain(|l| !ids.contains(&l.id));
            p.pinned.keys.retain(|_, k| !gone(k));
            for pg in &mut p.pages {
                pg.keys.retain(|_, k| !gone(k));
            }
        });
    }

    /// Copy library keys `ids` (from the Library tray), in library order; on the deck they paste
    /// side by side, as links in this profile.
    pub fn copy_library_keys(&mut self, ids: &[Uuid]) {
        let items: Vec<ClipKey> = self
            .profile
            .key_library
            .iter()
            .filter(|l| ids.contains(&l.id))
            .zip(0..)
            .map(|(l, at)| ClipKey {
                at,
                raw: linked(l.id),
                key: l.key.clone(),
                name: Some(l.name.clone()),
            })
            .collect();
        if !items.is_empty() {
            self.clipboard = Some(Clip::Keys {
                items,
                profile: self.profile.id,
                library: true,
            });
        }
    }

    /// Paste the copied keys into the library as new library keys, after library key `after`
    /// (else at the end). A name the library has already gets a number. Returns the new keys.
    pub fn paste_into_key_library(&mut self, after: Option<Uuid>) -> Vec<Uuid> {
        let Some(Clip::Keys { items, .. }) = self.clipboard.clone() else {
            return Vec::new();
        };
        let at = after
            .and_then(|id| self.profile.key_library.iter().position(|l| l.id == id))
            .map_or(self.profile.key_library.len(), |i| i + 1);
        let mut ids = Vec::new();
        self.edit(|p, _| {
            for (i, item) in items.into_iter().enumerate() {
                let key = Key {
                    library: None,
                    ..item.key
                };
                let base = item.name.unwrap_or_else(|| key.suggested_name());
                let name = super::unique_name(&base, p.key_library.iter().map(|l| l.name.as_str()));
                let id = Uuid::new_v4();
                p.key_library.insert(at + i, LibraryKey { id, name, key });
                ids.push(id);
            }
        });
        ids
    }
}
