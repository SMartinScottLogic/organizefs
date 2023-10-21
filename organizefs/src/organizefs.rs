//use common::{DirEntry, Metadata};
//use arena::{Arena, Entry, NewArena};
use crate::libc_wrapper::{LibcWrapper, LibcWrapperReal};
use common::{FsFile, Normalize, expand};
use file_proc_macro::FsFile;
use humansize::FormatSize;
use store::{StorageEntry, TreeStorage, PatternLocalPath};
// use store::{Entry, StoragePath};
// use store::{OrganizeFSEntry, OrganizeFSStore};
//use file_proc_macro::FsFile;
use fuse_mt::{
    CallbackResult, DirectoryEntry, FileAttr, FileType, FilesystemMT, RequestInfo, ResultEmpty,
    ResultEntry, ResultOpen, ResultReaddir, ResultSlice, ResultStatfs, Statfs,
};
use std::ffi::OsString;
use std::ops::Index;
//use humansize::FormatSize;
//use std::collections::HashMap;
use std::fmt::{Debug, Display};
//use std::ops::{AddAssign, Index};
use std::{
    //    ffi::OsString,
    //    fmt::Display,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
use time::macros::format_description;
use tracing::{debug, info, instrument};
use walkdir::WalkDir;

static TTL: Duration = Duration::from_secs(1);

lazy_static::lazy_static! {
    static ref FORMAT: humansize::FormatSizeOptions = humansize::DECIMAL.space_after_value(false).decimal_zeroes(2);
}

#[derive(Debug, Clone, FsFile)]
pub struct OrganizeFSEntry {
    name: OsString,
    host_path: PathBuf,
    #[fsfile = "size"]
    size: String,
    #[fsfile = "meta"]
    mime: String,
    #[fsfile = "mdate"]
    modified_date: String,
}
impl Display for OrganizeFSEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({} {})", self.host_path.display(), self.size)
    }
}
impl PatternLocalPath for OrganizeFSEntry {
    fn new(root: &Path, entry: &dyn common::DirEntry, meta: &dyn common::Metadata) -> Self {
        debug!(root = debug(root), entry = debug(entry), meta = debug(meta), "new");
        let host_path = root.join(entry.path()).normalize();
        let size = meta.len().format_size(*FORMAT);
        let mime = tree_magic_mini::from_filepath(&host_path)
            .unwrap_or_default()
            .replace('/', "_");
        let name = entry.file_name().to_os_string();
        let modified_date: time::OffsetDateTime =
            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH).into();
        let modified_date = modified_date
            .format(format_description!("[year]-[month]-[day]"))
            .unwrap_or_else(|_| "1970-01-01".to_string());

        debug!(
            root = debug(root),
            entry = debug(entry),
            meta = debug(meta),
            path = debug(&host_path),
            size,
            mime,
            modified_date
        );
        Self {
            host_path,
            name,
            size,
            mime,
            modified_date,
        }
    }
    
    fn local_path(&self, pattern: &Path) -> PathBuf {
        debug!(self = debug(self), pattern = debug(pattern), "local_path");
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

    fn host_path(&self) -> PathBuf {
        self.host_path.clone()
    }
}
pub struct OrganizeFS<E> {
    root: PathBuf,
    store: Arc<parking_lot::RwLock<TreeStorage<E>>>,
    libc_wrapper: Box<dyn LibcWrapper + Send + Sync>,
    shutdown_signal: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
impl<E> Debug for OrganizeFS<E>
where
    E: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrganizeFS")
            .field("root", &self.root)
            //.field("store", &self.store)
            .finish()
    }
}

