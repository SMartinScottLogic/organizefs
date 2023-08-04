use core::fmt::Debug;
use std::{ffi::OsStr, fs, path::Path, time::SystemTime};

use mockall::automock;

#[automock]
pub trait DirEntry: Debug {
    fn path(&self) -> &Path;
    fn file_name(&self) -> &OsStr;
}
impl DirEntry for walkdir::DirEntry {
    fn path(&self) -> &Path {
        self.path()
    }
    fn file_name(&self) -> &OsStr {
        self.file_name()
    }
}

#[automock]
pub trait Metadata: Debug {
    fn len(&self) -> u64;
    fn modified(&self) -> std::io::Result<SystemTime>;
}
impl Metadata for fs::Metadata {
    fn len(&self) -> u64 {
        self.len()
    }
    fn modified(&self) -> std::io::Result<SystemTime> {
        self.modified()
    }
}
