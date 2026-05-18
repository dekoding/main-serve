use std::path::PathBuf;
use std::time::SystemTime;

/// Metadata for a file or directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Create a new FileMetadata instance.
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
    pub fn is_dir(&self) -> bool {
        !self.is_file
    }
}

/// A directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Name of the entry.
    pub name: String,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Size in bytes (0 for directories).
    pub size: u64,
    /// Full path to the entry (if available).
    pub path: PathBuf,
}

impl DirEntry {
    /// Create a new DirEntry instance.
    pub fn new(name: String, is_dir: bool, size: u64) -> Self {
        Self {
            name,
            is_dir,
            size,
            path: PathBuf::new(),
        }
    }

    /// Set the full path for this entry.
    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = path;
        self
    }

    /// Check if this entry is a file.
    pub fn is_file(&self) -> bool {
        !self.is_dir
    }
}
