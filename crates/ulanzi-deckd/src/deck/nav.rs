use ulanzi_core::model::{Page, PageKind};

/// Current page plus the folders opened to get there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NavState {
    pub page: usize,
    pub stack: Vec<usize>,
}

impl NavState {
    /// Next normal page (folders are skipped), wrapping around. Leaves any folder.
    pub fn next(&mut self, pages: &[Page]) {
        self.step(pages, 1);
    }

    pub fn prev(&mut self, pages: &[Page]) {
        self.step(pages, -1);
    }

    fn step(&mut self, pages: &[Page], dir: isize) {
        let normal: Vec<usize> = (0..pages.len())
            .filter(|i| pages[*i].kind == PageKind::Page)
            .collect();
        if normal.is_empty() {
            return;
        }
        let from = self.stack.first().copied().unwrap_or(self.page);
        let pos = normal.iter().position(|i| *i == from).unwrap_or(0) as isize;
        let n = normal.len() as isize;
        self.page = normal[((pos + dir).rem_euclid(n)) as usize];
        self.stack.clear();
    }

    pub fn goto(&mut self, idx: usize, pages: &[Page]) {
        if idx < pages.len() {
            self.page = idx;
            self.stack.clear();
        }
    }

    pub fn open_folder(&mut self, idx: usize, pages: &[Page]) {
        if idx < pages.len() && idx != self.page {
            self.stack.push(self.page);
            self.page = idx;
        }
    }

    pub fn back(&mut self) {
        if let Some(p) = self.stack.pop() {
            self.page = p;
        }
    }

    pub fn clamp(&mut self, pages: usize) {
        if self.page >= pages {
            self.page = 0;
            self.stack.clear();
        }
        self.stack.retain(|p| *p < pages);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages() -> Vec<Page> {
        let mut f = Page::new("F");
        f.kind = PageKind::Folder;
        vec![Page::new("A"), f, Page::new("B")]
    }

    #[test]
    fn next_skips_folders_and_wraps() {
        let p = pages();
        let mut n = NavState::default();
        n.next(&p);
        assert_eq!(n.page, 2);
        n.next(&p);
        assert_eq!(n.page, 0);
        n.prev(&p);
        assert_eq!(n.page, 2);
    }

    #[test]
    fn folder_and_back() {
        let p = pages();
        let mut n = NavState::default();
        n.open_folder(1, &p);
        assert_eq!((n.page, n.stack.clone()), (1, vec![0]));
        n.back();
        assert_eq!((n.page, n.stack.len()), (0, 0));
        n.open_folder(1, &p);
        n.next(&p);
        assert_eq!(
            (n.page, n.stack.len()),
            (2, 0),
            "next from inside a folder moves on from its parent"
        );
    }
}
