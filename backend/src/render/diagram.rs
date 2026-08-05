//! Markdown 图表代码块与浏览器预渲染资产校验。
//!
//! Mermaid / Graphviz / Vega 引擎随前端打包。浏览器生成 SVG 和高清 PNG，
//! 后端只做严格匹配、安全校验并分别嵌入 PDF / Word，不访问任何外部服务。

use anyhow::{anyhow, bail, Context};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::Diagrams;

const MARKER_PREFIX: &str = "DOCUMENTX_DIAGRAM_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Svg,
    Png,
}

#[derive(Debug, Clone)]
pub struct DiagramAsset {
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct PreparedMarkdown {
    pub markdown: String,
    pub diagrams: Vec<DiagramAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProvidedDiagram {
    pub kind: String,
    pub source: String,
    pub source_hash: String,
    pub svg: String,
    pub png_base64: String,
}

/// 识别图表 fenced code block，并按顺序绑定浏览器提交的渲染资产。
pub fn prepare(
    md: &str,
    provided: &[ProvidedDiagram],
    format: OutputFormat,
    config: &Diagrams,
) -> anyhow::Result<PreparedMarkdown> {
    if !config.enabled {
        return Ok(PreparedMarkdown {
            markdown: md.to_string(),
            diagrams: Vec::new(),
        });
    }

    let lines: Vec<&str> = md.lines().collect();
    let mut output = Vec::with_capacity(lines.len());
    let mut diagrams = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Web 端在生成资产时同步写入占位符，因此导出链路不需要再次猜测
        // Markdown fence 边界；API 客户端仍可直接提交标准图表代码块。
        if let Some(index) = marker_index(line) {
            if index != diagrams.len() {
                bail!("图表占位符顺序无效：期望 {}，收到 {index}", diagrams.len());
            }
            check_document_limit(diagrams.len(), config)?;
            let asset = provided.get(index).ok_or_else(|| {
                anyhow!(
                    "第 {} 个图表缺少浏览器渲染资产，请从 DocumentX 网页重新导出",
                    index + 1
                )
            })?;
            validate_asset_integrity(asset, index, config)?;
            diagrams.push(DiagramAsset {
                bytes: output_bytes(asset, format, config, index)?,
            });
            output.push(marker(index));
            i += 1;
            continue;
        }

        let Some((fence_marker, fence_len, info)) = opening_fence(line) else {
            output.push(line.to_string());
            i += 1;
            continue;
        };
        let Some(kind) = normalize_kind(info) else {
            let end = find_closing_fence(&lines, i + 1, fence_marker, fence_len);
            let next = if end < lines.len() {
                end + 1
            } else {
                lines.len()
            };
            output.extend(lines[i..next].iter().map(|line| (*line).to_string()));
            i = next;
            continue;
        };

        let end = find_closing_fence(&lines, i + 1, fence_marker, fence_len);
        if end == lines.len() {
            output.extend(lines[i..].iter().map(|line| (*line).to_string()));
            i = lines.len();
            continue;
        }
        check_document_limit(diagrams.len(), config)?;
        let source = lines[i + 1..end].join("\n");
        if source.len() > config.max_source_kb.saturating_mul(1024) {
            bail!("第 {} 个图表源码超过大小上限", diagrams.len() + 1);
        }
        let index = diagrams.len();
        let asset = provided.get(index).ok_or_else(|| {
            anyhow!(
                "第 {} 个 {kind} 图表缺少浏览器渲染资产，请从 DocumentX 网页重新导出",
                index + 1
            )
        })?;
        validate_binding(asset, kind, &source, index)?;
        diagrams.push(DiagramAsset {
            bytes: output_bytes(asset, format, config, index)?,
        });
        output.push(marker(index));
        i = end + 1;
    }

    if provided.len() != diagrams.len() {
        bail!(
            "图表资产数量与 Markdown 不一致：正文 {} 个，收到 {} 个",
            diagrams.len(),
            provided.len()
        );
    }
    Ok(PreparedMarkdown {
        markdown: output.join("\n"),
        diagrams,
    })
}

fn check_document_limit(count: usize, config: &Diagrams) -> anyhow::Result<()> {
    if count >= config.max_diagrams_per_document {
        bail!(
            "单篇文档图表数量超过 {} 个上限",
            config.max_diagrams_per_document
        );
    }
    Ok(())
}

fn validate_asset_integrity(
    asset: &ProvidedDiagram,
    index: usize,
    config: &Diagrams,
) -> anyhow::Result<&'static str> {
    let kind = normalize_kind(&asset.kind)
        .ok_or_else(|| anyhow!("第 {} 个图表类型不受支持", index + 1))?;
    if asset.source.len() > config.max_source_kb.saturating_mul(1024) {
        bail!("第 {} 个图表源码超过大小上限", index + 1);
    }
    if asset.source_hash != source_hash(kind, &asset.source) {
        bail!("第 {} 个图表源码校验失败", index + 1);
    }
    Ok(kind)
}

