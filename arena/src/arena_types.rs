use std::fmt::Debug;
use std::path::Path;

pub trait Entry {
    type Children<'a>
    where
        Self: 'a;
    type Arena;

    fn is_root(&self) -> bool;
    fn is_directory(&self) -> bool;
    fn is_file(&self) -> bool;
    fn filter<F>(&self, f: F) -> Option<&Self>
    where
        Self: std::marker::Sized,
        F: Fn(&Self) -> bool,
    {
        if f(self) {
            Some(self)
        } else {
            None
        }
    }
    fn children<'a, 'b>(&'a self, arena: &'b Self::Arena) -> Self::Children<'b>
    where
        'a: 'b;
}

pub trait Arena<T>: Debug + Send + Sync {
    type Entry;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;

    fn add_file(&mut self, file: &Path, entry: T) -> Result<(), ArenaError>;
    fn find(&self, path: &Path) -> Self::Entry;
}

#[derive(Debug)]
pub enum ArenaError {
    Unknown,
}
