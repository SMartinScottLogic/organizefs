use std::sync::Arc;

use axum::{
    extract::State,
    routing::{get, post},
    Router,
};
use parking_lot::RwLock;
use store::OrganizeFSStore;
use tokio::sync::oneshot::Receiver;

type Stats = Arc<RwLock<OrganizeFSStore>>;
type AxumState = State<Stats>;

/// Setup REST endpoints
pub async fn server(stats: Stats, rx: Receiver<()>) -> Result<(), hyper::Error> {
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
}
