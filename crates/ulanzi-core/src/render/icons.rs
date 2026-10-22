use std::collections::BTreeMap;
use std::sync::OnceLock;

use include_dir::{Dir, include_dir};

use crate::model::IconSet;

static LUCIDE: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/assets/lucide");
static BRANDS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/assets/brands");

/// Icon sets compiled into the binary.
pub struct IconLibrary {
    lucide: BTreeMap<&'static str, &'static str>,
    brands: BTreeMap<&'static str, &'static str>,
}

impl IconLibrary {
    pub fn bundled() -> &'static IconLibrary {
        static LIB: OnceLock<IconLibrary> = OnceLock::new();
        LIB.get_or_init(|| IconLibrary {
            lucide: load(&LUCIDE),
            brands: load(&BRANDS),
        })
    }

    pub fn svg(&self, set: IconSet, name: &str) -> Option<&'static str> {
        match set {
            IconSet::Lucide => self.lucide.get(name).copied(),
            IconSet::Brands => self.brands.get(name).copied(),
        }
    }

    pub fn names(&self, set: IconSet) -> impl Iterator<Item = &'static str> + '_ {
        let map = match set {
            IconSet::Lucide => Some(&self.lucide),
            IconSet::Brands => Some(&self.brands),
        };
        map.into_iter().flat_map(|m| m.keys().copied())
    }
}

fn load(dir: &'static Dir<'static>) -> BTreeMap<&'static str, &'static str> {
    dir.files()
        .filter_map(|f| {
            let name = f.path().file_name()?.to_str()?.strip_suffix(".svg")?;
            Some((name, f.contents_utf8()?))
        })
        .collect()
}
