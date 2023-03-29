use std::path::Path;

use common::Normalize;
use tracing::info;

fn main() {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let p = Path::new("/../s/../t/./{meta}/{size}")
        .to_path_buf()
        .normalize();
    info!("Hello, world: {p:?}");

    for i in 0..p.components().count() {
        info!(component = debug(p.components().nth(i)), "{i}");
    }
}
