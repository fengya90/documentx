use std::collections::BTreeMap;

use serde::Serialize;

/// 检索到的知识片段。
#[derive(Debug, Clone)]
pub struct Chunk {
    pub source: String,
    pub text: String,
}

/// 知识库目录树节点。文件和目录都使用相对于 `knowledge_dir` 的 `/` 分隔路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KnowledgeNode {
    Directory {
        name: String,
        path: String,
        count: usize,
        children: Vec<KnowledgeNode>,
    },
    File {
        name: String,
        path: String,
    },
}

/// 检索器抽象。当前实现为关键词匹配；后续可替换为向量检索（如 Qdrant），
/// 上层无需改动。
pub trait Retriever: Send + Sync {
    fn retrieve(&self, query: &str, k: usize) -> Vec<Chunk>;
    fn sources(&self) -> Vec<String>;
    fn tree(&self) -> Vec<KnowledgeNode>;
}

/// 关键词检索：支持英文单词与中文 bigram 匹配。
pub struct KeywordRetriever {
    chunks: Vec<Chunk>,
    lowered: Vec<String>,
    sources: Vec<String>,
    tree: Vec<KnowledgeNode>,
}

impl KeywordRetriever {
    pub fn from_documents(documents: &BTreeMap<String, String>) -> Self {
        let mut chunks = Vec::new();
        for (source, content) in documents {
            for text in chunk_text(content) {
                chunks.push(Chunk {
                    source: source.clone(),
                    text,
                });
            }
        }
        let sources: Vec<String> = documents.keys().cloned().collect();
        let tree = build_tree(&sources);
        let lowered = chunks
            .iter()
            .map(|chunk| chunk.text.to_lowercase())
            .collect();
        KeywordRetriever {
            chunks,
            lowered,
            sources,
            tree,
        }
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
        scored.sort_by_key(|item| std::cmp::Reverse(item.1));
        scored
            .into_iter()
            .take(k)
            .map(|(i, _)| self.chunks[i].clone())
            .collect()
    }

    fn sources(&self) -> Vec<String> {
        self.sources.clone()
    }

    fn tree(&self) -> Vec<KnowledgeNode> {
        self.tree.clone()
    }
}

#[derive(Default)]
struct DirectoryBuilder {
    directories: BTreeMap<String, DirectoryBuilder>,
    files: BTreeMap<String, String>,
}

impl DirectoryBuilder {
    fn insert(&mut self, source: &str) {
        let parts: Vec<&str> = source.split('/').filter(|part| !part.is_empty()).collect();
        let Some((file_name, directories)) = parts.split_last() else {
            return;
        };
        let mut current = self;
        for directory in directories {
            current = current
                .directories
                .entry((*directory).to_owned())
                .or_default();
        }
        current
            .files
            .insert((*file_name).to_owned(), source.to_owned());
    }

    fn file_count(&self) -> usize {
        self.files.len()
            + self
                .directories
                .values()
                .map(DirectoryBuilder::file_count)
                .sum::<usize>()
    }

    fn into_nodes(self, parent: &str) -> Vec<KnowledgeNode> {
        let mut nodes = Vec::with_capacity(self.directories.len() + self.files.len());
        for (name, directory) in self.directories {
            let path = join_source(parent, &name);
            let count = directory.file_count();
            let children = directory.into_nodes(&path);
            nodes.push(KnowledgeNode::Directory {
                name,
                path,
                count,
                children,
            });
        }
        for (name, path) in self.files {
            nodes.push(KnowledgeNode::File { name, path });
        }
        nodes
    }
}

fn join_source(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

fn build_tree(sources: &[String]) -> Vec<KnowledgeNode> {
    let mut root = DirectoryBuilder::default();
    for source in sources {
        root.insert(source);
    }
    root.into_nodes("")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_directories_before_files_with_recursive_counts() {
        let sources = vec![
            "根文档.md".to_owned(),
            "产品/API/鉴权.md".to_owned(),
            "产品/概览.md".to_owned(),
        ];

        assert_eq!(
            build_tree(&sources),
            vec![
                KnowledgeNode::Directory {
                    name: "产品".to_owned(),
                    path: "产品".to_owned(),
                    count: 2,
                    children: vec![
                        KnowledgeNode::Directory {
                            name: "API".to_owned(),
                            path: "产品/API".to_owned(),
                            count: 1,
                            children: vec![KnowledgeNode::File {
                                name: "鉴权.md".to_owned(),
                                path: "产品/API/鉴权.md".to_owned(),
                            }],
                        },
                        KnowledgeNode::File {
                            name: "概览.md".to_owned(),
                            path: "产品/概览.md".to_owned(),
                        },
                    ],
                },
                KnowledgeNode::File {
                    name: "根文档.md".to_owned(),
                    path: "根文档.md".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn snapshot_keeps_empty_nested_documents_and_full_source_paths() {
        let documents = BTreeMap::from([
            (
                "团队/接口/认证.md".to_owned(),
                "认证流程使用签名校验。".to_owned(),
            ),
            ("空文档.txt".to_owned(), String::new()),
        ]);
        let retriever = KeywordRetriever::from_documents(&documents);

        assert_eq!(
            retriever.sources(),
            vec!["团队/接口/认证.md".to_owned(), "空文档.txt".to_owned()]
        );
        assert_eq!(
            retriever.retrieve("认证流程", 1)[0].source,
            "团队/接口/认证.md"
        );
    }
}
