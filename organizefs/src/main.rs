use fuse_mt::{spawn_mount, FuseMT};
use organizefs::{server, OrganizeFS};
use std::{env, ffi::OsStr, path::PathBuf, str::FromStr, sync::Arc};
use store::TreeStorage;
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;

#[tokio::main]
async fn main() {
    // install global collector configured based on RUST_LOG env var.
    let level =
        env::var("RUST_LOG").map_or(Level::INFO, |v| Level::from_str(&v).unwrap_or(Level::INFO));
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::ACTIVE)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_max_level(level)
        .init();

    let args: Vec<String> = env::args().collect();

    let fuse_args = [
        OsStr::new("-o"),
        OsStr::new("fsname=organizefs"),
        OsStr::new("-o"),
        OsStr::new("allow_other"),
        // OsStr::new("-o"),
        // OsStr::new("auto_unmount"),
    ];

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let stats = Arc::new(parking_lot::RwLock::new(TreeStorage::new(PathBuf::from(
        "/../s/../t/./{meta}/{size}",
    ))));
    let organizefs = OrganizeFS::new(&args[1], stats.clone(), tx);
    let fs = spawn_mount(FuseMT::new(organizefs, 1), &args[2], &fuse_args[..]).unwrap();

    server(stats, rx).await.unwrap();
    fs.join();
}