impl<E> OrganizeFS<E>
where
    E: Debug + Display + Clone + PatternLocalPath,
{
    #[instrument]
    pub fn new(
        root: &str,
        store: Arc<parking_lot::RwLock<TreeStorage<E>>>,
        shutdown_signal: tokio::sync::oneshot::Sender<()>,
    ) -> Self {
        let root = std::env::current_dir().unwrap().as_path().join(root);
        {
            let mut store = store.write();
            info!(root = debug(&root), "init");
            for entry in Self::scan(&root) {
                store.add_entry(entry);
            }
            info!(store = store.len(), "store populated");
        }

        Self {
            root,
            store,
            shutdown_signal: Mutex::new(Some(shutdown_signal)),
            libc_wrapper: Box::new(LibcWrapperReal::new()),
        }
    }

    #[instrument]
    fn scan(root: &Path) -> impl Iterator<Item = E> + '_ {
        info!(root = debug(root), "scanning");
        WalkDir::new(root)
            .sort_by(|a, b| a.file_name().cmp(b.file_name()))
            .into_iter()
            .flatten()
            .filter_map(|entry| Self::process(root, &entry))
    }

    #[instrument(level = "debug")]
    fn process(root: &Path, entry: &walkdir::DirEntry) -> Option<E> {
        if entry.file_type().is_file() && entry.path().parent().is_some() {
            if let Ok(meta) = fs::symlink_metadata(entry.path()) {
                debug!(root = debug(root), entry = debug(entry), "found");
                let entry = E::new(root, entry, &meta);
                debug!(root = debug(root), entry = display(&entry));
                return Some(entry);
            }
        }
        None
    }
}

