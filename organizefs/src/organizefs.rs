use std::{ffi::OsString, path::{PathBuf, Path}, sync::{RwLock, Arc, Mutex}, fs, fmt::Display, time::{SystemTime, Duration}};
use std::fmt::Debug;
use fuse_mt::{Statfs, FileAttr, FilesystemMT, RequestInfo, ResultEmpty, FileType, ResultEntry, ResultStatfs, ResultOpen, DirectoryEntry, ResultReaddir, CallbackResult, ResultSlice};
use humansize::FormatSize;
use tracing::{instrument, debug, info};
use walkdir::WalkDir;

use crate::{libc_wrapper::{LibcWrapper, LibcWrapperReal}, arena::{NewArena, Arena, Entry}};
use crate::common::{File, expand, Normalize};

lazy_static::lazy_static! {
    static ref FORMAT: humansize::FormatSizeOptions = humansize::DECIMAL.space_after_value(false).decimal_zeroes(2);
}
static TTL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OrganizeFSEntry {
    name: OsString,
    host_path: PathBuf,
    size: String,
    mime: String,
}
impl File for OrganizeFSEntry {
    fn meta(&self) -> &str {
        &self.mime
    }

    fn size(&self) -> &str {
        &self.size
    }
}

impl OrganizeFSEntry {
    fn new(root: &Path, entry: &walkdir::DirEntry, meta: &fs::Metadata) -> Self {
        let host_path = root.join(entry.path()).canonicalize().unwrap();
        let size = meta.len().format_size(*FORMAT);
        let mime = tree_magic_mini::from_filepath(&host_path)
            .unwrap_or_default()
            .replace('/', "_");
        let name = entry.file_name().to_os_string();

        debug!(
            root = debug(root),
            entry = debug(entry),
            meta = debug(meta),
            path = debug(&host_path),
            size,
            mime
        );
        Self {
            host_path,
            name,
            size,
            mime,
        }
    }

    fn local_path(&self, pattern: &Path) -> PathBuf {
        let mut path = pattern
            .components()
            .map(|component| expand(&component, self))
            .fold(PathBuf::new(), |mut acc, c| {
                acc.push(c);
                acc
            });
        path.push(&self.name);
        path
    }
}

impl Display for OrganizeFSEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({} {})", self.host_path.display(), self.size)
    }
}

impl Debug for OrganizeFSStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("arena_len", &self.arena.len())
            .field("entries_len", &self.entries.len())
            .field("pattern", &self.pattern)
            .finish()
    }
}
type ArenaEntry = <NewArena<usize> as Arena<usize>>::Entry;
impl OrganizeFSStore {
    #[instrument]
    pub fn new(pattern: PathBuf) -> Self {
        Self {
            pattern: pattern.normalize(),
            arena: NewArena::<usize>::default(),
            entries: Vec::new(),
        }
    }
    #[instrument]
    fn remove_entry(&mut self, entry: &OrganizeFSEntry) {
        // TODO Remove file from entries
    }

    #[instrument]
    fn add_entry(&mut self, entry: OrganizeFSEntry) {
        let id = self.entries.len();
        self.entries.push(entry.clone());

        let local_path = entry.local_path(&self.pattern);
        Self::add_entry_to_arena(&mut self.arena, &local_path, id);
    }

    #[instrument]
    fn add_entry_to_arena(arena: &mut NewArena<usize>, local_path: &Path, id: usize) {
        debug!(
            arena = debug(&arena),
            id = debug(&id),
            path = debug(&local_path),
            "add to arena"
        );
        arena.add_file(local_path, id).unwrap();
    }

    #[instrument]
    fn find(&self, path: &Path) -> ArenaEntry {
        self.arena.find(path)
    }

    #[instrument]
    fn find_file(&self, path: &Path) -> Option<usize> {
        // match self.find(path) {
        //     Entry::Root | Entry::Directory(_) | Entry::None => None,
        //     Entry::File(_p, index) => Some(index),
        // }
        self.find(path)
            .filter(|e| e.is_file())
            .and_then(|entry| entry.inner())
    }

