//! 把常见 Markdown 子集转换为 Typst 标记。
//! Typst 语法与 Markdown 相近（标题 =、列表 - / +、代码块 ```），
//! 因此这里做的是逐行/逐 token 的映射与转义。

use super::diagram;

/// 转换整篇 Markdown 为 Typst body。
pub fn convert(md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let t = raw.trim_start();

        if let Some(index) = diagram::marker_index(t) {
            out.push_str(&format!(
                "#figure(image(\"diagram-{index}.svg\", width: 100%, fit: \"contain\"))\n\n"
            ));
            i += 1;
            continue;
        }

        // 代码块（围栏）
        if let Some(rest) = t.strip_prefix("```") {
            let lang = rest.trim();
            let mut code = String::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            i += 1; // 跳过结束围栏
            out.push_str("```");
            out.push_str(lang);
            out.push('\n');
            out.push_str(&code);
            out.push_str("```\n\n");
            continue;
        }

        if t.is_empty() {
            out.push('\n');
            i += 1;
            continue;
        }

        // 水平线
        if t == "---" || t == "***" || t == "___" {
            out.push_str(
                "#block(above: 1.5em, below: 1.5em, line(length: 100%, stroke: 0.75pt + rgb(\"#d0d7de\")))\n\n",
            );
            i += 1;
            continue;
        }

        // 标题
        if let Some((level, text)) = parse_heading(t) {
            for _ in 0..level {
                out.push('=');
            }
            out.push(' ');
            out.push_str(&inline(text));
            out.push_str("\n\n");
            i += 1;
            continue;
        }

        // 表格
        if t.starts_with('|') && i + 1 < lines.len() && is_separator(lines[i + 1]) {
            let header = t;
            i += 2;
            let mut body = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                body.push(lines[i]);
                i += 1;
            }
            out.push_str(&table_to_typst(header, &body));
            out.push_str("\n\n");
            continue;
        }

        // 引用（合并连续的 > 行为一个引用块）
        if t.starts_with('>') {
            let mut parts = Vec::new();
            while i < lines.len() {
                let lt = lines[i].trim_start();
                if !lt.starts_with('>') {
                    break;
                }
                let q = lt
                    .strip_prefix("> ")
                    .or_else(|| lt.strip_prefix(">"))
                    .unwrap_or("");
                parts.push(inline(q.trim()));
                i += 1;
            }
            out.push_str("#quote(block: true)[");
            out.push_str(&parts.join(" \\\n"));
            out.push_str("]\n\n");
            continue;
        }

        // 无序列表
        if let Some(item) = strip_ul(t) {
            out.push_str("- ");
            out.push_str(&inline(item));
            out.push('\n');
            i += 1;
            continue;
        }

        // 有序列表
        if let Some(item) = strip_ol(t) {
            out.push_str("+ ");
            out.push_str(&inline(item));
            out.push('\n');
            i += 1;
            continue;
        }

        // 普通段落
        out.push_str(&inline(t));
        out.push_str("\n\n");
        i += 1;
    }
    out
}

fn parse_heading(t: &str) -> Option<(usize, &str)> {
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

fn strip_ol(t: &str) -> Option<&str> {
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i + 1 < b.len() && (b[i] == b'.' || b[i] == b')') && b[i + 1] == b' ' {
        Some(&t[i + 2..])
    } else {
        None
    }
}

fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn table_cells(row: &str) -> Vec<String> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

fn table_to_typst(header: &str, body: &[&str]) -> String {
    let h = table_cells(header);
    let n = h.len().max(1);
    let mut s = String::new();
    // 不设 stroke/inset：交给全局 #set table 统一样式；表头加粗由 show 规则负责。
    s.push_str(&format!("#table(\n  columns: {n},\n"));
    s.push_str("  table.header(");
    for c in &h {
        s.push_str(&format!("[{}], ", inline(c)));
    }
    s.push_str("),\n");
    for r in body {
        let cells = table_cells(r);
        s.push_str("  ");
        for j in 0..n {
            let c = cells.get(j).map(|x| x.as_str()).unwrap_or("");
            s.push_str(&format!("[{}], ", inline(c)));
        }
        s.push('\n');
    }
    s.push(')');
    s
}

/// 行内转换：处理 **粗体** / *斜体* / `代码` / [文本](链接)，其余字符做 Typst 转义。
fn inline(s: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    let len = s.len();
    while i < len {
        let rest = &s[i..];

        if rest.starts_with('[') {
            if let Some((text, url, next)) = parse_link(s, i) {
                out.push_str(&format!(
                    "#link(\"{}\")[{}]",
                    escape_str(&url),
                    inline(&text)
                ));
                i = next;
                continue;
            }
        }
        if rest.starts_with('`') {
            if let Some((code, next)) = parse_delim(s, i, '`') {
                out.push('`');
                out.push_str(&code);
                out.push('`');
                i = next;
                continue;
            }
        }
        if rest.starts_with("**") {
            if let Some((inner, next)) = parse_double(s, i, "**") {
                out.push('*');
                out.push_str(&inline(&inner));
                out.push('*');
                i = next;
                continue;
            }
        }
        if rest.starts_with("__") {
            if let Some((inner, next)) = parse_double(s, i, "__") {
                out.push('*');
                out.push_str(&inline(&inner));
                out.push('*');
                i = next;
                continue;
            }
        }
        if rest.starts_with('*') {
            if let Some((inner, next)) = parse_delim(s, i, '*') {
                out.push('_');
                out.push_str(&inline(&inner));
                out.push('_');
                i = next;
                continue;
            }
        }
        if rest.starts_with('_') {
            if let Some((inner, next)) = parse_delim(s, i, '_') {
                out.push('_');
                out.push_str(&inline(&inner));
                out.push('_');
                i = next;
                continue;
            }
        }

        let ch = rest.chars().next().unwrap();
        out.push_str(&escape_char(ch));
        i += ch.len_utf8();
    }
    out
}

fn parse_delim(s: &str, start: usize, d: char) -> Option<(String, usize)> {
    let after = &s[start + d.len_utf8()..];
    let pos = after.find(d)?;
    if pos == 0 {
        return None;
    }
    let content = after[..pos].to_string();
    Some((content, start + d.len_utf8() + pos + d.len_utf8()))
}

fn parse_double(s: &str, start: usize, d: &str) -> Option<(String, usize)> {
    let after = &s[start + d.len()..];
    let pos = after.find(d)?;
    if pos == 0 {
        return None;
    }
    Some((after[..pos].to_string(), start + d.len() + pos + d.len()))
}

fn parse_link(s: &str, start: usize) -> Option<(String, String, usize)> {
    let after = &s[start + 1..];
    let close = after.find(']')?;
    let text = after[..close].to_string();
    let tail = &after[close + 1..];
    if !tail.starts_with('(') {
        return None;
    }
    let end = tail.find(')')?;
    let url = tail[1..end].to_string();
    let next = start + 1 + close + 1 + end + 1;
    Some((text, url, next))
}

fn escape_char(c: char) -> String {
    match c {
        '#' | '*' | '_' | '`' | '$' | '\\' | '<' | '>' | '@' | '~' | '[' | ']' => {
            format!("\\{c}")
        }
        _ => c.to_string(),
    }
}

pub fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
