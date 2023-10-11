use std::{ffi::OsString, fmt::{Debug, Display}, ops::{Index, AddAssign}, path::{PathBuf, Path}, time::SystemTime, collections::HashMap};

use arena::{NewArena, Arena, Entry};
use common::{FsFile, DirEntry, Metadata, expand, Normalize};
use file_proc_macro::FsFile;
use humansize::FormatSize;
use time::macros::format_description;
use tracing::{debug, instrument};

#[derive(Debug, Clone, PartialEq, Eq, Hash, FsFile)]
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


lazy_static::lazy_static! {
    static ref FORMAT: humansize::FormatSizeOptions = humansize::DECIMAL.space_after_value(false).decimal_zeroes(2);
}

impl OrganizeFSEntry {
    fn new(root: &Path, entry: &impl DirEntry, meta: &impl Metadata) -> Self {
        debug!(
            root = debug(root.join(entry.path()).normalize()),
            "normalize"
        );
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
type ArenaType = NewArena<Inode>;
type ArenaEntry = <ArenaType as Arena<Inode>>::Entry;
impl OrganizeFSStore {
    #[instrument]
    pub fn new(pattern: PathBuf) -> Self {
        Self {
            pattern: pattern.normalize(),
            arena: ArenaType::default(),
            entries: HashMap::new(),
            max_entries: Inode::from(0),
        }
    }

    #[instrument(level = "debug")]
    fn add_entry(&mut self, entry: OrganizeFSEntry) {
        let id = self.max_entries;
        self.max_entries += 1;
        self.entries.insert(id, entry.clone());

        let local_path = entry.local_path(&self.pattern);
        Self::add_entry_to_arena(&mut self.arena, &local_path, id);
    }

    #[instrument(level = "debug")]
    fn add_entry_to_arena(arena: &mut ArenaType, local_path: &Path, id: Inode) {
        debug!(
            arena = debug(&arena),
            id = debug(&id),
            path = debug(&local_path),
            "add to arena"
        );
        arena.add_file(local_path, id).unwrap();
    }

    #[instrument(level = "debug")]
    fn find(&self, path: &Path) -> ArenaEntry {
        self.arena.find(path)
    }

    #[instrument(level = "debug")]
    fn find_file(&self, path: &Path) -> Option<Inode> {
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

#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone)]
struct Inode {
    value: usize,
}
impl From<usize> for Inode {
    fn from(value: usize) -> Self {
        Self { value }
    }
}
impl AddAssign<usize> for Inode {
    fn add_assign(&mut self, rhs: usize) {
        self.value += rhs;
    }
}

pub struct OrganizeFSStore {
    arena: ArenaType,
    entries: HashMap<Inode, OrganizeFSEntry>,
    max_entries: Inode,
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
            let mut arena = ArenaType::default();
            for (id, entry) in self.entries.iter() {
                let local_path = entry.local_path(&pattern);
                Self::add_entry_to_arena(&mut arena, &local_path, *id);
            }
            self.arena = arena;
            self.pattern = pattern;
        }
    }
}

