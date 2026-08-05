use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures::Stream;
use serde::Deserialize;
use serde_json::json;
use tower_http::{
    compression::{
        predicate::{NotForContentType, Predicate, SizeAbove},
        CompressionLayer,
    },
    cors::CorsLayer,
    limit::RequestBodyLimitLayer,
    services::{ServeDir, ServeFile},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{
    config::Config,
    error::AppError,
    knowledge::Retriever,
    llm::{LlmClient, Message, StreamEvent},
    render::{self, Format},
    templates::{self, Template},
};

pub const DEFAULT_SYSTEM_PROMPT: &str =
    "你是一个文档智能体。你基于内部知识库回答问题，并能按要求产出规范的文档。\
回答使用简洁、专业的中文，结构清晰，优先使用 Markdown 组织内容（标题、列表、表格、代码块）。\
当提供了「知识库参考」时，以其为事实依据，不要编造；若参考中没有答案，如实说明。\
技术文档确有助于理解时，可使用 mermaid、graphviz、vega 或 vegalite fenced code block；不要引用外部 URL 或文件。";

/// 加载指导 documentx 行为的指令文件（如 AGENTS.md）作为系统提示；
/// 未配置或文件为空则回退到内置默认提示。
pub fn load_instructions(cfg: &Config) -> String {
    if let Some(path) = &cfg.paths.agents_file {
        if std::path::Path::new(path).exists() {
            if let Ok(s) = std::fs::read_to_string(path) {
                if !s.trim().is_empty() {
                    tracing::info!("已加载智能体指令文件：{}", path);
                    return s;
                }
            }
        }
        tracing::warn!("指令文件 {} 不存在或为空，使用内置默认提示", path);
    }
    DEFAULT_SYSTEM_PROMPT.to_string()
}

#[derive(Clone)]
pub struct AppState {
    pub llm: Arc<LlmClient>,
    pub retriever: Arc<dyn Retriever>,
    pub templates: Arc<Vec<Template>>,
    pub instructions: Arc<String>,
    pub config: Arc<Config>,
}

pub fn build_router(state: AppState) -> Router {
    let cfg = state.config.clone();

    // 流式路由：不套 Timeout（否则长回答会被整体掐断）。
    let streaming = Router::new().route("/chat", post(chat));

    // 非流式路由：套一个整体超时。
    let blocking = Router::new()
        .route("/health", get(health))
        .route("/templates", get(list_templates))
        .route("/templates/content", get(template_content))
        .route("/knowledge", get(list_knowledge))
        .route("/knowledge/content", get(knowledge_content))
        .route("/generate", post(generate))
        .route("/export", post(export))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(cfg.llm.request_timeout_secs),
        ));

    let api = streaming
        .merge(blocking)
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(
            cfg.server.max_body_mb * 1024 * 1024,
        ))
        .with_state(state);

    // 压缩：br/gzip，但对 SSE（text/event-stream）关闭，且仅压缩 >1KB 的响应。
    let compression = CompressionLayer::new()
        .br(true)
        .gzip(true)
        .compress_when(SizeAbove::new(1024).and(NotForContentType::const_new("text/event-stream")));

    let index = format!("{}/index.html", cfg.paths.static_dir);
    let serve_dir = ServeDir::new(&cfg.paths.static_dir).not_found_service(ServeFile::new(index));

    Router::new()
        .nest("/api", api)
        .fallback_service(serve_dir)
        .layer(compression)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

// ---------- 元信息 ----------

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

async fn list_templates(State(st): State<AppState>) -> impl IntoResponse {
    let names: Vec<&String> = st.templates.iter().map(|t| &t.name).collect();
    Json(json!({ "templates": names }))
}

async fn list_knowledge(State(st): State<AppState>) -> impl IntoResponse {
    Json(json!({ "sources": st.retriever.sources() }))
}

#[derive(Deserialize)]
struct SourceQuery {
    source: String,
}

/// 只读查看某个知识库文档的原文。以 `sources()` 为白名单，杜绝路径穿越。
async fn knowledge_content(
    State(st): State<AppState>,
    Query(q): Query<SourceQuery>,
) -> Result<Response, AppError> {
    if !st.retriever.sources().iter().any(|s| s == &q.source) {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "未找到该文档" })),
        )
            .into_response());
    }
    let path = std::path::Path::new(&st.config.paths.knowledge_dir).join(&q.source);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| AppError(anyhow::anyhow!("读取文档失败：{e}")))?;
    Ok(Json(json!({ "source": q.source, "content": content })).into_response())
}

#[derive(Deserialize)]
struct NameQuery {
    name: String,
}

/// 只读查看某个模板的内容（模板已加载在内存里）。
async fn template_content(
    State(st): State<AppState>,
    Query(q): Query<NameQuery>,
) -> Result<Response, AppError> {
    match templates::find(&st.templates, &q.name) {
        Some(t) => Ok(Json(json!({ "name": t.name, "content": t.content })).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "未找到该模板" })),
        )
            .into_response()),
    }
}

