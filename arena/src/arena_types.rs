use std::fmt::Debug;
use std::{
    ffi::{OsStr, OsString},
    marker::PhantomData,
    path::Path,
};

use tracing::instrument;

#[derive(Debug)]
pub struct FoundEntry<T> {
    data: PhantomData<T>,
}
impl<T> FoundEntry<T>
where
    T: Debug,
{
    #[instrument]
    pub fn inner(&self) -> Option<T> {
        todo!()
    }
    #[instrument]
    pub fn is_file(&self) -> bool {
        todo!()
    }
    #[instrument]
    pub fn is_directory(&self) -> bool {
        todo!()
    }
    #[instrument]
    pub fn children<'a, U>(&'a self, arena: &'a U) -> Children<T>
    where
        U: Debug + Arena<T> + ?Sized,
    {
        todo!()
    }
}
pub struct Children<T> {
    data: PhantomData<T>,
}
impl<T> Iterator for Children<T> {
    type Item = Entry<T>;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Entry<T> {
    Root,
    Directory(OsString),
    File(OsString, T),
    None,
}
impl<T> Entry<T>
where
    T: Debug,
{
    #[instrument]
    pub fn inner(&self) -> Option<T> {
        todo!()
    }
    #[instrument]
    pub fn is_root(&self) -> bool {
        matches!(*self, Self::Root)
    }

    #[instrument]
    pub fn is_directory(&self, path: &OsStr) -> bool {
        matches!(self, Self::Directory(p) if p == path)
    }

    #[instrument]
    pub fn is_file(&self, path: &OsStr) -> bool {
        matches!(self, Self::File(p, _) if p == path)
    }
    #[instrument]
    pub fn children<'a, U>(&'a self, arena: &'a U) -> Children<T>
    where
        U: Debug + Arena<T> + ?Sized,
    {
        todo!()
    }
}

pub trait Arena<T>: Debug + Send + Sync {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;

    fn add_file(&mut self, file: &Path, entry: T) -> Result<(), ()>;
    fn find(&self, path: &Path) -> Entry<T>;
}
