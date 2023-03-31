use std::path::{Path, PathBuf};

use common::Normalize;
use tracing::{info, instrument, span, Level};
use tracing_subscriber::fmt::format::FmtSpan;

#[derive(Debug)]
struct Entry {
    meta: String,
    size: String,
    name: String,
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
    let probe_path = Path::new("/t/image_jpeg/12.0KB/image_jpeg_12.0KB");
    span!(Level::INFO, "components").in_scope(|| {
        for i in 0..pattern.components().count() {
            let field = pattern.components().nth(i);
            span!(Level::INFO, "get_children", field = debug(field)).in_scope(|| {
                let cur_path = probe_path.components().take(i + 1).collect::<PathBuf>();
                let children = get_children(&files, &pattern, &cur_path);
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

#[instrument(level = "debug")]
fn get_children(files: &[Entry], pattern: &Path, cur_path: &Path) -> Vec<String> {
    info!(cur_path = debug(cur_path), "get_children");
    for file in files {
    for (i, path_component) in cur_path.components().enumerate() {
        let pattern_component = pattern.components().nth(i).unwrap();
        let pattern_component_str = pattern_component.as_os_str().to_string_lossy();
        let np = pattern_component_str
            .replace("{meta}", &file.meta)
            .replace("{size}", &file.size);
        info!(
            file = debug(file),
            path_component = debug(path_component),
            pattern_component = debug(pattern_component),
            np, "extraction"
        );
    }
    }
    Vec::new()
}
