pub mod diagram;
mod docx;
mod md2typst;
mod pdf;

use crate::config::Diagrams;

#[derive(Debug, Clone, Copy)]
pub enum Format {
    Markdown,
    Pdf,
    Docx,
}

impl Format {
    pub fn parse(s: &str) -> Format {
        match s.to_ascii_lowercase().as_str() {
            "pdf" => Format::Pdf,
            "docx" | "word" => Format::Docx,
            _ => Format::Markdown,
        }
    }
}

/// 渲染结果：文件字节、Content-Type、文件名。
pub struct Rendered {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub filename: String,
}

/// 把 Markdown 内容渲染为指定格式的可下载文件。
pub fn render(
    content: &str,
    title: &str,
    format: Format,
    diagrams: &Diagrams,
    diagram_assets: &[diagram::ProvidedDiagram],
) -> anyhow::Result<Rendered> {
    let content = unwrap_outer_fence(content);
    let content = content.as_str();
    match format {
        Format::Markdown => Ok(Rendered {
            bytes: content.as_bytes().to_vec(),
            content_type: "text/markdown; charset=utf-8".into(),
            filename: "document.md".into(),
        }),
        Format::Pdf => {
            let bytes = pdf::render_pdf(content, title, diagrams, diagram_assets)?;
            Ok(Rendered {
                bytes,
                content_type: "application/pdf".into(),
                filename: "document.pdf".into(),
            })
        }
        Format::Docx => {
            let bytes = docx::render_docx(content, title, diagrams, diagram_assets)?;
            Ok(Rendered {
                bytes,
                content_type:
                    "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
                filename: "document.docx".into(),
            })
        }
    }
}

/// 若整篇内容被单个 ```` ```lang ... ``` ```` 代码围栏包裹（模型有时会把整份文档
/// 包进代码块），则拆掉最外层围栏，还原为真正的 Markdown。仅在恰好只有一对围栏、
/// 且分别位于首行和末行时才拆，避免误伤正文里正常的代码块。
fn unwrap_outer_fence(content: &str) -> String {
    let t = content.trim();
    if !t.starts_with("```") {
        return content.to_string();
    }
    let lines: Vec<&str> = t.lines().collect();
    let fence_lines: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim_start().starts_with("```"))
        .map(|(i, _)| i)
        .collect();
    if fence_lines.len() == 2 && fence_lines[0] == 0 && fence_lines[1] == lines.len() - 1 {
        return lines[1..lines.len() - 1].join("\n");
    }
    content.to_string()
}
