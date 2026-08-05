use std::{
    collections::BTreeMap,
    env, fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context};
use futures::{stream, StreamExt, TryStreamExt};
use object_store::{aws::AmazonS3Builder, path::Path as ObjectPath, ObjectStore, ObjectStoreExt};
use serde::Serialize;
use tokio::sync::Mutex;
use walkdir::WalkDir;

use crate::{
    config::{Config, ContentMode, S3},
    knowledge::KeywordRetriever,
    templates::{self, Template},
};

pub struct ContentSnapshot {
    pub generation: u64,
    pub loaded_at_unix: u64,
    pub instructions: String,
    pub knowledge: BTreeMap<String, String>,
    pub retriever: KeywordRetriever,
    pub templates: Vec<Template>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentStatus {
    pub mode: String,
    pub generation: u64,
    pub refresh_interval_secs: u64,
    pub loaded_at_unix: u64,
    pub last_attempt_unix: u64,
    pub last_refresh_duration_ms: u64,
    pub knowledge_count: usize,
    pub template_count: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeUploadResult {
    pub uploaded: Vec<String>,
    pub overwritten: Vec<String>,
    pub generation: u64,
    pub knowledge_count: usize,
}

#[derive(Clone, Copy)]
struct Limits {
    max_file_bytes: usize,
    max_files: usize,
}

#[derive(Clone)]
enum ContentSource {
    Local {
        knowledge_dir: PathBuf,
        templates_dir: PathBuf,
        agents_file: Option<PathBuf>,
    },
    S3 {
        store: Arc<dyn ObjectStore>,
        root_prefix: String,
        download_concurrency: usize,
    },
}

struct LoadedBundle {
    instructions: Option<String>,
    knowledge: BTreeMap<String, String>,
    templates: BTreeMap<String, String>,
}

pub struct ContentManager {
    mode: ContentMode,
    source: ContentSource,
    limits: Limits,
    fallback_instructions: String,
    refresh_interval_secs: u64,
    snapshot: RwLock<Arc<ContentSnapshot>>,
    status: RwLock<ContentStatus>,
    refresh_guard: Mutex<()>,
}

impl ContentManager {
    pub async fn initialize(
        config: &Config,
        fallback_instructions: &str,
    ) -> anyhow::Result<Arc<Self>> {
        let source = ContentSource::from_config(config)?;
        let limits = Limits {
            max_file_bytes: config
                .content
                .max_file_kb
                .checked_mul(1024)
                .context("content.max_file_kb 过大")?,
            max_files: config.content.max_files,
        };
        let started = Instant::now();
        let loaded = source
            .load(limits)
            .await
            .with_context(|| format!("{} 内容源首次加载失败", config.content.mode))?;
        let snapshot = Arc::new(build_snapshot(
            1,
            config.content.mode,
            loaded,
            fallback_instructions,
            limits,
        )?);
        let status = ContentStatus {
            mode: config.content.mode.to_string(),
            generation: snapshot.generation,
            refresh_interval_secs: config.content.refresh_interval_secs,
            loaded_at_unix: snapshot.loaded_at_unix,
            last_attempt_unix: snapshot.loaded_at_unix,
            last_refresh_duration_ms: millis(started.elapsed()),
            knowledge_count: snapshot.knowledge.len(),
            template_count: snapshot.templates.len(),
            last_error: None,
        };
        tracing::info!(
            mode = %status.mode,
            generation = status.generation,
            knowledge = status.knowledge_count,
            templates = status.template_count,
            "内容快照首次加载完成"
        );
        Ok(Arc::new(Self {
            mode: config.content.mode,
            source,
            limits,
            fallback_instructions: fallback_instructions.to_owned(),
            refresh_interval_secs: config.content.refresh_interval_secs,
            snapshot: RwLock::new(snapshot),
            status: RwLock::new(status),
            refresh_guard: Mutex::new(()),
        }))
    }

    pub fn snapshot(&self) -> Arc<ContentSnapshot> {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn status(&self) -> ContentStatus {
        self.status
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub async fn refresh(&self) -> anyhow::Result<()> {
        let Ok(_guard) = self.refresh_guard.try_lock() else {
            tracing::info!("内容刷新仍在执行，跳过本次触发");
            return Ok(());
        };
        self.reload().await
    }

    /// 写入一批知识文件，并在同一把锁内立即重建内存快照。
    pub async fn upload_knowledge(
        &self,
        files: Vec<(String, Vec<u8>)>,
    ) -> anyhow::Result<KnowledgeUploadResult> {
        if files.is_empty() {
            bail!("没有收到知识文件");
        }
        let mut seen = std::collections::BTreeSet::new();
        for (path, bytes) in &files {
            validate_knowledge_upload(path, bytes, self.limits)?;
            if !seen.insert(path.clone()) {
                bail!("一次请求中存在重复文件：{path}");
            }
        }

        let _guard = self.refresh_guard.lock().await;
        let before = self.snapshot();
        let overwritten = files
            .iter()
            .filter(|(path, _)| before.knowledge.contains_key(path))
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        let new_files = files.len().saturating_sub(overwritten.len());
        let total_after = before
            .knowledge
            .len()
            .saturating_add(before.templates.len())
            .saturating_add(1)
            .saturating_add(new_files);
        if total_after > self.limits.max_files {
            bail!(
                "上传后内容文件数 {total_after} 超过上限 {}",
                self.limits.max_files
            );
        }

        let uploaded = files.iter().map(|(path, _)| path.clone()).collect();
        self.source.put_knowledge(files).await?;
        self.reload().await?;
        let snapshot = self.snapshot();
        Ok(KnowledgeUploadResult {
            uploaded,
            overwritten,
            generation: snapshot.generation,
            knowledge_count: snapshot.knowledge.len(),
        })
    }

    async fn reload(&self) -> anyhow::Result<()> {
        let started = Instant::now();
        let attempted_at = unix_now();
        let generation = self.snapshot().generation.saturating_add(1);
        let result = async {
            let loaded = self.source.load(self.limits).await?;
            build_snapshot(
                generation,
                self.mode,
                loaded,
                &self.fallback_instructions,
                self.limits,
            )
        }
        .await;

        match result {
            Ok(snapshot) => {
                let snapshot = Arc::new(snapshot);
                *self
                    .snapshot
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot.clone();
                *self
                    .status
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = ContentStatus {
                    mode: self.mode.to_string(),
                    generation: snapshot.generation,
                    refresh_interval_secs: self.refresh_interval_secs,
                    loaded_at_unix: snapshot.loaded_at_unix,
                    last_attempt_unix: attempted_at,
                    last_refresh_duration_ms: millis(started.elapsed()),
                    knowledge_count: snapshot.knowledge.len(),
                    template_count: snapshot.templates.len(),
                    last_error: None,
                };
                tracing::info!(
                    generation = snapshot.generation,
                    knowledge = snapshot.knowledge.len(),
                    templates = snapshot.templates.len(),
                    elapsed_ms = millis(started.elapsed()),
                    "内容快照刷新完成"
                );
                Ok(())
            }
            Err(error) => {
                let mut status = self
                    .status
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                status.last_attempt_unix = attempted_at;
                status.last_refresh_duration_ms = millis(started.elapsed());
                status.last_error = Some(format!("{error:#}"));
                Err(error)
            }
        }
    }

    pub fn spawn_refresh_task(self: &Arc<Self>) {
        if self.refresh_interval_secs == 0 {
            tracing::info!("内容定时刷新已关闭");
            return;
        }
        let manager = Arc::clone(self);
        let interval = Duration::from_secs(self.refresh_interval_secs);
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if let Err(error) = manager.refresh().await {
                    tracing::error!(error = %format!("{error:#}"), "内容快照刷新失败，继续使用上一版本");
                }
            }
        });
    }
}

impl ContentSource {
    fn from_config(config: &Config) -> anyhow::Result<Self> {
        match config.content.mode {
            ContentMode::Local => Ok(ContentSource::Local {
                knowledge_dir: PathBuf::from(&config.paths.knowledge_dir),
                templates_dir: PathBuf::from(&config.paths.templates_dir),
                agents_file: config.paths.agents_file.as_deref().map(PathBuf::from),
            }),
            ContentMode::S3 => Ok(ContentSource::S3 {
                store: build_s3_store(&config.content.s3)?,
                root_prefix: config.content.s3.root_prefix.clone(),
                download_concurrency: config.content.s3.download_concurrency,
            }),
        }
    }

