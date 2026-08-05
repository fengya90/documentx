use std::{sync::Arc, time::Duration};

use anyhow::{bail, Context};
use futures::Stream;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Message {
            role: "system".into(),
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: "user".into(),
            content: content.into(),
        }
    }
}

/// token 用量（来自服务端返回的 usage 字段）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

/// 非流式调用结果。
pub struct ChatResult {
    pub content: String,
    #[allow(dead_code)]
    pub usage: Option<Usage>,
}

/// 流式事件：正文增量 / 状态提示（重连、压缩等）/ token 用量。
pub enum StreamEvent {
    Delta(String),
    Status(String),
    Usage(Usage),
}

/// 指数退避 + 抖动：200ms × 2^(attempt-1) × (0.9~1.1)。
fn backoff(attempt: u64) -> Duration {
    let base = 200f64 * 2f64.powi(attempt.saturating_sub(1) as i32);
    let jitter = rand::thread_rng().gen_range(0.9..1.1);
    Duration::from_millis((base * jitter) as u64)
}

fn is_retryable(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// 解析 Retry-After 头（仅支持秒数形式）。
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// OpenAI Chat Completions 兼容客户端。
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    generate_model: String,
    temperature: f32,
    max_retries: u64,
    stream_idle_timeout: Duration,
}

impl LlmClient {
    pub fn new(cfg: &Config) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(cfg.llm.read_timeout_secs))
            .pool_max_idle_per_host(8)
            .pool_idle_timeout(Duration::from_secs(90))
            .gzip(true)
            .build()
            .context("构建 HTTP 客户端失败")?;

        Ok(LlmClient {
            http,
            base_url: cfg.llm.base_url.clone(),
            api_key: cfg.llm.api_key.clone(),
            model: cfg.llm.model.clone(),
            generate_model: cfg
                .llm
                .generate_model
                .clone()
                .unwrap_or_else(|| cfg.llm.model.clone()),
            temperature: cfg.llm.temperature,
            max_retries: cfg.llm.max_retries,
            stream_idle_timeout: Duration::from_secs(cfg.llm.stream_idle_timeout_secs),
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn ensure_key(&self) -> anyhow::Result<()> {
        if self.api_key.is_empty() {
            bail!("未配置 LLM api_key，请在 config.toml 或环境变量 LLM_API_KEY 中设置");
        }
        Ok(())
    }

    /// 流式对话：内建"建连阶段重试"（首个 token 前）与"流空闲超时"。
    /// 返回逐步产出的事件流（正文增量 / 状态 / usage）。
    pub fn chat_stream(
        self: Arc<Self>,
        messages: Vec<Message>,
    ) -> impl Stream<Item = anyhow::Result<StreamEvent>> {
        let idle = self.stream_idle_timeout;
        let max_retries = self.max_retries;

        async_stream::stream! {
            use futures::StreamExt;

            if let Err(e) = self.ensure_key() {
                yield Err(e);
                return;
            }

            let body = json!({
                "model": self.model,
                "messages": messages,
                "stream": true,
                "temperature": self.temperature,
                // 让服务端在流末尾附带 usage（多数 OpenAI 兼容端点支持）。
                "stream_options": { "include_usage": true },
            });

            // ---- 建连阶段：失败可重试（此时还没吐 token，重发安全）----
            let mut attempt = 0u64;
            let resp = loop {
                let sent = self
                    .http
                    .post(self.endpoint())
                    .bearer_auth(&self.api_key)
                    .json(&body)
                    .send()
                    .await;

                match sent {
                    Ok(r) if r.status().is_success() => break r,
                    Ok(r) => {
                        let status = r.status();
                        let retry_after = parse_retry_after(r.headers());
                        let text = r.text().await.unwrap_or_default();
                        if is_retryable(status) && attempt < max_retries {
                            attempt += 1;
                            let delay = retry_after.unwrap_or_else(|| backoff(attempt));
                            yield Ok(StreamEvent::Status(format!("重连中 {attempt}/{max_retries}…")));
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        yield Err(anyhow::anyhow!("LLM 返回 {status}: {text}"));
                        return;
                    }
                    Err(e) => {
                        if attempt < max_retries {
                            attempt += 1;
                            let delay = backoff(attempt);
                            yield Ok(StreamEvent::Status(format!("重连中 {attempt}/{max_retries}…")));
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        yield Err(anyhow::anyhow!("请求 LLM 失败：{e}"));
                        return;
                    }
                }
            };

            // ---- 解析 SSE：按字节缓冲、按行切分（避免多字节字符被截断），每次拉流套空闲超时 ----
            let mut bytes = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            loop {
                let polled = tokio::time::timeout(idle, bytes.next()).await;
                let chunk = match polled {
                    Err(_) => {
                        yield Err(anyhow::anyhow!("流空闲超时（{}s 无数据）", idle.as_secs()));
                        return;
                    }
                    Ok(None) => break,
                    Ok(Some(Err(e))) => {
                        yield Err(anyhow::anyhow!("读取流失败：{e}"));
                        return;
                    }
                    Ok(Some(Ok(c))) => c,
                };
                buf.extend_from_slice(&chunk);
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line_bytes);
                    let line = line.trim();
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data.is_empty() { continue; }
                    if data == "[DONE]" { return; }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(c) = v["choices"][0]["delta"]["content"].as_str() {
                            if !c.is_empty() {
                                yield Ok(StreamEvent::Delta(c.to_string()));
                            }
                        }
                        if let Some(u) = v.get("usage") {
                            if !u.is_null() {
                                if let Ok(usage) = serde_json::from_value::<Usage>(u.clone()) {
                                    yield Ok(StreamEvent::Usage(usage));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// 非流式对话（含重试），返回完整内容与 usage。
    pub async fn chat_once(&self, messages: Vec<Message>) -> anyhow::Result<ChatResult> {
        self.ensure_key()?;
        let body = json!({
            "model": self.generate_model,
            "messages": messages,
            "stream": false,
            "temperature": self.temperature,
        });

        let mut attempt = 0u64;
        loop {
            let sent = self
                .http
                .post(self.endpoint())
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;

            match sent {
                Ok(r) if r.status().is_success() => {
                    let v: serde_json::Value = r.json().await.context("解析 LLM 响应失败")?;
                    let content = v["choices"][0]["message"]["content"]
                        .as_str()
                        .context("LLM 响应缺少 content")?
                        .to_string();
                    let usage = serde_json::from_value::<Usage>(v["usage"].clone()).ok();
                    return Ok(ChatResult { content, usage });
                }
                Ok(r) => {
                    let status = r.status();
                    let retry_after = parse_retry_after(r.headers());
                    let text = r.text().await.unwrap_or_default();
                    if is_retryable(status) && attempt < self.max_retries {
                        attempt += 1;
                        let delay = retry_after.unwrap_or_else(|| backoff(attempt));
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    bail!("LLM 返回 {status}: {text}");
                }
                Err(e) => {
                    if attempt < self.max_retries {
                        attempt += 1;
                        tokio::time::sleep(backoff(attempt)).await;
                        continue;
                    }
                    return Err(anyhow::anyhow!("请求 LLM 失败：{e}"));
                }
            }
        }
    }
}
