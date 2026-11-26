//! Where a library tile carried over the tray's other tiles would land. On a tile's picture it
//! swaps with that tile; between two pictures it goes into that gap. This is worked out from where
//! the tiles are laid out, not from where they are drawn, so tiles moving aside to open a gap
//! never change what is under the pointer.

/// One shown tile's place in the page area, as laid out (moving aside does not change it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// Its place among the shown tiles.
    pub index: usize,
    pub row: usize,
    /// The whole tile's top and bottom.
    pub top: f64,
    pub bottom: f64,
    /// Its picture.
    pub pic_left: f64,
    pub pic_right: f64,
    pub pic_top: f64,
    pub pic_bottom: f64,
}

/// What letting go of the carried tile would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Aim {
    /// Nothing changes (over the carried tile itself, or next to it, or off the tiles).
    #[default]
    Nothing,
    /// Trade places with shown tile `n`.
    Swap(usize),
    /// Go into the gap before shown tile `before` (one past the row's last tile for after it), in
    /// row `row`.
    Gap { before: usize, row: usize },
}

/// A picture is aimed at once the pointer is this far inside it, and stays aimed at until the
/// pointer is `HOLD` past its edge: resting on the edge never flickers between swap and gap.
pub const ENTER: f64 = 4.0;
pub const HOLD: f64 = 10.0;
/// How far above or below a row (between rows, or under the last one) still counts as that row.
pub const REACH: f64 = 24.0;

/// What a tile carried to `(x, y)` would do. `carried` is the carried tile's place among the
/// shown tiles (if shown) and `was` the aim just before, for the hold at picture edges.
pub fn aim_at(slots: &[Slot], x: f64, y: f64, carried: Option<usize>, was: Aim) -> Aim {
    // The row under the pointer, or the nearest one within reach.
    let Some(row) = nearest_row(slots, y) else {
        return Aim::Nothing;
    };
    let mut in_row: Vec<&Slot> = slots.iter().filter(|s| s.row == row).collect();
    in_row.sort_by(|a, b| a.pic_left.total_cmp(&b.pic_left));
    for s in &in_row {
        let edge = if was == Aim::Swap(s.index) { HOLD } else { -ENTER };
        if x >= s.pic_left - edge && x <= s.pic_right + edge {
            return match carried == Some(s.index) {
                true => Aim::Nothing,
                false => Aim::Swap(s.index),
            };
        }
    }
    let Some(last) = in_row.last() else {
        return Aim::Nothing;
    };
    // The gap before the first picture whose middle is right of the pointer (by middles, so the
    // few pixels inside a picture's edge, before it is aimed at, are still that side's gap).
    let before = in_row
        .iter()
        .find(|s| x < (s.pic_left + s.pic_right) / 2.0)
        .map_or(last.index + 1, |s| s.index);
    // Either side of the carried tile is where it already is.
    if carried.is_some_and(|c| before == c || before == c + 1) {
        return Aim::Nothing;
    }
    Aim::Gap { before, row }
}

fn nearest_row(slots: &[Slot], y: f64) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for s in slots {
        let off = if y < s.top {
            s.top - y
        } else if y > s.bottom {
            y - s.bottom
        } else {
            0.0
        };
        if off <= REACH && best.is_none_or(|(b, _)| off < b) {
            best = Some((off, s.row));
        }
    }
    best.map(|(_, row)| row)
}

/// The tiles either side of a gap that move apart to open it: `(left, right)`, each only when it
/// is in the gap's row.
pub fn gap_neighbours(slots: &[Slot], aim: Aim) -> (Option<usize>, Option<usize>) {
    let Aim::Gap { before, row } = aim else {
        return (None, None);
    };
    let in_row = |i: usize| slots.iter().any(|s| s.index == i && s.row == row);
    let left = before.checked_sub(1).filter(|i| in_row(*i));
    let right = Some(before).filter(|i| in_row(*i));
    (left, right)
}

