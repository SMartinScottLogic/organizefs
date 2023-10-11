use std::{fmt::Debug, ops::Index, path::Component};

use tracing::instrument;

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
        .replace("{mdate}", &file["mdate"])
}
