mod api;

use axum::{
    Router,
    response::Html,
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

/// `GET /next` — serve the SPA entrypoint.
async fn serve_next_root() -> axum::response::Response {
    index_html_response()
}

/// `GET /next/{*path}` — serve an embedded asset by path, or fall back to the
/// SPA `index.html` for client-side routes (paths without a file extension or
/// not found), so deep links into the SPA work.
pub async fn serve_next(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match WEB_DIST.get_file(&path) {
        Some(f) => (
            [(axum::http::header::CONTENT_TYPE, content_type_for(&path))],
            f.contents(),
        )
            .into_response(),
        None => index_html_response(),
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
        .route("/", get(index))
        .route("/next", get(serve_next_root))
        .route("/next/", get(serve_next_root))
        .route("/next/{*path}", get(serve_next))
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

async fn index() -> Html<&'static str> {
    Html(include_str!("index.html"))
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
    async fn next_root_serves_html() {
        let (status, ct, len) = parts(serve_next_root().await).await;
        assert_eq!(status, StatusCode::OK);
        assert!(ct.starts_with("text/html"));
        assert!(len > 0);
    }

    #[tokio::test]
    async fn unknown_client_route_falls_back_to_index_html() {
        // A path with no file extension = client route → SPA fallback (index.html).
        let (status, ct, _) = parts(serve_next(Path("agents".to_string())).await).await;
        assert_eq!(status, StatusCode::OK);
        assert!(ct.starts_with("text/html"));
    }
}