/// Where the mark showing a gap goes: its middle `x` and its `top` and `bottom` (the pictures'
/// height), half way between the two pictures, or half a usual gap out at a row's end.
pub fn gap_mark(slots: &[Slot], aim: Aim) -> Option<(f64, f64, f64)> {
    let (left, right) = gap_neighbours(slots, aim);
    let slot = |i: Option<usize>| i.and_then(|i| slots.iter().find(|s| s.index == i));
    let (l, r) = (slot(left), slot(right));
    // The usual space between two pictures in a row.
    let space = slots
        .windows(2)
        .find(|w| w[0].row == w[1].row)
        .map_or(40.0, |w| w[1].pic_left - w[0].pic_right);
    let x = match (l, r) {
        (Some(l), Some(r)) => (l.pic_right + r.pic_left) / 2.0,
        (Some(l), None) => l.pic_right + space / 2.0,
        (None, Some(r)) => r.pic_left - space / 2.0,
        (None, None) => return None,
    };
    let s = l.or(r)?;
    Some((x, s.pic_top, s.pic_bottom))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two rows of six 100-wide tiles with 56-wide pictures in their middles; row 1 has three.
    fn slots() -> Vec<Slot> {
        (0..9)
            .map(|i| {
                let (row, col) = (i / 6, i % 6);
                let left = col as f64 * 100.0;
                let top = row as f64 * 108.0;
                Slot {
                    index: i,
                    row,
                    top,
                    bottom: top + 100.0,
                    pic_left: left + 22.0,
                    pic_right: left + 78.0,
                    pic_top: top + 6.0,
                    pic_bottom: top + 62.0,
                }
            })
            .collect()
    }

    #[test]
    fn on_a_picture_swaps_and_between_pictures_opens_a_gap() {
        let s = slots();
        // Tile 3's picture is 322..378.
        assert_eq!(aim_at(&s, 350.0, 30.0, Some(0), Aim::Nothing), Aim::Swap(3));
        // Between tiles 2 and 3 (pictures end at 278 and start at 322).
        assert_eq!(
            aim_at(&s, 300.0, 30.0, Some(0), Aim::Nothing),
            Aim::Gap { before: 3, row: 0 }
        );
        // Over the name under the picture: still by x.
        assert_eq!(aim_at(&s, 350.0, 90.0, Some(0), Aim::Nothing), Aim::Swap(3));
        assert_eq!(
            aim_at(&s, 300.0, 90.0, Some(0), Aim::Nothing),
            Aim::Gap { before: 3, row: 0 }
        );
    }

    #[test]
    fn resting_on_a_picture_edge_does_not_flicker() {
        let s = slots();
        let mut was = Aim::Nothing;
        let mut changes = 0;
        // Tile 3's picture starts at 322: wobble a few pixels either side of it.
        for x in [
            315.0, 318.0, 321.0, 322.0, 323.0, 321.0, 319.0, 323.0, 320.0, 318.0,
        ] {
            let now = aim_at(&s, x, 30.0, Some(0), was);
            if now != was {
                changes += 1;
            }
            was = now;
        }
        // Into the gap once and never onto the picture: it takes 4 px inside to aim at it.
        assert_eq!(changes, 1);
        assert_eq!(was, Aim::Gap { before: 3, row: 0 });
        // Once on the picture, it takes 10 px outside to leave it.
        let on = aim_at(&s, 330.0, 30.0, Some(0), was);
        assert_eq!(on, Aim::Swap(3));
        assert_eq!(aim_at(&s, 314.0, 30.0, Some(0), on), Aim::Swap(3));
        assert_eq!(
            aim_at(&s, 311.0, 30.0, Some(0), on),
            Aim::Gap { before: 3, row: 0 }
        );
    }

    #[test]
    fn the_carried_tile_and_its_own_sides_do_nothing() {
        let s = slots();
        assert_eq!(aim_at(&s, 250.0, 30.0, Some(2), Aim::Nothing), Aim::Nothing);
        // The gaps before and after tile 2.
        assert_eq!(aim_at(&s, 200.0, 30.0, Some(2), Aim::Nothing), Aim::Nothing);
        assert_eq!(aim_at(&s, 300.0, 30.0, Some(2), Aim::Nothing), Aim::Nothing);
        // A tile from another page has no sides here.
        assert_eq!(
            aim_at(&s, 300.0, 30.0, None, Aim::Nothing),
            Aim::Gap { before: 3, row: 0 }
        );
    }

    #[test]
    fn row_ends_and_rows() {
        let s = slots();
        // Before the first tile of row 1, after its last tile, and past the end of row 0.
        assert_eq!(
            aim_at(&s, 10.0, 150.0, Some(0), Aim::Nothing),
            Aim::Gap { before: 6, row: 1 }
        );
        assert_eq!(
            aim_at(&s, 290.0, 150.0, Some(0), Aim::Nothing),
            Aim::Gap { before: 9, row: 1 }
        );
        assert_eq!(
            aim_at(&s, 590.0, 30.0, Some(0), Aim::Nothing),
            Aim::Gap { before: 6, row: 0 }
        );
        // Between the rows (100..108) goes to the nearer; far below the last row is nothing.
        assert_eq!(aim_at(&s, 350.0, 103.0, Some(0), Aim::Nothing), Aim::Swap(3));
        assert_eq!(aim_at(&s, 50.0, 106.0, None, Aim::Nothing), Aim::Swap(6));
        assert_eq!(aim_at(&s, 50.0, 260.0, None, Aim::Nothing), Aim::Nothing);
    }

    #[test]
    fn gaps_move_their_neighbours_and_show_a_mark() {
        let s = slots();
        let mid = Aim::Gap { before: 3, row: 0 };
        assert_eq!(gap_neighbours(&s, mid), (Some(2), Some(3)));
        assert_eq!(gap_mark(&s, mid), Some((300.0, 6.0, 62.0)));
        // Row 1's start: only the tile after moves; the mark sits half a gap before it.
        let start = Aim::Gap { before: 6, row: 1 };
        assert_eq!(gap_neighbours(&s, start), (None, Some(6)));
        assert_eq!(gap_mark(&s, start), Some((22.0 - 22.0, 114.0, 170.0)));
        // Row 0's end: only the tile before moves, though tile 6 comes next.
        let end = Aim::Gap { before: 6, row: 0 };
        assert_eq!(gap_neighbours(&s, end), (Some(5), None));
        assert_eq!(gap_mark(&s, end), Some((578.0 + 22.0, 6.0, 62.0)));
        assert_eq!(gap_neighbours(&s, Aim::Swap(1)), (None, None));
        assert_eq!(gap_mark(&s, Aim::Nothing), None);
    }
}
