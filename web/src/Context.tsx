import { useEffect, useRef, useState } from "react";

export type ContextItem = { scope: string; path: string; source_path: string; content_digest: string; byte_len: number };
type ContextPage = { items: ContextItem[]; next_after: string | null; limit: number };
type ContextRead = { metadata: ContextItem & { material_id: string; revision: number; origin_kind: "imported-file" | "native"; source_digest: string | null }; content: string };
type Request = <T>(url: string, options?: RequestInit) => Promise<T>;
type Phase = "idle" | "loading" | "ready" | "error";
type Search = { scope: string; q: string; after: string | null };

export function contextUrl(kind: "search" | "read" | "download", scope: string, value: string, after: string | null = null): string {
  const params = new URLSearchParams({ scope });
  if (kind === "search") {
    params.set("q", value); params.set("limit", "20");
    if (after !== null) params.set("after", after);
  } else params.set("path", value);
  return `/api/context${kind === "search" ? "" : `/${kind}`}?${params}`;
}
export function originalFilename(path: string): string { return path.slice(path.lastIndexOf("/") + 1); }
export function OriginalText({ content }: { content: string }) {
  return <pre className="source-text context-text" aria-label="원본 텍스트">{content}</pre>;
}
export function failure(error: unknown, reading = false): string {
  const status = error && typeof error === "object" && "status" in error ? error.status : null;
  if (status === 403) return "세션을 확인할 수 없습니다. 페이지를 새로고침해 주세요.";
  if (status === 409) return "자료 반영 또는 복구가 진행 중입니다. 완료 후 다시 시도해 주세요.";
  if (status === 404) return "자료가 없거나 이 범위에서 열 수 없습니다.";
  if (status === 413) return reading ? "텍스트 보기 한도(1 MiB)를 넘었습니다. 원본 다운로드를 이용해 주세요." : "조회 또는 다운로드 한도를 넘었습니다.";
  if (status === 400 && reading) return "UTF-8 텍스트로 열 수 없는 자료입니다. 원본 다운로드를 이용해 주세요.";
  return "요청을 완료하지 못했습니다. 다시 시도해 주세요.";
}

export function ContextProvenance({ origin_kind, source_digest }: { origin_kind: ContextRead["metadata"]["origin_kind"]; source_digest: string | null }) {
  return <><dt>{origin_kind === "native" ? "출처" : "출처 SHA-256"}</dt><dd>{origin_kind === "native" ? "온톨로지에서 작성" : source_digest}</dd></>;
}

