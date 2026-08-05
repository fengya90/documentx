import { useEffect, useLayoutEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  ChatMessage,
  streamChat,
  fetchTemplates,
  fetchKnowledge,
  fetchKnowledgeContent,
  fetchTemplateContent,
  exportContent,
  generateDocument,
} from "./api";

type Viewer = { title: string; kind: "知识库" | "模板"; content: string };
type Theme = "light" | "dark";

const EXAMPLES = [
  "总结一下知识库里的核心内容",
  "按对外API文档模板生成一份文档",
  "列出所有接口及其用途",
  "解释其中的认证与鉴权流程",
];

/* ---------- 图标（内联 SVG，无依赖） ---------- */
const Icon = {
  Send: () => (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m22 2-7 20-4-9-9-4Z" /><path d="M22 2 11 13" /></svg>
  ),
  Stop: () => (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="2" /></svg>
  ),
  Plus: () => (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M12 5v14M5 12h14" /></svg>
  ),
  Doc: () => (
    <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8Z" /><path d="M14 2v6h6" /></svg>
  ),
  Template: () => (
    <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" /><path d="M3 9h18M9 21V9" /></svg>
  ),
  Copy: () => (
    <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="9" y="9" width="13" height="13" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></svg>
  ),
  Sun: () => (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" /></svg>
  ),
  Moon: () => (
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" /></svg>
  ),
  Sparkle: () => (
    <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor"><path d="M12 2l1.9 5.6L19.5 9l-5.6 1.9L12 16l-1.9-5.1L4.5 9l5.6-1.4Z" /></svg>
  ),
};

/** 智能体吉祥物「小文」：一个可爱的紫发女孩（纯 SVG，矢量、随主题缩放）。 */
function Mascot({ size = 32 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" fill="none" aria-hidden>
      <defs>
        <linearGradient id="mx-bg" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#efe9ff" />
          <stop offset="1" stopColor="#e2d6ff" />
        </linearGradient>
        <linearGradient id="mx-hair" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#8f7dff" />
          <stop offset="1" stopColor="#6a54e8" />
        </linearGradient>
      </defs>
      <circle cx="32" cy="32" r="32" fill="url(#mx-bg)" />
      {/* 后发 */}
      <ellipse cx="32" cy="31" rx="20.5" ry="21" fill="url(#mx-hair)" />
      {/* 两侧发绺 */}
      <path d="M12.5 30 q-2 12 3.5 20 q3.2 -3 3 -9.5 q-4.5 -5.5 -3.2 -11.5 z" fill="url(#mx-hair)" />
      <path d="M51.5 30 q2 12 -3.5 20 q-3.2 -3 -3 -9.5 q4.5 -5.5 3.2 -11.5 z" fill="url(#mx-hair)" />
      {/* 脸 */}
      <ellipse cx="32" cy="35" rx="14.5" ry="14" fill="#ffe3d2" />
      {/* 腮红 */}
      <ellipse cx="22.5" cy="39.5" rx="3.2" ry="1.9" fill="#ff9db3" opacity="0.6" />
      <ellipse cx="41.5" cy="39.5" rx="3.2" ry="1.9" fill="#ff9db3" opacity="0.6" />
      {/* 刘海 */}
      <path
        d="M17 33 C17 22 23 17.5 32 17.5 C41 17.5 47 22 47 33 C44 27.5 40 27 37 30.2 C35 26 30 26 27.5 30.2 C24 27 20 27.5 17 33 Z"
        fill="url(#mx-hair)"
      />
      {/* 呆毛 */}
      <path d="M31 12 C30 6.5 36.5 5 39.5 8.2 C36.5 8.2 34 10.5 34.6 14.2 Z" fill="url(#mx-hair)" />
      {/* 眼睛 */}
      <ellipse cx="26" cy="35.6" rx="2.7" ry="3.6" fill="#3b3557" />
      <ellipse cx="38" cy="35.6" rx="2.7" ry="3.6" fill="#3b3557" />
      <circle cx="27" cy="34.2" r="1.05" fill="#fff" />
      <circle cx="39" cy="34.2" r="1.05" fill="#fff" />
      {/* 微笑 */}
      <path d="M28.8 41.4 Q32 44.6 35.2 41.4" stroke="#e0728a" strokeWidth="1.7" fill="none" strokeLinecap="round" />
      {/* 发夹（小星星） */}
      <path
        d="M20.8 20.4 l0.9 1.95 2.15 0.2 -1.62 1.44 0.5 2.05 -1.93 -1.08 -1.93 1.08 0.5 -2.05 -1.62 -1.44 2.15 -0.2 z"
        fill="#ffd166"
      />
    </svg>
  );
}

