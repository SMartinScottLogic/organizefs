use std::{ffi::OsString, path::PathBuf};

use tracing::debug;

/// Generate a normalized version of self
pub trait Normalize {
    /// Perform the normalization.
    fn normalize(&self) -> Self;
}

/// Generate a normalized version of a PathBuf
impl Normalize for PathBuf {
/// Normalize a PathBuf.
/// 
/// Removes CurDir (/./), and ParentDir(/../) components appropriately
/// 
/// # Warning
/// DOES NOT follow symbolic links - resultant path may therefore NOT exist
/// # Examples
/// ```
/// # use std::path::PathBuf;
/// # use common::Normalize;
/// assert_eq!(PathBuf::from("/test/a/./b/../../").normalize(), PathBuf::from("/test/"));
/// ```
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
    use std::path::Path;

    use super::*;

    #[test]
    fn normalize() {
        let input = Path::new("/../s/../t/./m_{meta}/s_{size}/{meta}_{size}").to_path_buf();
        let result = input.normalize();
        assert_eq!(
            "/t/m_{meta}/s_{size}/{meta}_{size}",
            result.to_str().unwrap()
        );
    }
}
