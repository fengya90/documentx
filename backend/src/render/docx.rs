//! Markdown → Word(.docx)，纯 Rust（docx-rs）。
//! 中文由 Word 打开时用系统字体渲染，无需打包字体。

use std::io::Cursor;

use anyhow::Context;
use docx_rs::*;

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

pub fn render_docx(md: &str, _title: &str) -> anyhow::Result<Vec<u8>> {
    let docx = build_docx(md);
    let mut buf = Vec::new();
    docx.pack(Cursor::new(&mut buf))
        .context("生成 docx 失败")?;
    Ok(buf)
}

fn mono() -> RunFonts {
    RunFonts::new().ascii("Consolas").hi_ansi("Consolas")
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
    Some((after[..pos].to_string(), start + d.len_utf8() + pos + d.len_utf8()))
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
            let mut r = Run::new().add_text(seg.text.clone()).size(size);
            if seg.code {
                r = r
                    .fonts(mono())
                    .size(SZ_CODE)
                    .shading(Shading::new().shd_type(ShdType::Clear).color("auto").fill(FILL_CODE));
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

fn build_docx(md: &str) -> Docx {
    let mut docx = Docx::new();
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let t = raw.trim_start();

        // 代码块
        if let Some(_rest) = t.strip_prefix("```") {
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                let line = lines[i];
                let text = if line.is_empty() { " ".to_string() } else { line.to_string() };
                let p = Paragraph::new().add_run(
                    Run::new()
                        .add_text(text)
                        .fonts(mono())
                        .size(SZ_CODE)
                        .shading(Shading::new().shd_type(ShdType::Clear).color("auto").fill(FILL_BLOCK)),
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
                let mut r = Run::new().add_text(seg.text).size(size).bold().color(COLOR_HEADING);
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
                let q = lt.strip_prefix("> ").or_else(|| lt.strip_prefix(">")).unwrap_or("");
                if !first {
                    p = p.add_run(Run::new().add_break(BreakType::TextWrapping));
                }
                for seg in parse_inline(q.trim()) {
                    let mut r = Run::new().add_text(seg.text).size(SZ_BODY).italic().color(COLOR_MUTED);
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
                .add_run(Run::new().add_text("•  ").size(SZ_BODY).color(COLOR_TEXT));
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
                .add_run(Run::new().add_text(format!("{num}.  ")).size(SZ_BODY).color(COLOR_TEXT));
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
    docx
}

fn build_table(header: &[String], body: &[Vec<String>]) -> Table {
    let ncol = header.len().max(1);
    let mut rows = Vec::new();

    let mut hcells = Vec::new();
    for c in header {
        let p = Paragraph::new().add_run(Run::new().add_text(c.clone()).size(SZ_BODY).bold());
        hcells.push(
            TableCell::new()
                .add_paragraph(p)
                .shading(Shading::new().shd_type(ShdType::Clear).color("auto").fill(FILL_HEADER)),
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