    async fn load(&self, limits: Limits) -> anyhow::Result<LoadedBundle> {
        match self {
            ContentSource::Local {
                knowledge_dir,
                templates_dir,
                agents_file,
            } => {
                let knowledge_dir = knowledge_dir.clone();
                let templates_dir = templates_dir.clone();
                let agents_file = agents_file.clone();
                tokio::task::spawn_blocking(move || {
                    load_local_bundle(
                        &knowledge_dir,
                        &templates_dir,
                        agents_file.as_deref(),
                        limits,
                    )
                })
                .await
                .context("等待本地内容加载任务失败")?
            }
            ContentSource::S3 {
                store,
                root_prefix,
                download_concurrency,
            } => {
                load_s3_bundle(
                    Arc::clone(store),
                    root_prefix,
                    *download_concurrency,
                    limits,
                )
                .await
            }
        }
    }

    async fn put_knowledge(&self, files: Vec<(String, Vec<u8>)>) -> anyhow::Result<()> {
        match self {
            ContentSource::Local { knowledge_dir, .. } => {
                let knowledge_dir = knowledge_dir.clone();
                tokio::task::spawn_blocking(move || {
                    for (relative, bytes) in files {
                        let destination = relative
                            .split('/')
                            .fold(knowledge_dir.clone(), |path, part| path.join(part));
                        let parent = destination.parent().context("知识文件缺少父目录")?;
                        fs::create_dir_all(parent)
                            .with_context(|| format!("创建知识目录失败：{}", parent.display()))?;
                        fs::write(&destination, bytes).with_context(|| {
                            format!("写入知识文件失败：{}", destination.display())
                        })?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .context("等待本地知识文件写入任务失败")??;
            }
            ContentSource::S3 {
                store, root_prefix, ..
            } => {
                for (relative, bytes) in files {
                    let key =
                        join_object_key(&join_object_key(root_prefix, "knowledge"), &relative);
                    let location = ObjectPath::parse(&key)
                        .with_context(|| format!("S3 知识对象路径无效：{key}"))?;
                    store
                        .put(&location, bytes.into())
                        .await
                        .with_context(|| format!("上传 S3 知识对象 {key} 失败"))?;
                }
            }
        }
        Ok(())
    }
}

fn build_snapshot(
    generation: u64,
    mode: ContentMode,
    loaded: LoadedBundle,
    fallback_instructions: &str,
    limits: Limits,
) -> anyhow::Result<ContentSnapshot> {
    let file_count = loaded.knowledge.len()
        + loaded.templates.len()
        + usize::from(loaded.instructions.is_some());
    if file_count > limits.max_files {
        bail!("内容文件数 {file_count} 超过上限 {}", limits.max_files);
    }
    let instructions = match loaded.instructions {
        Some(value) if !value.trim().is_empty() => value,
        Some(_) if mode == ContentMode::S3 => bail!("S3 根目录下的 AGENTS.md 不能为空"),
        None if mode == ContentMode::S3 => bail!("S3 根目录下缺少 AGENTS.md"),
        _ => fallback_instructions.to_owned(),
    };
    let templates = templates::from_documents(&loaded.templates)?;
    let retriever = KeywordRetriever::from_documents(&loaded.knowledge);
    Ok(ContentSnapshot {
        generation,
        loaded_at_unix: unix_now(),
        instructions,
        knowledge: loaded.knowledge,
        retriever,
        templates,
    })
}

fn load_local_bundle(
    knowledge_dir: &Path,
    templates_dir: &Path,
    agents_file: Option<&Path>,
    limits: Limits,
) -> anyhow::Result<LoadedBundle> {
    let knowledge = read_local_directory(knowledge_dir, "知识库", limits)?;
    let templates = read_local_directory(templates_dir, "模板", limits)?;
    let instructions = match agents_file {
        Some(path) if path.exists() => Some(read_local_file(path, limits)?),
        Some(path) => {
            tracing::warn!(path = %path.display(), "智能体指令文件不存在，使用内置默认提示");
            None
        }
        None => None,
    };
    Ok(LoadedBundle {
        instructions,
        knowledge,
        templates,
    })
}

fn read_local_directory(
    root: &Path,
    label: &str,
    limits: Limits,
) -> anyhow::Result<BTreeMap<String, String>> {
    if !root.is_dir() {
        bail!("{label}目录不存在或不是目录：{}", root.display());
    }
    let mut documents = BTreeMap::new();
    for entry in WalkDir::new(root).sort_by_file_name() {
        let entry = entry.with_context(|| format!("遍历{label}目录失败：{}", root.display()))?;
        if !entry.file_type().is_file() || !is_document(entry.path()) {
            continue;
        }
        let source = relative_file_path(root, entry.path())?;
        let content = read_local_file(entry.path(), limits)?;
        if documents.insert(source.clone(), content).is_some() {
            bail!("{label}存在重复相对路径：{source}");
        }
        if documents.len() > limits.max_files {
            bail!("{label}文件数超过上限 {}", limits.max_files);
        }
    }
    Ok(documents)
}

fn read_local_file(path: &Path, limits: Limits) -> anyhow::Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("读取文件元数据失败：{}", path.display()))?;
    let size = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if size > limits.max_file_bytes {
        bail!(
            "文件 {} 大小 {} 字节，超过上限 {} 字节",
            path.display(),
            size,
            limits.max_file_bytes
        );
    }
    let bytes = fs::read(path).with_context(|| format!("读取文件失败：{}", path.display()))?;
    decode_utf8(bytes, &path.display().to_string(), limits)
}

async fn load_s3_bundle(
    store: Arc<dyn ObjectStore>,
    root_prefix: &str,
    download_concurrency: usize,
    limits: Limits,
) -> anyhow::Result<LoadedBundle> {
    let agents_key = join_object_key(root_prefix, "AGENTS.md");
    let knowledge_prefix = join_object_key(root_prefix, "knowledge");
    let templates_prefix = join_object_key(root_prefix, "templates");

    let instructions = download_object(&store, &agents_key, limits)
        .await
        .with_context(|| format!("读取 s3://.../{agents_key} 失败"))?;
    let (knowledge, templates) = tokio::try_join!(
        download_prefix(
            Arc::clone(&store),
            &knowledge_prefix,
            download_concurrency,
            limits,
        ),
        download_prefix(
            Arc::clone(&store),
            &templates_prefix,
            download_concurrency,
            limits,
        )
    )?;
    Ok(LoadedBundle {
        instructions: Some(instructions),
        knowledge,
        templates,
    })
}

async fn download_prefix(
    store: Arc<dyn ObjectStore>,
    prefix: &str,
    download_concurrency: usize,
    limits: Limits,
) -> anyhow::Result<BTreeMap<String, String>> {
    let object_prefix =
        ObjectPath::parse(prefix).with_context(|| format!("S3 prefix 无效：{prefix}"))?;
    let prefix_with_slash = format!("{prefix}/");
    let mut objects = store
        .list(Some(&object_prefix))
        .map_err(anyhow::Error::from)
        .try_filter_map(|meta| {
            let key = meta.location.to_string();
            let relative = key.strip_prefix(&prefix_with_slash).map(str::to_owned);
            async move {
                let Some(relative) = relative else {
                    return Ok(None);
                };
                if !is_document(Path::new(&relative)) {
                    return Ok(None);
                }
                validate_object_relative_path(&relative)?;
                Ok(Some((relative, meta.location, meta.size)))
            }
        })
        .try_collect::<Vec<_>>()
        .await
        .with_context(|| format!("列举 S3 prefix {prefix} 失败"))?;
    objects.sort_by(|left, right| left.0.cmp(&right.0));
    if objects.len() > limits.max_files {
        bail!("S3 prefix {prefix} 文件数超过上限 {}", limits.max_files);
    }

    let downloads = stream::iter(objects)
        .map(|(relative, location, advertised_size)| {
            let store = Arc::clone(&store);
            async move {
                let size = usize::try_from(advertised_size).unwrap_or(usize::MAX);
                if size > limits.max_file_bytes {
                    bail!(
                        "S3 对象 {} 大小 {} 字节，超过上限 {} 字节",
                        location,
                        size,
                        limits.max_file_bytes
                    );
                }
                let result = store
                    .get(&location)
                    .await
                    .with_context(|| format!("下载 S3 对象 {location} 失败"))?;
                let bytes = result
                    .bytes()
                    .await
                    .with_context(|| format!("读取 S3 对象 {location} 内容失败"))?;
                let content = decode_utf8(bytes.to_vec(), location.as_ref(), limits)?;
                Ok::<_, anyhow::Error>((relative, content))
            }
        })
        .buffer_unordered(download_concurrency)
        .collect::<Vec<_>>()
        .await;

    let mut documents = BTreeMap::new();
    for result in downloads {
        let (relative, content) = result?;
        if documents.insert(relative.clone(), content).is_some() {
            bail!("S3 prefix {prefix} 存在重复相对路径：{relative}");
        }
    }
    Ok(documents)
}

async fn download_object(
    store: &Arc<dyn ObjectStore>,
    key: &str,
    limits: Limits,
) -> anyhow::Result<String> {
    let location = ObjectPath::parse(key).with_context(|| format!("S3 key 无效：{key}"))?;
    let result = store
        .get(&location)
        .await
        .with_context(|| format!("下载 S3 对象 {key} 失败"))?;
    let bytes = result
        .bytes()
        .await
        .with_context(|| format!("读取 S3 对象 {key} 内容失败"))?;
    decode_utf8(bytes.to_vec(), key, limits)
}

fn build_s3_store(config: &S3) -> anyhow::Result<Arc<dyn ObjectStore>> {
    let mut builder = AmazonS3Builder::from_env()
        .with_bucket_name(&config.bucket)
        .with_region(&config.region)
        .with_allow_http(config.allow_http)
        .with_virtual_hosted_style_request(!config.force_path_style);
    if let Some(endpoint) = &config.endpoint {
        builder = builder.with_endpoint(endpoint);
    }

    let access_key = env::var("DOCUMENTX_CONTENT_S3_ACCESS_KEY_ID").ok();
    let secret_key = env::var("DOCUMENTX_CONTENT_S3_SECRET_ACCESS_KEY").ok();
    match (access_key, secret_key) {
        (Some(access_key), Some(secret_key)) => {
            builder = builder
                .with_access_key_id(access_key)
                .with_secret_access_key(secret_key);
        }
        (None, None) => {}
        _ => bail!(
            "DOCUMENTX_CONTENT_S3_ACCESS_KEY_ID 和 DOCUMENTX_CONTENT_S3_SECRET_ACCESS_KEY 必须同时设置"
        ),
    }
    if let Ok(token) = env::var("DOCUMENTX_CONTENT_S3_SESSION_TOKEN") {
        if !token.trim().is_empty() {
            builder = builder.with_token(token);
        }
    }
    Ok(Arc::new(builder.build().context("创建 S3 客户端失败")?))
}

fn relative_file_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} 不在目录 {} 内", path.display(), root.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .with_context(|| format!("路径不是 UTF-8：{}", path.display()))?
                    .to_owned(),
            ),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("非法相对路径：{}", path.display())
            }
        }
    }
    if parts.is_empty() {
        bail!("空相对路径：{}", path.display());
    }
    Ok(parts.join("/"))
}

