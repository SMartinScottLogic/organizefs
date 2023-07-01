use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fmt::Debug,
    path::{Path, PathBuf},
    str::FromStr,
};

use tracing::{debug, error, info, instrument};

use crate::arena::{
    arena_types::{Arena, Entry},
    ArenaError,
};

pub struct NewArena<T> {
    data: HashMap<usize, NewArenaElement<T>>,
}
impl<T> Default for NewArena<T> {
    fn default() -> Self {
        let mut data = HashMap::new();
        data.insert(0, NewArenaElement::Root(HashMap::new()));
        Self { data }
    }
}
impl<T> Debug for NewArena<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewArena")
            .field("data_len", &self.data.len())
            .finish()
    }
}
impl<T> Arena<T> for NewArena<T>
where
    T: Clone + Debug + PartialEq + Send + Sync,
{
    type Entry = NewArenaElement<T>;

    #[instrument]
    fn len(&self) -> usize {
        self.data.len()
    }

    #[instrument]
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn add_file(&mut self, file: &Path, entry: T) -> Result<(), ArenaError> {
        info!(file = debug(file), entry = debug(&entry), "add_file");
        debug!(
            file = debug(file),
            entry = debug(&entry),
            arena = debug(&self),
            "add_file"
        );

        let mut parent_id = 0_usize;
        for component in file.parent().unwrap().components() {
            debug!(component = debug(component), "find parent");
            parent_id = match component {
                std::path::Component::RootDir => 0_usize,
                std::path::Component::Normal(component_name) => {
                    match self.upsert(
                        parent_id,
                        component_name,
                        NewArenaElement::Branch(HashMap::new()),
                    ) {
                        Err(e) => return Err(e),
                        Ok(id) => id,
                    }
                }
                _ => unreachable!(),
            }
        }
        let file_name = file.file_name().unwrap();
        self.upsert(parent_id, file_name, NewArenaElement::Leaf(entry))
            .map(|_id| ())
    }

    #[instrument]
    fn find(&self, path: &Path) -> Self::Entry {
        info!(path = debug(path), "find");
        debug!(path = debug(path), data = debug(&self.data), "find");

        let mut found = self.data.get(&0).unwrap();
        for component in path.components() {
            debug!(component = debug(component), "find parent");
            found = match component {
                std::path::Component::RootDir => self.data.get(&0).unwrap(),
                std::path::Component::Normal(p) => {
                    debug!("search for {p:?} in children of {found:?}");
                    match found.children() {
                        Some(children) => {
                            let f = match children.get(p) {
                                None => return Self::Entry::None,
                                Some(c) => self.data.get(c).unwrap(),
                            };
                            debug!(
                                parent = debug(found),
                                needle = debug(p),
                                found = debug(f),
                                "found child"
                            );
                            f
                        }
                        _ => {
                            error!("{:?} has no children, expected at least {:?}", found, p);
                            return Self::Entry::None;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        debug!(
            seek = debug(path.components().last()),
            found = debug(found),
            "find"
        );
        match path.components().last() {
            Some(std::path::Component::RootDir) if found.is_root() => found.clone(),
            Some(std::path::Component::Normal(_)) => found.clone(),
            _ => Self::Entry::None,
        }
    }
}

impl<T: Clone> NewArena<T> {
    fn find_parent_mut(&mut self, path: &Path) -> Option<&mut NewArenaElement<T>> {
        let binding = PathBuf::from_str("/").unwrap();
        let path = match path.parent() {
            None => binding.as_path(),
            Some(p) => p,
        };
        info!(path = debug(path), "find");
        debug!(path = debug(path), data = debug(&self.data), "find");

        let mut parent_id = 0_usize;
        for component in path.components() {
            debug!(component = debug(component), "find parent");
            parent_id = match component {
                std::path::Component::RootDir => 0_usize,
                std::path::Component::Normal(p) => {
                    debug!("search for {p:?} in children of {parent_id:?}");
                    match self.data.get(&parent_id).and_then(|p| p.children()) {
                        Some(children) => {
                            let f = match children.get(p) {
                                None => return None,
                                Some(c) => c,
                            };
                            debug!(
                                needle = debug(p),
                                found = debug(f),
                                "found child"
                            );
                            *f
                        }
                        _ => {
                            error!("{:?} has no children, expected at least {:?}", parent_id, p);
                            return None;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        let found = self.data.get_mut(&parent_id);
        debug!(
            seek = debug(path.components().last()),
            found = debug(&found),
            "find"
        );
        found
    }

    pub fn remove(&mut self, path: &Path) -> bool {
        let parent = self.find_parent_mut(path);
        if let Some(parent) = parent {
            if let Some(children) = parent.children_mut() {
                debug!(path = debug(path), children = debug(&children), "remove");
                if let Some(id) = children.remove(path.file_name().unwrap()) {
                    let dropped = self.data.remove(&id);
                    debug!(dropped = debug(&dropped), id, path = debug(path), "dropped");
                    return dropped.is_some();
                }
            }
        }
        false
    }
}

impl<T: Debug> NewArena<T> {
    fn upsert(
        &mut self,
        parent_id: usize,
        name: &OsStr,
        element: NewArenaElement<T>,
    ) -> Result<usize, ArenaError> {
        debug!("upsert {name:?}=>{element:?} in children of {parent_id}");
        let branch_id = 1 + self.data.len();

        let children = match self.data.get_mut(&parent_id).and_then(|p| p.children_mut()) {
            None => return Err(ArenaError::Unknown),
            Some(c) => c,
        };

        let (id, insert) = match children.get(name) {
            None => {
                children.insert(name.into(), branch_id);
                (branch_id, true)
            }
            Some(b) => (*b, false),
        };
        if insert {
            self.data.insert(branch_id, element);
        }
        Ok(id)
    }
}

#[derive(Clone, PartialEq)]
pub enum NewArenaElement<T> {
    Root(HashMap<OsString, usize>),
    Leaf(T),
    Branch(HashMap<OsString, usize>),
    None,
}

impl<T> NewArenaElement<T> {
    pub fn inner(&self) -> Option<T>
    where
        T: Copy,
    {
        match self {
            Self::Leaf(l) => Some(*l),
            _ => None,
        }
    }

    fn children(&self) -> Option<&HashMap<std::ffi::OsString, usize>> {
        match self {
            NewArenaElement::Root(c) => Some(c),
            NewArenaElement::Leaf(_) => None,
            NewArenaElement::Branch(c) => Some(c),
            NewArenaElement::None => None,
        }
    }

    fn children_mut(&mut self) -> Option<&mut HashMap<std::ffi::OsString, usize>> {
        match self {
            NewArenaElement::Root(c) => Some(c),
            NewArenaElement::Leaf(_) => None,
            NewArenaElement::Branch(c) => Some(c),
            NewArenaElement::None => None,
        }
    }
}
impl<T> Entry for NewArenaElement<T> {
    type Children<'a> = Children<'a, T> where Self: 'a;
    type Arena = NewArena<T>;

    fn is_root(&self) -> bool {
        matches!(&self, Self::Root(_))
    }

    fn is_directory(&self) -> bool {
        matches!(&self, Self::Root(_) | Self::Branch(_))
    }

    fn is_file(&self) -> bool {
        matches!(&self, Self::Leaf(_))
    }

    fn children<'a, 'b>(&'a self, arena: &'b Self::Arena) -> Self::Children<'b>
    where
        'a: 'b,
    {
        let r = match self {
            NewArenaElement::Root(c) => Some(c),
            NewArenaElement::Leaf(_) => None,
            NewArenaElement::Branch(c) => Some(c),
            NewArenaElement::None => None,
        };
        Children::from(arena, r)
    }
}
impl<T> Debug for NewArenaElement<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Root(_) => write!(f, "Root"),
            Self::Leaf(_) => write!(f, "Leaf"),
            Self::Branch(_) => write!(f, "Branch"),
            Self::None => write!(f, "None"),
        }
    }
}
pub struct Children<'a, T> {
    arena: &'a NewArena<T>,
    children: Option<std::collections::hash_map::Iter<'a, OsString, usize>>,
}
impl<'a, T> Children<'a, T> {
    fn from(arena: &'a NewArena<T>, value: Option<&'a HashMap<OsString, usize>>) -> Self {
        Self {
            arena,
            children: value.map(|c| c.iter()),
        }
    }
}
impl<'a, T> Iterator for Children<'a, T> {
    type Item = (&'a OsString, &'a NewArenaElement<T>);

    fn next(&mut self) -> Option<Self::Item> {
        self.children.as_mut().and_then(|iter| {
            if let Some((name, idx)) = iter.next() {
                self.arena.data.get(idx).map(|v| (name, v))
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tracing_test::traced_test;

    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    #[traced_test]
    fn add_file() {
        let mut arena = NewArena::default();
        assert!(arena.add_file(&PathBuf::from("/f1/f2/f3/file"), 1).is_ok());
    }
}
