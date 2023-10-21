#![warn(missing_docs)]
//! Definition of storage types for representations of hierarchical tree.
 
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fmt::Debug,
    path::{Path, PathBuf},
};

use common::{DirEntry, Metadata, Normalize};
use tracing::{debug, instrument, Value, error};

/// A trait used to define types which have both local and host paths.
pub trait PatternLocalPath {
    /// Standard constructor
    fn new(root: &Path, entry: &dyn DirEntry, meta: &dyn Metadata) -> Self;
    /// Retrieve the *local path* - the path to this entry, based on the supplied pattern
    fn local_path(&self, pattern: &Path) -> PathBuf;
    /// Retrieve the *host path* - the path to this entry, in the backing store
    fn host_path(&self) -> PathBuf;
}

#[derive(Debug)]
pub struct StorageEntry<'a, E> {
    node_id: usize,
    nodes: &'a HashMap<usize, Node<E>>,
}
impl <'a, E> StorageEntry<'a, E>
where
E: Debug + PatternLocalPath {
    pub fn is_directory(&self) -> bool {
        self.nodes.get(&self.node_id).filter(|n| matches!(n, Node::Branch(_))).is_some()
    }
    pub fn is_file(&self) -> bool {
        self.nodes.get(&self.node_id).filter(|n| matches!(n, Node::Leaf(_))).is_some()
    }
    pub fn host_path(&self) -> PathBuf {
        self.nodes.get(&self.node_id)
        .and_then(|n| match n {
            Node::Leaf(e) => Some(e),
            _ => None
        })
        .map(|e| e.host_path())
        .unwrap()
    }
    pub fn children(&self) -> Children<E> {
        let children = self.nodes.get(&self.node_id)
        .and_then(|n| if let Node::Branch(c) = n {Some(c)}else {None});
        Children::from((self.nodes, children))
    }
}

pub struct Children<'a, E> {
    nodes: &'a HashMap<usize, Node<E>>,
    children: Option<std::collections::hash_map::Iter<'a, OsString, usize>>,
}
impl<'a, E: 'a> Iterator for Children<'a, E> {
    type Item = (OsString, StorageEntry<'a, E>);

    fn next(&mut self) -> Option<Self::Item> {
        self.children.as_mut().and_then(|iter| {
            if let Some((name, idx)) = iter.next() {
                Some((name.to_owned(), StorageEntry { node_id: *idx, nodes: self.nodes }))
            } else {
                None
            }
        })
    }
}
impl <'a, E: Debug> From<(&'a std::collections::HashMap<usize, Node<E>>, std::option::Option<&'a std::collections::HashMap<std::ffi::OsString, usize>>)> for Children<'a, E> {
    fn from((nodes, children): (&'a std::collections::HashMap<usize, Node<E>>, std::option::Option<&'a std::collections::HashMap<std::ffi::OsString, usize>>)) -> Self {
        error!(children = debug(children), nodes = debug(nodes), "construct child iterator");
        Self { nodes, children: children.map(|c| c.into_iter()) }
    }
}

#[derive(Debug)]
enum Node<E> {
    Branch(HashMap<OsString, usize>),
    Leaf(E),
}

