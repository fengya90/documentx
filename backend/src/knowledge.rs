use std::path::Path;

use walkdir::WalkDir;

/// 检索到的知识片段。
#[derive(Debug, Clone)]
pub struct Chunk {
    pub source: String,
    pub text: String,
}

/// 检索器抽象。当前实现为关键词匹配；后续可替换为向量检索（如 Qdrant），
/// 上层无需改动。
pub trait Retriever: Send + Sync {
    fn retrieve(&self, query: &str, k: usize) -> Vec<Chunk>;
    fn sources(&self) -> Vec<String>;
}

/// 关键词检索：支持英文单词与中文 bigram 匹配。
pub struct KeywordRetriever {
    chunks: Vec<Chunk>,
    lowered: Vec<String>,
}

impl KeywordRetriever {
    pub fn load(dir: &str) -> anyhow::Result<Self> {
        let mut chunks = Vec::new();
        if Path::new(dir).exists() {
            for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                let is_doc = matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("md") | Some("markdown") | Some("txt")
                );
                if !is_doc {
                    continue;
                }
                let source = path
                    .strip_prefix(dir)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                let content = std::fs::read_to_string(path).unwrap_or_default();
                for text in chunk_text(&content) {
                    chunks.push(Chunk {
                        source: source.clone(),
                        text,
                    });
                }
            }
        }
        tracing::info!("知识库加载完成：{} 个片段（目录 {}）", chunks.len(), dir);
        let lowered = chunks.iter().map(|c| c.text.to_lowercase()).collect();
        Ok(KeywordRetriever { chunks, lowered })
    }
}

impl Retriever for KeywordRetriever {
    fn retrieve(&self, query: &str, k: usize) -> Vec<Chunk> {
        let terms = tokenize(query);
        if terms.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, usize)> = self
            .lowered
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let score = terms.iter().map(|t| text.matches(t.as_str()).count()).sum();
                (i, score)
            })
            .filter(|(_, s)| *s > 0)
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored
            .into_iter()
            .take(k)
            .map(|(i, _)| self.chunks[i].clone())
            .collect()
    }

    fn sources(&self) -> Vec<String> {
        let mut v: Vec<String> = self.chunks.iter().map(|c| c.source.clone()).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// 按段落聚合成 ~800 字符的片段。
fn chunk_text(content: &str) -> Vec<String> {
    const MAX: usize = 800;
    let mut chunks = Vec::new();
    let mut cur = String::new();
    for para in content.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if cur.chars().count() + para.chars().count() > MAX && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
    }
    if !cur.trim().is_empty() {
        chunks.push(cur);
    }
    chunks
}

/// 把查询拆成检索项：ASCII 单词（长度≥2）+ 中文 bigram。
fn tokenize(s: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let chars: Vec<char> = s.chars().collect();

    let mut latin = String::new();
    for &c in &chars {
        if c.is_ascii_alphanumeric() {
            latin.push(c.to_ascii_lowercase());
        } else {
            if latin.len() >= 2 {
                terms.push(std::mem::take(&mut latin));
            } else {
                latin.clear();
            }
        }
    }
    if latin.len() >= 2 {
        terms.push(latin);
    }

    let cjk: Vec<char> = chars.into_iter().filter(|c| is_cjk(*c)).collect();
    for w in cjk.windows(2) {
        terms.push(w.iter().collect());
    }
    if cjk.len() == 1 {
        terms.push(cjk[0].to_string());
    }

    terms.sort();
    terms.dedup();
    terms
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF)
}
