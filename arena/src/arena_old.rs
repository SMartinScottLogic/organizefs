use std::{
    cmp::PartialEq,
    ffi::{OsStr, OsString},
    fmt::Debug,
    path::{Component, Path},
};

use indextree_ng::NodeId;
use tracing::{debug, error, info, instrument};

use crate::{Arena, Entry};

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

// #[derive(Debug, Clone, PartialEq, Eq, Hash)]
// pub enum Entry<T> {
//     Root,
//     Directory(OsString),
//     File(OsString, T),
//     None,
// }
// impl<T> Entry<T>
// where
//     T: Debug + PartialEq + Clone,
// {
//     pub fn is_root(&self) -> bool {
//         matches!(*self, Self::Root)
//     }

//     pub fn is_directory(&self, path: &OsStr) -> bool {
//         matches!(self, Self::Directory(p) if p == path)
//     }

//     pub fn is_file(&self, path: &OsStr) -> bool {
//         matches!(self, Self::File(p, _) if p == path)
//     }
// }

#[derive(Debug)]
pub struct FoundEntry<T> {
    node_id: NodeId,
    entry: Entry<T>,
}
impl<T> FoundEntry<T>
where
    T: Debug + PartialEq + Clone + Send + Sync,
{
    fn new(node_id: NodeId, entry: Entry<T>) -> Self {
        info!(node_id = debug(node_id), entry = debug(&entry), "new");
        Self { node_id, entry }
    }

    pub fn children<'a>(&'a self, arena: &'a OldArena<T>) -> Children<T> {
        match &self.entry {
            Entry::Directory(_) => Children::from(arena, Some(self.node_id)),
            Entry::Root => Children::from(arena, Some(arena.root_node)),
            _ => Children::from(arena, None),
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self.entry, Entry::Root | Entry::Directory(_))
    }

    pub fn is_file(&self) -> bool {
        matches!(self.entry, Entry::File(_, _))
    }

    pub fn inner(&self) -> Option<T> {
        if let Entry::File(_, inner) = &self.entry {
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
            Entry::Root => Entry::Root,
            Entry::Directory(d) => Entry::Directory(d.clone()),
            Entry::File(f, t) => Entry::File(f.clone(), mapper(t)),
            Entry::None => Entry::None,
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
    type Item = &'a Entry<T>;

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
    arena: indextree_ng::Arena<Entry<T>>,
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
        let root_node = arena.new_node(Entry::Root);
        Self { arena, root_node }
    }
}

impl<T> Arena<T> for OldArena<T>
where
    T: Debug + PartialEq + Clone + Send + Sync,
{
    #[instrument]
    fn len(&self) -> usize {
        self.arena.len()
    }

    #[instrument]
    fn is_empty(&self) -> bool {
        self.arena.is_empty()
    }

    #[instrument]
    fn add_file(&mut self, file: &Path, entry: T) -> Result<(), ()> {
        match self.add_file_internal(file, entry) {
            UpsertResult::Existing(_) => Ok(()),
            UpsertResult::New(_) => Ok(()),
            UpsertResult::Error(e) => {
                error!(file = debug(file), error = debug(e), "add_file");
                Err(())
            }
        }
    }

    #[instrument]
    fn find(&self, file: &Path) -> Entry<T> {
        info!(file = debug(file), "find");
        match self.find_node(file) {
            Some(node_id) => self.arena[node_id].data.to_owned(),
            None => Entry::None,
        }
        //self.find_node(file).map(|node_id| FoundEntry::new(node_id, self.arena[node_id].data.to_owned()))
    }
}

impl<T> OldArena<T>
where
    T: Debug,
{
    fn add_file_internal(&mut self, file: &Path, entry: T) -> UpsertResult
    where
        T: Clone,
    {
        let mut parent = self.root_node;
        for component in file.parent().unwrap().components() {
            let new_child = match component {
                Component::RootDir => self.root_node,
                Component::CurDir => parent,
                Component::ParentDir => self.arena[parent].parent().unwrap_or(self.root_node),
                Component::Normal(c) => self.upsert(parent, &Entry::Directory(c.into())).unwrap(),
                Component::Prefix(_) => unreachable!(),
            };
            debug!(component = debug(component), new_child = debug(new_child));
            parent = new_child;
        }
        let name = file.file_name().unwrap();
        self.upsert(parent, &Entry::File(name.into(), entry))
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

    fn upsert(&mut self, parent: NodeId, entry: &Entry<T>) -> UpsertResult
    where
        T: Clone,
    {
        match parent
            .children(&self.arena)
            .find(|child| match &self.arena[*child].data {
                Entry::Root => entry.is_root(),
                Entry::Directory(d) => entry.is_directory(d),
                Entry::File(f, _) => entry.is_file(f),
                Entry::None => unreachable!("`Entry::None` present in tree"),
            }) {
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
                Entry::Directory(child_name) | Entry::File(child_name, _) => child_name == name,
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
        arena.upsert(arena.root_node, &Entry::Directory(OsString::from("t")));
        let c = arena
            .upsert(arena.root_node, &Entry::Directory(OsString::from("t")))
            .unwrap();
        arena.upsert(c, &Entry::Directory(OsString::from("2")));
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
        assert!(file.is_file(&OsString::from("file.txt")));
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
        assert!(data.is_directory(&OsString::from("t")));
    }
}