impl<E> OrganizeFS<E> {
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

impl<E> FilesystemMT for OrganizeFS<E>
where
    E: Debug + Clone + PatternLocalPath,
{
    fn init(&self, req: RequestInfo) -> ResultEmpty {
        info!(req = debug(req), "init");
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
        debug!(req = debug(req), path = debug(path), fh, "getattr");
        if let Some(fh) = fh {
            match self.libc_wrapper.fstat(fh) {
                Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
            }
        } else {
            let store = self.store.read();
            let r = store.find(path);
            debug!(found = debug(&r), "found");
            match r {
                Some(e) if e.is_directory() => match self.libc_wrapper.lstat(&self.root) {
                    Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                },
                Some(e) if e.is_file() => match self.libc_wrapper.lstat(&e.host_path()) {
                    Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                },
                _ => Err(libc::ENOENT),
            }
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
        let store = self.store.read();
        match store.find(path) {
            Some(entry) if entry.is_directory() => Ok((0, 0)),
            _ => Err(libc::ENOENT),
        }
    }

    #[instrument(level = "info")]
    fn readdir(&self, req: RequestInfo, path: &Path, fh: u64) -> ResultReaddir {
        debug!(req = debug(req), path = debug(path), fh, "readdir");

        let store = self.store.read();
        let entry = store.find(path);
        if entry.is_none() {
            return Err(libc::ENOENT);
        }
        let entry = entry.unwrap();
        let children = entry
            .children()
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
        // let children = store
        //         .find_dir(path)
        //         .unwrap()
        //         .children(&store.arena)
        //         //.unique()
        //         .filter_map(|(name, entry)| {
        //             //let entry = store.entries.get(id).unwrap();
        //             info!(
        //                 path = debug(&path),
        //                 name = debug(&name),
        //                 entry = debug(&entry),
        //                 "child"
        //             );
        //             if entry.is_directory() {
        //                 Some((FileType::Directory, name))
        //             } else if entry.is_file() {
        //                 Some((FileType::RegularFile, name))
        //             } else {
        //                 None
        //             }
        //         })
        //         .fold(
        //             vec![
        //                 DirectoryEntry {
        //                     name: ".".into(),
        //                     kind: FileType::Directory,
        //                 },
        //                 DirectoryEntry {
        //                     name: "..".into(),
        //                     kind: FileType::Directory,
        //                 },
        //             ],
        //             |mut acc, (kind, name)| {
        //                 acc.push(DirectoryEntry {
        //                     name: name.clone(),
        //                     kind,
        //                 });
        //                 acc
        //             },
        //         );

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
        let store = self.store.read();
        match store.find(path) {
            Some(d) if d.is_file() => {
                match self
                    .libc_wrapper
                    .open(&d.host_path(), flags.try_into().unwrap())
                {
                    Ok(fh) => Ok((fh as u64, flags)),
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                }
            }
            _ => Err(libc::ENOENT),
        }
        // store.find_file(path).map_or_else(
        //     || Err(libc::ENOENT),
        //     |e| {
        //         let entry = store.entries.get(&e).unwrap();
        //         match self
        //             .libc_wrapper
        //             .open(entry.host_path.to_owned(), flags.try_into().unwrap())
        //         {
        //             Ok(fh) => Ok((fh as u64, flags)),
        //             Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
        //         }
        //     },
        // )
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

        let mut store = self.store.write();
        match store.find(&path) {
            Some(d) if d.is_file() => {
                info!(entry = debug(&d), "get");
                match self.libc_wrapper.unlink(&d.host_path()) {
                    Ok(_) => {
                        info!("unlinked");
                        if store.remove(&path) {
                            info!(dropped = debug(&path), "dropped");
                        }
                        Ok(())
                    }
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                }
            }
            _ => Err(libc::ENOENT),
        }
        // store.find_file(&path).map_or_else(
        //     || Err(libc::ENOENT),
        //     |e| {
        //         let entry = store.entries.get(&e).unwrap().to_owned();
        //         info!(inode = debug(e), entry = debug(&entry), "get");
        //         match self.libc_wrapper.unlink(entry.host_path) {
        //             Ok(_) => {
        //                 info!("unlinked");
        //                 if store.arena.remove(&path) {
        //                     let dropped = store.entries.remove(&e);
        //                     info!(dropped = debug(dropped), "dropped");
        //                 }
        //                 Ok(())
        //             }
        //             Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
        //         }
        //     },
        // )
    }

    fn rename(
        &self,
        req: RequestInfo,
        parent: &Path,
        name: &std::ffi::OsStr,
        newparent: &Path,
        newname: &std::ffi::OsStr,
    ) -> ResultEmpty {
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
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    use std::{ffi::OsString, io, ops::Index, path::PathBuf};

    use file_proc_macro::FsFile;
    use store::PatternLocalPath;
    use tracing_test::traced_test;

    use libc_wrapper::MockLibcWrapper;

    use crate::libc_wrapper;
    use common::{expand, FsFile, MockDirEntry, MockMetadata};

    #[derive(Debug, Clone, PartialEq, Eq, Hash, FsFile)]
    struct TestEntry {
        name: OsString,
        #[fsfile = "size"]
        size: String,
        #[fsfile = "meta"]
        mime: String,
        #[fsfile = "mdate"]
        modified_date: String,
    }
    impl PatternLocalPath for TestEntry {
        #[instrument]
        fn new(root: &Path, entry: &dyn common::DirEntry, meta: &dyn common::Metadata) -> Self {
            todo!()
        }

        #[instrument]
        fn local_path(&self, pattern: &Path) -> PathBuf {
            debug!(self = debug(self), pattern = debug(pattern), "local_path");
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

        #[instrument]
        fn host_path(&self) -> PathBuf {
            PathBuf::from("/").join(&self.name)
        }
    }

    #[instrument(ret, skip(libc_wrapper))]
    fn new_test_fs(
        libc_wrapper: impl LibcWrapper + Send + Sync + 'static,
    ) -> OrganizeFS<TestEntry> {
        let root = PathBuf::from("/");
        let pattern = PathBuf::from("/");
        let store = Arc::new(parking_lot::RwLock::new(TreeStorage::<TestEntry>::new(
            pattern,
        )));
        let libc_wrapper = Box::new(libc_wrapper);
        OrganizeFS {
            root,
            store,
            libc_wrapper,
            shutdown_signal: Mutex::new(None),
        }
    }

    // #[test]
    // #[traced_test]
    // fn organize_fsentry_new() {
    //     let root = PathBuf::from("/test/data/path");
    //     let entry = {
    //         let mut entry = MockDirEntry::new();
    //         entry.expect_path().return_const(PathBuf::from("path/"));
    //         entry
    //             .expect_file_name()
    //             .return_const(OsString::from("file"));
    //         entry
    //     };
    //     let meta = {
    //         let mut metadata = MockMetadata::new();
    //         metadata
    //             .expect_len()
    //             .return_const(1024_u64 * 1024 * 1024 * 100);
    //         metadata.expect_modified().returning(|| {
    //             Ok(SystemTime::UNIX_EPOCH + Duration::from_secs(40 * 365 * 24 * 60 * 60))
    //         });
    //         metadata
    //     };
    //     let entry = OrganizeFSEntry::new(&root, &entry, &meta);
    //     assert_eq!(entry.size, "107.37GB");
    //     assert_eq!(entry.name, "file");
    //     assert_eq!(entry.host_path, PathBuf::from("/test/data/path/path"));
    //     assert_eq!(entry.modified_date, "2009-12-22");
    //     assert_eq!(entry.mime, "");
    // }

    #[test]
    #[traced_test]
    fn mode_to_filetype() {
        assert_eq!(
            OrganizeFS::<TestEntry>::mode_to_filetype(libc::S_IFDIR + 0o644),
            FileType::Directory
        );
        assert_eq!(
            OrganizeFS::<TestEntry>::mode_to_filetype(libc::S_IFREG + 0o644),
            FileType::RegularFile
        );
        assert_eq!(
            OrganizeFS::<TestEntry>::mode_to_filetype(libc::S_IFLNK + 0o644),
            FileType::Symlink
        );
        assert_eq!(
            OrganizeFS::<TestEntry>::mode_to_filetype(libc::S_IFBLK + 0o644),
            FileType::BlockDevice
        );
        assert_eq!(
            OrganizeFS::<TestEntry>::mode_to_filetype(libc::S_IFCHR + 0o644),
            FileType::CharDevice
        );
        assert_eq!(
            OrganizeFS::<TestEntry>::mode_to_filetype(libc::S_IFIFO + 0o644),
            FileType::NamedPipe
        );
        assert_eq!(
            OrganizeFS::<TestEntry>::mode_to_filetype(libc::S_IFSOCK + 0o644),
            FileType::Socket
        );
    }

    #[test]
    #[traced_test]
    #[should_panic(expected = "unknown file type")]
    fn mode_to_filetype_err() {
        OrganizeFS::<TestEntry>::mode_to_filetype(0);
    }

    #[test]
    #[traced_test]
    fn get_pattern() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let store = fs.store.read();
        assert_eq!("/", store.get_pattern());
    }

    #[test]
    #[traced_test]
    fn set_pattern() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        // Add file
        {
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "present".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        // Alter pattern
        {
            let mut store = fs.store.write();
            store.set_pattern("/s/../t/{meta}/");
        }
        let store = fs.store.read();
        assert_eq!("/t/{meta}", store.get_pattern());
        assert_eq!(store.len(), 1);
        let entry = store.find(&PathBuf::from("/t/text_plain/present"));
        assert!(entry.is_some_and(|e| e.is_file()));
    }

    // init tests
    #[test]
    #[traced_test]
    fn init() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };

        assert!(fs.init(req).is_ok());
    }

    // destroy tests
    #[test]
    #[traced_test]
    fn destroy_nosignal() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        fs.destroy();
    }