// ---------- 对话（流式） ----------

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct ChatReq {
    messages: Vec<Message>,
    #[serde(default = "default_true")]
    use_knowledge: bool,
}

fn build_system_prompt(st: &AppState, query: &str, use_knowledge: bool) -> String {
    let mut system = st.instructions.as_ref().clone();
    if use_knowledge && !query.is_empty() {
        let chunks = st.retriever.retrieve(query, 5);
        if !chunks.is_empty() {
            system.push_str("\n\n# 知识库参考\n");
            for c in chunks {
                system.push_str(&format!("\n## 来源：{}\n{}\n", c.source, c.text));
            }
        }
    }
    system
}

/// 让对话也知道有哪些输出模板：始终列出模板名；若用户消息里提到了某个模板
/// （或提到“模板/模版”且只有一个模板），则把该模板全文注入，供模型参照。
fn append_templates(system: &mut String, st: &AppState, query: &str) {
    if st.templates.is_empty() {
        return;
    }
    let names: Vec<&str> = st.templates.iter().map(|t| t.name.as_str()).collect();
    system.push_str(&format!(
        "\n\n# 可用输出模板\n当用户要求“按某模板”产出文档时，参照对应模板的结构、章节顺序与写作风格来组织内容；\
模板只是格式参考，事实仍以知识库为准。当前可用模板：{}。\n",
        names.join("、")
    ));

    let q = normalize(query);
    let mut injected: Vec<&Template> = st
        .templates
        .iter()
        .filter(|t| q.contains(&normalize(&t.name)))
        .collect();
    // 未精确命中，但用户提到“模板/模版”，且只有一个模板时，默认注入它。
    if injected.is_empty()
        && st.templates.len() == 1
        && (query.contains("模板") || query.contains("模版"))
    {
        injected.push(&st.templates[0]);
    }
    for t in injected {
        system.push_str(&format!("\n## 模板：{}\n{}\n", t.name, t.content));
    }
}

/// 归一化用于模板名匹配：转小写并去除空白（兼容 API/api、有无空格等）。
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

