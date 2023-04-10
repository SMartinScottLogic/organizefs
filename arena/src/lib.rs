use std::{
    cmp::PartialEq,
    ffi::{OsStr, OsString},
    fmt::Debug,
    path::{Component, Path, PathBuf},
};

use indextree_ng::NodeId;
use tracing::{debug, instrument, info};

pub(crate) enum UpsertResult {
    Existing(NodeId),
    New(NodeId),
    Error(indextree_ng::IndexTreeError),
}
impl UpsertResult {
    pub fn is_new(&self) -> bool {
        matches!(*self, Self::New(_))
    }

    pub fn is_existing(&self) -> bool {
        matches!(*self, Self::Existing(_))
    }

    pub fn unwrap(self) -> NodeId {
        match self {
            Self::Existing(val) => val,
            Self::New(val) => val,
            Self::Error(_) => panic!("called `UpsertResult::unwrap()` on a `Error` value"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Entry<T> {
    Root,
    Directory(OsString),
    File(OsString, T),
    None,
}
impl<T> Entry<T>
where
    T: Debug + PartialEq + Clone,
{
    pub fn is_root(&self) -> bool {
        matches!(*self, Self::Root)
    }

    pub fn is_directory(&self, path: &OsStr) -> bool {
        matches!(self, Self::Directory(p) if p == path)
    }

    pub fn is_file(&self, path: &OsStr) -> bool {
        matches!(self, Self::File(p, _) if p == path)
    }

    pub fn children<'a>(&'a self, arena: &'a Arena<T>) -> Children<T> {
        info!(node = debug(self), "children");
        match self {
            Entry::Directory(p) => {
                let path = PathBuf::from(p);
                Children::from(arena, arena.find_node(&path))
            }
            _ => Children::from(arena, None)
        }
    }
}

pub struct Children<'a, T> {
    arena: &'a Arena<T>,
    node: Option<NodeId>,
}
impl <'a, T> Children <'a, T> {
    fn from(arena: &'a Arena<T>, node_id: Option<NodeId>) -> Self {
        Self {
            arena,
            node: node_id.and_then(|n| arena.arena[n].first_child()),
        }
    }
}
impl <'a, T> Iterator for Children<'a, T> {
    type Item = &'a Entry<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.node.take() {
            Some(node) => {
                self.node = node.following_siblings(&self.arena.arena).next();
                Some(&self.arena.arena[node].data)
            }
            None => None
        }
    }
}

#[derive(Debug)]
pub struct Arena<T> {
    arena: indextree_ng::Arena<Entry<T>>,
    root_node: NodeId,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Arena<T> {
    #[instrument]
    pub fn new() -> Self {
        let mut arena = indextree_ng::Arena::new();
        let root_node = arena.new_node(Entry::Root);
        Self { arena, root_node }
    }
}

impl<T> Arena<T>
where
    T: Debug + PartialEq + Clone,
{
    #[instrument]
    pub fn len(&self) -> usize {
        self.arena.len()
    }

    #[instrument]
    pub fn is_empty(&self) -> bool {
        self.arena.is_empty()
    }

    pub fn add_file(&mut self, file: &Path, entry: T) -> Result<(), ()>{
        match self.add_file_internal(file, entry) {
            UpsertResult::Existing(_) => Ok(()),
            UpsertResult::New(_) => Ok(()),
            UpsertResult::Error(e) => Err(()),        
        }
    }
    
    fn add_file_internal(&mut self, file: &Path, entry: T) -> UpsertResult {
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
    pub fn find(&self, file: &Path) -> Entry<T> {
        info!(file = debug(file), "find");
        self.find_node(file)
            .map(|c| self.arena[c].data.to_owned())
            .unwrap_or(Entry::None)
    }

    #[instrument]
    pub(crate) fn find_node(&self, file: &Path) -> Option<NodeId> {
        let mut parent = self.root_node;
        for component in file.parent().unwrap().components() {
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

    fn upsert(&mut self, parent: NodeId, entry: &Entry<T>) -> UpsertResult {
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
                let new_child = self.arena.new_node(entry.to_owned());
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
        let mut arena = Arena::new();
        assert!(arena
            .add_file_internal(
                &PathBuf::from("/t/file"),
                TestFile {
                    meta: "test".into(),
                    size: "0".into()
                }
            )
            .is_new());
        assert!(arena
            .add_file_internal(
                &PathBuf::from("/t/file"),
                TestFile {
                    meta: "test".into(),
                    size: "1".into()
                }
            )
            .is_existing());
        println!("{arena:#?}");
        assert_eq!(arena.len(), 3);
    }

    #[test]
    #[traced_test]
    fn upsert() {
        let mut arena = Arena::<TestFile>::new();
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
        let mut arena = Arena::new();
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
}
