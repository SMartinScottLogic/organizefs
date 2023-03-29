use std::{ffi::OsString, path::PathBuf};

use tracing::debug;

pub fn add(left: usize, right: usize) -> usize {
    left + right
}

pub trait Normalize {
    fn normalize(&self) -> Self;
}

impl Normalize for PathBuf {
    fn normalize(&self) -> Self {
        let mut comps = Vec::new();

        for c in self.components() {
            match c {
                std::path::Component::Prefix(_) => todo!(),
                std::path::Component::RootDir => {
                    comps.clear();
                    comps.push(c);
                }
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    if let Some(std::path::Component::Normal(_)) = comps.last() {
                        comps.pop();
                    }
                }
                std::path::Component::Normal(_) => comps.push(c),
            }
        }
        let mut res = OsString::new();
        let mut need_sep = false;

        for c in comps {
            if need_sep && c != std::path::Component::RootDir {
                res.push(std::path::MAIN_SEPARATOR_STR);
            }
            res.push(c.as_os_str());

            need_sep = match c {
                std::path::Component::RootDir => false,
                std::path::Component::Prefix(_) => todo!(),
                _ => true,
            }
        }
        debug!(source = debug(self), target = debug(&res), "normalize");
        PathBuf::from(&res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
