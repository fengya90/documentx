use std::{env, fmt, fs, path::Path, str::FromStr};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: Server,
    #[serde(default)]
    pub llm: Llm,
    #[serde(default)]
    pub paths: Paths,
    #[serde(default)]
    pub diagrams: Diagrams,
    #[serde(default)]
    pub ui: Ui,
    #[serde(default)]
    pub content: Content,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ui {
    #[serde(default = "d_ui_brand_title")]
    pub brand_title: String,
    #[serde(default = "d_ui_brand_subtitle")]
    pub brand_subtitle: String,
    #[serde(default = "d_ui_welcome_title")]
    pub welcome_title: String,
    #[serde(default = "d_ui_welcome_description")]
    pub welcome_description: String,
    #[serde(default = "d_ui_suggestions")]
    pub suggestions: Vec<String>,
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
    /// 对外 Web 基路径；空字符串表示根路径，例如 `/documentx` 表示所有页面和 API 均挂载于其下。
    #[serde(default)]
    pub base_path: String,
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
    /// local 内容模式的知识库目录。
    #[serde(default = "d_knowledge_dir")]
    pub knowledge_dir: String,
    /// local 内容模式的模板目录。
    #[serde(default = "d_templates_dir")]
    pub templates_dir: String,
    #[serde(default = "d_static_dir")]
    pub static_dir: String,
    /// local 内容模式下的智能体指令文件；不存在时使用内置默认提示。
    #[serde(default)]
    pub agents_file: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContentMode {
    #[default]
    Local,
    #[serde(alias = "oss")]
    S3,
}

impl fmt::Display for ContentMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentMode::Local => f.write_str("local"),
            ContentMode::S3 => f.write_str("s3"),
        }
    }
}

impl FromStr for ContentMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" => Ok(ContentMode::Local),
            "s3" | "oss" => Ok(ContentMode::S3),
            _ => bail!("只支持 local 或 s3"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Content {
    #[serde(default)]
    pub mode: ContentMode,
    /// 定时刷新间隔；0 表示关闭定时刷新，但启动时仍会全量加载。
    #[serde(default = "d_refresh_interval")]
    pub refresh_interval_secs: u64,
    /// 单个 AGENTS/知识/模板文件大小上限。
    #[serde(default = "d_content_max_file_kb")]
    pub max_file_kb: usize,
    /// AGENTS、知识和模板合计文件数上限。
    #[serde(default = "d_content_max_files")]
    pub max_files: usize,
    #[serde(default)]
    pub s3: S3,
}

#[derive(Debug, Clone, Deserialize)]
pub struct S3 {
    /// S3 兼容端点；不填时使用 AWS 区域默认端点。
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default = "d_s3_region")]
    pub region: String,
    #[serde(default)]
    pub bucket: String,
    /// Bucket 内的统一根目录，固定读取其下 AGENTS.md、knowledge/、templates/。
    #[serde(default = "d_s3_root_prefix")]
    pub root_prefix: String,
    #[serde(default = "d_true")]
    pub force_path_style: bool,
    #[serde(default)]
    pub allow_http: bool,
    #[serde(default = "d_s3_download_concurrency")]
    pub download_concurrency: usize,
}

impl Config {
    /// 从 TOML 加载（文件不存在则用默认值），再用环境变量覆盖所有配置项。
    pub fn load(path: &str) -> anyhow::Result<Config> {
        let mut config: Config = if Path::new(path).exists() {
            toml::from_str(&fs::read_to_string(path)?)?
        } else {
            Config::default()
        };
        config.apply_environment(|name| env::var(name).ok())?;
        config.normalize_and_validate()?;
        Ok(config)
    }

