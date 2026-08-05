import type { DiagramAsset } from "./diagrams";

// 页面入口始终由后端规范到带 `/` 的应用根路径，因此相对 URL 同时兼容
// `/` 与 `/documentx/`，无需把部署路径编译进前端产物。
const pageBase = new URL("./", window.location.href);

function apiUrl(path: string): string {
  return new URL(`api/${path.replace(/^\/+/, "")}`, pageBase).toString();
}

export type Role = "user" | "assistant";

export interface ChatMessage {
  role: Role;
  content: string;
  tokens?: number;
}

export interface Usage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

export type KnowledgeNode =
  | {
      type: "directory";
      name: string;
      path: string;
      count: number;
      children: KnowledgeNode[];
    }
  | {
      type: "file";
      name: string;
      path: string;
    };

export interface KnowledgeIndex {
  sources: string[];
  tree: KnowledgeNode[];
}

export interface KnowledgeUploadResult {
  uploaded: string[];
  overwritten: string[];
  generation: number;
  knowledge_count: number;
}

export interface UiConfig {
  brand_title: string;
  brand_subtitle: string;
  welcome_title: string;
  welcome_description: string;
  suggestions: string[];
}

export const DEFAULT_UI_CONFIG: UiConfig = {
  brand_title: "DocumentX",
  brand_subtitle: "文档智能体 · 小文",
  welcome_title: "嗨，我是小文 👋",
  welcome_description:
    "DocumentX 的文档助手。我会基于你的知识库回答，也能按模板产出可下载的 PDF / Word / Markdown。",
  suggestions: [
    "总结一下知识库里的核心内容",
    "按对外API文档模板生成一份文档",
    "列出所有接口及其用途",
    "解释其中的认证与鉴权流程",
  ],
};

export interface ChatOptions {
  messages: ChatMessage[];
  useKnowledge: boolean;
  onDelta: (text: string) => void;
  onError: (msg: string) => void;
  onStatus?: (msg: string) => void;
  onUsage?: (usage: Usage) => void;
  signal?: AbortSignal;
}

/** 流式对话：读取 SSE，逐段回调 delta。 */
export async function streamChat(opts: ChatOptions): Promise<void> {
  const resp = await fetch(apiUrl("chat"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      messages: opts.messages,
      use_knowledge: opts.useKnowledge,
    }),
    signal: opts.signal,
  });

  if (!resp.ok || !resp.body) {
    opts.onError(`请求失败：HTTP ${resp.status}`);
    return;
  }

  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    // SSE 事件以空行分隔。
    let sep: number;
    while ((sep = buffer.indexOf("\n\n")) !== -1) {
      const rawEvent = buffer.slice(0, sep);
      buffer = buffer.slice(sep + 2);
      handleEvent(rawEvent, opts);
    }
  }
}

function handleEvent(rawEvent: string, opts: ChatOptions) {
  let event = "message";
  const dataLines: string[] = [];
  for (const line of rawEvent.split("\n")) {
    if (line.startsWith("event:")) event = line.slice(6).trim();
    else if (line.startsWith("data:")) dataLines.push(line.slice(5).trim());
  }
  const data = dataLines.join("\n");
  if (!data) return;

  if (event === "error") {
    opts.onError(data);
    return;
  }
  if (event === "status") {
    opts.onStatus?.(data);
    return;
  }
  if (event === "usage") {
    try {
      opts.onUsage?.(JSON.parse(data) as Usage);
    } catch {
      /* ignore */
    }
    return;
  }
  if (data === "[DONE]") return;

  try {
    const obj = JSON.parse(data);
    if (typeof obj.delta === "string") opts.onDelta(obj.delta);
  } catch {
    /* 忽略无法解析的心跳等 */
  }
}

export async function fetchTemplates(): Promise<string[]> {
  const r = await fetch(apiUrl("templates"));
  if (!r.ok) return [];
  const j = await r.json();
  return j.templates ?? [];
}

