use std::{
    cmp::PartialEq,
    ffi::{OsStr, OsString},
    fmt::Debug,
    path::{Component, Path},
};

use indextree_ng::NodeId;
use tracing::{debug, error, info, instrument};

use crate::{Arena, ArenaError, Entry};

enum UpsertResult {
    Existing(NodeId),
    New(NodeId),
    Error(indextree_ng::IndexTreeError),
}
impl UpsertResult {
    fn unwrap(self) -> NodeId {
        match self {
            Self::Existing(val) => val,
            Self::New(val) => val,
            Self::Error(_) => panic!("called `UpsertResult::unwrap()` on a `Error` value"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OldEntry<T> {
    Root,
    Directory(OsString),
    File(OsString, T),
    None,
}
impl<T: 'static> Entry for OldEntry<T> {
    type Children<'a> = Children<'a, T>;
    type Arena = OldArena<T>;

    fn is_root(&self) -> bool {
        matches!(*self, Self::Root)
    }

    fn is_directory(&self) -> bool {
        matches!(*self, Self::Root | Self::Directory(_))
    }

    fn is_file(&self) -> bool {
        matches!(*self, Self::File(_, _))
    }

    fn children<'a, 'b>(&'a self, _arena: &'b Self::Arena) -> Self::Children<'b>
    where
        'a: 'b,
    {
        todo!()
    }
}
impl<T> OldEntry<T> {
    fn name(&self) -> Option<&std::ffi::OsString> {
        match self {
            OldEntry::Root => None,
            OldEntry::Directory(n) => Some(n),
            OldEntry::File(n, _) => Some(n),
            OldEntry::None => None,
        }
    }

    fn has_name(&self, name: &OsStr) -> bool {
        match self {
            OldEntry::Root => false,
            OldEntry::Directory(n) => n == name,
            OldEntry::File(n, _) => n == name,
            OldEntry::None => false,
        }
    }
}
#[derive(Debug)]
struct FoundEntry<T> {
    node_id: NodeId,
    entry: OldEntry<T>,
}
impl<T> FoundEntry<T>
where
    T: Debug,
{
    fn new(node_id: NodeId, entry: OldEntry<T>) -> Self {
        info!(node_id = debug(node_id), entry = debug(&entry), "new");
        Self { node_id, entry }
    }

    pub fn children<'a>(&'a self, arena: &'a OldArena<T>) -> Children<T> {
        match &self.entry {
            OldEntry::Directory(_) => Children::from(arena, Some(self.node_id)),
            OldEntry::Root => Children::from(arena, Some(arena.root_node)),
            _ => Children::from(arena, None),
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self.entry, OldEntry::Root | OldEntry::Directory(_))
    }

    pub fn is_file(&self) -> bool {
        matches!(self.entry, OldEntry::File(_, _))
    }

    pub fn inner(&self) -> Option<&T> {
        if let OldEntry::File(_, inner) = &self.entry {
            Some(inner.to_owned())
        } else {
            None
        }
    }
}

impl<T> FoundEntry<T> {
    pub fn map_file<F, U>(&self, mapper: F) -> FoundEntry<U>
    where
        F: Fn(&T) -> U,
        U: Debug + PartialEq + Clone,
    {
        let entry = match &self.entry {
            OldEntry::Root => OldEntry::Root,
            OldEntry::Directory(d) => OldEntry::Directory(d.clone()),
            OldEntry::File(f, t) => OldEntry::File(f.clone(), mapper(t)),
            OldEntry::None => OldEntry::None,
        };
        FoundEntry {
            node_id: self.node_id,
            entry,
        }
    }
}

pub struct Children<'a, T> {
    arena: &'a OldArena<T>,
    node: Option<NodeId>,
}
impl<'a, T> Children<'a, T> {
    fn from(arena: &'a OldArena<T>, node_id: Option<NodeId>) -> Self {
        Self {
            arena,
            node: node_id.and_then(|n| arena.arena[n].first_child()),
        }
    }
}
impl<'a, T> Iterator for Children<'a, T> {
    type Item = &'a OldEntry<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.node.take() {
            Some(node) => {
                self.node = node.following_siblings(&self.arena.arena).nth(1);
                info!(node = debug(node), next = debug(self.node), "next");
                Some(&self.arena.arena[node].data)
            }
            None => None,
        }
    }
}

