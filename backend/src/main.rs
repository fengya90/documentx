mod api;
mod config;
mod error;
mod knowledge;
mod llm;
mod render;
mod templates;

use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

use crate::{
    api::{build_router, load_instructions, AppState},
    config::Config,
    knowledge::{KeywordRetriever, Retriever},
    llm::LlmClient,
    templates::load_templates,
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
    let retriever: Arc<dyn Retriever> =
        Arc::new(KeywordRetriever::load(&config.paths.knowledge_dir)?);
    let templates = load_templates(&config.paths.templates_dir)?;
    let instructions = load_instructions(&config);

    let state = AppState {
        llm: Arc::new(llm),
        retriever,
        templates: Arc::new(templates),
        instructions: Arc::new(instructions),
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