    fn apply_environment<F>(&mut self, get: F) -> anyhow::Result<()>
    where
        F: Fn(&str) -> Option<String>,
    {
        macro_rules! string {
            ($name:literal, $target:expr) => {
                if let Some(value) = get($name) {
                    $target = value;
                }
            };
        }
        macro_rules! optional_string {
            ($name:literal, $target:expr) => {
                if let Some(value) = get($name) {
                    $target = non_empty(value);
                }
            };
        }
        macro_rules! parsed {
            ($name:literal, $target:expr) => {
                if let Some(value) = get($name) {
                    $target = value.parse().with_context(|| {
                        format!("环境变量 {} 的值 {:?} 无效", $name, value)
                    })?;
                }
            };
        }
        macro_rules! boolean {
            ($name:literal, $target:expr) => {
                if let Some(value) = get($name) {
                    $target = parse_bool($name, &value)?;
                }
            };
        }

        // 兼容已有部署变量；下方 DOCUMENTX_* 同名项优先级更高。
        string!("LLM_BASE_URL", self.llm.base_url);
        string!("LLM_API_KEY", self.llm.api_key);
        string!("LLM_MODEL", self.llm.model);
        optional_string!("LLM_GENERATE_MODEL", self.llm.generate_model);
        parsed!("PORT", self.server.port);
        boolean!("DIAGRAMS_ENABLED", self.diagrams.enabled);

        string!("DOCUMENTX_SERVER_HOST", self.server.host);
        parsed!("DOCUMENTX_SERVER_PORT", self.server.port);
        string!("DOCUMENTX_SERVER_BASE_PATH", self.server.base_path);
        parsed!("DOCUMENTX_SERVER_MAX_BODY_MB", self.server.max_body_mb);

        string!("DOCUMENTX_LLM_BASE_URL", self.llm.base_url);
        string!("DOCUMENTX_LLM_API_KEY", self.llm.api_key);
        string!("DOCUMENTX_LLM_MODEL", self.llm.model);
        optional_string!("DOCUMENTX_LLM_GENERATE_MODEL", self.llm.generate_model);
        parsed!("DOCUMENTX_LLM_TEMPERATURE", self.llm.temperature);
        parsed!(
            "DOCUMENTX_LLM_READ_TIMEOUT_SECS",
            self.llm.read_timeout_secs
        );
        parsed!(
            "DOCUMENTX_LLM_REQUEST_TIMEOUT_SECS",
            self.llm.request_timeout_secs
        );
        parsed!("DOCUMENTX_LLM_MAX_RETRIES", self.llm.max_retries);
        parsed!(
            "DOCUMENTX_LLM_STREAM_IDLE_TIMEOUT_SECS",
            self.llm.stream_idle_timeout_secs
        );
        parsed!(
            "DOCUMENTX_LLM_CONTEXT_WINDOW_TOKENS",
            self.llm.context_window_tokens
        );
        parsed!(
            "DOCUMENTX_LLM_MAX_RESPONSE_TOKENS",
            self.llm.max_response_tokens
        );
        boolean!(
            "DOCUMENTX_LLM_ENABLE_COMPACTION",
            self.llm.enable_compaction
        );

        string!("DOCUMENTX_PATHS_KNOWLEDGE_DIR", self.paths.knowledge_dir);
        string!("DOCUMENTX_PATHS_TEMPLATES_DIR", self.paths.templates_dir);
        string!("DOCUMENTX_PATHS_STATIC_DIR", self.paths.static_dir);
        optional_string!("DOCUMENTX_PATHS_AGENTS_FILE", self.paths.agents_file);

        boolean!("DOCUMENTX_DIAGRAMS_ENABLED", self.diagrams.enabled);
        parsed!(
            "DOCUMENTX_DIAGRAMS_MAX_SOURCE_KB",
            self.diagrams.max_source_kb
        );
        parsed!(
            "DOCUMENTX_DIAGRAMS_MAX_PER_DOCUMENT",
            self.diagrams.max_diagrams_per_document
        );
        parsed!("DOCUMENTX_DIAGRAMS_MAX_SVG_KB", self.diagrams.max_svg_kb);
        parsed!("DOCUMENTX_DIAGRAMS_MAX_PNG_KB", self.diagrams.max_png_kb);

        string!("DOCUMENTX_UI_BRAND_TITLE", self.ui.brand_title);
        string!("DOCUMENTX_UI_BRAND_SUBTITLE", self.ui.brand_subtitle);
        string!("DOCUMENTX_UI_WELCOME_TITLE", self.ui.welcome_title);
        string!(
            "DOCUMENTX_UI_WELCOME_DESCRIPTION",
            self.ui.welcome_description
        );
        if let Some(value) = get("DOCUMENTX_UI_SUGGESTIONS") {
            self.ui.suggestions = serde_json::from_str(&value).with_context(|| {
                "环境变量 DOCUMENTX_UI_SUGGESTIONS 必须是 JSON 字符串数组".to_owned()
            })?;
        }

        parsed!("DOCUMENTX_CONTENT_MODE", self.content.mode);
        parsed!(
            "DOCUMENTX_CONTENT_REFRESH_INTERVAL_SECS",
            self.content.refresh_interval_secs
        );
        parsed!("DOCUMENTX_CONTENT_MAX_FILE_KB", self.content.max_file_kb);
        parsed!("DOCUMENTX_CONTENT_MAX_FILES", self.content.max_files);
        optional_string!("DOCUMENTX_CONTENT_S3_ENDPOINT", self.content.s3.endpoint);
        string!("DOCUMENTX_CONTENT_S3_REGION", self.content.s3.region);
        string!("DOCUMENTX_CONTENT_S3_BUCKET", self.content.s3.bucket);
        string!(
            "DOCUMENTX_CONTENT_S3_ROOT_PREFIX",
            self.content.s3.root_prefix
        );
        boolean!(
            "DOCUMENTX_CONTENT_S3_FORCE_PATH_STYLE",
            self.content.s3.force_path_style
        );
        boolean!(
            "DOCUMENTX_CONTENT_S3_ALLOW_HTTP",
            self.content.s3.allow_http
        );
        parsed!(
            "DOCUMENTX_CONTENT_S3_DOWNLOAD_CONCURRENCY",
            self.content.s3.download_concurrency
        );
        Ok(())
    }

