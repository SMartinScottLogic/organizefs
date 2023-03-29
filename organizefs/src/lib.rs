use std::{
    ffi::OsString,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use fuse_mt::{
    DirectoryEntry, FileAttr, FileType, FilesystemMT, RequestInfo, ResultEmpty, ResultEntry,
    ResultOpen, ResultReaddir, ResultStatfs, Statfs,
};
use humansize::FormatSize;
use itertools::Itertools;
use tracing::{debug, error, info};
use walkdir::WalkDir;
use common::Normalize;

mod libc_wrapper;

#[derive(Debug)]
struct OrganizeFSEntry {
    name: OsString,
    host_path: PathBuf,
    size: String,
    mime: String,
}

lazy_static::lazy_static! {
static ref FORMAT: humansize::FormatSizeOptions = humansize::DECIMAL.space_after_value(false).decimal_zeroes(2);
}

static TTL: Duration = Duration::from_secs(1);

impl OrganizeFSEntry {
    fn new(root: &Path, entry: &walkdir::DirEntry, meta: &fs::Metadata) -> Self {
        let cookie =
            magic::Cookie::open(magic::CookieFlags::ERROR | magic::CookieFlags::MIME_TYPE).unwrap();
        cookie.load::<&str>(&[]).unwrap();

        let host_path = root.join(entry.path()).canonicalize().unwrap();
        let size = meta.len().format_size(*FORMAT);
        let mime = cookie
            .file(&host_path)
            .unwrap_or_default()
            .replace('/', "_");
        let name = entry.file_name().to_os_string();

        info!(
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
    components: Vec<Component>,
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
    pub fn new(root: &str, pattern: &str) -> Self {
        let root = std::env::current_dir().unwrap().as_path().join(root);
        info!(root = debug(&root), "init");
        let entries = Self::scan(&root);
        debug!(root = debug(&root), entries = debug(&entries), "created");

        let components = PathBuf::from(&format!("/{pattern}"))
            .normalize()
            .components()
            .map(Component::from)
            .collect();
        Self {
            root,
            entries,
            components,
        }
    }

    fn scan(root: &Path) -> Vec<OrganizeFSEntry> {
        info!(root = debug(root), "scanning");
        WalkDir::new(root)
            .into_iter()
            .flatten()
            .filter_map(|entry| Self::process(root, &entry))
            .collect()
    }

    fn process(root: &Path, entry: &walkdir::DirEntry) -> Option<OrganizeFSEntry> {
        if let Ok(meta) = fs::metadata(entry.path()) {
            if meta.is_file() && entry.path().parent().is_some() {
                debug!(root = debug(root), entry = debug(entry), "found");
                let entry = OrganizeFSEntry::new(root, entry, &meta);
                info!(root = debug(root), entry = display(&entry));
                return Some(entry);
            };
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

    fn stat_to_filetype(stat: &libc::stat) -> FileType {
        Self::mode_to_filetype(stat.st_mode)
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
            info!(components = path.components().count());
            if path.components().count() <= 3 {
                match libc_wrapper::lstat(&self.root) {
                    Ok(stat) => Ok((TTL, Self::stat_to_fuse(stat))),
                    Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
                }
            } else {
                Err(libc::ENOENT)
            }
            // match self.stat_real(path) {
            //     Ok(attr) => Ok((TTL, attr)),
            //     Err(e) => Err(e.raw_os_error().unwrap_or(libc::ENOENT))
            // }
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
            flags,
            "opendir (flags = {:#o})",
            flags
        );
        if path.components().count() <= 3 {
            Ok((0, 0))
        } else {
            Err(libc::ENOENT)
        }
    }

    fn readdir(&self, req: RequestInfo, path: &Path, fh: u64) -> ResultReaddir {
        debug!(req = debug(req), path = debug(path), fh, "readdir");
        for (id, component) in path.components().enumerate() {
            debug!(
                actual = debug(component),
                pattern = debug(self.components.get(id)),
                "lookup"
            )
        }
        match path.components().count() {
            1 => {
                let entries = self
                    .entries
                    .iter()
                    .map(|e| e.mime.to_owned())
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
                        |mut acc, name| {
                            acc.push(DirectoryEntry {
                                name: name.into(),
                                kind: FileType::Directory,
                            });
                            acc
                        },
                    );
                Ok(entries)
            }
            2 => {
                let path_name = path.file_name().unwrap().to_string_lossy();
                let entries = self
                    .entries
                    .iter()
                    .filter(|e| e.mime == path_name)
                    .map(|e| e.size.to_owned())
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
                        |mut acc, name| {
                            acc.push(DirectoryEntry {
                                name: name.into(),
                                kind: FileType::Directory,
                            });
                            acc
                        },
                    );
                Ok(entries)
            }
            _ => Err(libc::ENOENT),
        }
        // let real = self.real_path(path);
        // debug!("readdir: {:?} {:?}", path, real);
        // let mut entries: Vec<DirectoryEntry> = vec![];
        // // Consider using libc::readdir to prevent need for always stat-ing entries
        // let iter = match fs::read_dir(&real) {
        //     Ok(iter) => iter,
        //     Err(e) => return Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
        // };
        // for entry in iter {
        //     match entry {
        //         Ok(entry) => {
        //             let real_path = entry.path();
        //             debug!("readdir: {:?} {:?}", real, real_path);
        //             let stat = match libc_wrapper::lstat(&real_path) {
        //                 Ok(stat) => stat,
        //                 Err(e) => return Err(e.raw_os_error().unwrap_or(libc::ENOENT)),
        //             };
        //             let filetype = Self::stat_to_filetype(&stat);

        //             entries.push(DirectoryEntry {
        //                 name: real_path.file_name().unwrap().to_os_string(),
        //                 kind: filetype,
        //             });
        //         }
        //         Err(e) => {
        //             error!("readdir: {:?}: {}", path, e);
        //             return Err(e.raw_os_error().unwrap_or(libc::ENOENT));
        //         }
        //     }
        // }
        // info!("entries: {:?}", entries);
        // Ok(entries)
    }

    fn releasedir(&self, req: RequestInfo, path: &Path, fh: u64, flags: u32) -> ResultEmpty {
        debug!(
            req = debug(req),
            path = debug(path),
            fh,
            "opendir (flags = {:#o})",
            flags
        );
        Ok(())
    }
}