    #[instrument(ret)]
    fn find_dir(&self, path: &Path) -> Option<ArenaEntry> {
        // match self.find(path) {
        //     Entry::File(_, _) => None,
        //     e => Some(e),
        // }
        self.find(path).filter(|e| e.is_directory()).cloned()
    }
}

pub struct OrganizeFSStore {
    arena: NewArena<usize>,
    entries: Vec<OrganizeFSEntry>,
    pattern: PathBuf,
}
impl OrganizeFSStore {
    pub fn get_pattern(&self) -> String {
        self.pattern.to_string_lossy().to_string()
    }

    pub fn set_pattern(&mut self, pattern: &str) {
        let pattern = PathBuf::from(pattern).normalize();
        if pattern != self.pattern {
            // Re-patterning of filesystem
            let mut arena = NewArena::<usize>::default();
            for (id, entry) in self.entries.iter().enumerate() {
                let local_path = entry.local_path(&pattern);
                Self::add_entry_to_arena(&mut arena, &local_path, id);
            }
            self.arena = arena;
            self.pattern = pattern;
        }
    }
}

pub struct OrganizeFS {
    root: PathBuf,
    store: Arc<RwLock<OrganizeFSStore>>,
    libc_wrapper: Box<dyn LibcWrapper + Send + Sync>,
    shutdown_signal: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
impl Debug for OrganizeFS {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrganizeFS")
            .field("root", &self.root)
            .field("store", &self.store)
            .finish()
    }
}

impl OrganizeFS {
    #[instrument]
    pub fn new(root: &str, store: Arc<RwLock<OrganizeFSStore>>, shutdown_signal: tokio::sync::oneshot::Sender<()>) -> Self {
        let root = std::env::current_dir().unwrap().as_path().join(root);
        {
            let mut store = store.write().unwrap();
            info!(root = debug(&root), "init");
            for entry in Self::scan(&root) {
                store.add_entry(entry);
            }
            info!(store = debug(&store), "store populated");
        }

        Self {
            root,
            store,
            shutdown_signal: Mutex::new(Some(shutdown_signal)),
            libc_wrapper: Box::new(LibcWrapperReal::new()),
        }
    }

    #[instrument]
    fn scan(root: &Path) -> impl Iterator<Item = OrganizeFSEntry> + '_ {
        info!(root = debug(root), "scanning");
        WalkDir::new(root)
            .sort_by(|a, b| a.file_name().cmp(b.file_name()))
            .into_iter()
            .flatten()
            .filter_map(|entry| Self::process(root, &entry))
    }

    #[instrument]
    fn process(root: &Path, entry: &walkdir::DirEntry) -> Option<OrganizeFSEntry> {
        if entry.file_type().is_file() && entry.path().parent().is_some() {
            if let Ok(meta) = fs::symlink_metadata(entry.path()) {
                debug!(root = debug(root), entry = debug(entry), "found");
                let entry = OrganizeFSEntry::new(root, entry, &meta);
                debug!(root = debug(root), entry = display(&entry));
                return Some(entry);
            }
        }
        None
    }

    fn statfs_to_fuse(statfs: libc::statfs) -> Statfs {
        Statfs {
            blocks: statfs.f_blocks,
            bfree: statfs.f_bfree,
            bavail: statfs.f_bavail,
            files: statfs.f_files,
            ffree: statfs.f_ffree,
            bsize: statfs.f_bsize as u32,
            namelen: statfs.f_namelen as u32,
            frsize: statfs.f_frsize as u32,
        }
    }

    fn mode_to_filetype(mode: libc::mode_t) -> FileType {
        match mode & libc::S_IFMT {
            libc::S_IFDIR => FileType::Directory,
            libc::S_IFREG => FileType::RegularFile,
            libc::S_IFLNK => FileType::Symlink,
            libc::S_IFBLK => FileType::BlockDevice,
            libc::S_IFCHR => FileType::CharDevice,
            libc::S_IFIFO => FileType::NamedPipe,
            libc::S_IFSOCK => FileType::Socket,
            _ => {
                panic!("unknown file type");
            }
        }
    }

    fn stat_to_fuse(stat: libc::stat) -> FileAttr {
        // st_mode encodes both the kind and the permissions
        let kind = Self::mode_to_filetype(stat.st_mode);
        let perm = (stat.st_mode & 0o7777) as u16;

        FileAttr {
            size: stat.st_size as u64,
            blocks: stat.st_blocks as u64,
            atime: SystemTime::UNIX_EPOCH
                + Duration::from_secs(stat.st_atime.try_into().unwrap())
                + Duration::from_nanos(stat.st_atime_nsec.try_into().unwrap()),
            mtime: SystemTime::UNIX_EPOCH
                + Duration::from_secs(stat.st_mtime.try_into().unwrap())
                + Duration::from_nanos(stat.st_mtime_nsec.try_into().unwrap()),
            ctime: SystemTime::UNIX_EPOCH
                + Duration::from_secs(stat.st_ctime.try_into().unwrap())
                + Duration::from_nanos(stat.st_ctime_nsec.try_into().unwrap()),
            crtime: SystemTime::UNIX_EPOCH,
            kind,
            perm,
            nlink: stat.st_nlink as u32,
            uid: stat.st_uid,
            gid: stat.st_gid,
            rdev: stat.st_rdev as u32,
            flags: 0,
        }
    }
}

