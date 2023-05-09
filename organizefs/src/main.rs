use axum::{
    extract::State,
    routing::{get, post},
    Router,
};
use fuse_mt::{spawn_mount, FuseMT};
use organizefs::{OrganizeFS, OrganizeFSStore};
use std::{env, ffi::OsStr, path::PathBuf, str::FromStr, sync::Arc};
use tracing::Level;
use tracing_subscriber::fmt::format::FmtSpan;

type AxumState = State<Arc<parking_lot::RwLock<OrganizeFSStore>>>;

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
        // OsStr::new("-o"),
        // OsStr::new("auto_unmount"),
    ];

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let stats = Arc::new(parking_lot::RwLock::new(OrganizeFSStore::new(
        PathBuf::from("/../s/../t/./{meta}/{size}"),
    )));
    let organizefs = OrganizeFS::new(&args[1], stats.clone(), tx);
    let fs = spawn_mount(FuseMT::new(organizefs, 1), &args[2], &fuse_args[..]).unwrap();

    // Setup REST endpoints
    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route(
            "/stats",
            get(|s: AxumState| async move {
                let stats = s.read();
                format!("{:?}", *stats)
            }),
        )
        .route(
            "/pattern",
            get(|s: AxumState| async move { s.read().get_pattern() }),
        )
        .route(
            "/pattern",
            post(|s: AxumState, body: String| async move {
                // TODO reduce write lock time
                s.write().set_pattern(&body);
            }),
        )
        .with_state(stats.clone());

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .with_graceful_shutdown(async {
            rx.await.ok();
        })
        .await
        .unwrap();
    fs.join();
}