fn output_bytes(
    asset: &ProvidedDiagram,
    format: OutputFormat,
    config: &Diagrams,
    index: usize,
) -> anyhow::Result<Vec<u8>> {
    match format {
        OutputFormat::Svg => {
            if asset.svg.len() > config.max_svg_kb.saturating_mul(1024) {
                bail!("第 {} 个图表 SVG 超过大小上限", index + 1);
            }
            let bytes = asset.svg.as_bytes().to_vec();
            validate_output(&bytes, OutputFormat::Svg)?;
            Ok(bytes)
        }
        OutputFormat::Png => {
            let bytes = BASE64
                .decode(&asset.png_base64)
                .with_context(|| format!("第 {} 个图表 PNG 编码无效", index + 1))?;
            if bytes.len() > config.max_png_kb.saturating_mul(1024) {
                bail!("第 {} 个图表 PNG 超过大小上限", index + 1);
            }
            validate_output(&bytes, OutputFormat::Png)?;
            Ok(bytes)
        }
    }
}

fn validate_binding(
    asset: &ProvidedDiagram,
    expected_kind: &str,
    expected_source: &str,
    index: usize,
) -> anyhow::Result<()> {
    let provided_kind = normalize_kind(&asset.kind)
        .ok_or_else(|| anyhow!("第 {} 个图表类型不受支持", index + 1))?;
    if provided_kind != expected_kind || asset.source != expected_source {
        bail!("第 {} 个图表资产与 Markdown 源码不匹配", index + 1);
    }
    let expected_hash = source_hash(expected_kind, expected_source);
    if asset.source_hash != expected_hash {
        bail!("第 {} 个图表源码校验失败", index + 1);
    }
    Ok(())
}

fn source_hash(kind: &str, source: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    hash.update(b"\0");
    hash.update(source.as_bytes());
    format!("{:x}", hash.finalize())
}

pub fn marker(index: usize) -> String {
    format!("@@{MARKER_PREFIX}{index}@@")
}

pub fn marker_index(line: &str) -> Option<usize> {
    line.trim()
        .strip_prefix(&format!("@@{MARKER_PREFIX}"))?
        .strip_suffix("@@")?
        .parse()
        .ok()
}

pub fn normalize_kind(info: &str) -> Option<&'static str> {
    let raw = info
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_start_matches("language-")
        .to_ascii_lowercase();
    match raw.as_str() {
        "mermaid" => Some("mermaid"),
        "graphviz" | "dot" => Some("graphviz"),
        "vega" => Some("vega"),
        "vegalite" | "vega-lite" => Some("vegalite"),
        _ => None,
    }
}

fn opening_fence(line: &str) -> Option<(char, usize, &str)> {
    let t = line.trim_start();
    let marker = t.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let count = t.chars().take_while(|c| *c == marker).count();
    if count < 3 {
        return None;
    }
    Some((marker, count, t[count..].trim()))
}

fn find_closing_fence(lines: &[&str], start: usize, marker: char, min_len: usize) -> usize {
    let mut end = start;
    while end < lines.len() && !closing_fence(lines[end], marker, min_len) {
        end += 1;
    }
    end
}

