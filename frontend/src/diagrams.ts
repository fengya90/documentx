export type DiagramKind = "mermaid" | "graphviz" | "vega" | "vegalite";

export interface DiagramSource {
  kind: DiagramKind;
  source: string;
}

export interface DiagramAsset extends DiagramSource {
  source_hash: string;
  svg: string;
  png_base64: string;
}

const renderCache = new Map<string, Promise<string>>();
const assetCache = new Map<string, Promise<DiagramAsset>>();
const DIAGRAM_MARKER_PREFIX = "DOCUMENTX_DIAGRAM_";
let mermaidInitialized = false;
let diagramSequence = 0;

export function normalizeDiagramKind(value: string): DiagramKind | null {
  switch (value.trim().toLowerCase().replace(/^language-/, "")) {
    case "mermaid": return "mermaid";
    case "graphviz":
    case "dot": return "graphviz";
    case "vega": return "vega";
    case "vegalite":
    case "vega-lite": return "vegalite";
    default: return null;
  }
}

/** 按 Markdown fence 顺序提取图表，规则与 Rust 导出端保持一致。 */
export function collectDiagrams(markdown: string): DiagramSource[] {
  return extractDiagrams(markdown).diagrams;
}

/**
 * 一次解析同时生成后端可识别的占位正文和图表资产，避免浏览器与后端
 * 分别解析 Markdown 后因容器/围栏边界差异产生数量不一致。
 */
export async function prepareDiagramExport(
  markdown: string
): Promise<{ content: string; diagrams: DiagramAsset[] }> {
  const extracted = extractDiagrams(markdown);
  if (extracted.diagrams.length > 32) throw new Error("单篇文档最多支持 32 个图表");
  return {
    content: extracted.content,
    diagrams: await Promise.all(extracted.diagrams.map(renderAsset)),
  };
}

function extractDiagrams(markdown: string): { content: string; diagrams: DiagramSource[] } {
  const lines = markdown.split(/\r?\n/);
  const diagrams: DiagramSource[] = [];
  const output: string[] = [];
  let index = 0;
  while (index < lines.length) {
    const open = openingFence(lines[index]);
    if (!open) {
      output.push(lines[index]);
      index += 1;
      continue;
    }
    const kind = normalizeDiagramKind(open.info);
    if (!kind) {
      const end = findClosingFence(lines, index + 1, open);
      const next = end < lines.length ? end + 1 : lines.length;
      output.push(...lines.slice(index, next));
      index = next;
      continue;
    }
    const end = findClosingFence(lines, index + 1, open);
    if (end >= lines.length) {
      output.push(...lines.slice(index));
      index = lines.length;
      continue;
    }
    diagrams.push({ kind, source: lines.slice(index + 1, end).join("\n") });
    output.push(`@@${DIAGRAM_MARKER_PREFIX}${diagrams.length - 1}@@`);
    index = end + 1;
  }
  return { content: output.join("\n"), diagrams };
}

export function renderDiagramSvg(kindValue: string, source: string): Promise<string> {
  const kind = normalizeDiagramKind(kindValue);
  if (!kind) return Promise.reject(new Error(`不支持的图表类型：${kindValue}`));
  if (!source.trim()) return Promise.reject(new Error("图表内容为空"));
  if (new Blob([source]).size > 256 * 1024) {
    return Promise.reject(new Error("图表源码超过 256 KB 上限"));
  }
  const key = `${kind}\0${source}`;
  const cached = renderCache.get(key);
  if (cached) return cached;
  const task = renderByKind(kind, source).then(sanitizeSvg).catch((error) => {
    renderCache.delete(key);
    throw error;
  });
  setBoundedCache(renderCache, key, task);
  return task;
}

/** 并行生成 PDF 所需 SVG 和 Word 所需 2x PNG。 */
export async function renderDiagramAssets(markdown: string): Promise<DiagramAsset[]> {
  return (await prepareDiagramExport(markdown)).diagrams;
}

async function renderAsset(diagram: DiagramSource): Promise<DiagramAsset> {
  const key = `${diagram.kind}\0${diagram.source}`;
  const cached = assetCache.get(key);
  if (cached) return cached;
  const task = (async () => {
    const svg = await renderDiagramSvg(diagram.kind, diagram.source);
    const [source_hash, png_base64] = await Promise.all([
      sourceHash(diagram.kind, diagram.source),
      svgToPngBase64(svg, 2),
    ]);
    return { ...diagram, source_hash, svg, png_base64 };
  })().catch((error) => {
    assetCache.delete(key);
    throw error;
  });
  setBoundedCache(assetCache, key, task);
  return task;
}

async function renderByKind(kind: DiagramKind, source: string): Promise<string> {
  switch (kind) {
    case "mermaid": {
      const { default: mermaid } = await import("mermaid");
      if (!mermaidInitialized) {
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: "neutral",
          deterministicIds: true,
          deterministicIDSeed: "documentx",
          htmlLabels: false,
          fontFamily: '"PingFang SC", "Microsoft YaHei", Arial, sans-serif',
          flowchart: { htmlLabels: false, useMaxWidth: true },
        });
        mermaidInitialized = true;
      }
      await mermaid.parse(source);
      const id = `documentx-diagram-${++diagramSequence}`;
      const result = await mermaid.render(id, source);
      return result.svg;
    }
    case "graphviz": {
      const { instance } = await import("@viz-js/viz");
      const viz = await instance();
      return new XMLSerializer().serializeToString(viz.renderSVGElement(source));
    }
    case "vega":
    case "vegalite": {
      const [vega, vegaLite] = await Promise.all([import("vega"), import("vega-lite")]);
      const parsed = JSON.parse(source) as Record<string, unknown>;
      rejectExternalResources(parsed);
      const spec = kind === "vegalite" ? vegaLite.compile(parsed as never).spec : parsed;
      const view = new vega.View(vega.parse(spec as never), {
        renderer: "none",
        logLevel: vega.Warn,
      });
      try {
        return await view.toSVG(1);
      } finally {
        view.finalize();
      }
    }
  }
}

