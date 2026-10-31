//! A row that packs its children and wraps when they no longer fit, or with [`WrapBox::grid`] a
//! grid of equal cells that fills the width.

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;

const SPACING: i32 = 4;

/// Which children share a row: each row as a start index and an end index. `width` is the space
/// available; a child wider than that gets a row to itself.
fn rows(widths: &[i32], width: i32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (i, w) in widths.iter().enumerate() {
        if i > start && used + SPACING + w > width {
            out.push((start, i));
            (start, used) = (i, *w);
        } else {
            used += if i > start { SPACING + w } else { *w };
        }
    }
    if start < widths.len() {
        out.push((start, widths.len()));
    }
    out
}

/// How many cells of at least `cell` fit in `width`, and how wide each one comes out.
fn grid_cells(width: i32, cell: i32) -> (i32, i32) {
    let columns = ((width + SPACING) / (cell + SPACING)).max(1);
    (columns, (width - SPACING * (columns - 1)) / columns)
}

mod imp {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    pub struct WrapBox {
        /// The smallest width of a grid cell; 0 packs children instead.
        pub cell: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WrapBox {
        const NAME: &'static str = "UlanziWrapBox";
        type Type = super::WrapBox;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for WrapBox {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WrapBox {
        fn children(&self) -> Vec<gtk::Widget> {
            let mut out = Vec::new();
            let mut child = self.obj().first_child();
            while let Some(c) = child {
                child = c.next_sibling();
                if c.should_layout() {
                    out.push(c);
                }
            }
            out
        }

        /// How wide each child is laid out in `width`.
        fn widths(&self, children: &[gtk::Widget], width: i32) -> Vec<i32> {
            match self.cell.get() {
                0 => children.iter().map(natural_width).collect(),
                cell => vec![grid_cells(width, cell).1; children.len()],
            }
        }
    }

    fn natural_width(w: &gtk::Widget) -> i32 {
        w.measure(gtk::Orientation::Horizontal, -1).1
    }

    fn natural_height(w: &gtk::Widget) -> i32 {
        w.measure(gtk::Orientation::Vertical, -1).1
    }

    impl WidgetImpl for WrapBox {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let children = self.children();
            let cell = self.cell.get();
            if cell > 0 && orientation == gtk::Orientation::Horizontal {
                let gaps = SPACING * (children.len() as i32 - 1).max(0);
                return (cell, cell * children.len() as i32 + gaps, -1, -1);
            }
            if orientation == gtk::Orientation::Horizontal {
                let widest = children.iter().map(natural_width).max().unwrap_or(0);
                let total: i32 = children.iter().map(natural_width).sum();
                let gaps = SPACING * (children.len() as i32 - 1).max(0);
                return (widest, total + gaps, -1, -1);
            }
            let width = if for_size < 0 { i32::MAX / 2 } else { for_size };
            let widths = self.widths(&children, width);
            let list = rows(&widths, width);
            let height: i32 = list
                .iter()
                .map(|(a, b)| children[*a..*b].iter().map(natural_height).max().unwrap_or(0))
                .sum();
            let total = height + SPACING * (list.len() as i32 - 1).max(0);
            (total, total, -1, -1)
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let children = self.children();
            let widths = self.widths(&children, width);
            let mut y = 0;
            for (a, b) in rows(&widths, width) {
                let row = &children[a..b];
                let tallest = row.iter().map(natural_height).max().unwrap_or(0);
                let mut x = 0;
                for (child, w) in row.iter().zip(&widths[a..b]) {
                    let w = (*w).min(width);
                    let h = natural_height(child);
                    // Grid cells line up along their tops; a packed row centres.
                    let top = if self.cell.get() > 0 {
                        y
                    } else {
                        y + (tallest - h) / 2
                    };
                    let spot = gtk::Allocation::new(x, top, w, h);
                    child.size_allocate(&spot, -1);
                    x += w + SPACING;
                }
                y += tallest + SPACING;
            }
        }
    }
}

glib::wrapper! {
    pub struct WrapBox(ObjectSubclass<imp::WrapBox>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for WrapBox {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl WrapBox {
    /// A grid of equal cells at least `cell` wide.
    pub fn grid(cell: i32) -> Self {
        let grid = Self::default();
        grid.imp().cell.set(cell);
        grid
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.set_parent(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_share_a_row_until_it_is_full() {
        assert_eq!(rows(&[40, 40, 40], 130), [(0, 3)]);
        assert_eq!(rows(&[40, 40, 40], 100), [(0, 2), (2, 3)]);
    }

    #[test]
    fn a_child_too_wide_gets_its_own_row() {
        assert_eq!(rows(&[300, 40], 100), [(0, 1), (1, 2)]);
        assert_eq!(rows(&[40, 300], 100), [(0, 1), (1, 2)]);
    }

    #[test]
    fn grid_cells_fill_the_width() {
        assert_eq!(grid_cells(724, 136), (5, 141));
        assert_eq!(grid_cells(100, 136), (1, 100));
    }

    #[test]
    fn nothing_makes_no_rows() {
        assert!(rows(&[], 100).is_empty());
    }
}