export default function Context({ request, onClose }: { request: Request; onClose: () => void }) {
  const [scopes, setScopes] = useState<string[]>([]), [scopePhase, setScopePhase] = useState<Phase>("loading");
  const [scopeError, setScopeError] = useState(""), [scopeRetry, setScopeRetry] = useState(0);
  const [scope, setScope] = useState(""), [query, setQuery] = useState("");
  const [page, setPage] = useState<ContextPage | null>(null), [pagePhase, setPagePhase] = useState<Phase>("idle"), [pageError, setPageError] = useState("");
  const [selected, setSelected] = useState<ContextItem | null>(null), [read, setRead] = useState<ContextRead | null>(null);
  const [readPhase, setReadPhase] = useState<Phase>("idle"), [readError, setReadError] = useState("");
  const [downloadPhase, setDownloadPhase] = useState<Phase>("idle"), [downloadError, setDownloadError] = useState("");
  const listRequest = useRef<AbortController | null>(null), readRequest = useRef<AbortController | null>(null), downloadRequest = useRef<AbortController | null>(null);
  const lastSearch = useRef<Search | null>(null);
  const panel = useRef<HTMLElement>(null), close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const controller = new AbortController(); setScopePhase("loading"); setScopeError("");
    request<{ scopes: string[] }>("/api/context/scopes", { signal: controller.signal }).then(value => {
      if (!controller.signal.aborted) { setScopes(value.scopes); setScopePhase("ready"); }
    }).catch(error => { if (!controller.signal.aborted) { setScopeError(failure(error)); setScopePhase("error"); } });
    return () => controller.abort();
  }, [request, scopeRetry]);
  useEffect(() => {
    close.current?.focus();
    return () => { listRequest.current?.abort(); readRequest.current?.abort(); downloadRequest.current?.abort(); };
  }, []);
  function clearSelection() {
    readRequest.current?.abort(); downloadRequest.current?.abort();
    setSelected(null); setRead(null); setReadPhase("idle"); setReadError(""); setDownloadPhase("idle"); setDownloadError("");
  }
  function loadPage(search: Search) {
    listRequest.current?.abort(); clearSelection();
    lastSearch.current = search; setPage(null); setPageError("");
    if (new TextEncoder().encode(search.q).byteLength > 240) {
      setPageError("검색어는 UTF-8 240바이트 이내로 입력해 주세요."); setPagePhase("error"); return;
    }
    const controller = new AbortController(); listRequest.current = controller; setPagePhase("loading");
    request<ContextPage>(contextUrl("search", search.scope, search.q, search.after), { signal: controller.signal }).then(value => {
      if (!controller.signal.aborted) { setPage(value); setPagePhase("ready"); }
    }).catch(error => { if (!controller.signal.aborted) { setPageError(failure(error)); setPagePhase("error"); } });
  }
  function changeScope(next: string) {
    listRequest.current?.abort(); clearSelection(); lastSearch.current = null;
    setScope(next); setQuery(""); setPage(null); setPageError(""); setPagePhase("idle");
    if (next) loadPage({ scope: next, q: "", after: null });
  }
  function loadRead(item: ContextItem) {
    readRequest.current?.abort(); downloadRequest.current?.abort();
    const controller = new AbortController(); readRequest.current = controller;
    setSelected(item); setRead(null); setReadError(""); setReadPhase("loading"); setDownloadPhase("idle"); setDownloadError("");
    request<ContextRead>(contextUrl("read", item.scope, item.path), { signal: controller.signal }).then(value => {
      if (!controller.signal.aborted) { setRead(value); setReadPhase("ready"); }
    }).catch(error => { if (!controller.signal.aborted) { setReadError(failure(error, true)); setReadPhase("error"); } });
  }
  async function download(item: ContextItem) {
    downloadRequest.current?.abort();
    const controller = new AbortController(); downloadRequest.current = controller;
    setDownloadPhase("loading"); setDownloadError("");
    try {
      const response = await fetch(contextUrl("download", item.scope, item.path), { credentials: "same-origin", signal: controller.signal });
      if (!response.ok) throw { status: response.status };
      const blob = await response.blob();
      if (controller.signal.aborted) return;
      const url = URL.createObjectURL(blob), anchor = document.createElement("a");
      anchor.href = url; anchor.download = originalFilename(item.path); anchor.hidden = true;
      document.body.append(anchor); anchor.click(); anchor.remove();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
      setDownloadPhase("ready");
    } catch (error) {
      if (!controller.signal.aborted) { setDownloadError(failure(error)); setDownloadPhase("error"); }
    }
  }
  return <aside ref={panel} className="management-panel context-panel" role="dialog" aria-modal="true" aria-labelledby="context-title" onKeyDown={event => {
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); }
    if (event.key !== "Tab") return;
    const controls = [...(panel.current?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), a[href]') ?? [])].filter(element => element.getClientRects().length > 0);
    const first = controls[0], last = controls.at(-1);
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  }}>
    <div className="panel-heading"><h2 id="context-title">자료 보관함</h2><button ref={close} onClick={onClose} aria-label="자료 보관함 닫기">닫기 ×</button></div>
    <p className="hint">저장된 원본 자료입니다. 검증된 지식이나 현재 적용할 정책을 뜻하지 않습니다.</p>
    <label className="topic-label" htmlFor="context-scope">자료 범위</label>
    <select id="context-scope" value={scope} disabled={scopePhase !== "ready" || !scopes.length} onChange={event => changeScope(event.target.value)}>
      <option value="">범위를 선택하세요</option>{scopes.map(value => <option key={value} value={value}>{value}</option>)}
    </select>
    {scopePhase === "loading" && <p className="loading" role="status">자료 범위를 불러오는 중…</p>}
    {scopeError && <p className="error" role="alert">{scopeError} <button onClick={() => setScopeRetry(value => value + 1)}>범위 다시 불러오기</button></p>}
    {scopePhase === "ready" && !scopes.length && <p className="empty">열 수 있는 원본 자료가 없습니다.</p>}
    {!scope && scopes.length > 0 && <p className="hint">범위를 선택한 뒤 목록에서 자료를 선택하면 원문을 읽습니다.</p>}
    <form className="context-search" role="search" aria-label="원본 자료 검색" onSubmit={event => { event.preventDefault(); if (scope) loadPage({ scope, q: query.trim(), after: null }); }}>
      <label className="sr-only" htmlFor="context-query">선택한 자료 범위 검색</label>
      <input id="context-query" disabled={!scope} value={query} maxLength={240} onChange={event => setQuery(event.target.value)} placeholder="경로·내용 검색" />
      <button type="submit" disabled={!scope || pagePhase === "loading"}>검색</button>
    </form>
    <section aria-label="원본 자료 목록" aria-busy={pagePhase === "loading"}>
      {pagePhase === "loading" && <p className="loading" role="status">자료 목록을 불러오는 중…</p>}
      {pageError && <p className="error" role="alert">{pageError} <button onClick={() => { if (lastSearch.current) loadPage(lastSearch.current); }}>목록 다시 불러오기</button></p>}
      {page && !page.items.length && <p className="empty">일치하는 원본 자료가 없습니다.</p>}
      {page && page.items.length > 0 && <><p className="hint">이번 목록 {page.items.length}개 · 원문은 선택할 때 불러옵니다.</p><ul className="context-list">{page.items.map(item => <li key={item.path}><button aria-pressed={selected?.path === item.path} onClick={() => { if (selected?.path !== item.path) loadRead(item); }}><strong>{item.path}</strong><small>{item.byte_len.toLocaleString("ko-KR")} 바이트</small></button></li>)}</ul></>}
      {page && <div className="context-pagination">{lastSearch.current?.after && <button onClick={() => { if (lastSearch.current) loadPage({ ...lastSearch.current, after: null }); }}>처음 목록</button>}{page.next_after !== null && <button onClick={() => { if (lastSearch.current) loadPage({ ...lastSearch.current, after: page.next_after }); }}>다음 20개</button>}</div>}
    </section>
    {selected && <section className="section context-detail" aria-label="선택한 원본 자료" aria-busy={readPhase === "loading"}>
      <h3>{selected.path}</h3><p className="source-identity">원본 자료 · {selected.source_path}</p>
      <button disabled={downloadPhase === "loading"} onClick={() => void download(selected)}>{downloadPhase === "loading" ? "다운로드 준비 중…" : downloadPhase === "error" ? "원본 다운로드 다시 시도" : "원본 다운로드"}</button>
      {downloadError && <p className="error" role="alert">{downloadError}</p>}
      {downloadPhase === "ready" && <p className="hint" role="status">브라우저에 원본 다운로드를 요청했습니다.</p>}
      {readPhase === "loading" && <p className="loading" role="status">선택한 원문을 불러오는 중…</p>}
      {readError && <p className="error" role="alert">{readError} <button onClick={() => loadRead(selected)}>원문 다시 불러오기</button></p>}
      {read && <><dl><dt>크기</dt><dd>{read.metadata.byte_len.toLocaleString("ko-KR")} 바이트</dd><dt>내용 SHA-256</dt><dd>{read.metadata.content_digest}</dd><ContextProvenance origin_kind={read.metadata.origin_kind} source_digest={read.metadata.source_digest} /></dl>{read.content === "" && <p className="hint">빈 원본 파일입니다.</p>}<OriginalText content={read.content} /></>}
    </section>}
  </aside>;
}
