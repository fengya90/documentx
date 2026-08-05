# AGENTS.md — 维护 DocumentX 项目的指南

本文件面向**参与维护本仓库的编码智能体 / 开发者**。它描述项目结构、约定与注意事项，帮助你安全地修改代码。
（注意：本文件与 `release/AGENTS.md` / `deploy/AGENTS.md` 用途不同——那两个是**运行时**喂给 documentx 的行为指令；本文件只给维护者看，程序不会加载。）

## 项目是什么

DocumentX 是一个文档智能体，以网页服务形式对外提供：

- **对外**：网页问答（基于内部知识库），可把回答或按模板生成的文档导出为 Markdown / PDF / Word。
- **对内**：内容源支持本地与 S3 兼容对象存储；`AGENTS.md`、`knowledge/**`、`templates/**` 组成同一份内存快照，启动全量加载并定时原子刷新。

## 技术栈

- **后端**：Rust + Axum（`backend/`）。LLM 走 **OpenAI Chat Completions 兼容协议**（`base_url`/`api_key`/`model` 可配置），流式用 SSE。
- **前端**：Vite + React + TypeScript（`frontend/`）。
- **PDF / Word**：内嵌 **typst 库**（非 CLI），中文字体 Noto Sans SC 用 `include_bytes!` 打包进二进制；PDF 直接使用，Word 会把混淆后的字体嵌入 DOCX。
- **技术图表**：Mermaid / Graphviz / Vega(Lite) 引擎随前端按需加载；浏览器一次解析生成内部占位正文、SVG 与 PNG 资产，PDF 嵌入 SVG，Word 嵌入 PNG；后端校验占位顺序、源码哈希和资产安全，不访问图表外部服务。

## 目录结构

```
backend/src/
  main.rs         启动：加载配置/内容快照，启动刷新任务，建路由
  config.rs       配置（config.toml + 全字段 DOCUMENTX_* 环境变量覆盖）
  content.rs      local/S3 读取、完整快照验证、原子切换、刷新状态
  llm.rs          OpenAI 兼容客户端（chat_stream 流式 / chat_once 非流式）
  knowledge.rs    递归目录索引 + Retriever trait + KeywordRetriever（关键词+中文 bigram 检索）
  templates.rs    从内存文档构建模板（递归路径去扩展名作为模板名）
  render/
    mod.rs        Format(Markdown/Pdf) + render 入口
    diagram.rs    图表识别、浏览器资产匹配与安全校验
    md2typst.rs   Markdown -> Typst 标记转换
    pdf.rs        内嵌 typst 编译成 PDF（字体在 backend/assets/fonts/）
  api.rs          路由、AppState、中间件、各 handler
frontend/src/     App.tsx / api.ts / styles.css
knowledge/        知识库文档（.md/.markdown/.txt，递归扫描）
templates/        模板（每个 .md 一个模板，文件名即模板名）
deploy/           发布用源文件：AGENTS.md（运行时指令）、config.release.toml
scripts/package.sh 打包成 release/ 的脚本
```

## 构建 / 运行 / 测试

从**仓库根目录**操作（相对路径依赖此 CWD）：

```bash
# 前端
cd frontend && npm install && npm run build && cd ..
# 后端（开发）
cargo run --manifest-path backend/Cargo.toml
# 编译检查
cargo check --manifest-path backend/Cargo.toml
# 打包发布成品到 release/
./scripts/package.sh
```

源码开发配置默认 http://localhost:8080；发布配置与 Docker 镜像默认 http://localhost:8080/documentx/。前端 dev 模式：`cd frontend && npm run dev`（5173，/api 代理到 8080）。

## HTTP 接口

- `POST /api/chat` — 流式对话（SSE）。body: `{ messages, use_knowledge }`
- `POST /api/generate` — 按模板+知识库生成文档并下载。body: `{ instruction, template, format, title, use_knowledge }`
- `POST /api/export` — 把给定 Markdown 导出为 md/pdf/docx。body: `{ content, format, title }`
- `POST /api/knowledge/upload` — multipart 批量上传 `.md/.markdown/.txt`，写入 local 或 S3 并立即刷新快照
- `GET /api/templates` · `GET /api/ui-config` · `GET /api/knowledge`（`sources` + `tree`）· `GET /api/knowledge/content?source=...` · `GET /api/content/status` · `GET /api/health`

## 分析项目并写入知识库

当任务是「分析某个项目，并把分析结果写入 DocumentX 知识库」时，按下面的原则产出。

**总纲：代码是唯一事实来源；分析要细致；可以（且鼓励）包含设计与架构；但绝不外泄敏感信息。**

### 1. 代码第一（最高优先级）

- 所有结论必须来自**实际源码**。README、接口文档、注释、wiki、历史说明只当线索用来定位代码，**不能直接照抄**。
- **文档/注释与代码冲突时，一律以代码的当前行为为准**，并在知识文档里点出差异（例如"文档写 X，代码实际是 Y"）。
- 不确定就去读代码求证，不要猜；确实读不到的部分明确标注，不要用文档内容顶替。

### 2. 细致：从入口沿调用链追踪真实行为

不要只看文件名或单个文件就下结论。系统性地追这几类：

- **入口与装配**：启动入口、依赖注入、路由与中间件注册、服务初始化顺序。
- **接口契约**：每个对外端点的方法、路径、请求/响应结构、字段类型与是否必填、默认值、分页。
- **鉴权与安全**：认证方式、签名算法与待签名串拼接、token 结构与有效期、权限校验点。
- **数据与状态**：数据库 schema / 迁移、实体关系、状态机与状态值、缓存。
- **横切关注点**：错误码与错误响应格式、限流、超时、重试、日志与可观测性、配置项与环境变量。
- **依赖与边界**：调用了哪些上游 / 外部服务。
- **佐证**：测试用例、构建与部署文件（能反映真实行为与默认值）。

