use anyhow::anyhow;
use typst_as_lib::TypstEngine;
use typst_layout::PagedDocument;
use typst_pdf::PdfOptions;

use super::md2typst;

/// 内嵌到二进制里的中文字体（Noto Sans SC，覆盖中文 + 拉丁 + 数字）。
/// 这样导出 PDF 无需系统安装任何字体或 typst 命令，换任意机器都能跑。
static FONT_NOTO_SANS_SC: &[u8] = include_bytes!("../../assets/fonts/NotoSansSC-Regular.otf");
static FONT_NOTO_SANS_SC_BOLD: &[u8] = include_bytes!("../../assets/fonts/NotoSansSC-Bold.otf");
static FONT_JETBRAINS_MONO: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
const FONT_FAMILY: &str = "Noto Sans SC";

/// 用内嵌的 typst 库把 Markdown 渲染成 PDF（零外部依赖）。
pub fn render_pdf(md: &str, title: &str) -> anyhow::Result<Vec<u8>> {
    let body = md2typst::convert(md);
    let source = wrap_document(title, &body);

    let engine = TypstEngine::builder()
        .main_file(source)
        .fonts([
            FONT_NOTO_SANS_SC,
            FONT_NOTO_SANS_SC_BOLD,
            FONT_JETBRAINS_MONO,
        ])
        .build();

    let doc: PagedDocument = engine
        .compile()
        .output
        .map_err(|e| anyhow!("typst 编译失败：{e:?}"))?;

    let pdf = typst_pdf::pdf(&doc, &PdfOptions::default())
        .map_err(|e| anyhow!("PDF 生成失败：{e:?}"))?;
    Ok(pdf)
}

fn wrap_document(title: &str, body: &str) -> String {
    // 用占位符替换而非 format!，以便在模板里自然书写 typst 的花括号。
    // GitHub 风格：黑色加粗标题、等宽代码字体、灰底表头、完整边框、克制留白。
    const TEMPLATE: &str = r##"#set document(title: "@@TITLE@@")
#set page(paper: "a4", margin: (x: 2.3cm, y: 2.3cm), numbering: "1", number-align: center)
#set text(font: "@@FONT@@", size: 10.5pt, fill: rgb("#24292f"), lang: "zh")
#set par(justify: false, leading: 0.85em, spacing: 1.15em)

// 标题层级：黑色加粗，H1/H2 带底部分隔线
#show heading.where(level: 1): it => {
  set text(size: 22pt, weight: "bold", fill: rgb("#1f2328"))
  block(above: 0.2em, below: 0.35em, it)
  v(-0.25em)
  line(length: 100%, stroke: 0.75pt + rgb("#d0d7de"))
  v(0.55em)
}
#show heading.where(level: 2): it => {
  set text(size: 16.5pt, weight: "bold", fill: rgb("#1f2328"))
  block(above: 1.6em, below: 0.3em, it)
  v(-0.15em)
  line(length: 100%, stroke: 0.6pt + rgb("#d8dee4"))
  v(0.5em)
}
#show heading.where(level: 3): it => {
  set text(size: 13pt, weight: "bold", fill: rgb("#1f2328"))
  block(above: 1.2em, below: 0.5em, it)
}
#show heading.where(level: 4): it => {
  set text(size: 11pt, weight: "bold", fill: rgb("#57606a"))
  block(above: 1em, below: 0.4em, it)
}

// 表格：灰底表头 + 完整浅灰边框
#set table(
  stroke: 0.75pt + rgb("#d0d7de"),
  inset: (x: 10pt, y: 7pt),
  fill: (_, y) => if y == 0 { rgb("#f6f8fa") } else { none },
)
#show table.cell.where(y: 0): set text(weight: "bold", fill: rgb("#1f2328"))

// 引用块：灰色左条 + 灰字（GitHub 风）
#show quote.where(block: true): it => block(
  width: 100%, inset: (left: 14pt, y: 3pt),
  stroke: (left: 3pt + rgb("#d0d7de")),
  text(fill: rgb("#57606a"), it.body),
)

// 代码：等宽字体；块级灰底描边，行内灰底
#show raw.where(block: true): it => block(
  width: 100%, fill: rgb("#f6f8fa"), inset: 11pt, radius: 6pt,
  stroke: 0.75pt + rgb("#d0d7de"),
  text(font: ("JetBrains Mono", "@@FONT@@"), size: 9pt, fill: rgb("#24292f"), it),
)
#show raw.where(block: false): it => box(
  fill: rgb("#eff1f3"), inset: (x: 4pt), outset: (y: 3.5pt), radius: 3pt,
  text(font: ("JetBrains Mono", "@@FONT@@"), size: 9pt, fill: rgb("#24292f"), it),
)

// 分隔线与链接
#show link: set text(fill: rgb("#0969da"))

@@BODY@@
"##;
    TEMPLATE
        .replace("@@FONT@@", FONT_FAMILY)
        .replace("@@TITLE@@", &md2typst::escape_str(title))
        .replace("@@BODY@@", body)
}