    fn normalize_and_validate(&mut self) -> anyhow::Result<()> {
        self.server.host = self.server.host.trim().to_owned();
        self.server.base_path = normalize_base_path(&self.server.base_path)?;
        self.llm.base_url = self.llm.base_url.trim_end_matches('/').to_owned();
        self.content.s3.endpoint = self
            .content
            .s3
            .endpoint
            .take()
            .and_then(non_empty)
            .map(|value| value.trim_end_matches('/').to_owned());
        self.content.s3.root_prefix = normalize_prefix(&self.content.s3.root_prefix)?;

        if self.server.host.is_empty() {
            bail!("server.host 不能为空");
        }
        if self.server.max_body_mb == 0 {
            bail!("server.max_body_mb 必须大于 0");
        }
        if self.llm.base_url.is_empty() || self.llm.model.trim().is_empty() {
            bail!("llm.base_url 和 llm.model 不能为空");
        }
        if self.content.max_file_kb == 0 || self.content.max_files == 0 {
            bail!("content.max_file_kb 和 content.max_files 必须大于 0");
        }
        if self.ui.suggestions.len() > 12 {
            bail!("ui.suggestions 最多配置 12 条");
        }
        if self.content.mode == ContentMode::S3 {
            if self.content.s3.bucket.trim().is_empty() {
                bail!("S3 模式必须配置 content.s3.bucket");
            }
            if self.content.s3.download_concurrency == 0 {
                bail!("content.s3.download_concurrency 必须大于 0");
            }
        }
        Ok(())
    }
}

fn parse_bool(name: &str, value: &str) -> anyhow::Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => bail!("环境变量 {name} 的布尔值 {value:?} 无效"),
    }
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn normalize_prefix(prefix: &str) -> anyhow::Result<String> {
    let normalized = prefix.trim().trim_matches('/');
    if normalized.contains('\\')
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("content.s3.root_prefix 不是安全的对象路径：{prefix:?}");
    }
    Ok(normalized.to_owned())
}

