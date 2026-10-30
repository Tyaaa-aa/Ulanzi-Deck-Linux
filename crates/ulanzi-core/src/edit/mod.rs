//! Editing a profile: selection, bulk edits, clipboard and undo. No GUI code, so it is tested
//! directly; the GTK app is a thin layer over [`Editor`].

mod selection;

pub use selection::{Selection, Slot};

use crate::Color;
use crate::catalog::default_look;
use crate::model::*;

/// Number of undo steps kept.
pub const HISTORY_LIMIT: usize = 200;
const KEYS: u8 = 13;

/// Small buttons and the wide screen belong to the profile, everything else to a page.
fn global(slot: Slot) -> bool {
    matches!(slot, Slot::Small(_) | Slot::Screen)
}

/// Whether `slot` holds something from a newer version, which edits must leave alone.
fn taken(p: &Profile, page: usize, slot: Slot) -> bool {
    let id = if global(slot) {
        None
    } else {
        Some(p.pages[page].id)
    };
    p.foreign_at(id, slot).is_some()
}

/// One appearance property, applied to every selected key.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Background(Color),
    Foreground(Color),
    IconScale(f32),
    LabelVisible(bool),
    LabelText(String),
    Source(ImageSource),
    Dynamic(bool),
    /// `None` follows Preferences.
    LabelPosition(Option<LabelPosition>),
}

/// The common value of a property across a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mixed<T> {
    /// Nothing selected.
    None,
    /// Every selected key has this value.
    One(T),
    /// The keys differ.
    Mixed,
}

/// Set one appearance field on a look (also used for live previews).
pub fn apply_field(look: &mut KeyLook, field: &Field) {
    match field {
        Field::Background(c) => look.background = *c,
        Field::Foreground(c) => look.foreground = *c,
        Field::IconScale(s) => look.icon_scale = s.clamp(0.1, 1.0),
        Field::LabelVisible(v) => look.label.visible = *v,
        Field::LabelText(t) => look.label.text = t.clone(),
        Field::Source(s) => look.source = s.clone(),
        Field::Dynamic(d) => look.dynamic = *d,
        Field::LabelPosition(p) => look.label.position = *p,
    }
}

pub fn shared<T: PartialEq + Clone>(mut values: impl Iterator<Item = T>) -> Mixed<T> {
    let Some(first) = values.next() else {
        return Mixed::None;
    };
    if values.all(|v| v == first) {
        Mixed::One(first)
    } else {
        Mixed::Mixed
    }
}

#[derive(Debug, Clone)]
enum Clip {
    /// Keys relative to the first copied index.
    Keys(Vec<(u8, Key)>),
    Style(KeyLook),
}

/// A profile being edited, with undo/redo.
#[derive(Debug, Clone)]
pub struct Editor {
    profile: Profile,
    page: usize,
    undo: Vec<Profile>,
    redo: Vec<Profile>,
    clipboard: Option<Clip>,
    /// Edits since the profile was last sent to the daemon.
    dirty: bool,
    /// Consecutive edits with the same key share one undo step (typing in a field).
    merge: Option<&'static str>,
    last_merge: Option<&'static str>,
}