impl FilesystemMT for OrganizeFS {
    fn init(&self, req: RequestInfo) -> ResultEmpty {
        info!(req=debug(req), "init");
        Ok(())
    }

    fn destroy(&self) {
        info!("destroy");
        let mut mutex = self.shutdown_signal.lock().unwrap();
        if let Some(signal) = mutex.take() {
            signal.send(()).unwrap();
        }
    }

    fn getattr(&self, req: RequestInfo, path: &Path, fh: Option<u64>) -> ResultEntry {
        info!(req = debug(req), path = debug(path), fh, "getattr");
        if let Some(fh) = fh {
            match self.libc_wrapper.fstat(fh) {
                Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
            }
        } else {
            let store = self.store.read().unwrap();
            let r = store.find(path);
            info!(found = debug(&r), "found");
            if r.is_directory() {
                match self.libc_wrapper.lstat(self.root.to_owned()) {
                    Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                }
            } else if r.is_file() {
                let entry = store.entries.get(r.inner().unwrap()).unwrap();
                match self.libc_wrapper.lstat(entry.host_path.to_owned()) {
                    Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                }
            } else {
                Err(libc::ENOENT)
            }
            // match store.find(path) {
            //     Entry::Root | Entry::Directory(_) => {
            //         match self.libc_wrapper.lstat(self.root.to_owned()) {
            //             Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
            //             Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
            //         }
            //     }
            //     Entry::File(_p, e) => {
            //         let entry = store.entries.get(e).unwrap();
            //         match self.libc_wrapper.lstat(entry.host_path.to_owned()) {
            //             Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
            //             Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
            //         }
            //     }
            //     Entry::None => Err(libc::ENOENT),
            // }
        }
    }

    fn statfs(&self, req: RequestInfo, path: &Path) -> ResultStatfs {
        debug!(req = debug(req), path = debug(path), "statfs");
        match self.libc_wrapper.statfs(self.root.to_owned()) {
            Ok(stat) => Ok(Self::statfs_to_fuse(stat)),
            Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
        }
    }

    fn opendir(&self, req: RequestInfo, path: &Path, flags: u32) -> ResultOpen {
        debug!(
            req = debug(req),
            path = debug(path),
            flags,
            "opendir (flags = {:#o})",
            flags
        );
        if self.store.read().unwrap().find_dir(path).is_some() {
            Ok((0, 0))
        } else {
            Err(libc::ENOENT)
        }
    }

