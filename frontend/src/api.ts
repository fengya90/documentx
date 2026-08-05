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
  const resp = await fetch("/api/chat", {
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
  const r = await fetch("/api/templates");
  if (!r.ok) return [];
  const j = await r.json();
  return j.templates ?? [];
}

export async function fetchKnowledge(): Promise<string[]> {
  const r = await fetch("/api/knowledge");
  if (!r.ok) return [];
  const j = await r.json();
  return j.sources ?? [];
}

/** 读取某个知识库文档的原文。 */
export async function fetchKnowledgeContent(source: string): Promise<string> {
  const r = await fetch(`/api/knowledge/content?source=${encodeURIComponent(source)}`);
  const j = await r.json();
  if (!r.ok) throw new Error(j.error ?? `HTTP ${r.status}`);
  return j.content ?? "";
}

/** 读取某个模板的内容。 */
export async function fetchTemplateContent(name: string): Promise<string> {
  const r = await fetch(`/api/templates/content?name=${encodeURIComponent(name)}`);
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
  title: string
): Promise<void> {
  const resp = await fetch("/api/export", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ content, format, title }),
  });
  await download(resp, `document.${format}`);
}

/** 依据模板 + 知识库生成文档并下载。 */
export async function generateDocument(params: {
  instruction: string;
  template: string | null;
  format: ExportFormat;
  useKnowledge: boolean;
  title: string;
}): Promise<void> {
  const resp = await fetch("/api/generate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      instruction: params.instruction,
      template: params.template,
      format: params.format,
      use_knowledge: params.useKnowledge,
      title: params.title,
    }),
  });
  await download(resp, `document.${params.format}`);
}
