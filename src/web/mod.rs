mod api;

use axum::{Router, response::Html, routing::get};
use tower_http::cors::CorsLayer;

/// Serve the web UI on the given port.
pub async fn serve(port: u16) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(index))
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
        .layer(CorsLayer::permissive());

    let addr = format!("0.0.0.0:{port}");
    println!("Web UI available at: http://localhost:{port}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}
