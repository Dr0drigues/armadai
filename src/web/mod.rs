mod api;

use axum::{
    Router,
    routing::{get, post},
};
use include_dir::{Dir, include_dir};
use tower_http::cors::CorsLayer;

static WEB_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web/ui/dist");

/// Guess a content-type from a path extension (the few types the SPA emits).
fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

fn index_html_response() -> axum::response::Response {
    use axum::response::IntoResponse;
    let body = WEB_DIST
        .get_file("index.html")
        .map(|f| f.contents())
        .unwrap_or(b"<!doctype html><title>ArmadAI</title>");
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// `GET /` — serve the Svelte SPA entrypoint. Client routing is hash-based, so
/// every in-app view is this same document.
async fn serve_spa() -> axum::response::Response {
    index_html_response()
}

/// `GET /assets/{*path}` — serve an embedded build asset (JS/CSS/fonts/…).
async fn serve_asset(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let full = format!("assets/{path}");
    match WEB_DIST.get_file(&full) {
        Some(f) => (
            [(axum::http::header::CONTENT_TYPE, content_type_for(&full))],
            f.contents(),
        )
            .into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

/// Wait for Ctrl+C signal.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl+c");
}

/// Serve the web UI on the given port.
pub async fn serve(port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(serve_spa))
        .route("/assets/{*path}", get(serve_asset))
        .route("/api/agents", get(api::list_agents))
        .route("/api/agents/{name}", get(api::get_agent))
        .route("/api/history", get(api::get_history))
        .route("/api/costs", get(api::get_costs))
        .route("/api/prompts", get(api::list_prompts))
        .route("/api/prompts/{name}", get(api::get_prompt))
        .route("/api/skills", get(api::list_skills))
        .route("/api/skills/{name}", get(api::get_skill))
        .route("/api/starters", get(api::list_starters))
        .route("/api/starters/{name}", get(api::get_starter))
        .route("/api/starters/{name}/config", get(api::get_starter_config))
        .route("/api/models", get(api::list_models))
        .route("/api/models/refresh", post(api::refresh_models))
        .route(
            "/api/orchestration/trace",
            get(api::get_orchestration_trace),
        )
        .route(
            "/api/orchestration/trace/{run_id}",
            get(api::get_orchestration_trace_detail),
        )
        .route(
            "/api/orchestration/topology",
            get(api::get_orchestration_topology),
        )
        .layer(CorsLayer::permissive());

    let addr = format!("0.0.0.0:{port}");
    println!("Web UI available at: http://localhost:{port}");
    println!("Press Ctrl+C to stop.");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    println!("\nWeb UI stopped.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Path;
    use axum::http::{StatusCode, header};

    async fn parts(resp: axum::response::Response) -> (StatusCode, String, usize) {
        let status = resp.status();
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let len = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .len();
        (status, ct, len)
    }

    #[tokio::test]
    async fn root_serves_the_spa_html() {
        let (status, ct, len) = parts(serve_spa().await).await;
        assert_eq!(status, StatusCode::OK);
        assert!(ct.starts_with("text/html"));
        assert!(len > 0);
    }

    #[tokio::test]
    async fn unknown_asset_is_404() {
        let (status, _, _) = parts(serve_asset(Path("does-not-exist.js".to_string())).await).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