pub struct OldArena<T> {
    arena: indextree_ng::Arena<OldEntry<T>>,
    root_node: NodeId,
}

impl<T> Debug for OldArena<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Arena")
            .field("arena_len", &self.arena.len())
            .field("root_node", &self.root_node)
            .finish()
    }
}

impl<T> Default for OldArena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> OldArena<T> {
    #[instrument]
    pub fn new() -> Self {
        let mut arena = indextree_ng::Arena::new();
        let root_node = arena.new_node(OldEntry::Root);
        Self { arena, root_node }
    }
}

impl<T> Arena<T> for OldArena<T>
where
    T: Debug + PartialEq + Clone + Send + Sync,
{
    type Entry = OldEntry<T>;

    #[instrument]
    fn len(&self) -> usize {
        self.arena.len()
    }

    #[instrument]
    fn is_empty(&self) -> bool {
        self.arena.is_empty()
    }

    #[instrument]
    fn add_file(&mut self, file: &Path, entry: T) -> Result<(), ArenaError> {
        match self.add_file_internal(file, entry) {
            UpsertResult::Existing(_) => Ok(()),
            UpsertResult::New(_) => Ok(()),
            UpsertResult::Error(e) => {
                error!(file = debug(file), error = debug(e), "add_file");
                Err(ArenaError::Unknown)
            }
        }
    }

    #[instrument]
    fn find(&self, file: &Path) -> OldEntry<T> {
        info!(file = debug(file), "find");
        match self.find_node(file) {
            Some(node_id) => self.arena[node_id].data.to_owned(),
            None => OldEntry::None,
        }
        //self.find_node(file).map(|node_id| FoundEntry::new(node_id, self.arena[node_id].data.to_owned()))
    }
}

impl<T: PartialEq + Clone> OldArena<T> {
    fn add_file_internal(&mut self, file: &Path, entry: T) -> UpsertResult
    where
        T: Debug,
    {
        let mut parent = self.root_node;
        for component in file.parent().unwrap().components() {
            let new_child = match component {
                Component::RootDir => self.root_node,
                Component::CurDir => parent,
                Component::ParentDir => self.arena[parent].parent().unwrap_or(self.root_node),
                Component::Normal(c) => {
                    self.upsert(parent, &OldEntry::Directory(c.into())).unwrap()
                }
                Component::Prefix(_) => unreachable!(),
            };
            debug!(component = debug(component), new_child = debug(new_child));
            parent = new_child;
        }
        let name = file.file_name().unwrap();
        self.upsert(parent, &OldEntry::File(name.into(), entry))
    }

    #[instrument]
    pub(crate) fn find_node(&self, file: &Path) -> Option<NodeId> {
        info!(file = debug(file), "find_node");
        let r = match file.parent() {
            None => Some(self.root_node),
            Some(p) => {
                info!(parent = debug(p), "find_node");
                let mut parent = self.root_node;
                for component in p.components() {
                    let new_child = match component {
                        Component::RootDir => self.root_node,
                        Component::Normal(c) => match self.find_child(c, parent) {
                            Some(child) => child,
                            None => return None,
                        },
                        Component::Prefix(_) => unreachable!(),
                        Component::CurDir => unreachable!(),
                        Component::ParentDir => unreachable!(),
                    };
                    debug!(component = debug(component), new_child = debug(new_child));
                    parent = new_child;
                }
                let name = file.file_name().unwrap();
                self.find_child(name, parent)
            }
        };
        info!(result = debug(&r), "find_node");
        r
    }