    #[instrument(level = "info")]
    fn readdir(&self, req: RequestInfo, path: &Path, fh: u64) -> ResultReaddir {
        debug!(req = debug(req), path = debug(path), fh, "readdir");

        let store = self.store.read().unwrap();
        let children = store
            .find_dir(path)
            .unwrap()
            .children(&store.arena)
            //.unique()
            .filter_map(|(name, entry)| {
                //let entry = store.entries.get(id).unwrap();
                info!(
                    path = debug(&path),
                    name = debug(&name),
                    entry = debug(&entry),
                    "child"
                );
                if entry.is_directory() {
                    Some((FileType::Directory, name))
                } else if entry.is_file() {
                    Some((FileType::RegularFile, name))
                } else {
                    None
                }
                // match entry {
                //     arena::Entry::Directory(name) => Some((FileType::Directory, name)),
                //     arena::Entry::File(name, _) => Some((FileType::RegularFile, name)),
                //     _ => None,
                // }
            })
            .fold(
                vec![
                    DirectoryEntry {
                        name: ".".into(),
                        kind: FileType::Directory,
                    },
                    DirectoryEntry {
                        name: "..".into(),
                        kind: FileType::Directory,
                    },
                ],
                |mut acc, (kind, name)| {
                    acc.push(DirectoryEntry {
                        name: name.clone(),
                        kind,
                    });
                    acc
                },
            );

        debug!(
            req = debug(req),
            path = debug(path),
            children = debug(&children),
            fh,
            "readdir"
        );
        Ok(children)
    }

    fn releasedir(&self, req: RequestInfo, path: &Path, fh: u64, flags: u32) -> ResultEmpty {
        debug!(
            req = debug(req),
            path = debug(path),
            fh,
            "releasedir (flags = {:#o})",
            flags
        );
        Ok(())
    }

    fn open(&self, req: RequestInfo, path: &Path, flags: u32) -> ResultOpen {
        debug!(
            req = debug(req),
            path = debug(path),
            "open (flags = {:#o})",
            flags
        );
        let store = self.store.read().unwrap();
        store.find_file(path).map_or_else(
            || Err(libc::ENOENT),
            |e| {
                let entry = store.entries.get(e).unwrap();
                match self
                    .libc_wrapper
                    .open(entry.host_path.to_owned(), flags.try_into().unwrap())
                {
                    Ok(fh) => Ok((fh as u64, flags)),
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                }
            },
        )
    }

    fn read(
        &self,
        req: RequestInfo,
        path: &Path,
        fh: u64,
        offset: u64,
        size: u32,
        callback: impl FnOnce(ResultSlice<'_>) -> CallbackResult,
    ) -> CallbackResult {
        debug!(
            req = debug(req),
            path = debug(path),
            fh,
            offset,
            size,
            "read"
        );
        if fh > 0 {
            match self
                .libc_wrapper
                .read(fh.try_into().unwrap(), offset.try_into().unwrap(), size)
            {
                Ok(content) => callback(Ok(content.as_slice())),
                Err(e) => callback(Err(e.raw_os_error().unwrap_or(libc::ENOENT))),
            }
        } else {
            callback(Err(libc::ENOENT))
        }
    }

    fn flush(&self, req: RequestInfo, path: &Path, fh: u64, lock_owner: u64) -> ResultEmpty {
        debug!(
            req = debug(req),
            path = debug(path),
            fh,
            lock_owner,
            "flush"
        );
        Err(libc::ENOSYS)
    }

    fn release(
        &self,
        req: RequestInfo,
        path: &Path,
        fh: u64,
        flags: u32,
        lock_owner: u64,
        flush: bool,
    ) -> ResultEmpty {
        debug!(
            req = debug(req),
            path = debug(path),
            fh,
            lock_owner,
            flush,
            "release (flags = {:#o})",
            flags
        );
        if fh > 0 {
            match self.libc_wrapper.close(fh.try_into().unwrap()) {
                Ok(_) => Ok(()),
                Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
            }
        } else {
            Err(libc::ENOENT)
        }
    }

    fn unlink(&self, req: RequestInfo, parent: &Path, name: &std::ffi::OsStr) -> ResultEmpty {
        info!(
            req = debug(req),
            parent = debug(parent),
            name = debug(name),
            "unlink",
        );
        let mut path = parent.to_path_buf();
        path.push(name);

        let mut store = self.store.write().unwrap();
        store.find_file(&path).map_or_else(
            || Err(libc::ENOENT),
            |e| {
                let entry = store.entries.get(e).unwrap().to_owned();
                match self.libc_wrapper.unlink(entry.host_path.to_owned()) {
                    Ok(_) => {
                        store.remove_entry(&entry);
                        Ok(())
                    }
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                }
            },
        )
    }

    fn rename(&self, req: RequestInfo, parent: &Path, name: &std::ffi::OsStr, newparent: &Path, newname: &std::ffi::OsStr) -> ResultEmpty {
        info!(
            req = debug(req),
            parent = debug(parent),
            name = debug(name),
            newparent = debug(newparent),
            newname = debug(newname),
            "rename",
        );
        Err(libc::ENOSYS)
    }

}

#[cfg(test)]
mod tests {
    use std::{io, path::PathBuf};

