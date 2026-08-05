use std::{env, fs, path::Path};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub llm: Llm,
    #[serde(default)]
    pub paths: Paths,
    #[serde(default)]
    pub diagrams: Diagrams,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Diagrams {
    /// 是否允许渲染 Markdown 图表代码块。
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default = "d_diagram_max_source_kb")]
    pub max_source_kb: usize,
    #[serde(default = "d_diagram_max_count")]
    pub max_diagrams_per_document: usize,
    #[serde(default = "d_diagram_max_svg_kb")]
    pub max_svg_kb: usize,
    #[serde(default = "d_diagram_max_png_kb")]
    pub max_png_kb: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    #[serde(default = "d_host")]
    pub host: String,
    #[serde(default = "d_port")]
    pub port: u16,
    /// 请求体大小上限（MB）。axum 默认仅 2MB，文档场景显式放大。
    #[serde(default = "d_max_body_mb")]
    pub max_body_mb: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Llm {
    #[serde(default = "d_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "d_model")]
    pub model: String,
    /// 文档生成用的（可选更强）模型；缺省则复用 `model`。
    #[serde(default)]
    pub generate_model: Option<String>,
    #[serde(default = "d_temperature")]
    pub temperature: f32,
    /// 单次读超时（秒）：两次数据块之间的空闲上限，而非整段流的总时长。
    #[serde(default = "d_read_timeout")]
    pub read_timeout_secs: u64,
    /// /generate、/export 等非流式路由的整体超时（秒）。
    #[serde(default = "d_request_timeout")]
    pub request_timeout_secs: u64,
    /// 失败重试次数（429/5xx/网络抖动），指数退避 + 抖动，尊重 Retry-After。
    #[serde(default = "d_max_retries")]
    pub max_retries: u64,
    /// 流式空闲超时（秒）：两次数据块之间超过此值视为卡死/断线。
    #[serde(default = "d_stream_idle")]
    pub stream_idle_timeout_secs: u64,
    /// 模型上下文窗口（token）。用于历史裁剪/压缩预算判断。
    #[serde(default = "d_context_window")]
    pub context_window_tokens: usize,
    /// 为回答预留的 token（从窗口里扣除，作为输入预算上限）。
    #[serde(default = "d_max_response")]
    pub max_response_tokens: usize,
    /// 历史超预算时是否用 LLM 摘要压缩较早对话（否则直接丢弃最老的）。
    #[serde(default = "d_true")]
    pub enable_compaction: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Paths {
    #[serde(default = "d_knowledge_dir")]
    pub knowledge_dir: String,
    #[serde(default = "d_templates_dir")]
    pub templates_dir: String,
    #[serde(default = "d_static_dir")]
    pub static_dir: String,
    /// 指导 documentx 行为的指令文件（如 AGENTS.md）；存在则作为系统提示加载。
    /// 缺省不设置，使用内置默认提示。
    #[serde(default)]
    pub agents_file: Option<String>,
}

impl Config {
    /// 从 toml 文件加载（文件不存在则用默认值），再用环境变量覆盖敏感项。
    pub fn load(path: &str) -> anyhow::Result<Config> {
        let mut cfg: Config = if Path::new(path).exists() {
            toml::from_str(&fs::read_to_string(path)?)?
        } else {
            Config::default()
        };

        if let Ok(v) = env::var("LLM_BASE_URL") {
            cfg.llm.base_url = v;
        }
        if let Ok(v) = env::var("LLM_API_KEY") {
            cfg.llm.api_key = v;
        }
        if let Ok(v) = env::var("LLM_MODEL") {
            cfg.llm.model = v;
        }
        if let Ok(v) = env::var("LLM_GENERATE_MODEL") {
            cfg.llm.generate_model = Some(v);
        }
        if let Ok(v) = env::var("PORT") {
            if let Ok(p) = v.parse() {
                cfg.server.port = p;
            }
        }
        if let Ok(v) = env::var("DIAGRAMS_ENABLED") {
            cfg.diagrams.enabled =
                matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on");
        }

        // 归一化 base_url：去掉末尾斜杠。
        while cfg.llm.base_url.ends_with('/') {
            cfg.llm.base_url.pop();
        }
        Ok(cfg)
    }
}

fn d_host() -> String {
    "0.0.0.0".into()
}
fn d_port() -> u16 {
    8080
}
fn d_max_body_mb() -> usize {
    16
}
fn d_base_url() -> String {
    "https://api.openai.com/v1".into()
}
fn d_model() -> String {
    "gpt-4o-mini".into()
}
fn d_temperature() -> f32 {
    0.3
}
fn d_read_timeout() -> u64 {
    120
}
fn d_request_timeout() -> u64 {
    240
}
fn d_max_retries() -> u64 {
    4
}
fn d_stream_idle() -> u64 {
    120
}
fn d_context_window() -> usize {
    128_000
}
fn d_max_response() -> usize {
    4_096
}
fn d_true() -> bool {
    true
}
fn d_knowledge_dir() -> String {
    "knowledge".into()
}
fn d_templates_dir() -> String {
    "templates".into()
}
fn d_static_dir() -> String {
    "frontend/dist".into()
}
fn d_diagram_max_source_kb() -> usize {
    256
}
fn d_diagram_max_count() -> usize {
    32
}
fn d_diagram_max_svg_kb() -> usize {
    1024
}
fn d_diagram_max_png_kb() -> usize {
    8192
}

impl Default for Config {
    fn default() -> Self {
        Config {
            server: Server::default(),
            llm: Llm::default(),
            paths: Paths::default(),
            diagrams: Diagrams::default(),
        }
    }
}
impl Default for Diagrams {
    fn default() -> Self {
        Diagrams {
            enabled: true,
            max_source_kb: d_diagram_max_source_kb(),
            max_diagrams_per_document: d_diagram_max_count(),
            max_svg_kb: d_diagram_max_svg_kb(),
            max_png_kb: d_diagram_max_png_kb(),
        }
    }
}
impl Default for Server {
    fn default() -> Self {
        Server {
            host: d_host(),
            port: d_port(),
            max_body_mb: d_max_body_mb(),
        }
    }
}
impl Default for Llm {
    fn default() -> Self {
        Llm {
            base_url: d_base_url(),
            api_key: String::new(),
            model: d_model(),
            generate_model: None,
            temperature: d_temperature(),
            read_timeout_secs: d_read_timeout(),
            request_timeout_secs: d_request_timeout(),
            max_retries: d_max_retries(),
            stream_idle_timeout_secs: d_stream_idle(),
            context_window_tokens: d_context_window(),
            max_response_tokens: d_max_response(),
            enable_compaction: true,
        }
    }
}
impl Default for Paths {
    fn default() -> Self {
        Paths {
            knowledge_dir: d_knowledge_dir(),
            templates_dir: d_templates_dir(),
            static_dir: d_static_dir(),
            agents_file: None,
        }
    }
}