function sanitizeSvg(svg: string): string {
  const documentNode = new DOMParser().parseFromString(svg, "image/svg+xml");
  if (documentNode.querySelector("parsererror")) throw new Error("图表引擎返回了无效 SVG");
  const root = documentNode.documentElement;
  if (root.localName.toLowerCase() !== "svg") throw new Error("图表引擎未返回 SVG");

  root.querySelectorAll("script, foreignObject, iframe, object, embed, image").forEach((node) => node.remove());
  root.querySelectorAll("*").forEach((node) => {
    for (const attribute of Array.from(node.attributes)) {
      const name = attribute.name.toLowerCase();
      const value = attribute.value.trim().toLowerCase();
      if (
        name.startsWith("on") ||
        ((name === "href" || name === "xlink:href") && !value.startsWith("#")) ||
        ((name === "style" || name === "fill" || name === "stroke" || name === "filter") &&
          (/javascript:|@import|url\((?!["']?#)/i.test(value)))
      ) {
        node.removeAttribute(attribute.name);
      }
    }
  });
  root.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  return new XMLSerializer().serializeToString(root);
}

function rejectExternalResources(value: unknown): void {
  if (Array.isArray(value)) {
    value.forEach(rejectExternalResources);
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    if (["url", "href"].includes(key.toLowerCase())) {
      throw new Error("Vega 图表不允许引用外部 URL，请把数据直接写入 values");
    }
    rejectExternalResources(child);
  }
}

function svgToPngBase64(svg: string, scale: number): Promise<string> {
  return new Promise((resolve, reject) => {
    const blobUrl = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml;charset=utf-8" }));
    const image = new Image();
    image.onload = () => {
      try {
        const intrinsic = svgDimensions(svg, image.naturalWidth || 800, image.naturalHeight || 450);
        const maxScale = Math.min(scale, 4096 / intrinsic.width, 4096 / intrinsic.height);
        const width = Math.max(1, Math.round(intrinsic.width * maxScale));
        const height = Math.max(1, Math.round(intrinsic.height * maxScale));
        const canvas = document.createElement("canvas");
        canvas.width = width;
        canvas.height = height;
        const context = canvas.getContext("2d");
        if (!context) throw new Error("浏览器无法创建图表画布");
        context.fillStyle = "#ffffff";
        context.fillRect(0, 0, width, height);
        context.drawImage(image, 0, 0, width, height);
        canvas.toBlob((png) => {
          if (!png) {
            reject(new Error("生成 Word 图表图片失败"));
            return;
          }
          const reader = new FileReader();
          reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
          reader.onerror = () => reject(new Error("读取 Word 图表图片失败"));
          reader.readAsDataURL(png);
        }, "image/png");
      } catch (error) {
        reject(error);
      } finally {
        URL.revokeObjectURL(blobUrl);
      }
    };
    image.onerror = () => {
      URL.revokeObjectURL(blobUrl);
      reject(new Error("浏览器无法解析图表 SVG"));
    };
    image.src = blobUrl;
  });
}

function svgDimensions(svg: string, fallbackWidth: number, fallbackHeight: number) {
  const root = new DOMParser().parseFromString(svg, "image/svg+xml").documentElement;
  const viewBox = root.getAttribute("viewBox")?.trim().split(/[ ,]+/).map(Number);
  if (viewBox?.length === 4 && viewBox.every(Number.isFinite) && viewBox[2] > 0 && viewBox[3] > 0) {
    return { width: viewBox[2], height: viewBox[3] };
  }
  const width = Number.parseFloat(root.getAttribute("width") ?? "") || fallbackWidth;
  const height = Number.parseFloat(root.getAttribute("height") ?? "") || fallbackHeight;
  return { width, height };
}

async function sourceHash(kind: DiagramKind, source: string): Promise<string> {
  const bytes = new TextEncoder().encode(`${kind}\0${source}`);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

type Fence = { marker: "`" | "~"; length: number; info: string };

function openingFence(line: string): Fence | null {
  const trimmed = line.trimStart();
  const match = trimmed.match(/^(`{3,}|~{3,})(.*)$/);
  return match
    ? { marker: match[1][0] as Fence["marker"], length: match[1].length, info: match[2].trim() }
    : null;
}

function findClosingFence(lines: string[], start: number, open: Fence): number {
  let end = start;
  while (end < lines.length && !isClosingFence(lines[end], open)) end += 1;
  return end;
}

function isClosingFence(line: string, open: Fence): boolean {
  const trimmed = line.trim();
  const marker = open.marker === "`" ? "`" : "~";
  return trimmed.length >= open.length && [...trimmed].every((value) => value === marker);
}

function setBoundedCache<T>(cache: Map<string, T>, key: string, value: T) {
  if (cache.size >= 100) cache.delete(cache.keys().next().value as string);
  cache.set(key, value);
}
