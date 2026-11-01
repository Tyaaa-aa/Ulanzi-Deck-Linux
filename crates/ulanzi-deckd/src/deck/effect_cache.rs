//! Finished GIFs on disk, so an unchanged key is never drawn twice. It is only a cache: nothing that
//! goes wrong here may stop a key from working.

use std::path::PathBuf;

/// Where the cache lives under the app's cache folder.
pub const FOLDER: &str = "effects";
/// The oldest files go when the folder passes this.
pub const LIMIT: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct EffectCache {
    dir: PathBuf,
    limit: u64,
}

impl EffectCache {
    pub fn new(dir: PathBuf, limit: u64) -> EffectCache {
        EffectCache { dir, limit }
    }

    fn path(&self, hash: u64) -> PathBuf {
        self.dir.join(format!("{hash:016x}.gif"))
    }

    pub fn has(&self, hash: u64) -> bool {
        self.path(hash).exists()
    }

    pub fn get(&self, hash: u64) -> Option<Vec<u8>> {
        std::fs::read(self.path(hash)).ok()
    }

    pub fn put(&self, hash: u64, gif: &[u8]) {
        if let Err(e) = self.write(hash, gif) {
            tracing::warn!("effect cache: {e}");
        }
        self.trim();
    }

    fn write(&self, hash: u64, gif: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        // A half-written file must never look like a finished GIF.
        let part = self.dir.join(format!("{hash:016x}.part"));
        std::fs::write(&part, gif)?;
        std::fs::rename(part, self.path(hash))
    }

    fn trim(&self) {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<_> = dir
            .flatten()
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                Some((meta.modified().ok()?, meta.len(), e.path()))
            })
            .collect();
        files.sort();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        for (_, size, path) in files {
            if total <= self.limit {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total -= size;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn a_stored_gif_comes_back() {
        let d = tempfile::tempdir().expect("dir");
        let c = EffectCache::new(d.path().join("effects"), 1024);
        assert_eq!(c.get(7), None);
        c.put(7, b"gif");
        assert_eq!(c.get(7).as_deref(), Some(&b"gif"[..]));
    }

    #[test]
    fn the_oldest_files_go_past_the_limit() {
        let d = tempfile::tempdir().expect("dir");
        let c = EffectCache::new(d.path().to_path_buf(), 2048);
        c.put(1, &[0; 1024]);
        c.put(2, &[0; 1024]);
        let old = SystemTime::now() - Duration::from_secs(600);
        let first = File::open(d.path().join(format!("{:016x}.gif", 1))).expect("file");
        first.set_modified(old).expect("time");
        c.put(3, &[0; 1024]);
        assert_eq!(c.get(1), None);
        assert!(c.get(2).is_some() && c.get(3).is_some());
    }

    #[test]
    fn a_folder_it_cannot_use_is_no_harm() {
        let d = tempfile::tempdir().expect("dir");
        let file = d.path().join("file");
        std::fs::write(&file, "x").expect("file");
        let c = EffectCache::new(file.join("effects"), 1024);
        c.put(1, b"gif");
        assert_eq!(c.get(1), None);
    }
}