沿调用链跟到底，直到能说清"一个请求进来后到底发生了什么"。

### 3. 设计与架构：鼓励写，但从代码推导

- **可以也应该**包含：系统总览、模块职责、数据流 / 调用时序、关键设计取舍与理由、扩展点。这些让知识库更有价值——**不要因为"追求代码事实"就把设计内容删掉**。
- 但设计叙述必须**从代码推导**、能落到具体模块或函数；**把"代码里读到的事实"和"合理推断"分开写**，推断处标注（如"推断""待确认"），不得把想象当事实。

### 4. 分清服务边界

明确区分：哪些接口 / 能力是**目标项目自己提供**的，哪些只是它**调用的上游**、测试客户端访问的平台能力、示例代码、或已删除的旧能力。**不要把别人的接口写成目标项目的公共 API**——这是最容易出错、也最误导人的地方。

### 5. 保护敏感信息（硬性红线）

- **绝不**把源码 / 配置 / 示例里的**真实**密钥、token、密码、连接串、私钥、手机号、内部主机地址、账号等复制进知识库。
- 只记录：变量名 / 字段名、算法、格式、**脱敏示例**（如 `sk-***`、`13800000000`、`https://<internal-host>/...`）。
- 若发现代码里**硬编码了密钥 / 密码**，作为一条"安全问题"**指出它的存在与位置**，但**绝不写出具体值**。

### 6. 可追溯、不臆造

重要结论应能在代码、配置、schema 或测试里找到依据。无法确认的内容标为"未确认 / 推断 / 知识缺口"，**不得补造**端点、参数、鉴权方式或业务规则。

### 7. 面向检索地组织，并在写入后验证

- 先按项目 / 产品 / 服务分子目录，再按组件或主题细分；除详细文档外，可加一份简短的 **API / 边界速查**，写清关键词，避免检索只命中架构片段而漏掉完整路由与鉴权。**不要为了目录化复制同一份内容**（会重复召回）。
- 写完验证：local 模式确认文件在配置的 `knowledge_dir`；S3 模式确认对象在 `<root_prefix>/knowledge/`；格式为 `.md` / `.markdown` / `.txt`；抽查关键接口与关键词确实存在；条件允许时跑一遍目标项目的测试或编译。提醒使用者：内容在下一次成功的定时刷新后生效（关闭定时刷新时需重启）。

## 关键约定与坑（改代码前务必知道）

1. **CWD 只影响本地路径**：`config.toml`、local 模式的 `knowledge_dir` / `templates_dir` / `agents_file` 与 `static_dir` 都相对当前工作目录解析；S3 对象路径不受 CWD 影响。`server.base_path` 是 URL 前缀而非文件路径，需同时覆盖页面、静态资源和全部 API；前端生产资源保持相对路径，API 以页面根路径相对解析。
2. **内容快照必须保持整体一致**：S3 模式固定读取 `<root_prefix>/AGENTS.md`、`<root_prefix>/knowledge/**`、`<root_prefix>/templates/**`。刷新必须先完整加载并验证三个区域，再原子替换 `Arc<ContentSnapshot>`；失败时保留上一代。每个请求只获取一次快照，不能跨代混用指令、检索器和模板。
3. **SSE 不能被压缩**：`api.rs` 的 CompressionLayer 用 `NotForContentType("text/event-stream")` 排除了流式响应。新增流式路由时别破坏这点，也别给流式路由套整体 `TimeoutLayer`（会掐断长回答）。
4. **中文流式**：`llm.rs` 按字节缓冲、按 `\n` 切行再转字符串，避免多字节字符被 chunk 边界截断。改流式解析时保留这个策略。
5. **typst 版本锁定**：`typst-as-lib` / `typst` / `typst-pdf` / `typst-layout` 版本需相互匹配（当前 0.16 / 0.15 系）。升级时四者一起动，并重新验证 PDF 渲染。
6. **字体**：`backend/assets/fonts/NotoSansSC-Regular.otf`（family = "Noto Sans SC"）通过 `include_bytes!` 编入。换字体要同步改 `pdf.rs` 里的 `FONT_FAMILY`，并验证 `docx.rs` 的 OOXML 字体嵌入与混淆逻辑。
7. **知识路径是稳定标识**：知识库递归扫描 `.md`/`.markdown`/`.txt`，完整相对路径（统一 `/` 分隔）贯穿目录树 API、原文读取和检索来源。不要退回只保留文件名的实现；原文必须读取内存快照，不能按用户路径临时访问磁盘或 OSS。
8. **配置可容器化**：每个 TOML 字段必须有对应的 `DOCUMENTX_*` 环境变量，环境变量值要严格解析并在启动时校验。S3 凭据只允许来自环境/标准凭据链，不能加进 TOML、日志或状态接口。
9. **请求体上限**：由 `config.server.max_body_mb` 控制（默认 16MB），在 `api.rs` 用 `RequestBodyLimitLayer` 显式设置，覆盖 axum 默认的 2MB。

## 修改后请验证

- `cargo check` 通过、无警告。
- 若动了 `render/` 或字体：实际生成含中文与图表的 PDF、Word，并逐页确认表格、代码块、中文和图片渲染正常。
- 若动了 `frontend/`：`npm run build` 通过。
- 提交信息用中文或英文均可，聚焦「做了什么、为什么」。

## 不要提交的东西

`.gitignore` 已忽略：`backend/target/`、`frontend/node_modules`、`frontend/dist`、`config.toml`、`.env`、`release/`。别把密钥、构建产物、发布目录提交进仓库。
