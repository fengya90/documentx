//! Markdown → Word(.docx)，纯 Rust（docx-rs）。
//! 中文由 Word 打开时用系统字体渲染，无需打包字体。

use std::io::{Cursor, Read, Write};

use anyhow::Context;
use docx_rs::*;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use super::diagram::{self, DiagramAsset, OutputFormat, ProvidedDiagram};
use crate::config::Diagrams;

const COLOR_HEADING: &str = "1F2328";
const COLOR_TEXT: &str = "24292F";
const COLOR_MUTED: &str = "57606A";
const FILL_CODE: &str = "EFF1F3";
const FILL_HEADER: &str = "F6F8FA";
const FILL_BLOCK: &str = "F6F8FA";

/// 半磅（half-point）字号
const SZ_BODY: usize = 21; // 10.5pt
const SZ_H1: usize = 36; // 18pt
const SZ_H2: usize = 30; // 15pt
const SZ_H3: usize = 26; // 13pt
const SZ_H4: usize = 23; // 11.5pt
const SZ_CODE: usize = 19; // 9.5pt

pub fn render_docx(
    md: &str,
    _title: &str,
    config: &Diagrams,
    assets: &[ProvidedDiagram],
) -> anyhow::Result<Vec<u8>> {
    let prepared = diagram::prepare(md, assets, OutputFormat::Png, config)?;
    let docx = build_docx(&prepared.markdown, &prepared.diagrams)?;
    let mut buf = Vec::new();
    docx.pack(Cursor::new(&mut buf)).context("生成 docx 失败")?;
    add_east_asia_font(buf)
}

/// docx-rs 的默认 fontTable 只声明西文字体。这里把已随服务发布的
/// Noto Sans SC 按 OOXML 规则混淆并嵌入，确保 Word 导出不依赖系统中文字体。
fn add_east_asia_font(buf: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    const FONT_TABLE: &str = "word/fontTable.xml";
    const SETTINGS: &str = "word/settings.xml";
    const CONTENT_TYPES: &str = "[Content_Types].xml";
    const FONT_RELS: &str = "word/_rels/fontTable.xml.rels";
    const FONT_PATH: &str = "word/fonts/documentx-noto-sans-sc.odttf";
    const FONT_XOR_KEY: [u8; 16] = [
        0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99, 0x88, 0x66, 0x77, 0x44, 0x55, 0x00, 0x11, 0x22,
        0x33,
    ];
    const FONT_ENTRY: &str = concat!(
        r#"<w:font w:name="Noto Sans SC">"#,
        r#"<w:charset w:val="86" />"#,
        r#"<w:family w:val="swiss" />"#,
        r#"<w:pitch w:val="variable" />"#,
        r#"<w:embedRegular r:id="rIdDocumentXFont1" w:fontKey="{00112233-4455-6677-8899-AABBCCDDEEFF}" />"#,
        "</w:font>"
    );
    const FONT_RELATIONSHIPS: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        r#"<Relationship Id="rIdDocumentXFont1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/font" Target="fonts/documentx-noto-sans-sc.odttf" />"#,
        "</Relationships>"
    );
    const CONTENT_TYPE_ENTRY: &str = r#"<Default Extension="odttf" ContentType="application/vnd.openxmlformats-officedocument.obfuscatedFont" />"#;

    let mut input = ZipArchive::new(Cursor::new(buf)).context("读取 docx 包失败")?;
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    let mut rewritten = 0;

    for index in 0..input.len() {
        let mut file = input.by_index(index).context("读取 docx 条目失败")?;
        let name = file.name().to_string();
        if !matches!(name.as_str(), FONT_TABLE | SETTINGS | CONTENT_TYPES) {
            output.raw_copy_file(file).context("复制 docx 条目失败")?;
            continue;
        }

        let options = file.options();
        let mut xml = String::new();
        file.read_to_string(&mut xml)
            .with_context(|| format!("读取 docx XML 失败：{name}"))?;
        let updated = match name.as_str() {
            FONT_TABLE => xml.replacen("</w:fonts>", &format!("{FONT_ENTRY}</w:fonts>"), 1),
            SETTINGS => xml.replacen("</w:settings>", "<w:embedTrueTypeFonts /></w:settings>", 1),
            CONTENT_TYPES => xml.replacen("</Types>", &format!("{CONTENT_TYPE_ENTRY}</Types>"), 1),
            _ => unreachable!(),
        };
        if updated == xml {
            anyhow::bail!("docx XML 结构无效：{name}");
        }
        output
            .start_file(&name, options)
            .with_context(|| format!("写入 docx XML 失败：{name}"))?;
        output
            .write_all(updated.as_bytes())
            .with_context(|| format!("写入 docx XML 失败：{name}"))?;
        rewritten += 1;
    }

    if rewritten != 3 {
        anyhow::bail!("docx 缺少字体嵌入所需的 XML 部件");
    }

    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    output
        .start_file(FONT_RELS, options)
        .context("写入 docx 字体关系失败")?;
    output
        .write_all(FONT_RELATIONSHIPS.as_bytes())
        .context("写入 docx 字体关系失败")?;

    let mut font = include_bytes!("../../assets/fonts/NotoSansSC-Regular.otf").to_vec();
    for (index, byte) in font.iter_mut().take(32).enumerate() {
        *byte ^= FONT_XOR_KEY[index % FONT_XOR_KEY.len()];
    }
    output
        .start_file(FONT_PATH, options)
        .context("写入 docx 内嵌字体失败")?;
    output.write_all(&font).context("写入 docx 内嵌字体失败")?;

    Ok(output.finish().context("封装 docx 失败")?.into_inner())
}

