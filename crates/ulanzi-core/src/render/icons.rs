use std::collections::BTreeMap;
use std::sync::OnceLock;

use include_dir::{Dir, include_dir};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

use crate::Color;
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

    /// Draw an icon `size` px square with its top-left at (x, y).
    pub fn draw_into(
        &self,
        pix: &mut Pixmap,
        set: IconSet,
        name: &str,
        (x, y): (f32, f32),
        size: f32,
        fg: Color,
    ) -> bool {
        let Some(svg) = self.svg(set, name) else {
            return false;
        };
        let Ok(tree) = usvg::Tree::from_str(&colorize(svg, fg), &usvg::Options::default()) else {
            return false;
        };
        let s = tree.size();
        let k = size / s.width().max(s.height());
        resvg::render(
            &tree,
            Transform::from_scale(k, k).post_translate(x, y),
            &mut pix.as_mut(),
        );
        true
    }

    /// The icon alone, `px` square on transparent, as PNG (for pickers and previews).
    pub fn icon_png(&self, set: IconSet, name: &str, px: u32, fg: Color) -> Option<Vec<u8>> {
        let svg = colorize(self.svg(set, name)?, fg);
        let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).ok()?;
        let size = tree.size();
        let k = px as f32 / size.width().max(size.height());
        let (ox, oy) = (
            (px as f32 - size.width() * k) / 2.0,
            (px as f32 - size.height() * k) / 2.0,
        );
        let mut pix = Pixmap::new(px, px)?;
        resvg::render(
            &tree,
            Transform::from_scale(k, k).post_translate(ox, oy),
            &mut pix.as_mut(),
        );
        pix.encode_png().ok()
    }
}

/// Give an icon the colour `fg` (Simple Icons have none, so their root gets a fill).
pub(crate) fn colorize(svg: &str, fg: Color) -> String {
    let svg = if svg.contains("currentColor") {
        svg.to_string()
    } else {
        svg.replacen("<svg ", "<svg fill=\"currentColor\" ", 1)
    };
    svg.replace(
        "currentColor",
        &format!("#{:02x}{:02x}{:02x}", fg.0[0], fg.0[1], fg.0[2]),
    )
}

fn load(dir: &'static Dir<'static>) -> BTreeMap<&'static str, &'static str> {
    dir.files()
        .filter_map(|f| {
            let name = f.path().file_name()?.to_str()?.strip_suffix(".svg")?;
            Some((name, f.contents_utf8()?))
        })
        .collect()
}
