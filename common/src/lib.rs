use std::{
    ffi::OsString,
    fmt::Debug,
    path::{Component, Path, PathBuf},
};

use tracing::{debug, instrument};

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

pub trait File {
    fn meta(&self) -> &str;
    fn size(&self) -> &str;
}

#[instrument(level = "debug")]
pub fn expand<T>(component: &Component, file: &T) -> String
where
    T: Debug + Clone + File,
{
    let component = component.as_os_str().to_string_lossy();
    let np = component
        .replace("{meta}", file.meta())
        .replace("{size}", file.size());
    np
}

#[instrument(level = "debug")]
pub fn get_child_files<T>(files: &[T], pattern: &Path, cur_path: &Path) -> Vec<T>
where
    T: Debug + Clone + File,
{
    let matching_files = files
        .iter()
        .filter(|file| {
            cur_path.components().zip(pattern.components()).all(
                |(path_component, pattern_component)| {
                    let np = expand(&pattern_component, *file);
                    let equivalent = path_component.as_os_str().to_string_lossy() == np;
                    equivalent
                },
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    debug!(
        cur_path = debug(cur_path),
        matching_files = debug(&matching_files),
        "child files"
    );
    matching_files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestFile<'a> {
        meta: &'a str,
        size: &'a str,
        id: usize,
    }
    impl<'a> File for TestFile<'a> {
        fn meta(&self) -> &str {
            self.meta
        }

        fn size(&self) -> &str {
            self.size
        }
    }

    #[test]
    fn normalize() {
        let input = Path::new("/../s/../t/./m_{meta}/s_{size}/{meta}_{size}").to_path_buf();
        let result = input.normalize();
        assert_eq!(
            "/t/m_{meta}/s_{size}/{meta}_{size}",
            result.to_str().unwrap()
        );
    }

    #[test]
    fn get_child_files_root() {
        let files = vec![
            TestFile {
                meta: "1",
                size: "1",
                id: 0,
            },
            TestFile {
                meta: "1",
                size: "2",
                id: 1,
            },
        ];
        let pattern = Path::new("/{meta}/{size}").to_path_buf().normalize();
        let cur_path = Path::new("/");
        let children = super::get_child_files(&files, &pattern, cur_path);
        assert_eq!(2, children.len());
        assert!(children.iter().any(|c| c.id == 0));
        assert!(children.iter().any(|c| c.id == 1));
    }

    #[test]
    fn get_child_files_meta() {
        let files = vec![
            TestFile {
                meta: "1",
                size: "1",
                id: 0,
            },
            TestFile {
                meta: "1",
                size: "2",
                id: 1,
            },
            TestFile {
                meta: "2",
                size: "0",
                id: 2,
            },
        ];
        let pattern = Path::new("/{meta}").to_path_buf().normalize();
        let cur_path = Path::new("/1");
        let children = super::get_child_files(&files, &pattern, cur_path);
        assert_eq!(2, children.len());
        assert!(children.iter().any(|c| c.id == 0));
        assert!(children.iter().any(|c| c.id == 1));
    }

    #[test]
    fn get_child_files_meta_size() {
        let files = vec![
            TestFile {
                meta: "1",
                size: "1",
                id: 0,
            },
            TestFile {
                meta: "1",
                size: "2",
                id: 1,
            },
            TestFile {
                meta: "1",
                size: "2",
                id: 2,
            },
            TestFile {
                meta: "2",
                size: "0",
                id: 3,
            },
        ];
        let pattern = Path::new("/{meta}/{size}").to_path_buf().normalize();
        let cur_path = Path::new("/1/2");
        let children = super::get_child_files(&files, &pattern, cur_path);
        assert_eq!(2, children.len());
        assert!(children.iter().any(|c| c.id == 1));
        assert!(children.iter().any(|c| c.id == 2));

        let cur_path = Path::new("/2/0");
        let children = super::get_child_files(&files, &pattern, cur_path);
        assert_eq!(1, children.len());
        assert!(children.iter().any(|c| c.id == 3));

        let cur_path = Path::new("/2/2");
        let children = super::get_child_files(&files, &pattern, cur_path);
        assert_eq!(0, children.len());
    }
}