fn mono() -> RunFonts {
    RunFonts::new()
        .ascii("Noto Sans SC")
        .hi_ansi("Noto Sans SC")
        .east_asia("Noto Sans SC")
        .hint("eastAsia")
}

/// docx 的东亚字体槽不能留空，否则部分 LibreOffice / Word 组合会只显示西文。
fn body_fonts() -> RunFonts {
    RunFonts::new()
        .ascii("Noto Sans SC")
        .hi_ansi("Noto Sans SC")
        .east_asia("Noto Sans SC")
        .hint("eastAsia")
}

// ---------- 行内 ----------

#[derive(Default, Clone)]
struct Seg {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
}

/// 解析行内 **粗体** / *斜体* / `代码` / [文本](链接)（链接仅取文本）。
fn parse_inline(s: &str) -> Vec<Seg> {
    let mut segs: Vec<Seg> = Vec::new();
    let mut plain = String::new();
    let flush = |plain: &mut String, segs: &mut Vec<Seg>| {
        if !plain.is_empty() {
            segs.push(Seg {
                text: std::mem::take(plain),
                ..Default::default()
            });
        }
    };

    let mut i = 0;
    let len = s.len();
    while i < len {
        let rest = &s[i..];
        if rest.starts_with('`') {
            if let Some((c, next)) = delim(s, i, '`') {
                flush(&mut plain, &mut segs);
                segs.push(Seg {
                    text: c,
                    code: true,
                    ..Default::default()
                });
                i = next;
                continue;
            }
        }
        if rest.starts_with("**") {
            if let Some((inner, next)) = dbl(s, i, "**") {
                flush(&mut plain, &mut segs);
                segs.push(Seg {
                    text: inner,
                    bold: true,
                    ..Default::default()
                });
                i = next;
                continue;
            }
        }
        if rest.starts_with('*') {
            if let Some((inner, next)) = delim(s, i, '*') {
                flush(&mut plain, &mut segs);
                segs.push(Seg {
                    text: inner,
                    italic: true,
                    ..Default::default()
                });
                i = next;
                continue;
            }
        }
        if rest.starts_with('[') {
            if let Some((text, next)) = link_text(s, i) {
                flush(&mut plain, &mut segs);
                segs.push(Seg {
                    text,
                    ..Default::default()
                });
                i = next;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut plain, &mut segs);
    segs
}

fn delim(s: &str, start: usize, d: char) -> Option<(String, usize)> {
    let after = &s[start + d.len_utf8()..];
    let pos = after.find(d)?;
    if pos == 0 {
        return None;
    }
    Some((
        after[..pos].to_string(),
        start + d.len_utf8() + pos + d.len_utf8(),
    ))
}

fn dbl(s: &str, start: usize, d: &str) -> Option<(String, usize)> {
    let after = &s[start + d.len()..];
    let pos = after.find(d)?;
    if pos == 0 {
        return None;
    }
    Some((after[..pos].to_string(), start + d.len() + pos + d.len()))
}

fn link_text(s: &str, start: usize) -> Option<(String, usize)> {
    let after = &s[start + 1..];
    let close = after.find(']')?;
    let text = after[..close].to_string();
    let tail = &after[close + 1..];
    if !tail.starts_with('(') {
        return None;
    }
    let end = tail.find(')')?;
    Some((text, start + 1 + close + 1 + end + 1))
}

fn runs_from_segs(segs: &[Seg], size: usize) -> Vec<Run> {
    segs.iter()
        .map(|seg| {
            let mut r = Run::new()
                .add_text(seg.text.clone())
                .fonts(body_fonts())
                .size(size);
            if seg.code {
                r = r.fonts(mono()).size(SZ_CODE).shading(
                    Shading::new()
                        .shd_type(ShdType::Clear)
                        .color("auto")
                        .fill(FILL_CODE),
                );
            }
            if seg.bold {
                r = r.bold();
            }
            if seg.italic {
                r = r.italic();
            }
            r
        })
        .collect()
}

// ---------- 块级 ----------

fn build_docx(md: &str, diagrams: &[DiagramAsset]) -> anyhow::Result<Docx> {
    let mut docx = Docx::new();
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let t = raw.trim_start();

        // 图表使用行内图片，跨 Word / LibreOffice 的布局最稳定。
        if let Some(index) = diagram::marker_index(t) {
            let asset = diagrams
                .get(index)
                .ok_or_else(|| anyhow::anyhow!("图表资源索引无效：{index}"))?;
            let (width_px, height_px) = png_dimensions(&asset.bytes)?;
            let (width_emu, height_emu) = fit_image(width_px, height_px);
            let pic = Pic::new_with_dimensions(asset.bytes.clone(), width_px, height_px)
                .size(width_emu, height_emu);
            docx = docx.add_paragraph(
                Paragraph::new()
                    .align(AlignmentType::Center)
                    .add_run(Run::new().add_image(pic)),
            );
            docx = docx.add_paragraph(Paragraph::new());
            i += 1;
            continue;
        }

        // 代码块
        if let Some(_rest) = t.strip_prefix("```") {
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                let line = lines[i];
                let text = if line.is_empty() {
                    " ".to_string()
                } else {
                    line.to_string()
                };
                let p = Paragraph::new().add_run(
                    Run::new()
                        .add_text(text)
                        .fonts(mono())
                        .size(SZ_CODE)
                        .shading(
                            Shading::new()
                                .shd_type(ShdType::Clear)
                                .color("auto")
                                .fill(FILL_BLOCK),
                        ),
                );
                docx = docx.add_paragraph(p);
                i += 1;
            }
            i += 1; // 跳过结束围栏
            docx = docx.add_paragraph(Paragraph::new());
            continue;
        }

        if t.is_empty() {
            i += 1;
            continue;
        }

        // 水平线
        if t == "---" || t == "***" || t == "___" {
            docx = docx.add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("―".repeat(30)).color("D0D7DE")),
            );
            i += 1;
            continue;
        }

        // 标题
        if let Some((level, text)) = heading(t) {
            let size = match level {
                1 => SZ_H1,
                2 => SZ_H2,
                3 => SZ_H3,
                _ => SZ_H4,
            };
            let mut p = Paragraph::new();
            for seg in parse_inline(text) {
                let mut r = Run::new()
                    .add_text(seg.text)
                    .fonts(body_fonts())
                    .size(size)
                    .bold()
                    .color(COLOR_HEADING);
                if seg.code {
                    r = r.fonts(mono());
                }
                p = p.add_run(r);
            }
            docx = docx.add_paragraph(p);
            i += 1;
            continue;
        }

        // 表格
        if t.starts_with('|') && i + 1 < lines.len() && is_sep(lines[i + 1]) {
            let header = cells(t);
            i += 2;
            let mut body = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                body.push(cells(lines[i]));
                i += 1;
            }
            docx = docx.add_table(build_table(&header, &body));
            docx = docx.add_paragraph(Paragraph::new());
            continue;
        }

        // 引用（合并连续 > 行）
        if t.starts_with('>') {
            let mut p = Paragraph::new().indent(Some(360), None, None, None);
            let mut first = true;
            while i < lines.len() {
                let lt = lines[i].trim_start();
                if !lt.starts_with('>') {
                    break;
                }
                let q = lt
                    .strip_prefix("> ")
                    .or_else(|| lt.strip_prefix(">"))
                    .unwrap_or("");
                if !first {
                    p = p.add_run(Run::new().add_break(BreakType::TextWrapping));
                }
                for seg in parse_inline(q.trim()) {
                    let mut r = Run::new()
                        .add_text(seg.text)
                        .fonts(body_fonts())
                        .size(SZ_BODY)
                        .italic()
                        .color(COLOR_MUTED);
                    if seg.code {
                        r = r.fonts(mono());
                    }
                    p = p.add_run(r);
                }
                first = false;
                i += 1;
            }
            docx = docx.add_paragraph(p);
            continue;
        }

        // 无序列表
        if let Some(item) = strip_ul(t) {
            let mut p = Paragraph::new()
                .indent(Some(360), None, None, None)
                .add_run(
                    Run::new()
                        .add_text("•  ")
                        .fonts(body_fonts())
                        .size(SZ_BODY)
                        .color(COLOR_TEXT),
                );
            for r in runs_from_segs(&parse_inline(item), SZ_BODY) {
                p = p.add_run(r.color(COLOR_TEXT));
            }
            docx = docx.add_paragraph(p);
            i += 1;
            continue;
        }

        // 有序列表
        if let Some((num, item)) = strip_ol(t) {
            let mut p = Paragraph::new()
                .indent(Some(360), None, None, None)
                .add_run(
                    Run::new()
                        .add_text(format!("{num}.  "))
                        .fonts(body_fonts())
                        .size(SZ_BODY)
                        .color(COLOR_TEXT),
                );
            for r in runs_from_segs(&parse_inline(item), SZ_BODY) {
                p = p.add_run(r.color(COLOR_TEXT));
            }
            docx = docx.add_paragraph(p);
            i += 1;
            continue;
        }

        // 普通段落
        let mut p = Paragraph::new();
        for r in runs_from_segs(&parse_inline(t), SZ_BODY) {
            p = p.add_run(r.color(COLOR_TEXT));
        }
        docx = docx.add_paragraph(p);
        i += 1;
    }
    Ok(docx)
}