    use tracing_test::traced_test;

    use libc_wrapper::MockLibcWrapper;

    use crate::libc_wrapper;

    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[instrument(ret, skip(libc_wrapper))]
    fn new_test_fs(libc_wrapper: impl LibcWrapper + Send + Sync + 'static) -> OrganizeFS {
        let root = PathBuf::from("/");
        let pattern = PathBuf::from("/");
        let store = Arc::new(RwLock::new(OrganizeFSStore::new(pattern)));
        let libc_wrapper = Box::new(libc_wrapper);
        OrganizeFS {
            root,
            store,
            libc_wrapper,
            shutdown_signal: Mutex::new(None),
        }
    }

    // open tests
    #[test]
    #[traced_test]
    fn open_missing() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let parent = PathBuf::from("/");
        let name = std::ffi::OsString::from("missing");
        let r = fs.open(req, &parent.join(name), 0);
        assert_eq!(r.err(), Some(libc::ENOENT));
    }

    #[test]
    #[traced_test]
    fn open_present() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper.expect_open().returning(|_, _| Ok(1));
            libc_wrapper
        };
        let fs = new_test_fs(libc_wrapper);
        {
            let mut store = fs.store.write().unwrap();
            let entry = OrganizeFSEntry {
                name: "present".into(),
                host_path: "".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
            };
            store.add_entry(entry);
        }
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let parent = PathBuf::from("/");
        let name = std::ffi::OsString::from("present");
        let r = fs.open(req, &parent.join(name), 0);
        assert!(r.is_ok());
    }

    #[test]
    #[traced_test]
    fn open_no_access() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper
                .expect_open()
                .returning(|_, _| Err(io::Error::from_raw_os_error(libc::EACCES)));
            libc_wrapper
        };
        let fs = new_test_fs(libc_wrapper);
        {
            let mut store = fs.store.write().unwrap();
            let entry = OrganizeFSEntry {
                name: "present".into(),
                host_path: "".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
            };
            store.add_entry(entry);
        }
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let parent = PathBuf::from("/");
        let name = std::ffi::OsString::from("present");
        let r = fs.open(req, &parent.join(name), 0);
        assert_eq!(r.err(), Some(libc::EACCES));
    }

    // unlink tests
    #[test]
    #[traced_test]
    fn unlink_missing() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let parent = PathBuf::from("/");
        let name = std::ffi::OsString::from("missing");
        let r = fs.unlink(req, &parent, &name);
        assert_eq!(r.err(), Some(libc::ENOENT));
    }

    #[test]
    #[traced_test]
    fn unlink_present() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper.expect_unlink().returning(|_| Ok(()));
            libc_wrapper
        };
        let fs = new_test_fs(libc_wrapper);
        {
            let mut store = fs.store.write().unwrap();
            let entry = OrganizeFSEntry {
                name: "present".into(),
                host_path: "".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
            };
            store.add_entry(entry);
        }
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let parent = PathBuf::from("/");
        let name = std::ffi::OsString::from("present");
        let r = fs.unlink(req, &parent, &name);
        assert!(r.is_ok());
    }

    #[test]
    #[traced_test]
    fn unlink_no_access() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper
                .expect_unlink()
                .returning(|_| Err(io::Error::from_raw_os_error(libc::EACCES)));
            libc_wrapper
        };
        let fs = new_test_fs(libc_wrapper);
        {
            let mut store = fs.store.write().unwrap();
            let entry = OrganizeFSEntry {
                name: "present".into(),
                host_path: "".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
            };
            store.add_entry(entry);
        }
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let parent = PathBuf::from("/");
        let name = std::ffi::OsString::from("present");
        let r = fs.unlink(req, &parent, &name);
        assert_eq!(r.err(), Some(libc::EACCES));
    }
}