async fn chat(
    State(st): State<AppState>,
    Json(req): Json<ChatReq>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_user = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();

    let mut system = build_system_prompt(&st, &last_user, req.use_knowledge);
    append_templates(&mut system, &st, &last_user);

    // 上下文管理：按 token 预算裁剪/压缩历史，避免撑爆窗口。
    let (history, notice) = prepare_history(&st, &system, req.messages).await;
    let mut messages = vec![Message::system(system)];
    messages.extend(history);

    let client = st.llm.clone();
    let stream = async_stream::stream! {
        use futures::StreamExt;

        // 若发生了裁剪/压缩，先给前端一个状态提示。
        if let Some(msg) = notice {
            yield Ok::<_, Infallible>(Event::default().event("status").data(msg));
        }

        let s = client.chat_stream(messages);
        futures::pin_mut!(s);
        while let Some(item) = s.next().await {
            match item {
                Ok(StreamEvent::Delta(delta)) => {
                    let data = json!({ "delta": delta }).to_string();
                    yield Ok::<_, Infallible>(Event::default().data(data));
                }
                Ok(StreamEvent::Status(msg)) => {
                    yield Ok::<_, Infallible>(Event::default().event("status").data(msg));
                }
                Ok(StreamEvent::Usage(u)) => {
                    let data = json!({
                        "prompt_tokens": u.prompt_tokens,
                        "completion_tokens": u.completion_tokens,
                        "total_tokens": u.total_tokens,
                    })
                    .to_string();
                    yield Ok::<_, Infallible>(Event::default().event("usage").data(data));
                }
                Err(e) => {
                    yield Ok::<_, Infallible>(Event::default().event("error").data(e.to_string()));
                    break;
                }
            }
        }
        yield Ok::<_, Infallible>(Event::default().data("[DONE]"));
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// 粗略估算 token：ASCII 约 4 字符/token，非 ASCII（中文等）约 1 token/字。
fn estimate_tokens(s: &str) -> usize {
    let mut ascii = 0usize;
    let mut other = 0usize;
    for c in s.chars() {
        if c.is_ascii() {
            ascii += 1;
        } else {
            other += 1;
        }
    }
    ascii / 4 + other
}

/// 按 token 预算准备要发给模型的历史：
/// - 预算内 → 原样返回；
/// - 超预算 → 保留最近若干轮（tail），较早的部分用 LLM 摘要压缩（或直接丢弃）。
async fn prepare_history(
    st: &AppState,
    system: &str,
    msgs: Vec<Message>,
) -> (Vec<Message>, Option<String>) {
    let llm = &st.config.llm;
    let budget = llm
        .context_window_tokens
        .saturating_sub(llm.max_response_tokens);
    let sys_tokens = estimate_tokens(system);
    let avail = budget.saturating_sub(sys_tokens);

    let total: usize = msgs.iter().map(|m| estimate_tokens(&m.content)).sum();
    if total <= avail {
        return (msgs, None);
    }

    // 从末尾开始保留最近消息（至少保留最后一条），给摘要预留一部分空间。
    const SUMMARY_RESERVE: usize = 800;
    let tail_budget = avail.saturating_sub(SUMMARY_RESERVE);
    let mut used = 0usize;
    let mut split = 0usize; // recent 从此下标开始
    for i in (0..msgs.len()).rev() {
        let t = estimate_tokens(&msgs[i].content);
        if used + t > tail_budget && used > 0 {
            split = i + 1;
            break;
        }
        used += t;
        split = i;
    }
    let older = &msgs[..split];
    let recent = msgs[split..].to_vec();

    if older.is_empty() {
        return (recent, Some("上下文较长，已省略部分历史".into()));
    }

    if llm.enable_compaction {
        match summarize(st, older).await {
            Ok(summary) => {
                let mut out = vec![Message::system(format!(
                    "以下是本次对话中较早内容的摘要（供你保持连贯，非事实来源）：\n{summary}"
                ))];
                out.extend(recent);
                (out, Some("已将较早的对话压缩为摘要以节省上下文".into()))
            }
            Err(e) => {
                tracing::warn!("摘要压缩失败，退回直接裁剪：{e:#}");
                (recent, Some("上下文较长，已省略较早的对话".into()))
            }
        }
    } else {
        (recent, Some("上下文较长，已省略较早的对话".into()))
    }
}

/// 用 LLM 把较早的对话压成简短摘要。
async fn summarize(st: &AppState, older: &[Message]) -> anyhow::Result<String> {
    let mut convo = String::new();
    for m in older {
        let who = match m.role.as_str() {
            "user" => "用户",
            "assistant" => "助手",
            _ => "系统",
        };
        convo.push_str(&format!("{who}：{}\n", m.content));
    }
    let messages = vec![
        Message::system(
            "你是对话压缩器。把下面的对话浓缩成简洁的要点，保留关键事实、结论、用户的偏好与已达成的决定，\
省略寒暄与冗余。只输出摘要本身。",
        ),
        Message::user(convo),
    ];
    let res = st.llm.chat_once(messages).await?;
    Ok(res.content)
}

// ---------- 文档生成 & 导出 ----------

#[derive(Deserialize)]
struct GenerateReq {
    instruction: String,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    format: String,
    #[serde(default = "default_true")]
    use_knowledge: bool,
    #[serde(default)]
    title: Option<String>,
}

async fn generate(
    State(st): State<AppState>,
    Json(req): Json<GenerateReq>,
) -> Result<Response, AppError> {
    let mut system = build_system_prompt(&st, &req.instruction, req.use_knowledge);

    if let Some(name) = &req.template {
        match templates::find(&st.templates, name) {
            Some(t) => {
                system.push_str(
                    "\n\n# 输出模板（参考其结构、章节与风格来组织你的文档；这是格式参考，不是事实来源）\n",
                );
                system.push_str(&t.content);
            }
            None => {
                return Err(AppError(anyhow::anyhow!("未找到模板：{name}")));
            }
        }
    }

    system.push_str(
        "\n\n只输出最终文档正文，使用规范的 Markdown（标题、列表、表格等）。\
不要有对话式开场白或结束语；不要用 ``` 代码块把整篇文档包起来\
（代码块只用于文档内部真正的代码/JSON 片段）。",
    );

    let messages = vec![
        Message::system(system),
        Message::user(req.instruction.clone()),
    ];
    let content = st.llm.chat_once(messages).await?.content;

    let title = req.title.unwrap_or_else(|| "文档".into());
    let format = Format::parse(&req.format);
    file_response(
        content,
        title,
        format,
        st.config.diagrams.clone(),
        Vec::new(),
    )
    .await
}

#[derive(Deserialize)]
struct ExportReq {
    content: String,
    #[serde(default)]
    format: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    diagrams: Vec<render::diagram::ProvidedDiagram>,
}

async fn export(
    State(st): State<AppState>,
    Json(req): Json<ExportReq>,
) -> Result<Response, AppError> {
    let title = req.title.unwrap_or_else(|| "文档".into());
    let format = Format::parse(&req.format);
    file_response(
        req.content,
        title,
        format,
        st.config.diagrams.clone(),
        req.diagrams,
    )
    .await
}

/// 在阻塞线程里渲染（typst 是外部进程），再包成下载响应。
async fn file_response(
    content: String,
    title: String,
    format: Format,
    diagrams: crate::config::Diagrams,
    diagram_assets: Vec<render::diagram::ProvidedDiagram>,
) -> Result<Response, AppError> {
    let rendered = tokio::task::spawn_blocking(move || {
        render::render(&content, &title, format, &diagrams, &diagram_assets)
    })
    .await??;

    let disposition = format!("attachment; filename=\"{}\"", rendered.filename);
    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_str(&rendered.content_type).unwrap(),
        ),
        (
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&disposition).unwrap(),
        ),
    ];
    Ok((headers, rendered.bytes).into_response())
}