fn png_dimensions(bytes: &[u8]) -> anyhow::Result<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        anyhow::bail!("图表不是有效 PNG");
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 {
        anyhow::bail!("图表 PNG 尺寸无效");
    }
    Ok((width, height))
}

/// 限制到约 16cm × 22cm 的正文区域，保持宽高比，不放大低分辨率图片。
fn fit_image(width_px: u32, height_px: u32) -> (u32, u32) {
    const EMU_PER_PX: f64 = 9_525.0;
    const MAX_WIDTH: f64 = 5_760_000.0;
    const MAX_HEIGHT: f64 = 7_920_000.0;
    let native_w = width_px as f64 * EMU_PER_PX;
    let native_h = height_px as f64 * EMU_PER_PX;
    let scale = 1.0_f64.min(MAX_WIDTH / native_w).min(MAX_HEIGHT / native_h);
    ((native_w * scale) as u32, (native_h * scale) as u32)
}

fn build_table(header: &[String], body: &[Vec<String>]) -> Table {
    let ncol = header.len().max(1);
    let mut rows = Vec::new();

    let mut hcells = Vec::new();
    for c in header {
        let p = Paragraph::new().add_run(
            Run::new()
                .add_text(c.clone())
                .fonts(body_fonts())
                .size(SZ_BODY)
                .bold(),
        );
        hcells.push(
            TableCell::new().add_paragraph(p).shading(
                Shading::new()
                    .shd_type(ShdType::Clear)
                    .color("auto")
                    .fill(FILL_HEADER),
            ),
        );
    }
    rows.push(TableRow::new(hcells));

    for row in body {
        let mut tcells = Vec::new();
        for j in 0..ncol {
            let text = row.get(j).cloned().unwrap_or_default();
            let mut p = Paragraph::new();
            for r in runs_from_segs(&parse_inline(&text), SZ_BODY) {
                p = p.add_run(r);
            }
            tcells.push(TableCell::new().add_paragraph(p));
        }
        rows.push(TableRow::new(tcells));
    }

    Table::new(rows)
        .width(5000, WidthType::Pct)
        .set_borders(TableBorders::new())
}

