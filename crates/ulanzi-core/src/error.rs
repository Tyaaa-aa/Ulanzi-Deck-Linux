use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{path}: made with a newer Ulanzi Deck (file format version {found}); update the app to open it")]
    UnsupportedVersion { path: PathBuf, found: i64 },
    #[error("could not write settings: {0}")]
    Serialize(String),
    #[error("HOME is not set")]
    NoHome,
    #[error("the old configuration was already imported (use --force to import it again)")]
    AlreadyMigrated,
    #[error("{0}")]
    Invalid(String),
}

impl CoreError {
    pub fn io(path: &Path, source: std::io::Error) -> CoreError {
        CoreError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}