pub struct TreeStorage<E> {
    pattern: PathBuf,
    nodes: HashMap<usize, Node<E>>,
}
impl <E> Debug for TreeStorage<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeStorage")
        .field("pattern", &self.pattern)
        .field("nodes", &self.nodes.len())
        .finish()
    }
}
impl<E> TreeStorage<E>
where
    E: Debug + Clone + PatternLocalPath,
{
    /// Initialize a new `TreeStorage` with an initial pattern for use for local path generation in its entry.
    #[instrument]
    pub fn new(pattern: PathBuf) -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(0, Node::Branch(HashMap::new()));
        Self {
            pattern: pattern.normalize(),
            nodes,
        }
    }

    /// Add an entry to the store.
    /// 
    /// # Panics
    /// Will panic if the tree would be inconsistent - have leaf and branch nodes with the same name from the same parent.
    #[instrument()]
    pub fn add_entry(&mut self, entry: E) {
        Self::add_entry_inner(&mut self.nodes, &self.pattern, &entry);
    }

    /// Remove an entry from store.
    /// 
    /// Returns `true` if the entry was successfully removed.
    #[instrument()]
    pub fn remove(&mut self, path: &Path) -> bool {
        if let Some(parent) = path.parent() {
            let mut parent_id = 0_usize;
            for component in parent.components() {
                parent_id = match component {
                std::path::Component::RootDir => 0_usize,
                std::path::Component::Normal(component_name) => {
                    match Self::find_child(&self.nodes, parent_id, component_name) {
                        Some(id) => id,
                        None => {
                            debug!(parent_id, name = debug(component_name), "couldn't find");
                            return false;
                        }
                    }
                }
                _ => unreachable!()
                }
            } 
            debug!(children = debug(self.nodes.get(&parent_id)), id = debug(parent_id), name = debug(path.file_name()), "find child");
            let r = if let Some(children) = self.nodes.get_mut(&parent_id).and_then(|n| match n {
                Node::Branch(c) => Some(c),
                Node::Leaf(_) => None,
            }) {
                match path.file_name().and_then(|f| children.remove(f)) {
                    Some(id) => self.nodes.remove(&id).is_some(),
                    None => false,
                }
            } else {
                false
            };
            debug!(r, children = debug(self.nodes.get(&parent_id)), id = debug(parent_id), name = debug(path.file_name()), "find child");
            r
        } else {
            false
        }
    }

    /// Returns details of the object at the requested `path`.
    /// 
    /// - If the supplied path doesn't exist in the tree, returns `None`
    /// - If the supplied path exists, returns a `StorageEntry` describing the tree node.
    #[instrument()]
    pub fn find(&self, path: &Path) -> Option<StorageEntry<E>> {
        let mut id = 0_usize;
        for component in path.components() {
            id = match component {
                std::path::Component::RootDir => 0_usize,
                std::path::Component::Normal(component_name) => {
                    match Self::find_child(&self.nodes, id, component_name) {
                        Some(id) => id,
                        None => {
                            debug!(id, name = debug(component_name), "couldn't find");
                            return None;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        debug!(id, "found");
        Some(StorageEntry { node_id: id, nodes: &self.nodes })
    }

    /// Number of leaves in tree.
    /// 
    /// # Examples
    /// ```
    /// # use store::{PatternLocalPath,TreeStorage};
    /// # use std::path::{Path,PathBuf};
    /// # #[derive(Clone, Debug)]
    /// # struct Entry {
    /// # local_path: PathBuf,
    /// # }
    /// # impl PatternLocalPath for Entry {
    /// # fn new(_: &Path, _: &dyn common::DirEntry, _: &dyn common::Metadata) -> Self { todo!() }
    /// # fn local_path(&self, _: &Path) -> PathBuf { self.local_path.clone() }
    /// # fn host_path(&self) -> PathBuf { todo!() }
    /// # }
    /// let mut tree = TreeStorage::<Entry>::new("/t/{meta}/{size}/".into());
    /// assert_eq!(tree.len(), 0);
    /// ```
    /// ```
    /// # use store::{PatternLocalPath,TreeStorage};
    /// # use std::path::{Path,PathBuf};
    /// # #[derive(Clone, Debug)]
    /// # struct Entry {
    /// # local_path: PathBuf,
    /// # }
    /// # impl PatternLocalPath for Entry {
    /// # fn new(_: &Path, _: &dyn common::DirEntry, _: &dyn common::Metadata) -> Self { todo!() }
    /// # fn local_path(&self, _: &Path) -> PathBuf { self.local_path.clone() }
    /// # fn host_path(&self) -> PathBuf { todo!() }
    /// # }
    /// let mut tree = TreeStorage::<Entry>::new("/t/{meta}/{size}/".into());
    /// tree.add_entry(Entry {local_path: "/t/meta/size/example.file".into()});
    /// assert_eq!(tree.len(), 1);
    /// ```
    #[instrument()]
    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|(_id, n)| matches!(n, Node::Leaf(_))).count()
    }

    /// Total size of the tree, including internal branch nodes.
    /// 
    /// # Examples
    /// ```
    /// # use store::{PatternLocalPath,TreeStorage};
    /// # use std::path::{Path,PathBuf};
    /// # #[derive(Clone, Debug)]
    /// # struct Entry {
    /// # local_path: PathBuf,
    /// # }
    /// # impl PatternLocalPath for Entry {
    /// # fn new(_: &Path, _: &dyn common::DirEntry, _: &dyn common::Metadata) -> Self { todo!() }
    /// # fn local_path(&self, _: &Path) -> PathBuf { self.local_path.clone() }
    /// # fn host_path(&self) -> PathBuf { todo!() }
    /// # }
    /// let mut tree = TreeStorage::<Entry>::new("/t/{meta}/{size}/".into());
    /// assert_eq!(tree.node_count(), 1);
    /// ```
    /// ```
    /// # use store::{PatternLocalPath,TreeStorage};
    /// # use std::path::{Path,PathBuf};
    /// # #[derive(Clone, Debug)]
    /// # struct Entry {
    /// # local_path: PathBuf,
    /// # }
    /// # impl PatternLocalPath for Entry {
    /// # fn new(_: &Path, _: &dyn common::DirEntry, _: &dyn common::Metadata) -> Self { todo!() }
    /// # fn local_path(&self, _: &Path) -> PathBuf { self.local_path.clone() }
    /// # fn host_path(&self) -> PathBuf { todo!() }
    /// # }
    /// let mut tree = TreeStorage::<Entry>::new("/t/{meta}/{size}/".into());
    /// tree.add_entry(Entry {local_path: "/t/meta/size/example.file".into()});
    /// assert_eq!(tree.node_count(), 5);
    /// ```
    #[instrument]
    pub fn node_count(&self) -> usize {
        println!("nodes = {:?}", &self.nodes);
        self.nodes.len()
    }

    /// Returns `true` if the tree has a length of 0.
    ///
    /// # Examples
    /// ```
    /// # use store::{PatternLocalPath,TreeStorage};
    /// # use std::path::{Path,PathBuf};
    /// # #[derive(Clone, Debug)]
    /// # struct Entry {
    /// # local_path: PathBuf,
    /// # }
    /// # impl PatternLocalPath for Entry {
    /// # fn new(_: &Path, _: &dyn common::DirEntry, _: &dyn common::Metadata) -> Self { todo!() }
    /// # fn local_path(&self, _: &Path) -> PathBuf { self.local_path.clone() }
    /// # fn host_path(&self) -> PathBuf { todo!() }
    /// # }
    /// let mut tree = TreeStorage::<Entry>::new("/t/{meta}/{size}/".into());
    /// assert!(tree.is_empty());
    /// ```
    /// ```
    /// # use store::{PatternLocalPath,TreeStorage};
    /// # use std::path::{Path,PathBuf};
    /// # #[derive(Clone, Debug)]
    /// # struct Entry {
    /// # local_path: PathBuf,
    /// # }
    /// # impl PatternLocalPath for Entry {
    /// # fn new(_: &Path, _: &dyn common::DirEntry, _: &dyn common::Metadata) -> Self { todo!() }
    /// # fn local_path(&self, _: &Path) -> PathBuf { self.local_path.clone() }
    /// # fn host_path(&self) -> PathBuf { todo!() }
    /// # }
    /// let mut tree = TreeStorage::<Entry>::new("/t/{meta}/{size}/".into());
    /// tree.add_entry(Entry {local_path: "/t/meta/size/example.file".into()});
    /// assert!(!tree.is_empty());
    /// ```
    #[instrument()]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[instrument()]
    pub fn set_pattern(&mut self, pattern: &str) {
        debug!(pattern = debug(pattern), "set pattern");
        let new_pattern = PathBuf::from(pattern).normalize();

        let mut new_nodes = HashMap::new();
        new_nodes.insert(0, Node::Branch(HashMap::new()));

        for node in self.nodes.values() {
            if let Node::Leaf(entry) = node {
                Self::add_entry_inner(&mut new_nodes, &new_pattern, entry);
            }
        }

        self.pattern = new_pattern;
        self.nodes = new_nodes;
    }

    #[instrument()]
    pub fn get_pattern(&self) -> String {
        self.pattern.to_string_lossy().to_string()
    }

}

impl<E> TreeStorage<E>
where
    E: Debug + Clone + PatternLocalPath,
{
    fn add_entry_inner(nodes: &mut HashMap<usize, Node<E>>, pattern: &Path, entry: &E) {
        let file = entry.local_path(&pattern);
        let mut parent_id = 0_usize;
        for component in file.parent().unwrap().components() {
            parent_id = match component {
                std::path::Component::RootDir => 0_usize,
                std::path::Component::Normal(component_name) => {
                    Self::upsert(nodes, parent_id, component_name, Node::Branch(HashMap::new()))
                }
                _ => unreachable!(),
            };
            debug!(
                file = debug(&file),
                component = debug(component),
                parent_id,
                "find parent"
            );
        }
        Self::upsert(nodes, parent_id, file.file_name().unwrap(), Node::Leaf(entry.clone()));
        debug!(file = debug(&file), nodes = debug(nodes), "added file");
    }

    fn upsert(nodes: &mut HashMap<usize, Node<E>>, parent_id: usize, name: &OsStr, node: Node<E>) -> usize {
        debug!(name = debug(name), node = debug(&node), parent_id, "upsert");
        let new_id = nodes.len();
        match nodes.get_mut(&parent_id) {
            Some(Node::Branch(children)) => {
                match children.get(name) {
                    None => {
                        children.insert(name.to_owned(), new_id);
                        nodes.insert(new_id, node);
                        new_id
                    },
                    Some(i) => *i
                }    
            },
            Some(Node::Leaf(_)) => panic!("Cannot add children to a Leaf: {parent_id}"),
            None => panic!("Cannot add children to missing parent: {parent_id}")
        }
    }

    fn find_child(nodes: &HashMap<usize, Node<E>>, parent_id: usize, name: &OsStr) -> Option<usize> {
        nodes
            .get(&parent_id)
            .and_then(|parent_node| match parent_node {
                Node::Branch(children) => Some(children),
                _ => None,
            })
            .and_then(|children| children.get(name))
            .copied()
    }
}
