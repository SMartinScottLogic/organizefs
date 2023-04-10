use std::{
    ffi::OsString,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use arena::Arena;
use common::{expand, File, Normalize};
use fuse_mt::{
    CallbackResult, DirectoryEntry, FileAttr, FileType, FilesystemMT, RequestInfo, ResultEmpty,
    ResultEntry, ResultOpen, ResultReaddir, ResultSlice, ResultStatfs, Statfs,
};
use humansize::FormatSize;
use itertools::Itertools;
use tracing::{debug, info, instrument};
use walkdir::WalkDir;

mod libc_wrapper;

#[derive(Debug, Clone, PartialEq)]
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

lazy_static::lazy_static! {
static ref FORMAT: humansize::FormatSizeOptions = humansize::DECIMAL.space_after_value(false).decimal_zeroes(2);
}

static TTL: Duration = Duration::from_secs(1);

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
}

impl Display for OrganizeFSEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({} {})", self.host_path.display(), self.size)
    }
}
#[derive(Debug, Default)]
pub struct OrganizeFS {
    root: PathBuf,
    entries: Vec<OrganizeFSEntry>,
    components: PathBuf,
    arena: Arena<OrganizeFSEntry>,
}

#[derive(Debug)]
enum Component {
    RootDir,
    Normal(OsString),
}
impl<'a> From<std::path::Component<'a>> for Component {
    fn from(component: std::path::Component) -> Self {
        match component {
            std::path::Component::Prefix(_) => todo!(),
            std::path::Component::RootDir => Component::RootDir,
            std::path::Component::CurDir => todo!(),
            std::path::Component::ParentDir => todo!(),
            std::path::Component::Normal(p) => Component::Normal(p.to_os_string()),
        }
    }
}

impl OrganizeFS {
    #[instrument]
    pub fn new(root: &str, pattern: &str, stats: Arc<Mutex<usize>>) -> Self {
        let root = std::env::current_dir().unwrap().as_path().join(root);

        info!(root = debug(&root), "init");
        let entries = Self::scan(&root);
        debug!(root = debug(&root), entry_count = entries.len(), "created");
        {
            let mut s = stats.lock().unwrap();
            *s = entries.len();
        }

        let components = PathBuf::from(&format!("/{pattern}")).normalize();
        let arena = entries.iter().fold(Arena::new(), |mut arena, entry| {
            let mut path = components
                .components()
                .map(|component| expand(&component, entry))
                .fold(PathBuf::new(), |mut acc, c| {
                    acc.push(c);
                    acc
                });
            path.push(&entry.name);
            info!(entry = debug(&entry), path = debug(&path), "add to arena");
            arena.add_file(&path, entry.to_owned()).unwrap();
            arena
        });
        info!(arena = debug(&arena), "arena populated");

        Self {
            root,
            entries,
            components,
            arena,
        }
    }

    #[instrument]
    fn scan(root: &Path) -> Vec<OrganizeFSEntry> {
        info!(root = debug(root), "scanning");
        WalkDir::new(root)
            .sort_by(|a, b| a.file_name().cmp(b.file_name()))
            .into_iter()
            .flatten()
            .filter_map(|entry| Self::process(root, &entry))
            .fold(Vec::new(), |mut acc, e| {
                if acc.len() % 1_000 == 0 {
                    info!(entry = debug(&e), "{}", acc.len());
                }
                acc.push(e);
                acc
            })
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
    fn init(&self, _req: RequestInfo) -> ResultEmpty {
        debug!("init");
        Ok(())
    }

    fn destroy(&self) {
        debug!("destroy");
    }

    fn getattr(&self, req: RequestInfo, path: &Path, fh: Option<u64>) -> ResultEntry {
        info!(req = debug(req), path = debug(path), fh, "getattr");
        if let Some(fh) = fh {
            match libc_wrapper::fstat(fh) {
                Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
            }
        } else {
            let field = self
                .components
                .components()
                .nth(path.components().count() - 1);
            info!(
                components = path.components().count(),
                pattern_components = self.components.components().count(),
                field = debug(field)
            );
            if field.is_some() {
                match libc_wrapper::lstat(&self.root) {
                    Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                }
            } else {
                let children = common::get_child_files(&self.entries, &self.components, path);
                let children = children
                    .iter()
                    .filter(|e| e.name == path.file_name().unwrap())
                    .collect::<Vec<_>>();
                info!(children = debug(&children));
                if children.len() == 1 {
                    let child = children.get(0).unwrap();
                    match libc_wrapper::lstat(&child.host_path) {
                        Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                        Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                    }
                } else {
                    Err(libc::ENOENT)
                }
            }
        }
    }

    fn statfs(&self, req: RequestInfo, path: &Path) -> ResultStatfs {
        debug!(req = debug(req), path = debug(path), "statfs");
        match libc_wrapper::statfs(&self.root) {
            Ok(stat) => Ok(Self::statfs_to_fuse(stat)),
            Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
        }
    }

    fn opendir(&self, req: RequestInfo, path: &Path, flags: u32) -> ResultOpen {
        debug!(
            req = debug(req),
            path = debug(path),
            components = debug(&self.components),
            path_component_count = debug(path.components().count()),
            pattern_count = debug(self.components.components().count()),
            flags,
            "opendir (flags = {:#o})",
            flags
        );
        if path.components().count() <= self.components.components().count() {
            Ok((0, 0))
        } else {
            Err(libc::ENOENT)
        }
    }

    #[instrument(level = "info")]
    fn readdir(&self, req: RequestInfo, path: &Path, fh: u64) -> ResultReaddir {
        let field = self.components.components().nth(path.components().count());
        debug!(
            req = debug(req),
            path = debug(path),
            pattern = debug(&self.components),
            field = debug(field),
            fh,
            "readdir"
        );

        for child in self.arena.find(path).children(&self.arena) {
            debug!(child = debug(child), "arena child");
        }

        let children = common::get_child_files(&self.entries, &self.components, path);
        let children = children
            .iter()
            .map(|c| match field {
                None => (FileType::RegularFile, c.name.clone()),
                Some(component) => (FileType::Directory, expand(&component, c).into()),
            })
            .unique()
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
                    acc.push(DirectoryEntry { name, kind });
                    acc
                },
            );

        debug!(
            req = debug(req),
            path = debug(path),
            children = debug(&children),
            pattern = debug(&self.components),
            field = debug(field),
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
        let children = common::get_child_files(&self.entries, &self.components, path);
        let children = children
            .iter()
            .filter(|e| e.name == path.file_name().unwrap())
            .collect::<Vec<_>>();
        info!(children = debug(&children));
        if children.len() == 1 {
            let child = children.get(0).unwrap();
            match libc_wrapper::open(&child.host_path, flags.try_into().unwrap()) {
                Ok(fh) => Ok((fh as u64, flags)),
                Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
            }
        } else {
            Err(libc::ENOENT)
        }
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
            match libc_wrapper::read(fh.try_into().unwrap(), offset.try_into().unwrap(), size) {
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
            match libc_wrapper::close(fh.try_into().unwrap()) {
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
        let children = common::get_child_files(&self.entries, &self.components, &path);
        let children = children
            .iter()
            .filter(|e| e.name == path.file_name().unwrap())
            .collect::<Vec<_>>();
        info!(children = debug(&children));
        if children.len() == 1 {
            let child = children.get(0).unwrap();
            match libc_wrapper::unlink(&child.host_path) {
                Ok(_) => {
                    // TODO Remove file from entries
                    Ok(())
                }
                Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
            }
        } else {
            Err(libc::ENOENT)
        }
    }
}
