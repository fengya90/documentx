use std::collections::{btree_map::Entry, BTreeMap};

use anyhow::bail;

/// 一个模板 = 一份指导性参考文档，整篇会被塞进 prompt 供模型参考。
#[derive(Debug, Clone)]
pub struct Template {
    /// 相对于 templates/ 的路径（不含扩展名）；根目录旧模板名保持不变。
    pub name: String,
    pub content: String,
}

pub fn from_documents(documents: &BTreeMap<String, String>) -> anyhow::Result<Vec<Template>> {
    let mut templates = BTreeMap::new();
    for (source, content) in documents {
        let name = strip_extension(source);
        match templates.entry(name.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(Template {
                    name,
                    content: content.clone(),
                });
            }
            Entry::Occupied(_) => {
                bail!("模板名称冲突：{name}（不同扩展名会映射为同一个模板名）");
            }
        }
    }
    Ok(templates.into_values().collect())
}

fn strip_extension(source: &str) -> String {
    source
        .rsplit_once('.')
        .map(|(name, _)| name)
        .unwrap_or(source)
        .to_owned()
}

pub fn find<'a>(templates: &'a [Template], name: &str) -> Option<&'a Template> {
    templates.iter().find(|template| template.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_template_names_preserve_their_relative_path() {
        let templates = from_documents(&BTreeMap::from([
            ("对外API文档.md".to_owned(), "root".to_owned()),
            ("产品/发布说明.markdown".to_owned(), "nested".to_owned()),
        ]))
        .unwrap();

        assert_eq!(templates[0].name, "产品/发布说明");
        assert_eq!(templates[1].name, "对外API文档");
    }

    #[test]
    fn duplicate_names_across_extensions_are_rejected() {
        let error = from_documents(&BTreeMap::from([
            ("API.md".to_owned(), "one".to_owned()),
            ("API.txt".to_owned(), "two".to_owned()),
        ]))
        .unwrap_err();
        assert!(error.to_string().contains("模板名称冲突"));
    }
}
