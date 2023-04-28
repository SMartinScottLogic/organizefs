use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fmt::Debug,
    path::Path,
};

use tracing::{debug, error, info, instrument};

use crate::arena_types::{Arena, Entry};

#[derive(Debug)]
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
impl<T> Arena<T> for NewArena<T>
where
    T: Clone + Debug + Send + Sync,
{
    #[instrument]
    fn len(&self) -> usize {
        self.data.len()
    }

    #[instrument]
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn add_file(&mut self, file: &Path, entry: T) -> Result<(), ()> {
        info!(
            file = debug(file),
            entry = debug(&entry),
            data = debug(&self.data),
            "add_file"
        );

        let mut parent_id = 0_usize;
        for component in file.parent().unwrap().components() {
            info!(component = debug(component), "find parent");
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
    fn find(&self, path: &Path) -> Entry<T> {
        info!(path = debug(path), data = debug(&self.data), "find");

        let mut found = self.data.get(&0).unwrap();
        for component in path.components() {
            info!(component = debug(component), "find parent");
            found = match component {
                std::path::Component::RootDir => self.data.get(&0).unwrap(),
                std::path::Component::Normal(p) => {
                    debug!("search for {p:?} in children of {found:?}");
                    match found.children() {
                        Some(children) => {
                            let f = match children.get(p) {
                                None => return Entry::None,
                                Some(c) => self.data.get(c).unwrap(),
                            };
                            info!(
                                parent = debug(found),
                                needle = debug(p),
                                found = debug(f),
                                "found child"
                            );
                            f
                        }
                        _ => {
                            error!("{:?} has no children, expected at least {:?}", found, p);
                            return Entry::None;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        info!(found = debug(found), "find");
        if let Some(file_name) = path.file_name() {
            match found {
                NewArenaElement::Root(_) => Entry::<T>::Root,
                NewArenaElement::Leaf(id) => Entry::File(file_name.into(), (*id).to_owned()),
                NewArenaElement::Branch(_) => Entry::Directory(file_name.into()),
            }
        } else {
            Entry::None
        }
    }
}

impl<T: Debug> NewArena<T> {
    fn upsert(
        &mut self,
        parent_id: usize,
        name: &OsStr,
        element: NewArenaElement<T>,
    ) -> Result<usize, ()> {
        info!("upsert {name:?}=>{element:?} in children of {parent_id}");
        let branch_id = 1 + self.data.keys().max().unwrap_or(&0);

        let children = match self.data.get_mut(&parent_id).and_then(|p| p.children_mut()) {
            None => return Err(()),
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

#[derive(Debug)]
enum NewArenaElement<T> {
    Root(HashMap<OsString, usize>),
    Leaf(T),
    Branch(HashMap<OsString, usize>),
}

impl<T> NewArenaElement<T> {
    fn children(&self) -> Option<&HashMap<std::ffi::OsString, usize>> {
        match self {
            NewArenaElement::Root(c) => Some(c),
            NewArenaElement::Leaf(_) => None,
            NewArenaElement::Branch(c) => Some(c),
        }
    }

    fn children_mut(&mut self) -> Option<&mut HashMap<std::ffi::OsString, usize>> {
        match self {
            NewArenaElement::Root(c) => Some(c),
            NewArenaElement::Leaf(_) => None,
            NewArenaElement::Branch(c) => Some(c),
        }
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
