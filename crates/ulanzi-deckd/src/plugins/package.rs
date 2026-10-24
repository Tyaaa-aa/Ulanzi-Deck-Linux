//! Unpacking a `.ulanziplugin` package: a ZIP file holding `plugin.toml`, `plugin.wasm` and an
//! optional `assets/` folder. Nothing in the file is trusted: every entry name is checked, links
//! are refused, and the sizes are counted while extracting, never read from the headers.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use super::PluginError;

/// The package file itself.
pub(super) const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;
/// Everything in the package, unpacked.
pub(super) const MAX_UNPACKED_BYTES: u64 = 64 * 1024 * 1024;
/// Entries (files and folders) in the package.
pub(super) const MAX_ENTRIES: usize = 1000;

fn bad(why: impl Into<String>) -> PluginError {
    PluginError::Invalid(why.into())
}

const NOT_A_PACKAGE: &str = "That file is not a valid .ulanziplugin package";

fn mib(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

/// Check an entry name and say whether it is a folder. Only `plugin.toml`, `plugin.wasm` and
/// things under `assets/` are allowed.
fn check_name(name: &str) -> Result<bool, PluginError> {
    let refuse = |why: &str| {
        bad(format!(
            "The package has an entry that is not allowed ({name}): {why}"
        ))
    };
    if name.contains('\0') || name.contains('\\') {
        return Err(refuse("names may not hold backslashes or NUL"));
    }
    if name.starts_with('/') {
        return Err(refuse("absolute paths are not allowed"));
    }
    let is_dir = name.ends_with('/');
    let parts: Vec<&str> = name.trim_end_matches('/').split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err(refuse("paths may not hold empty, . or .. parts"));
    }
    match parts.as_slice() {
        ["plugin.toml" | "plugin.wasm"] if !is_dir => Ok(false),
        ["assets"] if !is_dir => Err(refuse("assets must be a folder")),
        ["assets", ..] => Ok(is_dir),
        _ => Err(refuse(
            "only plugin.toml, plugin.wasm and assets/ belong in a package",
        )),
    }
}

