use std::path::Path;

use walkdir::WalkDir;

/// 一个模板 = 一份指导性参考文档，整篇会被塞进 prompt 供模型参考。
#[derive(Debug, Clone)]
pub struct Template {
    pub name: String,
    pub content: String,
}

pub fn load_templates(dir: &str) -> anyhow::Result<Vec<Template>> {
    let mut v = Vec::new();
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
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let content = std::fs::read_to_string(path).unwrap_or_default();
            v.push(Template { name, content });
        }
    }
    v.sort_by(|a, b| a.name.cmp(&b.name));
    tracing::info!("模板加载完成：{} 个（目录 {}）", v.len(), dir);
    Ok(v)
}

pub fn find<'a>(templates: &'a [Template], name: &str) -> Option<&'a Template> {
    templates.iter().find(|t| t.name == name)
}
