# AGENTS.md — 维护 DocumentX 项目的指南

本文件面向**参与维护本仓库的编码智能体 / 开发者**。它描述项目结构、约定与注意事项，帮助你安全地修改代码。
（注意：本文件与 `release/AGENTS.md` / `deploy/AGENTS.md` 用途不同——那两个是**运行时**喂给 documentx 的行为指令；本文件只给维护者看，程序不会加载。）

## 项目是什么

DocumentX 是一个文档智能体，以网页服务形式对外提供：

- **对外**：网页问答（基于内部知识库），可把回答或按模板生成的文档导出为 Markdown / PDF / Word。
- **对内**：`knowledge/` 放知识库文档，`templates/` 放输出模板（每个模板是一份指导性参考 `.md`）。

## 技术栈

- **后端**：Rust + Axum（`backend/`）。LLM 走 **OpenAI Chat Completions 兼容协议**（`base_url`/`api_key`/`model` 可配置），流式用 SSE。
- **前端**：Vite + React + TypeScript（`frontend/`）。
- **PDF / Word**：内嵌 **typst 库**（非 CLI），中文字体 Noto Sans SC 用 `include_bytes!` 打包进二进制；PDF 直接使用，Word 会把混淆后的字体嵌入 DOCX。
- **技术图表**：Mermaid / Graphviz / Vega(Lite) 引擎随前端按需加载；浏览器一次解析生成内部占位正文、SVG 与 PNG 资产，PDF 嵌入 SVG，Word 嵌入 PNG；后端校验占位顺序、源码哈希和资产安全，不访问图表外部服务。

## 目录结构

```
backend/src/
  main.rs         启动：加载配置/知识库/模板/指令，建路由
  config.rs       配置（config.toml + 环境变量覆盖）
  llm.rs          OpenAI 兼容客户端（chat_stream 流式 / chat_once 非流式）
  knowledge.rs    Retriever trait + KeywordRetriever（关键词+中文 bigram 检索）
  templates.rs    模板加载
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

服务默认 http://localhost:8080。前端 dev 模式：`cd frontend && npm run dev`（5173，/api 代理到 8080）。

## HTTP 接口

- `POST /api/chat` — 流式对话（SSE）。body: `{ messages, use_knowledge }`
- `POST /api/generate` — 按模板+知识库生成文档并下载。body: `{ instruction, template, format, title, use_knowledge }`
- `POST /api/export` — 把给定 Markdown 导出为 md/pdf/docx。body: `{ content, format, title }`
- `GET /api/templates` · `GET /api/knowledge` · `GET /api/health`

## 分析项目并写入知识库

当任务要求分析某个项目，并把分析结果写入 DocumentX 知识库时，必须遵循以下规则：

1. **以当前代码实现为准**：尽量阅读实际源码，不得只摘抄 README、接口文档、注释或历史说明。文档与代码冲突时，以当前代码行为为准，并在知识文档中指出差异。
2. **从入口追踪真实行为**：优先检查启动入口、路由注册、handler/controller、请求与响应结构、鉴权和签名、中间件、配置、数据库 schema、外部依赖、错误码、测试及构建部署文件；必要时沿调用链继续追踪，避免只根据文件名或局部代码推断。
3. **区分服务边界**：明确标注哪些接口和能力由目标项目直接提供，哪些只是它调用的上游、测试客户端调用的平台能力、示例代码或已经删除的旧能力，不能混写成目标项目的公共 API。
4. **结论可追溯**：重要结论应能在代码、配置、schema 或测试中找到依据。无法从代码确认的内容要标为“未确认”“推断”或“知识缺口”，不得补造端点、参数、认证方式和业务规则。
5. **覆盖可运维信息**：除功能说明外，尽量整理默认基路径、环境变量、存储模型、状态值、限流、超时、日志、可观测性、安全边界和启动/重载要求。
6. **保护敏感信息**：不得把源码、配置或示例中的真实密钥、Token、手机号和内部凭据复制进知识库；只记录变量名、算法、字段和脱敏示例。
7. **面向检索组织内容**：详细文档之外，可增加简短的 API/边界速查文档，并写入明确关键词，避免检索器只命中架构片段而漏掉完整路由和鉴权信息。
8. **写入后验证**：确认文件位于实际配置的 `knowledge_dir`，格式为可加载的 `.md`/`.markdown`/`.txt`，检查关键接口和关键词确实存在；条件允许时运行目标项目已有测试或编译检查。提醒使用者知识库在服务重启后才会重新加载。

## 关键约定与坑（改代码前务必知道）

1. **CWD 敏感**：`config.toml`、`knowledge/`、`templates/`、`static_dir` 都是相对当前工作目录解析的。服务必须在包含这些的目录里启动（开发时是仓库根，部署时是 `release/`）。
2. **运行时系统提示可外置**：若 `config.toml` 的 `[paths].agents_file` 指向一个存在的文件（如 `AGENTS.md`），其内容会作为系统提示加载（见 `api::load_instructions`）；否则用 `api::DEFAULT_SYSTEM_PROMPT`。**开发时仓库根的 config 不要把 agents_file 指向本文件**——本文件是给维护者的，不是给模型的。
3. **SSE 不能被压缩**：`api.rs` 的 CompressionLayer 用 `NotForContentType("text/event-stream")` 排除了流式响应。新增流式路由时别破坏这点，也别给流式路由套整体 `TimeoutLayer`（会掐断长回答）。
4. **中文流式**：`llm.rs` 按字节缓冲、按 `\n` 切行再转字符串，避免多字节字符被 chunk 边界截断。改流式解析时保留这个策略。
5. **typst 版本锁定**：`typst-as-lib` / `typst` / `typst-pdf` / `typst-layout` 版本需相互匹配（当前 0.16 / 0.15 系）。升级时四者一起动，并重新验证 PDF 渲染。
6. **字体**：`backend/assets/fonts/NotoSansSC-Regular.otf`（family = "Noto Sans SC"）通过 `include_bytes!` 编入。换字体要同步改 `pdf.rs` 里的 `FONT_FAMILY`，并验证 `docx.rs` 的 OOXML 字体嵌入与混淆逻辑。
7. **知识库/模板/指令为启动时加载**：改这些文件需重启服务。（热更新尚未实现，是已知的可扩展点。）
8. **请求体上限**：由 `config.server.max_body_mb` 控制（默认 16MB），在 `api.rs` 用 `RequestBodyLimitLayer` 显式设置，覆盖 axum 默认的 2MB。

## 修改后请验证

- `cargo check` 通过、无警告。
- 若动了 `render/` 或字体：实际生成含中文与图表的 PDF、Word，并逐页确认表格、代码块、中文和图片渲染正常。
- 若动了 `frontend/`：`npm run build` 通过。
- 提交信息用中文或英文均可，聚焦「做了什么、为什么」。

## 不要提交的东西

`.gitignore` 已忽略：`backend/target/`、`frontend/node_modules`、`frontend/dist`、`config.toml`、`.env`、`release/`。别把密钥、构建产物、发布目录提交进仓库。
