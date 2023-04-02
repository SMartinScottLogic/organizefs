use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use common::{expand, get_child_files, File, Normalize};
use tracing::{info, span, Level};
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Debug, Clone)]
struct Entry {
    meta: String,
    size: String,
    name: String,
}

impl File for Entry {
    fn meta(&self) -> &str {
        self.meta.as_str()
    }

    fn size(&self) -> &str {
        self.size.as_str()
    }
}

#[derive(Debug, Hash, PartialEq, Eq)]
enum FileType {
    RegularFile,
    Directory,
}
fn main() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::NONE)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    let pattern = Path::new("/../s/../t/./m_{meta}/s_{size}/{meta}_{size}")
        .to_path_buf()
        .normalize();
    info!("Hello, world: {pattern:?}");

    let files = vec![
        Entry {
            meta: "image_jpeg".to_string(),
            size: "12.0KB".to_string(),
            name: "1.jpeg".to_string(),
        },
        Entry {
            meta: "image_jpeg".to_string(),
            size: "12.0KB".to_string(),
            name: "2.jpeg".to_string(),
        },
        Entry {
            meta: "image_jpeg".to_string(),
            size: "13.0KB".to_string(),
            name: "3.jpeg".to_string(),
        },
        Entry {
            meta: "text_plain".to_string(),
            size: "13.0KB".to_string(),
            name: "1.txt".to_string(),
        },
    ];
    let probe_path = Path::new("/t/m_image_jpeg/s_12.0KB/image_jpeg_12.0KB");
    span!(Level::INFO, "components").in_scope(|| {
        for i in 0..pattern.components().count() {
            let field = pattern.components().nth(i + 1);
            span!(Level::INFO, "get_children", field = debug(field)).in_scope(|| {
                let cur_path = probe_path.components().take(i + 1).collect::<PathBuf>();
                let children = get_child_files(&files, &pattern, &cur_path);
                let children = children
                    .iter()
                    .map(|c| match field {
                        None => (FileType::RegularFile, c.name.clone()),
                        Some(component) => (FileType::Directory, expand(&component, c)),
                    })
                    .collect::<HashSet<_>>();
                info!(
                    component = debug(pattern.components().nth(i)),
                    children = debug(children),
                    cur_path = debug(cur_path),
                    "{i}"
                );
            });
        }
    });
}