fn validate_object_relative_path(path: &str) -> anyhow::Result<()> {
    if path.contains('\\')
        || path.starts_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        bail!("S3 对象包含非法相对路径：{path}");
    }
    Ok(())
}

pub fn knowledge_upload_path(directory: &str, file_name: &str) -> anyhow::Result<String> {
    let file_name = file_name.trim();
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains(['/', '\\'])
        || file_name.chars().any(char::is_control)
    {
        bail!("文件名无效：{file_name:?}");
    }
    if !is_document(Path::new(file_name)) {
        bail!("只支持 .md、.markdown、.txt 文件：{file_name}");
    }

    let directory = directory.trim().trim_matches('/');
    if directory.is_empty() {
        return Ok(file_name.to_owned());
    }
    validate_object_relative_path(directory)?;
    Ok(format!("{directory}/{file_name}"))
}

fn validate_knowledge_upload(path: &str, bytes: &[u8], limits: Limits) -> anyhow::Result<()> {
    validate_object_relative_path(path)?;
    if !is_document(Path::new(path)) {
        bail!("只支持 .md、.markdown、.txt 文件：{path}");
    }
    if bytes.len() > limits.max_file_bytes {
        bail!(
            "文件 {path} 大小 {} 字节，超过上限 {} 字节",
            bytes.len(),
            limits.max_file_bytes
        );
    }
    std::str::from_utf8(bytes).with_context(|| format!("文件不是 UTF-8：{path}"))?;
    Ok(())
}