    fn upsert(&mut self, parent: NodeId, entry: &OldEntry<T>) -> UpsertResult
    where
        T: Debug,
    {
        match parent.children(&self.arena).find(
            |child| {
                let data = &self.arena[*child].data;
                info!(data = debug(data), entry = debug(entry), "find in upsert");
                if let Some(n) = entry.name() {
                    data.has_name(n)
                } else {
                    false
                }
            }, // {
               //     match &self.arena[*child].data {
               //     Entry::Root => entry.is_root(),
               //     Entry::Directory(d) => entry.is_directory(d),
               //     Entry::File(f, _) => entry.is_file(f),
               //     Entry::None => unreachable!("`Entry::None` present in tree"),
               // }}
        ) {
            Some(c) => UpsertResult::Existing(c),
            None => {
                let new_child = self.arena.new_node((*entry).to_owned());
                match parent.append(new_child, &mut self.arena) {
                    Ok(_) => UpsertResult::New(new_child),
                    Err(e) => UpsertResult::Error(e),
                }
            }
        }
    }

    #[instrument]
    fn find_child(&self, name: &OsStr, parent: NodeId) -> Option<NodeId> {
        parent
            .children(&self.arena)
            .find(|child| match &self.arena[*child].data {
                OldEntry::Directory(child_name) | OldEntry::File(child_name, _) => {
                    child_name == name
                }
                _ => false,
            })
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use tracing_test::traced_test;

    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct TestFile {
        meta: String,
        size: String,
    }
    impl common::File for TestFile {
        fn meta(&self) -> &str {
            todo!()
        }

        fn size(&self) -> &str {
            todo!()
        }
    }

    #[test]
    #[traced_test]
    fn t() {
        let mut arena = OldArena::new();
        let r = arena.add_file_internal(
            &PathBuf::from("/t/file"),
            TestFile {
                meta: "test".into(),
                size: "0".into(),
            },
        );
        assert!(matches!(r, UpsertResult::New(_)));
        let r = arena.add_file_internal(
            &PathBuf::from("/t/file"),
            TestFile {
                meta: "test".into(),
                size: "1".into(),
            },
        );
        assert!(matches!(r, UpsertResult::Existing(_)));
        println!("{arena:#?}");
        assert_eq!(arena.len(), 3);
    }

    #[test]
    #[traced_test]
    fn upsert() {
        let mut arena = OldArena::<TestFile>::new();
        arena.upsert(arena.root_node, &OldEntry::Directory(OsString::from("t")));
        let c = arena
            .upsert(arena.root_node, &OldEntry::Directory(OsString::from("t")))
            .unwrap();
        arena.upsert(c, &OldEntry::Directory(OsString::from("2")));
        println!("{arena:#?}");
        assert_eq!(arena.len(), 3);
    }

    #[test]
    #[traced_test]
    fn find() {
        let mut arena = OldArena::new();
        let file = PathBuf::from("/t/test/file.txt");
        arena
            .add_file(
                &file,
                TestFile {
                    meta: "test".into(),
                    size: "0".into(),
                },
            )
            .unwrap();
        let file = arena.find(&file);
        assert!(file.is_file() && file.has_name(&OsString::from("file.txt")));
    }

    #[test]
    #[traced_test]
    fn find_node_root() {
        let mut arena = OldArena::new();
        let file = PathBuf::from("/t/test/file.txt");
        arena
            .add_file(
                &file,
                TestFile {
                    meta: "test".into(),
                    size: "0".into(),
                },
            )
            .unwrap();

        let file = PathBuf::from("/");
        let file = arena.find_node(&file).unwrap();
        assert_eq!(arena.root_node, file);
    }

    #[test]
    #[traced_test]
    fn find_node_single() {
        let mut arena = OldArena::new();
        let file = PathBuf::from("/t/test/file.txt");
        arena
            .add_file(
                &file,
                TestFile {
                    meta: "test".into(),
                    size: "0".into(),
                },
            )
            .unwrap();

        let file = PathBuf::from("/t");
        let file = arena.find_node(&file).unwrap();
        let data = &arena.arena[file].data;
        assert!(data.is_directory() && data.has_name(&OsString::from("t")));
    }
}
