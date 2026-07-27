use std::path::PathBuf;
use std::time::SystemTime;

/// Metadata for a file or directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
/// FileMetadata
pub struct FileMetadata {
    /// Name of the file or directory.
    pub name: String,
    /// Whether this is a file (as opposed to a directory).
    pub is_file: bool,
    /// Size in bytes.
    pub size: u64,
    /// Creation time.
    pub created: Option<SystemTime>,
    /// Modification time.
    pub modified: Option<SystemTime>,
}

impl FileMetadata {
    /// Create a new `FileMetadata` instance.
    #[must_use]
    pub fn new(name: String, is_file: bool, size: u64) -> Self {
        Self {
            name,
            is_file,
            size,
            created: None,
            modified: None,
        }
    }

    /// Check if this metadata represents a directory.
    #[must_use]
    pub fn is_dir(&self) -> bool {
        !self.is_file
    }
}

/// A directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
/// DirEntry
pub struct DirEntry {
    /// Name of the entry.
    pub name: String,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Size in bytes (0 for directories).
    pub size: u64,
    /// Full path to the entry (if available).
    pub path: PathBuf,
    /// Unix file mode (permissions + file type bits).
    pub mode: u32,
    /// Numeric user ID (UID).
    pub uid: u32,
    /// Numeric group ID (GID).
    pub gid: u32,
    /// Last modification time, if available.
    pub modified: Option<std::time::SystemTime>,
}

impl DirEntry {
    /// Create a new `DirEntry` instance.
    #[must_use]
    pub fn new(name: String, is_dir: bool, size: u64) -> Self {
        Self {
            name,
            is_dir,
            size,
            path: PathBuf::new(),
            mode: 0,
            uid: 0,
            gid: 0,
            modified: None,
        }
    }

    /// Set the full path for this entry.
    #[must_use]
    pub(crate) fn with_path(mut self, path: PathBuf) -> Self {
        self.path = path;
        self
    }

    /// Set the Unix mode bits for this entry.
    #[must_use]
    pub(crate) fn with_mode(mut self, mode: u32) -> Self {
        self.mode = mode;
        self
    }

    /// Set the UID for this entry.
    #[must_use]
    pub(crate) fn with_uid(mut self, uid: u32) -> Self {
        self.uid = uid;
        self
    }

    /// Set the GID for this entry.
    #[must_use]
    pub(crate) fn with_gid(mut self, gid: u32) -> Self {
        self.gid = gid;
        self
    }

    /// Set the last modification time for this entry.
    #[must_use]
    pub(crate) fn with_modified(mut self, modified: Option<std::time::SystemTime>) -> Self {
        self.modified = modified;
        self
    }

    /// Check if this entry is a file.
    #[must_use]
    pub fn is_file(&self) -> bool {
        !self.is_dir
    }
}