/** 取第一个标题作为导出文件标题。 */
function deriveTitle(md: string): string {
  for (const line of md.split("\n")) {
    const m = line.match(/^#{1,3}\s+(.+)/);
    if (m) return m[1].trim();
  }
  return "文档";
}

/** 导出前去掉正文前的对话式引言：从第一个标题开始截取。 */
function stripPreamble(md: string): string {
  const lines = md.split("\n");
  const idx = lines.findIndex((l) => /^#{1,6}\s+\S/.test(l));
  return idx > 0 ? lines.slice(idx).join("\n") : md;
}

export default function App() {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [useKnowledge, setUseKnowledge] = useState(true);
  const [templates, setTemplates] = useState<string[]>([]);
  const [sources, setSources] = useState<string[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [theme, setTheme] = useState<Theme>(
    () =>
      (localStorage.getItem("dx-theme") as Theme) ||
      (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light")
  );

  const [viewer, setViewer] = useState<Viewer | null>(null);
  const [viewerRaw, setViewerRaw] = useState(false);

  const [genOpen, setGenOpen] = useState(false);
  const [genInstruction, setGenInstruction] = useState("");
  const [genTemplate, setGenTemplate] = useState<string>("");
  const [genTitle, setGenTitle] = useState("");
  const [genBusy, setGenBusy] = useState(false);

  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    fetchTemplates().then(setTemplates);
    fetchKnowledge().then(setSources);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("dx-theme", theme);
  }, [theme]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
  }, [messages]);

  // 输入框自动增高
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = Math.min(el.scrollHeight, 200) + "px";
  }, [input]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setViewer(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const flash = (msg: string) => {
    setNotice(msg);
    setTimeout(() => setNotice(null), 3500);
  };

  async function openKnowledge(source: string) {
    try {
      const content = await fetchKnowledgeContent(source);
      setViewerRaw(false);
      setViewer({ title: source, kind: "知识库", content });
    } catch (e: any) {
      flash(`打开失败：${e.message ?? e}`);
    }
  }

  async function openTemplate(name: string) {
    try {
      const content = await fetchTemplateContent(name);
      setViewerRaw(false);
      setViewer({ title: name, kind: "模板", content });
    } catch (e: any) {
      flash(`打开失败：${e.message ?? e}`);
    }
  }

  async function send(text?: string) {
    const q = (text ?? input).trim();
    if (!q || busy) return;
    setInput("");

    const next: ChatMessage[] = [...messages, { role: "user", content: q }];
    setMessages([...next, { role: "assistant", content: "" }]);
    setBusy(true);

    const ctrl = new AbortController();
    abortRef.current = ctrl;

    try {
      await streamChat({
        messages: next,
        useKnowledge,
        signal: ctrl.signal,
        onDelta: (delta) => {
          setMessages((cur) => {
            const copy = [...cur];
            const last = copy[copy.length - 1];
            copy[copy.length - 1] = { ...last, role: "assistant", content: last.content + delta };
            return copy;
          });
        },
        onError: (msg) => {
          setMessages((cur) => {
            const copy = [...cur];
            const last = copy[copy.length - 1];
            copy[copy.length - 1] = {
              ...last,
              role: "assistant",
              content: (last.content || "") + `\n\n> ⚠️ ${msg}`,
            };
            return copy;
          });
        },
        onStatus: (msg) => flash(msg),
        onUsage: (usage) => {
          setMessages((cur) => {
            const copy = [...cur];
            const last = copy[copy.length - 1];
            if (last?.role === "assistant") copy[copy.length - 1] = { ...last, tokens: usage.total_tokens };
            return copy;
          });
        },
      });
    } catch {
      /* 被 stop() 中断，保留已生成内容 */
    }
    abortRef.current = null;
    setBusy(false);
  }

  function stop() {
    abortRef.current?.abort();
    abortRef.current = null;
    setBusy(false);
  }

  function newChat() {
    if (busy) stop();
    setMessages([]);
    inputRef.current?.focus();
  }

  async function copyMessage(content: string) {
    try {
      await navigator.clipboard.writeText(content);
      flash("已复制到剪贴板");
    } catch {
      flash("复制失败");
    }
  }

  async function exportMessage(content: string, format: "md" | "pdf" | "docx") {
    try {
      const doc = stripPreamble(content);
      await exportContent(doc, format, deriveTitle(doc));
    } catch (e: any) {
      flash(`导出失败：${e.message ?? e}`);
    }
  }

  async function runGenerate(format: "md" | "pdf" | "docx") {
    if (!genInstruction.trim()) {
      flash("请填写生成要求");
      return;
    }
    setGenBusy(true);
    try {
      await generateDocument({
        instruction: genInstruction,
        template: genTemplate || null,
        format,
        useKnowledge,
        title: genTitle || "文档",
      });
    } catch (e: any) {
      flash(`生成失败：${e.message ?? e}`);
    }
    setGenBusy(false);
  }

  return (
    <div className="app">
      {/* ---------- 侧栏 ---------- */}
      <aside className="sidebar">
        <div className="brand">
          <span className="logo">
            <Mascot size={38} />
          </span>
          <div className="brand-txt">
            <h1>DocumentX</h1>
            <p>文档智能体 · 小文</p>
          </div>
          <button
            className="icon-btn theme-toggle"
            title={theme === "dark" ? "切换到浅色" : "切换到深色"}
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          >
            {theme === "dark" ? <Icon.Sun /> : <Icon.Moon />}
          </button>
        </div>

        <button className="btn primary block" onClick={newChat}>
          <Icon.Plus /> 新对话
        </button>

        <button className={`btn block ${genOpen ? "active" : ""}`} onClick={() => setGenOpen((v) => !v)}>
          <Icon.Doc /> 按模板生成文档
        </button>

        <label className="switch">
          <input type="checkbox" checked={useKnowledge} onChange={(e) => setUseKnowledge(e.target.checked)} />
          <span className="track"><span className="thumb" /></span>
          <span className="switch-label">使用知识库</span>
        </label>

        <section className="panel scroll">
          <h2>知识库 <span className="count">{sources.length}</span></h2>
          <ul className="list">
            {sources.length === 0 && <li className="muted">knowledge/ 为空</li>}
            {sources.map((s) => (
              <li key={s} title={`查看 ${s}`} onClick={() => openKnowledge(s)}>
                <Icon.Doc />
                <span className="ellip">{s}</span>
              </li>
            ))}
          </ul>
        </section>

        <section className="panel scroll">
          <h2>模板 <span className="count">{templates.length}</span></h2>
          <ul className="list">
            {templates.length === 0 && <li className="muted">templates/ 为空</li>}
            {templates.map((t) => (
              <li key={t} title={`查看 ${t}`} onClick={() => openTemplate(t)}>
                <Icon.Template />
                <span className="ellip">{t}</span>
              </li>
            ))}
          </ul>
        </section>
      </aside>

      {/* ---------- 主区 ---------- */}
      <main className="main">
        {genOpen && (
          <div className="gen-panel">
            <div className="gen-head">
              <Icon.Doc /> 按模板生成文档
            </div>
            <div className="gen-row">
              <input
                className="field"
                placeholder="文档标题（可选）"
                value={genTitle}
                onChange={(e) => setGenTitle(e.target.value)}
              />
              <select className="field" value={genTemplate} onChange={(e) => setGenTemplate(e.target.value)}>
                <option value="">不使用模板</option>
                {templates.map((t) => (
                  <option key={t} value={t}>{t}</option>
                ))}
              </select>
            </div>
            <textarea
              className="field area"
              placeholder="描述要生成什么文档，例如：为用户管理模块生成一份对外 API 文档"
              value={genInstruction}
              onChange={(e) => setGenInstruction(e.target.value)}
            />
            <div className="gen-actions">
              {genBusy && <span className="muted spin-hint">生成中…</span>}
              <button className="btn" disabled={genBusy} onClick={() => runGenerate("md")}>Markdown</button>
              <button className="btn" disabled={genBusy} onClick={() => runGenerate("docx")}>Word</button>
              <button className="btn primary" disabled={genBusy} onClick={() => runGenerate("pdf")}>PDF</button>
            </div>
          </div>
        )}

        <div className="messages" ref={scrollRef}>
          <div className="thread">
            {messages.length === 0 ? (
              <div className="empty">
                <div className="empty-badge"><Mascot size={84} /></div>
                <h2>嗨，我是小文 👋</h2>
                <p>DocumentX 的文档助手。我会基于你的知识库回答，也能按模板产出可下载的 PDF / Word / Markdown。</p>
                <div className="chips">
                  {EXAMPLES.map((ex) => (
                    <button key={ex} className="chip" onClick={() => send(ex)}>{ex}</button>
                  ))}
                </div>
              </div>
            ) : (
              messages.map((m, i) => {
                const streaming = busy && i === messages.length - 1 && m.role === "assistant";
                const showActions = m.role === "assistant" && !!m.content && !streaming;
                return (
                  <div key={i} className={`msg ${m.role}`}>
                    <div className="avatar">{m.role === "user" ? "你" : <Mascot size={32} />}</div>
                    <div className="col">
                      <div className="bubble">
                        {m.role === "assistant" ? (
                          m.content ? (
                            <div className="prose">
                              <ReactMarkdown remarkPlugins={[remarkGfm]}>{m.content}</ReactMarkdown>
                              {streaming && <span className="caret" />}
                            </div>
                          ) : (
                            <div className="typing"><span /><span /><span /></div>
                          )
                        ) : (
                          <div className="plain">{m.content}</div>
                        )}
                      </div>
                      {showActions && (
                        <div className="actions">
                          <button className="ghost-btn" onClick={() => copyMessage(m.content)}>
                            <Icon.Copy /> 复制
                          </button>
                          <span className="sep" />
                          <span className="lbl">导出</span>
                          <button className="ghost-btn" onClick={() => exportMessage(m.content, "md")}>Markdown</button>
                          <button className="ghost-btn" onClick={() => exportMessage(m.content, "pdf")}>PDF</button>
                          <button className="ghost-btn" onClick={() => exportMessage(m.content, "docx")}>Word</button>
                          {m.tokens ? <span className="tokens">{m.tokens.toLocaleString()} tokens</span> : null}
                        </div>
                      )}
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </div>

        <div className="composer-wrap">
          <div className="composer">
            <textarea
              ref={inputRef}
              rows={1}
              value={input}
              placeholder="输入你的问题…"
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  send();
                }
              }}
            />
            {busy ? (
              <button className="send stop" onClick={stop} title="停止生成">
                <Icon.Stop />
              </button>
            ) : (
              <button className="send" onClick={() => send()} disabled={!input.trim()} title="发送">
                <Icon.Send />
              </button>
            )}
          </div>
          <div className="composer-hint">
            {useKnowledge ? "已启用知识库" : "未启用知识库"} · Enter 发送，Shift+Enter 换行
          </div>
        </div>
      </main>

      {/* ---------- 查看弹窗 ---------- */}
      {viewer && (
        <div className="modal-backdrop" onClick={() => setViewer(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <div className="modal-title">
                <span className="tag">{viewer.kind}</span>
                <span className="ellip" title={viewer.title}>{viewer.title}</span>
              </div>
              <div className="modal-actions">
                <button className="btn small" onClick={() => copyMessage(viewer.content)}>复制</button>
                <button className="btn small" onClick={() => setViewerRaw((v) => !v)}>
                  {viewerRaw ? "渲染" : "原文"}
                </button>
                <button className="btn small" onClick={() => setViewer(null)}>关闭</button>
              </div>
            </div>
            <div className="modal-body">
              {viewerRaw ? (
                <pre className="raw">{viewer.content}</pre>
              ) : (
                <div className="prose">
                  <ReactMarkdown remarkPlugins={[remarkGfm]}>{viewer.content}</ReactMarkdown>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {notice && <div className="notice">{notice}</div>}
    </div>
  );
}
