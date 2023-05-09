use std::{
    fmt::Debug,
    ops::Index,
    path::{Component, Path},
};

use tracing::{debug, instrument};

pub trait FsFile: for<'a> Index<&'a str, Output = str> {}

#[instrument(level = "debug")]
pub fn expand<T>(component: &Component, file: &T) -> String
where
    T: Debug + Clone + FsFile,
{
    let component = component.as_os_str().to_string_lossy();
    component
        .replace("{meta}", &file["meta"])
        .replace("{size}", &file["size"])
}

#[instrument(level = "debug")]
pub fn get_child_files<T>(files: &[T], pattern: &Path, cur_path: &Path) -> Vec<T>
where
    T: Debug + Clone + FsFile,
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
    use file_proc_macro::FsFile;

    use super::*;

    #[derive(Debug, Clone)]
    #[derive(FsFile)]
    struct TestFile<'a> {
        #[fsfile="meta"] meta: &'a str,
        #[fsfile="size"] size: &'a str,
        id: usize,
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
        let pattern = Path::new("/{meta}/{size}").to_path_buf();
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
        let pattern = Path::new("/{meta}").to_path_buf();
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
        let pattern = Path::new("/{meta}/{size}").to_path_buf();
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