export async function fetchUiConfig(): Promise<UiConfig> {
  const r = await fetch(apiUrl("ui-config"));
  if (!r.ok) return DEFAULT_UI_CONFIG;
  const value = (await r.json()) as Partial<UiConfig>;
  return {
    ...DEFAULT_UI_CONFIG,
    ...value,
    suggestions: Array.isArray(value.suggestions)
      ? value.suggestions.filter((item): item is string => typeof item === "string")
      : DEFAULT_UI_CONFIG.suggestions,
  };
}

export async function fetchKnowledge(): Promise<KnowledgeIndex> {
  const r = await fetch(apiUrl("knowledge"));
  if (!r.ok) return { sources: [], tree: [] };
  const j = await r.json();
  const sources = Array.isArray(j.sources) ? j.sources : [];
  return {
    sources,
    tree: Array.isArray(j.tree)
      ? j.tree
      : sources.map((path: string) => {
          const parts = path.split("/");
          return {
            type: "file" as const,
            name: parts[parts.length - 1] || path,
            path,
          };
        }),
  };
}

export async function uploadKnowledge(
  files: File[],
  directory: string,
): Promise<KnowledgeUploadResult> {
  const body = new FormData();
  body.append("directory", directory.trim());
  for (const file of files) body.append("files", file, file.name);

  const response = await fetch(apiUrl("knowledge/upload"), {
    method: "POST",
    body,
  });
  const value = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(value.error ?? `HTTP ${response.status}`);
  return value as KnowledgeUploadResult;
}

/** 读取某个知识库文档的原文。 */
export async function fetchKnowledgeContent(source: string): Promise<string> {
  const url = new URL(apiUrl("knowledge/content"));
  url.searchParams.set("source", source);
  const r = await fetch(url);
  const j = await r.json();
  if (!r.ok) throw new Error(j.error ?? `HTTP ${r.status}`);
  return j.content ?? "";
}

/** 读取某个模板的内容。 */
export async function fetchTemplateContent(name: string): Promise<string> {
  const url = new URL(apiUrl("templates/content"));
  url.searchParams.set("name", name);
  const r = await fetch(url);
  const j = await r.json();
  if (!r.ok) throw new Error(j.error ?? `HTTP ${r.status}`);
  return j.content ?? "";
}

/** 触发浏览器下载一个 Blob 响应。 */
async function download(resp: Response, fallbackName: string) {
  if (!resp.ok) {
    let msg = `HTTP ${resp.status}`;
    try {
      const j = await resp.json();
      if (j.error) msg = j.error;
    } catch {
      /* ignore */
    }
    throw new Error(msg);
  }
  const blob = await resp.blob();
  const disp = resp.headers.get("Content-Disposition") ?? "";
  const m = disp.match(/filename="?([^"]+)"?/);
  const name = m ? m[1] : fallbackName;
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

export type ExportFormat = "md" | "pdf" | "docx";

/** 导出已有 Markdown 内容为 md/pdf/docx。 */
export async function exportContent(
  content: string,
  format: ExportFormat,
  title: string,
  diagrams: DiagramAsset[] = []
): Promise<void> {
  const resp = await fetch(apiUrl("export"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ content, format, title, diagrams }),
  });
  await download(resp, `document.${format}`);
}

/** 依据模板 + 知识库生成 Markdown；浏览器随后渲染图表并按目标格式导出。 */
export async function generateMarkdown(params: {
  instruction: string;
  template: string | null;
  useKnowledge: boolean;
  title: string;
}): Promise<string> {
  const resp = await fetch(apiUrl("generate"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      instruction: params.instruction,
      template: params.template,
      format: "md",
      use_knowledge: params.useKnowledge,
      title: params.title,
    }),
  });
  if (!resp.ok) {
    let message = `HTTP ${resp.status}`;
    try {
      const body = await resp.json();
      if (body.error) message = body.error;
    } catch {
      /* ignore */
    }
    throw new Error(message);
  }
  return resp.text();
}