/// Unpack the package `file` into the existing, empty folder `into`. Nothing is written outside
/// `into`; the caller removes it on failure.
pub(super) fn unpack(file: &Path, into: &Path) -> Result<(), PluginError> {
    let open = |e: io::Error| PluginError::Io(format!("cannot read {}: {e}", file.display()));
    let f = fs::File::open(file).map_err(open)?;
    let meta = f.metadata().map_err(open)?;
    if !meta.is_file() {
        return Err(PluginError::Io(format!(
            "{} is not a regular file",
            file.display()
        )));
    }
    if meta.len() > MAX_PACKAGE_BYTES {
        return Err(bad(format!(
            "The package is larger than {} MiB",
            mib(MAX_PACKAGE_BYTES)
        )));
    }
    let mut zip = zip::ZipArchive::new(f).map_err(|_| bad(NOT_A_PACKAGE))?;
    if zip.len() > MAX_ENTRIES {
        return Err(bad(format!("The package holds more than {MAX_ENTRIES} entries")));
    }
    // Catches names listed twice (the reader would silently keep one), headers that disagree
    // with each other, and entries that overlap.
    zip.validate_header_consistency().map_err(|e| {
        if e.to_string().contains("duplicate") {
            bad("The package lists the same entry twice")
        } else {
            bad(NOT_A_PACKAGE)
        }
    })?;

    let mut seen = BTreeSet::new();
    let mut left = MAX_UNPACKED_BYTES;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|_| bad(NOT_A_PACKAGE))?;
        let name = entry
            .name()
            .map_err(|_| bad("The package has an entry whose name is not valid text"))?
            .into_owned();
        if let Some(mode) = entry.unix_mode() {
            match mode & 0o170000 {
                0 | 0o100000 | 0o040000 => {}
                _ => {
                    return Err(bad(format!(
                        "The package holds a link ({name}); plugins may only hold files and folders"
                    )));
                }
            }
        }
        let is_dir = check_name(&name)?;
        if !seen.insert(name.trim_end_matches('/').to_string()) {
            return Err(bad(format!("The package lists {name} twice")));
        }
        let target = into.join(name.trim_end_matches('/'));
        let io_err = |e: io::Error| PluginError::Io(format!("cannot unpack {name}: {e}"));
        if is_dir {
            fs::create_dir_all(&target).map_err(io_err)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        // `create_new`: never writes through an existing file or link.
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(io_err)?;
        // One byte more than the budget shows a lying header without unpacking the rest.
        let copied = io::copy(&mut (&mut entry).take(left + 1), &mut out).map_err(io_err)?;
        if copied > left {
            return Err(bad(format!(
                "The package unpacks to more than {} MiB",
                mib(MAX_UNPACKED_BYTES)
            )));
        }
        left -= copied;
    }
    for needed in ["plugin.toml", "plugin.wasm"] {
        if !seen.contains(needed) {
            return Err(bad(format!("The package has no {needed}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;

    type Zip = zip::ZipWriter<Cursor<Vec<u8>>>;

    fn build(fill: impl FnOnce(&mut Zip)) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        fill(&mut z);
        z.finish().expect("finish").into_inner()
    }

    fn put(z: &mut Zip, name: &str, data: &[u8]) {
        z.start_file(name, SimpleFileOptions::default()).expect("start");
        z.write_all(data).expect("write");
    }

    fn good(z: &mut Zip) {
        put(z, "plugin.toml", b"id = \"x\"");
        put(z, "plugin.wasm", b"\0asm");
    }

    /// Unpack `bytes`; returns the temp dir (to look at) and the result.
    fn run(bytes: &[u8]) -> (tempfile::TempDir, Result<(), PluginError>) {
        let src = tempfile::tempdir().expect("src");
        let file = src.path().join("p.ulanziplugin");
        fs::write(&file, bytes).expect("write package");
        let out = tempfile::tempdir().expect("out");
        let r = unpack(&file, out.path());
        (out, r)
    }

    fn refused(bytes: &[u8]) -> String {
        let (out, r) = run(bytes);
        let e = r.expect_err("refused").to_string();
        drop(out);
        e
    }

    #[test]
    fn unpacks_a_good_package() {
        let bytes = build(|z| {
            good(z);
            z.add_directory("assets/", SimpleFileOptions::default())
                .expect("dir");
            put(z, "assets/a/b.png", b"png");
        });
        let (out, r) = run(&bytes);
        r.expect("unpacks");
        assert_eq!(
            fs::read(out.path().join("plugin.toml")).expect("toml"),
            b"id = \"x\""
        );
        assert_eq!(fs::read(out.path().join("plugin.wasm")).expect("wasm"), b"\0asm");
        assert_eq!(fs::read(out.path().join("assets/a/b.png")).expect("png"), b"png");
    }

    #[test]
    fn refuses_parent_dir_entry() {
        let bytes = build(|z| {
            good(z);
            put(z, "../x", b"x");
        });
        assert!(refused(&bytes).contains("../x"));
        let bytes = build(|z| {
            good(z);
            put(z, "assets/../../x", b"x");
        });
        assert!(refused(&bytes).contains("assets/../../x"));
    }

    #[test]
    fn refuses_absolute_entry() {
        let bytes = build(|z| {
            good(z);
            put(z, "/etc/x", b"x");
        });
        assert!(refused(&bytes).contains("/etc/x"));
    }

    #[test]
    fn refuses_backslash_and_nul_names() {
        let bytes = build(|z| {
            good(z);
            put(z, "assets\\x", b"x");
        });
        assert!(refused(&bytes).contains("assets"));
        let bytes = build(|z| {
            good(z);
            put(z, "assets/a\0b", b"x");
        });
        assert!(refused(&bytes).contains("not allowed"));
    }

    #[test]
    fn refuses_symlink_entry() {
        let bytes = build(|z| {
            good(z);
            z.add_symlink("assets/x", "/etc/passwd", SimpleFileOptions::default())
                .expect("link");
        });
        let e = refused(&bytes);
        assert!(e.contains("link (assets/x)"), "{e}");
    }

    #[test]
    fn refuses_zip_bomb() {
        let bytes = build(|z| {
            good(z);
            z.start_file(
                "assets/zeros",
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .expect("start");
            let chunk = vec![0u8; 1024 * 1024];
            for _ in 0..65 {
                z.write_all(&chunk).expect("zeros");
            }
        });
        assert!(bytes.len() < 1024 * 1024, "the package itself is small");
        let e = refused(&bytes);
        assert!(e.contains("64 MiB"), "{e}");
    }

    #[test]
    fn refuses_too_many_entries() {
        let bytes = build(|z| {
            good(z);
            for i in 0..998 {
                put(z, &format!("assets/f{i}"), b"");
            }
        });
        let (_out, r) = run(&bytes);
        r.expect("exactly 1000 entries is fine");
        let bytes = build(|z| {
            good(z);
            for i in 0..999 {
                put(z, &format!("assets/f{i}"), b"");
            }
        });
        let e = refused(&bytes);
        assert!(e.contains("1000 entries"), "{e}");
    }

    #[test]
    fn refuses_missing_wasm() {
        let bytes = build(|z| put(z, "plugin.toml", b"id = \"x\""));
        assert_eq!(refused(&bytes), "The package has no plugin.wasm");
        let bytes = build(|z| put(z, "plugin.wasm", b"\0asm"));
        assert_eq!(refused(&bytes), "The package has no plugin.toml");
    }

    #[test]
    fn refuses_unknown_top_level_entry() {
        let bytes = build(|z| {
            good(z);
            put(z, "README.md", b"hi");
        });
        assert!(refused(&bytes).contains("README.md"));
        let bytes = build(|z| {
            good(z);
            put(z, "assets", b"x");
        });
        assert!(refused(&bytes).contains("assets must be a folder"));
        // A package zipped from the folder itself has everything one level down.
        let bytes = build(|z| {
            put(z, "obs/plugin.toml", b"id = \"x\"");
            put(z, "obs/plugin.wasm", b"\0asm");
        });
        assert!(refused(&bytes).contains("obs/plugin.toml"));
    }

    #[test]
    fn refuses_duplicate_entry() {
        let mut bytes = build(|z| {
            good(z);
            put(z, "plugin.tomX", b"id = \"y\"");
        });
        // Rename the third entry to the first one's name (same length, both headers).
        let from = b"plugin.tomX";
        let to = b"plugin.toml";
        let mut i = 0;
        while i + from.len() <= bytes.len() {
            if &bytes[i..i + from.len()] == from {
                bytes[i..i + from.len()].copy_from_slice(to);
            }
            i += 1;
        }
        let e = refused(&bytes);
        assert!(e.contains("twice"), "{e}");
    }

    #[test]
    fn refuses_not_a_zip() {
        assert_eq!(
            refused(b"hello, not a zip"),
            "That file is not a valid .ulanziplugin package"
        );
    }
}