fn normalize_base_path(path: &str) -> anyhow::Result<String> {
    let path = path.trim();
    if path.is_empty() || path == "/" {
        return Ok(String::new());
    }
    let with_leading_slash = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let normalized = with_leading_slash.trim_end_matches('/');
    if normalized.contains('\\')
        || normalized.contains('?')
        || normalized.contains('#')
        || normalized
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("server.base_path 不是安全的 URL 路径：{path:?}");
    }
    Ok(normalized.to_owned())
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
fn d_ui_brand_title() -> String {
    "DocumentX".into()
}
fn d_ui_brand_subtitle() -> String {
    "文档智能体 · 小文".into()
}
fn d_ui_welcome_title() -> String {
    "嗨，我是小文 👋".into()
}
fn d_ui_welcome_description() -> String {
    "DocumentX 的文档助手。我会基于你的知识库回答，也能按模板产出可下载的 PDF / Word / Markdown。"
        .into()
}
fn d_ui_suggestions() -> Vec<String> {
    [
        "总结一下知识库里的核心内容",
        "按对外API文档模板生成一份文档",
        "列出所有接口及其用途",
        "解释其中的认证与鉴权流程",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
fn d_refresh_interval() -> u64 {
    300
}
fn d_content_max_file_kb() -> usize {
    2_048
}
fn d_content_max_files() -> usize {
    10_000
}
fn d_s3_region() -> String {
    "us-east-1".into()
}
fn d_s3_root_prefix() -> String {
    "documentx".into()
}
fn d_s3_download_concurrency() -> usize {
    8
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
impl Default for Ui {
    fn default() -> Self {
        Self {
            brand_title: d_ui_brand_title(),
            brand_subtitle: d_ui_brand_subtitle(),
            welcome_title: d_ui_welcome_title(),
            welcome_description: d_ui_welcome_description(),
            suggestions: d_ui_suggestions(),
        }
    }
}
impl Default for Server {
    fn default() -> Self {
        Server {
            host: d_host(),
            port: d_port(),
            base_path: String::new(),
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
impl Default for Content {
    fn default() -> Self {
        Content {
            mode: ContentMode::Local,
            refresh_interval_secs: d_refresh_interval(),
            max_file_kb: d_content_max_file_kb(),
            max_files: d_content_max_files(),
            s3: S3::default(),
        }
    }
}
impl Default for S3 {
    fn default() -> Self {
        S3 {
            endpoint: None,
            region: d_s3_region(),
            bucket: String::new(),
            root_prefix: d_s3_root_prefix(),
            force_path_style: true,
            allow_http: false,
            download_concurrency: d_s3_download_concurrency(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn every_section_can_be_overridden_by_environment() {
        let values = HashMap::from([
            ("DOCUMENTX_SERVER_HOST", "127.0.0.1"),
            ("DOCUMENTX_SERVER_PORT", "18080"),
            ("DOCUMENTX_SERVER_BASE_PATH", "documentx/"),
            ("DOCUMENTX_SERVER_MAX_BODY_MB", "32"),
            ("DOCUMENTX_LLM_BASE_URL", "https://llm.example/v1/"),
            ("DOCUMENTX_LLM_API_KEY", "test-key"),
            ("DOCUMENTX_LLM_MODEL", "chat-model"),
            ("DOCUMENTX_LLM_GENERATE_MODEL", "generate-model"),
            ("DOCUMENTX_LLM_TEMPERATURE", "0.7"),
            ("DOCUMENTX_LLM_READ_TIMEOUT_SECS", "61"),
            ("DOCUMENTX_LLM_REQUEST_TIMEOUT_SECS", "62"),
            ("DOCUMENTX_LLM_MAX_RETRIES", "3"),
            ("DOCUMENTX_LLM_STREAM_IDLE_TIMEOUT_SECS", "63"),
            ("DOCUMENTX_LLM_CONTEXT_WINDOW_TOKENS", "64000"),
            ("DOCUMENTX_LLM_MAX_RESPONSE_TOKENS", "2048"),
            ("DOCUMENTX_LLM_ENABLE_COMPACTION", "false"),
            ("DOCUMENTX_PATHS_KNOWLEDGE_DIR", "/data/knowledge"),
            ("DOCUMENTX_PATHS_TEMPLATES_DIR", "/data/templates"),
            ("DOCUMENTX_PATHS_STATIC_DIR", "/app/web"),
            ("DOCUMENTX_PATHS_AGENTS_FILE", "/data/AGENTS.md"),
            ("DOCUMENTX_DIAGRAMS_ENABLED", "false"),
            ("DOCUMENTX_DIAGRAMS_MAX_SOURCE_KB", "128"),
            ("DOCUMENTX_DIAGRAMS_MAX_PER_DOCUMENT", "16"),
            ("DOCUMENTX_DIAGRAMS_MAX_SVG_KB", "512"),
            ("DOCUMENTX_DIAGRAMS_MAX_PNG_KB", "4096"),
            ("DOCUMENTX_UI_BRAND_TITLE", "知识助手"),
            ("DOCUMENTX_UI_BRAND_SUBTITLE", "研发文档中心"),
            ("DOCUMENTX_UI_WELCOME_TITLE", "你好，我是研发助手"),
            ("DOCUMENTX_UI_WELCOME_DESCRIPTION", "请问我技术问题"),
            (
                "DOCUMENTX_UI_SUGGESTIONS",
                "[\"架构是什么？\",\"有哪些 API？\"]",
            ),
            ("DOCUMENTX_CONTENT_MODE", "oss"),
            ("DOCUMENTX_CONTENT_REFRESH_INTERVAL_SECS", "90"),
            ("DOCUMENTX_CONTENT_MAX_FILE_KB", "1024"),
            ("DOCUMENTX_CONTENT_MAX_FILES", "500"),
            ("DOCUMENTX_CONTENT_S3_ENDPOINT", "http://minio:9000/"),
            ("DOCUMENTX_CONTENT_S3_REGION", "cn-test"),
            ("DOCUMENTX_CONTENT_S3_BUCKET", "docs"),
            ("DOCUMENTX_CONTENT_S3_ROOT_PREFIX", "/documentx/"),
            ("DOCUMENTX_CONTENT_S3_FORCE_PATH_STYLE", "true"),
            ("DOCUMENTX_CONTENT_S3_ALLOW_HTTP", "true"),
            ("DOCUMENTX_CONTENT_S3_DOWNLOAD_CONCURRENCY", "4"),
        ]);
        let mut config = Config::default();
        config
            .apply_environment(|name| values.get(name).map(|value| (*value).to_owned()))
            .unwrap();
        config.normalize_and_validate().unwrap();

        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 18080);
        assert_eq!(config.server.base_path, "/documentx");
        assert_eq!(config.server.max_body_mb, 32);
        assert_eq!(config.llm.base_url, "https://llm.example/v1");
        assert_eq!(config.llm.api_key, "test-key");
        assert_eq!(config.llm.model, "chat-model");
        assert_eq!(config.llm.generate_model.as_deref(), Some("generate-model"));
        assert_eq!(config.llm.temperature, 0.7);
        assert_eq!(config.llm.read_timeout_secs, 61);
        assert_eq!(config.llm.request_timeout_secs, 62);
        assert_eq!(config.llm.max_retries, 3);
        assert_eq!(config.llm.stream_idle_timeout_secs, 63);
        assert_eq!(config.llm.context_window_tokens, 64_000);
        assert_eq!(config.llm.max_response_tokens, 2_048);
        assert!(!config.llm.enable_compaction);
        assert_eq!(config.paths.knowledge_dir, "/data/knowledge");
        assert_eq!(config.paths.templates_dir, "/data/templates");
        assert_eq!(config.paths.static_dir, "/app/web");
        assert_eq!(config.paths.agents_file.as_deref(), Some("/data/AGENTS.md"));
        assert!(!config.diagrams.enabled);
        assert_eq!(config.diagrams.max_source_kb, 128);
        assert_eq!(config.diagrams.max_diagrams_per_document, 16);
        assert_eq!(config.diagrams.max_svg_kb, 512);
        assert_eq!(config.diagrams.max_png_kb, 4_096);
        assert_eq!(config.ui.brand_title, "知识助手");
        assert_eq!(config.ui.brand_subtitle, "研发文档中心");
        assert_eq!(config.ui.welcome_title, "你好，我是研发助手");
        assert_eq!(config.ui.welcome_description, "请问我技术问题");
        assert_eq!(config.ui.suggestions, ["架构是什么？", "有哪些 API？"]);
        assert_eq!(config.content.mode, ContentMode::S3);
        assert_eq!(config.content.refresh_interval_secs, 90);
        assert_eq!(config.content.max_file_kb, 1_024);
        assert_eq!(config.content.max_files, 500);
        assert_eq!(
            config.content.s3.endpoint.as_deref(),
            Some("http://minio:9000")
        );
        assert_eq!(config.content.s3.region, "cn-test");
        assert_eq!(config.content.s3.bucket, "docs");
        assert_eq!(config.content.s3.root_prefix, "documentx");
        assert!(config.content.s3.force_path_style);
        assert!(config.content.s3.allow_http);
        assert_eq!(config.content.s3.download_concurrency, 4);
    }

    #[test]
    fn invalid_environment_values_fail_fast() {
        let mut config = Config::default();
        let error = config
            .apply_environment(|name| {
                (name == "DOCUMENTX_SERVER_PORT").then(|| "not-a-port".to_owned())
            })
            .unwrap_err();
        assert!(error.to_string().contains("DOCUMENTX_SERVER_PORT"));
    }

    #[test]
    fn base_path_is_normalized_and_rejects_ambiguous_segments() {
        assert_eq!(normalize_base_path(" /documentx/ ").unwrap(), "/documentx");
        assert_eq!(normalize_base_path("/").unwrap(), "");
        assert!(normalize_base_path("/a/../b").is_err());
        assert!(normalize_base_path("/a//b").is_err());
    }
}