    #[test]
    #[traced_test]
    fn destroy_signal() {
        let libc_wrapper = MockLibcWrapper::new();

        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let mut fs = new_test_fs(libc_wrapper);
        fs.shutdown_signal = Mutex::new(Some(tx));
        fs.destroy();
        assert!(rx.try_recv().is_ok());
    }

    // statfs tests
    #[test]
    #[traced_test]
    fn statfs_ok() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper.expect_statfs().returning(|_| {
                let mut s = std::mem::MaybeUninit::<libc::statfs>::zeroed();
                let stat = unsafe { s.assume_init_mut() };
                stat.f_blocks = 1024;
                stat.f_bfree = 512;
                stat.f_bavail = 500;
                stat.f_files = 2048;
                stat.f_ffree = 1000;
                stat.f_bsize = 4096;
                stat.f_namelen = 256;
                Ok(stat.to_owned())
            });
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let resp = fs.statfs(req, &PathBuf::from("/"));
        assert!(resp.is_ok());
    }

    #[test]
    #[traced_test]
    fn statfs_err() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper
                .expect_statfs()
                .returning(|_| Err(io::Error::from_raw_os_error(libc::EACCES)));
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let resp = fs.statfs(req, &PathBuf::from("/"));
        assert_eq!(resp.err(), Some(libc::EACCES));
    }

    // opendir tests
    #[test]
    #[traced_test]
    fn opendir_present() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        {
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "test".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        let resp = fs.opendir(
            req,
            &PathBuf::from("/"),
            libc::O_DIRECTORY.try_into().unwrap(),
        );
        info!(resp = debug(resp), "resp");
        assert!(resp.is_ok());
        assert_eq!(resp.unwrap(), (0, 0));
    }

    #[test]
    #[traced_test]
    fn opendir_missing() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        {
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "test".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        let resp = fs.opendir(
            req,
            &PathBuf::from("/missing"),
            libc::O_DIRECTORY.try_into().unwrap(),
        );
        assert_eq!(resp.err(), Some(libc::ENOENT));
    }

    // releasedir tests
    #[test]
    #[traced_test]
    fn releasedir() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let fh = 1;
        let resp = fs.releasedir(req, &PathBuf::from("/test"), fh, 0);
        assert!(resp.is_ok());
    }

    // getattr tests
    #[test]
    #[traced_test]
    fn getattr_withfh_err() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper
                .expect_fstat()
                .returning(|_| Err(io::Error::from_raw_os_error(libc::EACCES)));
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let fh = 1;
        let resp = fs.getattr(req, &PathBuf::from("/test"), Some(fh));
        assert_eq!(resp.err(), Some(libc::EACCES));
    }

    #[test]
    #[traced_test]
    fn getattr_withfh_ok() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper.expect_fstat().returning(|_| {
                let mut s = std::mem::MaybeUninit::<libc::stat>::zeroed();
                let stat = unsafe { s.assume_init_mut() };
                stat.st_mode = libc::S_IFREG + 0o0644;
                stat.st_size = 5;
                stat.st_nlink = 1;
                Ok(stat.to_owned())
            });
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let fh = 1;
        let resp = fs.getattr(req, &PathBuf::from("/test"), Some(fh));
        assert!(resp.is_ok());
    }

    #[test]
    #[traced_test]
    fn getattr_nofh_missing() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let resp = fs.getattr(req, &PathBuf::from("/test"), None);
        assert_eq!(resp.err(), Some(libc::ENOENT));
    }

    #[test]
    #[traced_test]
    fn getattr_nofh_file_err() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper
                .expect_lstat()
                .returning(|_| Err(io::Error::from_raw_os_error(libc::EACCES)));
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        {
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "test".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        let resp = fs.getattr(req, &PathBuf::from("/test"), None);
        assert_eq!(resp.err(), Some(libc::EACCES));
    }

    #[test]
    #[traced_test]
    fn getattr_nofh_file_ok() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper.expect_lstat().returning(|_| {
                let mut s = std::mem::MaybeUninit::<libc::stat>::zeroed();
                let stat = unsafe { s.assume_init_mut() };
                stat.st_mode = libc::S_IFREG + 0o0644;
                stat.st_size = 5;
                stat.st_nlink = 1;
                Ok(stat.to_owned())
            });
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        {
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "test".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        let resp = fs.getattr(req, &PathBuf::from("/test"), None);
        assert!(resp.is_ok());
    }

    #[test]
    #[traced_test]
    fn getattr_nofh_dir_err() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper
                .expect_lstat()
                .returning(|_| Err(io::Error::from_raw_os_error(libc::EACCES)));
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        {
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "test".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        let resp = fs.getattr(req, &PathBuf::from("/"), None);
        assert_eq!(resp.err(), Some(libc::EACCES));
    }

    #[test]
    #[traced_test]
    fn getattr_nofh_dir_ok() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper.expect_lstat().returning(|_| {
                let mut s = std::mem::MaybeUninit::<libc::stat>::zeroed();
                let stat = unsafe { s.assume_init_mut() };
                stat.st_mode = libc::S_IFDIR + 0o0755;
                stat.st_size = 5;
                stat.st_nlink = 1;
                Ok(stat.to_owned())
            });
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        {
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "test".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        let resp = fs.getattr(req, &PathBuf::from("/"), None);
        assert!(resp.is_ok());
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
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "present".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
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
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "present".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
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

    // flush tests
    #[test]
    #[traced_test]
    fn flush_unimplemented() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let path = PathBuf::from("/missing");
        let r = fs.flush(req, &path, 0, 0);
        assert_eq!(r.err(), Some(libc::ENOSYS));
    }

    // release tests
    #[test]
    #[traced_test]
    fn release_no_filehandle() {
        let libc_wrapper = MockLibcWrapper::new();

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let path = PathBuf::from("/missing");
        let r = fs.release(req, &path, 0, 0, 0, true);
        assert_eq!(r.err(), Some(libc::ENOENT));
    }

    #[test]
    #[traced_test]
    fn release_error() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper
                .expect_close()
                .returning(|_| Err(io::Error::from_raw_os_error(libc::EACCES)));
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let path = PathBuf::from("/missing");
        let r = fs.release(req, &path, 1, 0, 0, true);
        assert_eq!(r.err(), Some(libc::EACCES));
    }

    #[test]
    #[traced_test]
    fn release_ok() {
        let libc_wrapper = {
            let mut libc_wrapper = MockLibcWrapper::new();
            libc_wrapper.expect_close().returning(|_| Ok(()));
            libc_wrapper
        };

        let fs = new_test_fs(libc_wrapper);
        let req: RequestInfo = RequestInfo {
            unique: 0,
            pid: 0,
            gid: 0,
            uid: 0,
        };
        let path = PathBuf::from("/missing");
        let r = fs.release(req, &path, 1, 0, 0, true);
        assert!(r.is_ok());
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
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "present".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        {
            let store = fs.store.read();
            assert_eq!(store.len(), 1);
            assert!(!store.is_empty());
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
        {
            let store = fs.store.read();
            assert_eq!(store.len(), 0);
            assert!(store.is_empty());
        }
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
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "present".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
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

    // rename tests
    // TODO Proper implementation
    #[test]
    #[traced_test]
    fn rename_unimplemented() {
        let libc_wrapper = MockLibcWrapper::new();
        let fs = new_test_fs(libc_wrapper);
        {
            let mut store = fs.store.write();
            let entry = TestEntry {
                name: "present".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
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
        let newparent = PathBuf::from("/");
        let newname = std::ffi::OsString::from("missing");
        let r = fs.rename(req, &parent, &name, &newparent, &newname);
        assert_eq!(r.err(), Some(libc::ENOSYS));
    }

    #[test]
    #[traced_test]
    fn test() {
        let mut store = TreeStorage::new("/{meta}/{size}/".into());
        {
            let entry = TestEntry {
                name: "present".into(),
                size: "0 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        {
            let entry = TestEntry {
                name: "present".into(),
                size: "10 B".into(),
                mime: "text_plain".into(),
                modified_date: "2023-08-04".into(),
            };
            store.add_entry(entry);
        }
        assert_eq!(store.len(), 2);
        assert!(store.find(&PathBuf::from("/text_plain/10 B/present")).is_some());
        assert!(store.find(&PathBuf::from("/text_plain/0 B/present")).is_some());
        assert_eq!(store.node_count(), 6);
    }    
}
