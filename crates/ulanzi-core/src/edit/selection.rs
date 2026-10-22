/// A control that can be selected and edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    /// Screen key 0..=12.
    Key(u8),
    /// Small button: 0 left, 1 right.
    Small(u8),
    /// Dial 0..=2.
    Dial(u8),
    /// The wide screen.
    Screen,
}

/// What is selected on the canvas. Several keys can be selected together; any other slot is
/// selected alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    slots: Vec<Slot>,
    anchor: Option<Slot>,
}

impl Selection {
    /// Plain click: select only this slot.
    pub fn click(&mut self, slot: Slot) {
        self.slots = vec![slot];
        self.anchor = Some(slot);
    }

    /// Ctrl-click: add or remove a key; non-keys behave like a plain click.
    pub fn toggle(&mut self, slot: Slot) {
        if !matches!(slot, Slot::Key(_)) || self.slots.iter().any(|s| !matches!(s, Slot::Key(_))) {
            self.click(slot);
            return;
        }
        match self.slots.iter().position(|s| *s == slot) {
            Some(i) => {
                self.slots.remove(i);
            }
            None => self.slots.push(slot),
        }
        self.slots.sort();
        self.anchor = Some(slot);
    }

    /// Shift-click: every key from the anchor to this one, in index order.
    pub fn range_to(&mut self, slot: Slot) {
        match (self.anchor, slot) {
            (Some(Slot::Key(a)), Slot::Key(b)) => {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                self.slots = (lo..=hi).map(Slot::Key).collect();
            }
            _ => self.click(slot),
        }
    }

    pub fn select_all_keys(&mut self) {
        self.slots = (0..13).map(Slot::Key).collect();
        self.anchor = Some(Slot::Key(0));
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.anchor = None;
    }

    pub fn slots(&self) -> Vec<Slot> {
        self.slots.clone()
    }

    /// Selected screen keys, sorted.
    pub fn keys(&self) -> Vec<u8> {
        self.slots
            .iter()
            .filter_map(|s| if let Slot::Key(k) = s { Some(*k) } else { None })
            .collect()
    }

    pub fn contains(&self, slot: Slot) -> bool {
        self.slots.contains(&slot)
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