fn heading(t: &str) -> Option<(usize, &str)> {
    let b = t.as_bytes();
    let mut n = 0;
    while n < b.len() && b[n] == b'#' {
        n += 1;
    }
    if (1..=6).contains(&n) && n < b.len() && b[n] == b' ' {
        Some((n, &t[n + 1..]))
    } else {
        None
    }
}

fn strip_ul(t: &str) -> Option<&str> {
    for p in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(p) {
            return Some(r);
        }
    }
    None
}

fn strip_ol(t: &str) -> Option<(usize, &str)> {
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i + 1 < b.len() && (b[i] == b'.' || b[i] == b')') && b[i + 1] == b' ' {
        let num: usize = t[..i].parse().unwrap_or(1);
        Some((num, &t[i + 2..]))
    } else {
        None
    }
}

fn is_sep(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn cells(row: &str) -> Vec<String> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_self_contained_chinese_font() {
        let mut source = Vec::new();
        Docx::new().pack(Cursor::new(&mut source)).unwrap();
        let bytes = add_east_asia_font(source).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

        let mut table = String::new();
        archive
            .by_name("word/fontTable.xml")
            .unwrap()
            .read_to_string(&mut table)
            .unwrap();
        assert!(table.contains(r#"w:name="Noto Sans SC""#));
        assert!(table.contains("rIdDocumentXFont1"));

        let mut embedded = Vec::new();
        archive
            .by_name("word/fonts/documentx-noto-sans-sc.odttf")
            .unwrap()
            .read_to_end(&mut embedded)
            .unwrap();
        let key = [
            0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA, 0x99, 0x88, 0x66, 0x77, 0x44, 0x55, 0x00, 0x11,
            0x22, 0x33,
        ];
        for (index, byte) in embedded.iter_mut().take(32).enumerate() {
            *byte ^= key[index % key.len()];
        }
        assert_eq!(
            embedded,
            include_bytes!("../../assets/fonts/NotoSansSC-Regular.otf")
        );
    }
}
