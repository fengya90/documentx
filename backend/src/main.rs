mod api;
mod config;
mod content;
mod error;
mod knowledge;
mod llm;
mod render;
mod templates;

use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

use crate::{
    api::{build_router, AppState, DEFAULT_SYSTEM_PROMPT},
    config::Config,
    content::ContentManager,
    llm::LlmClient,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("documentx=info,tower_http=warn")),
        )
        .init();

    let config = Config::load("config.toml")?;

    let llm = LlmClient::new(&config)?;
    let content = ContentManager::initialize(&config, DEFAULT_SYSTEM_PROMPT).await?;
    content.spawn_refresh_task();

    let state = AppState {
        llm: Arc::new(llm),
        content,
        config: Arc::new(config.clone()),
    };

    let app = build_router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("绑定地址失败：{addr}"))?;

    tracing::info!("DocumentX 服务已启动：http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