fn is_document(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "txt"
            )
        })
}

fn decode_utf8(bytes: Vec<u8>, source: &str, limits: Limits) -> anyhow::Result<String> {
    if bytes.len() > limits.max_file_bytes {
        bail!(
            "文件 {source} 实际大小 {} 字节，超过上限 {} 字节",
            bytes.len(),
            limits.max_file_bytes
        );
    }
    String::from_utf8(bytes).with_context(|| format!("文件不是 UTF-8：{source}"))
}

fn join_object_key(prefix: &str, suffix: &str) -> String {
    if prefix.is_empty() {
        suffix.to_owned()
    } else {
        format!("{prefix}/{suffix}")
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use object_store::memory::InMemory;

    use super::*;

    struct TempContent {
        root: PathBuf,
    }

    impl TempContent {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "documentx-content-test-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            fs::create_dir_all(root.join("knowledge/产品")).unwrap();
            fs::create_dir_all(root.join("templates")).unwrap();
            fs::write(root.join("AGENTS.md"), "本地指令").unwrap();
            fs::write(root.join("knowledge/产品/API.md"), "接口正文").unwrap();
            fs::write(root.join("templates/对外文档.md"), "模板正文").unwrap();
            Self { root }
        }
    }

    impl Drop for TempContent {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn test_limits() -> Limits {
        Limits {
            max_file_bytes: 64 * 1024,
            max_files: 100,
        }
    }

    #[test]
    fn local_bundle_loads_agents_knowledge_and_templates_together() {
        let temp = TempContent::new();
        let bundle = load_local_bundle(
            &temp.root.join("knowledge"),
            &temp.root.join("templates"),
            Some(&temp.root.join("AGENTS.md")),
            test_limits(),
        )
        .unwrap();

        assert_eq!(bundle.instructions.as_deref(), Some("本地指令"));
        assert_eq!(bundle.knowledge["产品/API.md"], "接口正文");
        assert_eq!(bundle.templates["对外文档.md"], "模板正文");
    }

    #[tokio::test]
    async fn s3_bundle_loads_the_fixed_root_layout_recursively() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        for (key, value) in [
            ("documentx/AGENTS.md", "OSS 指令"),
            ("documentx/knowledge/产品/API.md", "接口正文"),
            ("documentx/templates/技术/API模板.md", "模板正文"),
            ("other/knowledge/不应加载.md", "越界内容"),
        ] {
            store
                .put(
                    &ObjectPath::parse(key).unwrap(),
                    value.as_bytes().to_vec().into(),
                )
                .await
                .unwrap();
        }

        let bundle = load_s3_bundle(Arc::clone(&store), "documentx", 2, test_limits())
            .await
            .unwrap();
        assert_eq!(bundle.instructions.as_deref(), Some("OSS 指令"));
        assert_eq!(bundle.knowledge.len(), 1);
        assert_eq!(bundle.knowledge["产品/API.md"], "接口正文");
        assert_eq!(bundle.templates["技术/API模板.md"], "模板正文");

        // 下一次加载会重新读取根 AGENTS.md，不会把首次内容永久缓存。
        let agents_key = ObjectPath::parse("documentx/AGENTS.md").unwrap();
        store
            .put(&agents_key, "更新后的 OSS 指令".as_bytes().to_vec().into())
            .await
            .unwrap();
        let refreshed = load_s3_bundle(store, "documentx", 2, test_limits())
            .await
            .unwrap();
        assert_eq!(refreshed.instructions.as_deref(), Some("更新后的 OSS 指令"));
    }

    #[tokio::test]
    async fn failed_refresh_keeps_the_previous_snapshot() {
        let temp = TempContent::new();
        let mut config = Config::default();
        config.paths.knowledge_dir = temp.root.join("knowledge").display().to_string();
        config.paths.templates_dir = temp.root.join("templates").display().to_string();
        config.paths.agents_file = Some(temp.root.join("AGENTS.md").display().to_string());
        config.content.refresh_interval_secs = 0;
        let manager = ContentManager::initialize(&config, "fallback")
            .await
            .unwrap();
        let before = manager.snapshot();

        fs::remove_dir_all(temp.root.join("knowledge")).unwrap();
        assert!(manager.refresh().await.is_err());
        let after = manager.snapshot();
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.knowledge["产品/API.md"], "接口正文");
        assert!(manager.status().last_error.is_some());
    }

    #[tokio::test]
    async fn local_upload_is_available_in_the_next_snapshot_immediately() {
        let temp = TempContent::new();
        let mut config = Config::default();
        config.paths.knowledge_dir = temp.root.join("knowledge").display().to_string();
        config.paths.templates_dir = temp.root.join("templates").display().to_string();
        config.paths.agents_file = Some(temp.root.join("AGENTS.md").display().to_string());
        config.content.refresh_interval_secs = 0;
        let manager = ContentManager::initialize(&config, "fallback")
            .await
            .unwrap();

        let result = manager
            .upload_knowledge(vec![(
                "新增/说明.md".to_owned(),
                "# 新知识\n\n上传后立即可检索。".as_bytes().to_vec(),
            )])
            .await
            .unwrap();

        assert_eq!(result.uploaded, ["新增/说明.md"]);
        assert!(result.overwritten.is_empty());
        assert_eq!(manager.snapshot().knowledge["新增/说明.md"], "# 新知识\n\n上传后立即可检索。");
        assert_eq!(
            fs::read_to_string(temp.root.join("knowledge/新增/说明.md")).unwrap(),
            "# 新知识\n\n上传后立即可检索。"
        );
    }

    #[tokio::test]
    async fn s3_upload_uses_the_configured_knowledge_prefix() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let source = ContentSource::S3 {
            store: Arc::clone(&store),
            root_prefix: "documentx".to_owned(),
            download_concurrency: 2,
        };
        source
            .put_knowledge(vec![(
                "业务/API.md".to_owned(),
                "接口正文".as_bytes().to_vec(),
            )])
            .await
            .unwrap();

        let location = ObjectPath::parse("documentx/knowledge/业务/API.md").unwrap();
        let value = store.get(&location).await.unwrap().bytes().await.unwrap();
        assert_eq!(value.as_ref(), "接口正文".as_bytes());
    }

    #[test]
    fn rejects_object_path_traversal() {
        assert!(validate_object_relative_path("产品/API.md").is_ok());
        assert!(validate_object_relative_path("../AGENTS.md").is_err());
        assert!(validate_object_relative_path("a//b.md").is_err());
        assert!(validate_object_relative_path("a\\b.md").is_err());
    }

    #[test]
    fn upload_path_accepts_directories_and_rejects_unsafe_names() {
        assert_eq!(
            knowledge_upload_path("业务服务/WorkBuddy/", "API.md").unwrap(),
            "业务服务/WorkBuddy/API.md"
        );
        assert!(knowledge_upload_path("../outside", "API.md").is_err());
        assert!(knowledge_upload_path("业务服务", "../API.md").is_err());
        assert!(knowledge_upload_path("业务服务", "API.pdf").is_err());
    }
}