impl Editor {
    pub fn new(profile: Profile) -> Editor {
        Editor {
            profile,
            page: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: None,
            dirty: false,
            merge: None,
            last_merge: None,
        }
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Replace the profile (e.g. reloaded from the daemon). History is kept so edits can be redone
    /// on top; the page index is clamped.
    pub fn replace_profile(&mut self, profile: Profile) {
        self.profile = profile;
        self.page = self.page.min(self.profile.pages.len().saturating_sub(1));
    }

    pub fn page(&self) -> usize {
        self.page
    }

    pub fn set_page(&mut self, page: usize) {
        if page < self.profile.pages.len() {
            self.page = page;
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Merge the following edits into one undo step while `key` stays the same (`None` ends it).
    pub fn set_merge(&mut self, key: Option<&'static str>) {
        self.merge = key;
        if key.is_none() {
            self.last_merge = None;
        }
    }

    /// Record the current profile as an undo step, then run `f` on it.
    fn edit(&mut self, f: impl FnOnce(&mut Profile, usize)) {
        let before = self.profile.clone();
        f(&mut self.profile, self.page);
        if self.profile == before {
            return;
        }
        self.dirty = true;
        self.redo.clear();
        let merging = self.merge.is_some() && self.merge == self.last_merge && !self.undo.is_empty();
        self.last_merge = self.merge;
        if merging {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn undo(&mut self) -> bool {
        self.last_merge = None;
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.profile, prev));
        self.page = self.page.min(self.profile.pages.len().saturating_sub(1));
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        self.last_merge = None;
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.profile, next));
        self.page = self.page.min(self.profile.pages.len().saturating_sub(1));
        self.dirty = true;
        true
    }

    fn keys(&self) -> &std::collections::BTreeMap<u8, Key> {
        &self.profile.pages[self.page].keys
    }

    pub fn key(&self, index: u8) -> Option<&Key> {
        self.keys().get(&index)
    }

    pub fn shared_foreground(&self, keys: &[u8]) -> Mixed<Color> {
        self.shared_look(keys, |l| l.foreground)
    }

    /// The common value of `get` over `keys` (missing keys count as the default look).
    pub fn shared_look<T: PartialEq + Clone>(&self, keys: &[u8], get: impl Fn(&KeyLook) -> T) -> Mixed<T> {
        let default = KeyLook::default();
        shared(
            keys.iter()
                .map(|k| get(self.key(*k).map(|k| &k.look).unwrap_or(&default))),
        )
    }

    /// Whether `slot` on the current page holds something from a newer version.
    pub fn is_foreign(&self, slot: Slot) -> bool {
        taken(&self.profile, self.page, slot)
    }

    /// Remove what a newer version left in `slot` (one undo step).
    pub fn clear_foreign(&mut self, slot: Slot) {
        self.edit(|p, page| {
            let id = if global(slot) {
                None
            } else {
                Some(p.pages[page].id)
            };
            p.foreign.retain(|f| !(f.page == id && f.slot == slot));
        });
    }

    /// Set one appearance field on every key in `keys` (one undo step).
    pub fn set_field(&mut self, keys: &[u8], field: Field) {
        self.edit(|p, page| {
            for &i in keys.iter().filter(|i| **i < KEYS) {
                if !taken(p, page, Slot::Key(i)) {
                    apply_field(&mut p.pages[page].keys.entry(i).or_default().look, &field);
                }
            }
        });
    }

    /// Assign `action` to a slot. An empty key also gets the action's default look.
    pub fn drop_action(&mut self, slot: Slot, action: Action) {
        if self.is_foreign(slot) {
            return;
        }
        self.edit(|p, page| match slot {
            Slot::Key(i) if i < KEYS => {
                let key = p.pages[page].keys.entry(i).or_default();
                crate::catalog::refresh_look(&mut key.look, key.tap.as_ref(), &action);
                key.tap = Some(action);
            }
            Slot::Key(_) => {}
            Slot::Small(0) => p.small_buttons.left = Some(action),
            Slot::Small(_) => p.small_buttons.right = Some(action),
            Slot::Dial(d) => {
                if let Some(dial) = p.pages[page].dials.get_mut(d as usize) {
                    dial.press = Some(action);
                }
            }
            Slot::Screen => match &mut p.screen {
                ScreenConfig::Custom { tap, .. } => *tap = Some(action),
                other => *other = ScreenConfig::custom(vec![Widget::now_playing()], Some(action), None),
            },
        });
    }

    /// Set a key's tap and hold actions (one undo step).
    pub fn set_key_actions(&mut self, index: u8, tap: Option<Action>, hold: Option<Action>) {
        if self.is_foreign(Slot::Key(index)) {
            return;
        }
        self.edit(|p, page| {
            if index < KEYS {
                let key = p.pages[page].keys.entry(index).or_default();
                if let Some(new) = &tap
                    && key.tap.as_ref() != Some(new)
                {
                    crate::catalog::refresh_look(&mut key.look, key.tap.as_ref(), new);
                }
                key.tap = tap;
                key.hold = hold;
            }
        });
    }

    /// The dials of the current page.
    pub fn dials(&self) -> &[Dial; 3] {
        &self.profile.pages[self.page].dials
    }

    /// Give every page the current page's setup of `dial`.
    pub fn dial_to_all_pages(&mut self, dial: u8) {
        self.edit(|p, page| {
            let Some(d) = p.pages[page].dials.get(dial as usize).cloned() else {
                return;
            };
            for pg in &mut p.pages {
                pg.dials[dial as usize] = d.clone();
            }
        });
    }

    pub fn set_dial(&mut self, dial: u8, f: impl FnOnce(&mut Dial)) {
        if self.is_foreign(Slot::Dial(dial)) {
            return;
        }
        self.edit(|p, page| {
            if let Some(d) = p.pages[page].dials.get_mut(dial as usize) {
                f(d);
            }
        });
    }

    pub fn set_small_button(&mut self, which: u8, action: Option<Action>) {
        if self.is_foreign(Slot::Small(which)) {
            return;
        }
        self.edit(|p, _| {
            if which == 0 {
                p.small_buttons.left = action;
            } else {
                p.small_buttons.right = action;
            }
        });
    }

    /// Rename the profile (surrounding spaces dropped; an empty name is ignored).
    pub fn rename_profile(&mut self, name: &str) {
        let name = name.trim();
        if !name.is_empty() {
            self.edit(|p, _| p.name = name.to_string());
        }
    }

    /// Every key back to the label settings from Preferences.
    pub fn clear_label_overrides(&mut self) {
        self.edit(|p, _| p.clear_label_overrides());
    }

    /// Which apps bring this profile to the front (auto profile switching).
    pub fn set_match(&mut self, rule: MatchRule) {
        self.edit(|p, _| p.r#match = rule);
    }

    pub fn set_model(&mut self, model: Model) {
        self.edit(|p, _| p.model = model);
    }

    pub fn set_screen(&mut self, screen: ScreenConfig) {
        if self.is_foreign(Slot::Screen) {
            return;
        }
        self.edit(|p, _| p.screen = screen);
    }

    pub fn swap(&mut self, a: u8, b: u8) {
        if a >= KEYS || b >= KEYS || a == b || self.is_foreign(Slot::Key(a)) || self.is_foreign(Slot::Key(b))
        {
            return;
        }
        self.edit(|p, page| {
            let keys = &mut p.pages[page].keys;
            let ka = keys.remove(&a);
            let kb = keys.remove(&b);
            if let Some(k) = ka {
                keys.insert(b, k);
            }
            if let Some(k) = kb {
                keys.insert(a, k);
            }
        });
    }

    pub fn copy_keys(&mut self, keys: &[u8]) {
        let Some(&first) = keys.iter().min() else {
            return;
        };
        let items = keys
            .iter()
            .filter_map(|i| self.key(*i).map(|k| (i - first, k.clone())))
            .collect();
        self.clipboard = Some(Clip::Keys(items));
    }

    /// Paste copied keys starting at `at`, keeping their spacing; keys past the last slot are dropped.
    pub fn paste_keys(&mut self, at: u8) {
        let Some(Clip::Keys(items)) = self.clipboard.clone() else {
            return;
        };
        self.edit(|p, page| {
            for (offset, key) in items {
                let i = at + offset;
                if i < KEYS && !taken(p, page, Slot::Key(i)) {
                    p.pages[page].keys.insert(i, key);
                }
            }
        });
    }

    pub fn copy_style(&mut self, from: u8) {
        if let Some(k) = self.key(from) {
            self.clipboard = Some(Clip::Style(k.look.clone()));
        }
    }

    pub fn paste_style(&mut self, keys: &[u8]) {
        let Some(Clip::Style(look)) = self.clipboard.clone() else {
            return;
        };
        self.edit(|p, page| {
            for &i in keys.iter().filter(|i| **i < KEYS) {
                if !taken(p, page, Slot::Key(i)) {
                    p.pages[page].keys.entry(i).or_default().look = look.clone();
                }
            }
        });
    }

    pub fn has_style_clip(&self) -> bool {
        matches!(self.clipboard, Some(Clip::Style(_)))
    }

    pub fn has_keys_clip(&self) -> bool {
        matches!(self.clipboard, Some(Clip::Keys(_)))
    }

    pub fn clear(&mut self, keys: &[u8]) {
        self.edit(|p, page| {
            for i in keys {
                p.pages[page].keys.remove(i);
                let id = Some(p.pages[page].id);
                p.foreign.retain(|f| !(f.page == id && f.slot == Slot::Key(*i)));
            }
        });
    }

    /// Give each key its action's default look again.
    pub fn reset_to_default_look(&mut self, keys: &[u8]) {
        self.edit(|p, page| {
            for i in keys {
                if let Some(k) = p.pages[page].keys.get_mut(i)
                    && let Some(a) = &k.tap
                {
                    k.look = default_look(a);
                }
            }
        });
    }

    /// Move keys to the first free slots of another page.
    pub fn move_to_page(&mut self, keys: &[u8], target: usize) {
        if target >= self.profile.pages.len() || target == self.page {
            return;
        }
        self.edit(|p, page| {
            let moving: Vec<Key> = keys.iter().filter_map(|i| p.pages[page].keys.remove(i)).collect();
            let mut free = (0..KEYS)
                .filter(|i| !p.pages[target].keys.contains_key(i) && !taken(p, target, Slot::Key(*i)))
                .collect::<Vec<_>>()
                .into_iter();
            for key in moving {
                match free.next() {
                    Some(i) => {
                        p.pages[target].keys.insert(i, key);
                    }
                    None => break,
                }
            }
        });
    }

    /// Place `groups` of keys (one group per device; a group stays on one page) in free slots,
    /// in order: on the current page unless `new_page`, then on new pages named `page_name` added
    /// after it. One undo step. Returns the first page that got keys.
    pub fn add_keys(&mut self, groups: Vec<Vec<Key>>, page_name: &str, new_page: bool) -> usize {
        let mut first = None;
        let start = self.page;
        let dials = self.profile.pages[self.page].dials.clone();
        self.edit(|p, page| {
            let mut at = page;
            let mut added = 0;
            let mut add_page = |p: &mut Profile, at: usize| {
                added += 1;
                let name = if added == 1 {
                    page_name.to_string()
                } else {
                    format!("{page_name} {added}")
                };
                let mut new = Page::new(&name);
                new.dials = dials.clone();
                p.pages.insert(at + 1, new);
                at + 1
            };
            if new_page {
                at = add_page(p, at);
            }
            for group in groups {
                let group: Vec<Key> = group.into_iter().take(KEYS as usize).collect();
                let free = |p: &Profile, at: usize| {
                    (0..KEYS)
                        .filter(|i| !p.pages[at].keys.contains_key(i) && !taken(p, at, Slot::Key(*i)))
                        .collect::<Vec<_>>()
                };
                if free(p, at).len() < group.len() {
                    at = add_page(p, at);
                }
                for (slot, key) in free(p, at).into_iter().zip(group) {
                    p.pages[at].keys.insert(slot, key);
                }
                first.get_or_insert(at);
            }
        });
        first.unwrap_or(start)
    }

    /// Free key slots on the current page.
    pub fn free_keys(&self) -> usize {
        let keys = &self.profile.pages[self.page].keys;
        (0..KEYS)
            .filter(|i| !keys.contains_key(i) && !self.is_foreign(Slot::Key(*i)))
            .count()
    }

    /// Add a page after the current one; returns its index.
    pub fn add_page(&mut self, name: &str) -> usize {
        let at = (self.page + 1).min(self.profile.pages.len());
        // A new page starts with the dials of the page it was added from.
        let dials = self.profile.pages[self.page].dials.clone();
        self.edit(|p, _| {
            let mut page = Page::new(name);
            page.dials = dials;
            p.pages.insert(at, page)
        });
        at
    }

    pub fn rename_page(&mut self, page: usize, name: &str) {
        self.edit(|p, _| {
            if let Some(pg) = p.pages.get_mut(page) {
                pg.name = name.to_string();
            }
        });
    }

    /// Remove a page. The last page cannot be removed.
    pub fn remove_page(&mut self, page: usize) -> bool {
        if self.profile.pages.len() <= 1 || page >= self.profile.pages.len() {
            return false;
        }
        self.edit(|p, _| {
            let gone = p.pages.remove(page).id;
            p.foreign.retain(|f| f.page != Some(gone));
        });
        self.page = self.page.min(self.profile.pages.len() - 1);
        true
    }

    pub fn move_page(&mut self, from: usize, to: usize) {
        let n = self.profile.pages.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let current = self.profile.pages[self.page].id;
        self.edit(|p, _| {
            let pg = p.pages.remove(from);
            p.pages.insert(to, pg);
        });
        // Stay on the page that was open.
        self.page = self
            .profile
            .pages
            .iter()
            .position(|p| p.id == current)
            .unwrap_or(0);
    }
}
