use axum::{routing::get, Router, extract::State};
use fuse_mt::{spawn_mount, FuseMT};
use organizefs::OrganizeFS;
use std::{env, ffi::OsStr, str::FromStr, sync::{Mutex, Arc}};
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;

#[tokio::main]
async fn main() {
    // install global collector configured based on RUST_LOG env var.
    let level = match env::var("RUST_LOG") {
        Ok(v) => match Level::from_str(&v) {
            Ok(l) => l,
            Err(_) => Level::INFO,
        },
        Err(_) => Level::INFO,
    };
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::NONE)
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
        // OsStr::new("-o"),
        // OsStr::new("auto_unmount"),
    ];

    let stats = Arc::new(Mutex::new(0));
    let organizefs = OrganizeFS::new(&args[1], "/../s/../t/./{meta}/{size}", stats.clone());
    return;
    let fs = spawn_mount(FuseMT::new(organizefs, 1), &args[2], &fuse_args[..]).unwrap();

    // build our application with a single route
    let app = Router::new()
    .route("/", get(|| async { "Hello, World!" }))
    .route("/stats", get(|s: State<Arc<Mutex<usize>>>| async move {
        let stats = s.lock().unwrap();
        format!("{}", *stats)
    })).with_state(stats.clone());

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
    fs.join();
}