fn closing_fence(line: &str, marker: char, min_len: usize) -> bool {
    let t = line.trim();
    let count = t.chars().take_while(|c| *c == marker).count();
    count >= min_len && t[count..].trim().is_empty()
}

fn validate_output(bytes: &[u8], format: OutputFormat) -> anyhow::Result<()> {
    if bytes.is_empty() {
        bail!("图表资产为空");
    }
    match format {
        OutputFormat::Png => {
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                bail!("图表资产不是有效 PNG");
            }
        }
        OutputFormat::Svg => {
            let text = std::str::from_utf8(bytes).context("图表 SVG 不是 UTF-8")?;
            let lower = text.to_ascii_lowercase();
            if !lower.contains("<svg") {
                bail!("图表资产不是有效 SVG");
            }
            for unsafe_fragment in [
                "<script",
                "<foreignobject",
                "<iframe",
                "<object",
                "<embed",
                "<image",
                "javascript:",
                " onload=",
                " onerror=",
                " onclick=",
                " onmouseover=",
                "@import",
                "url(http",
                "url(\"http",
                "url('http",
                "href=\"http",
                "href='http",
            ] {
                if lower.contains(unsafe_fragment) {
                    bail!("图表 SVG 包含不安全内容：{unsafe_fragment}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_asset() -> ProvidedDiagram {
        let source = "flowchart LR\n A-->B".to_string();
        ProvidedDiagram {
            kind: "mermaid".into(),
            source_hash: source_hash("mermaid", &source),
            source,
            svg: "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".into(),
            png_base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ".into(),
        }
    }

    #[test]
    fn aliases_are_normalized() {
        assert_eq!(normalize_kind("dot"), Some("graphviz"));
        assert_eq!(normalize_kind("language-vega-lite"), Some("vegalite"));
        assert_eq!(normalize_kind("plantuml"), None);
    }

    #[test]
    fn markers_round_trip() {
        assert_eq!(marker_index(&marker(12)), Some(12));
        assert_eq!(marker_index("not a marker"), None);
    }

    #[test]
    fn prepared_asset_must_match_source() {
        let md = "# Test\n\n```mermaid\nflowchart LR\n A-->B\n```";
        let prepared = prepare(
            md,
            &[example_asset()],
            OutputFormat::Svg,
            &Diagrams::default(),
        )
        .unwrap();
        assert!(prepared.markdown.contains("DOCUMENTX_DIAGRAM_0"));
        assert_eq!(prepared.diagrams.len(), 1);
    }

    #[test]
    fn prepared_marker_uses_same_asset_without_reparsing_fence() {
        let md = format!("# Test\n\n{}", marker(0));
        let prepared = prepare(
            &md,
            &[example_asset()],
            OutputFormat::Svg,
            &Diagrams::default(),
        )
        .unwrap();
        assert_eq!(prepared.markdown, md);
        assert_eq!(prepared.diagrams.len(), 1);
    }

    #[test]
    fn tilde_fence_is_supported() {
        let md = "# Test\n\n~~~mermaid\nflowchart LR\n A-->B\n~~~";
        let prepared = prepare(
            md,
            &[example_asset()],
            OutputFormat::Svg,
            &Diagrams::default(),
        )
        .unwrap();
        assert!(prepared.markdown.contains("DOCUMENTX_DIAGRAM_0"));
    }

    #[test]
    fn diagram_text_inside_regular_code_block_is_not_exported() {
        let md = "````markdown\n```mermaid\nflowchart LR\n A-->B\n```\n````";
        let prepared = prepare(md, &[], OutputFormat::Svg, &Diagrams::default()).unwrap();
        assert_eq!(prepared.markdown, md);
        assert!(prepared.diagrams.is_empty());
    }

    #[test]
    fn unsafe_svg_is_rejected() {
        assert!(
            validate_output(b"<svg><script>alert(1)</script></svg>", OutputFormat::Svg).is_err()
        );
        assert!(validate_output(
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
            OutputFormat::Svg
        )
        .is_ok());
    }
}
