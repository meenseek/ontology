import { useEffect, useRef, useState } from "react";
import { ContextProvenance, OriginalText, contextUrl, editorContent, editorDraft, editorNewlines, failure, originalFilename } from "./Original";
import DocumentPreview, { resolveRelativeContextFilePath, resolveRelativeContextPath } from "./DocumentPreview";
import type { GraphNode } from "./graph";
import { nodePresentation } from "./presentation";

type Request = <T>(url: string, options?: RequestInit) => Promise<T>;
type Original = { metadata: { scope: string; path: string; source_path: string; revision: number; origin_kind: "native" | "imported-file"; source_digest: string | null; content_digest: string; byte_len: number }; content: string; title: string | null };
type History = { items: { revision: number; content_digest: string; byte_len: number; recorded_at: number; change_kind: "manual" | "core" | "import" }[]; next_before: number | null };
type Version = { revision: number; content_digest: string; content: string; title: string | null };
type EditReceipt = { revision: number; content_digest: string; changed: boolean };
const MAX_ORIGINAL_EDIT_BYTES = 1024 * 1024;
const url = (kind: "history" | "version", scope: string, path: string, number?: number) => {
  const params = new URLSearchParams({ scope, path });
  if (number !== undefined) params.set(kind === "history" ? "before" : "revision", String(number));
  return `/api/context/${kind}?${params}`;
};

export default function OriginalDetail({ scope, path, request, related = [], onNavigate, csrf, onBusy, onChange, onDirtyChange }: { scope: string; path: string; request: Request; related?: GraphNode[]; onNavigate?: (id: string) => void; csrf?: string; onBusy?: (busy: boolean) => void; onChange?: () => void; onDirtyChange?: (dirty: boolean) => void }) {
  const [original, setOriginal] = useState<Original | null>(null), [readError, setReadError] = useState("");
  const [history, setHistory] = useState<History | null>(null), [historyError, setHistoryError] = useState("");
  const [selected, setSelected] = useState<Version | null>(null), [versionError, setVersionError] = useState("");
  const [raw, setRaw] = useState(false), [loadingVersion, setLoadingVersion] = useState(false);
  const [editing, setEditing] = useState(false), [draft, setDraft] = useState(""), [baseline, setBaseline] = useState<Original | null>(null);
  const [saving, setSaving] = useState(false), [saveError, setSaveError] = useState(""), [saveNotice, setSaveNotice] = useState("");
  const versionRequest = useRef<AbortController | null>(null), olderRequest = useRef<AbortController | null>(null), historyRequest = useRef<AbortController | null>(null);
  const [refresh, setRefresh] = useState(0);
  const newlines = editorNewlines(baseline?.content ?? original?.content ?? "");
  const dirty = editing && baseline !== null && draft !== editorDraft(baseline.content);
  const editAllowed = !!csrf && scope !== "profile" && /\.(md|markdown)$/i.test(path) && newlines !== null && (original?.metadata.byte_len ?? 0) <= MAX_ORIGINAL_EDIT_BYTES;
  const draftBytes = new TextEncoder().encode(editorContent(draft, newlines ?? "lf")).length;
  useEffect(() => { onDirtyChange?.(dirty); return () => onDirtyChange?.(false); }, [dirty, onDirtyChange]);
  function reloadHistory() {
    historyRequest.current?.abort(); olderRequest.current?.abort();
    const controller = new AbortController(); historyRequest.current = controller;
    setHistoryError("");
    request<History>(url("history", scope, path), { signal: controller.signal }).then(value => {
      if (!controller.signal.aborted) setHistory(value);
    }).catch(error => { if (!controller.signal.aborted) setHistoryError(failure(error)); });
  }
  useEffect(() => {
    const controller = new AbortController();
    versionRequest.current?.abort(); olderRequest.current?.abort(); historyRequest.current?.abort();
    setOriginal(null); setReadError(""); setHistory(null); setHistoryError(""); setSelected(null); setVersionError(""); setRaw(false); setLoadingVersion(false); setEditing(false); setBaseline(null); setDraft(""); setSaveError(""); setSaveNotice("");
    request<Original>(contextUrl("read", scope, path), { signal: controller.signal }).then(value => {
      if (!controller.signal.aborted) setOriginal(value);
    }).catch(error => { if (!controller.signal.aborted) setReadError(failure(error, true)); });
    reloadHistory();
    return () => { controller.abort(); versionRequest.current?.abort(); olderRequest.current?.abort(); historyRequest.current?.abort(); };
  }, [scope, path, request, refresh]);
  async function older(before: number) {
    olderRequest.current?.abort();
    const controller = new AbortController(); olderRequest.current = controller;
    try {
      const page = await request<History>(url("history", scope, path, before), { signal: controller.signal });
      if (!controller.signal.aborted) setHistory(current => current ? { items: [...current.items, ...page.items], next_before: page.next_before } : page);
    } catch (error) { if (!controller.signal.aborted) setHistoryError(failure(error)); }
  }
  async function version(revision: number) {
    if (editing) return;
    versionRequest.current?.abort();
    const controller = new AbortController(); versionRequest.current = controller;
    setVersionError("");
    if (original?.metadata.revision === revision) { setSelected(null); setLoadingVersion(false); return; }
    setLoadingVersion(true);
    try {
      const value = await request<Version>(url("version", scope, path, revision), { signal: controller.signal });
      if (!controller.signal.aborted) setSelected(value);
    } catch (error) { if (!controller.signal.aborted) setVersionError(failure(error, true)); }
    finally { if (!controller.signal.aborted) setLoadingVersion(false); }
  }
  async function save() {
    if (!baseline || !csrf || !dirty || saving || newlines === null || draftBytes > MAX_ORIGINAL_EDIT_BYTES) return;
    const submitted = editorContent(draft, newlines);
    setSaving(true); onBusy?.(true); setSaveError(""); setSaveNotice("");
    let verified = false;
    try {
      const receipt = await request<EditReceipt>("/api/context/edit", { method: "POST", headers: { "content-type": "application/json", "x-csrf-token": csrf }, body: JSON.stringify({ scope, path, expected_revision: baseline.metadata.revision, expected_digest: baseline.metadata.content_digest, content: submitted }) });
      const current = await request<Original>(contextUrl("read", scope, path));
      if (current.metadata.revision !== receipt.revision || current.metadata.content_digest !== receipt.content_digest || current.content !== submitted) throw new Error("저장 후 원문 재조회가 일치하지 않습니다. 초안을 유지했습니다.");
      setOriginal(current); setSelected(null); setBaseline(null); setDraft(editorDraft(submitted)); setEditing(false); setSaveNotice(receipt.changed ? "원문을 저장하고 다시 확인했습니다." : "현재 원문과 같습니다.");
      onDirtyChange?.(false);
      reloadHistory();
      verified = true;
    } catch (error) {
      const status = error && typeof error === "object" && "status" in error ? error.status : null;
      setSaveError(status === 409 ? "원문이 바뀌었거나 반영 작업 중입니다. 초안은 유지했습니다. 현재 원문을 확인한 뒤 다시 저장하세요." : error instanceof Error && error.message.startsWith("저장 후") ? error.message : `${failure(error)} 초안은 유지했습니다.`);
    } finally { setSaving(false); onBusy?.(false); }
    if (verified) onChange?.();
  }
  useEffect(() => {
    if (!editing) return;
    const keydown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") { event.preventDefault(); void save(); }
    };
    document.addEventListener("keydown", keydown);
    return () => document.removeEventListener("keydown", keydown);
  }, [editing, draft, baseline, saving, csrf]);
  const showing = selected?.content ?? original?.content;
  const resolveInternalLink = (href: string) => {
    const target = resolveRelativeContextPath(path, href);
    const match = target && related.find(item => item.context_scope === scope && item.context_path === target);
    return match && onNavigate ? () => onNavigate(match.id) : undefined;
  };
  const resolveInternalDownload = (href: string) => {
    const target = resolveRelativeContextFilePath(path, href);
    return target && !/\.(md|markdown)$/i.test(target) ? contextUrl("download", scope, target) : undefined;
  };
  return <section className="documents original-detail" aria-label="원문과 이력">
    <h2>{(selected ? selected.title : original?.title) || originalFilename(path)}</h2>
    <p className="source-identity">{scope === "profile" ? "공통 운영 규칙" : "원문"} · {scope}/{path}</p>
    <p className="hint">{scope === "profile" ? "공통 운영 규칙은 검토된 변경 절차로 수정합니다." : "원문 저장과 조회는 내용을 검증하거나 규칙으로 승인한 뜻이 아닙니다."}</p>
    <a href={contextUrl("download", scope, path)} download={originalFilename(path)}>현재 원본 다운로드</a>
    {editAllowed && original && !editing && !selected && <button className="original-edit-button" onClick={() => { setBaseline(original); setDraft(editorDraft(original.content)); setEditing(true); setRaw(true); setSaveError(""); setSaveNotice(""); }}>원문 편집</button>}
    {original && newlines === null && /\.(md|markdown)$/i.test(path) && <p className="hint">줄바꿈 형식이 섞인 원문은 바이트 보존을 위해 CLI에서 확인하세요.</p>}
    {saveNotice && <p className="notice" role="status">{saveNotice}</p>}
    {saveError && <p className="error" role="alert">{saveError}</p>}
    {readError && <p className="error" role="alert">{readError} <button onClick={() => setRefresh(value => value + 1)}>다시 불러오기</button></p>}
    {!original && !readError && <p role="status">원문을 불러오는 중…</p>}
    {original && <><dl className="section"><dt>출처</dt><dd>{original.metadata.source_path}</dd><dt>크기</dt><dd>{original.metadata.byte_len.toLocaleString("ko-KR")} 바이트</dd></dl><details className="section"><summary>기술 정보</summary><dl><dt>현재 내용 SHA-256</dt><dd>{original.metadata.content_digest}</dd><ContextProvenance origin_kind={original.metadata.origin_kind} source_digest={original.metadata.source_digest} />{selected && <><dt>선택한 버전 SHA-256</dt><dd>{selected.content_digest}</dd></>}</dl></details></>}
    {historyError && <p className="error" role="alert">이력 확인 실패: {historyError} <button onClick={reloadHistory}>다시 확인</button></p>}
    {history && <section className="section" aria-label="원문 이력"><h3>원문 이력</h3><div className="compact-list">{history.items.map(item => <button key={item.revision} disabled={editing} aria-pressed={(selected?.revision ?? original?.metadata.revision) === item.revision} onClick={() => void version(item.revision)}>버전 {item.revision} · {item.change_kind === "manual" ? "직접 편집" : item.change_kind === "core" ? "검토된 변경" : "최초 보존"} · {new Date(item.recorded_at * 1000).toLocaleString("ko-KR")} · {item.byte_len.toLocaleString("ko-KR")} 바이트</button>)}</div>{history.next_before !== null && <button disabled={editing} onClick={() => void older(history.next_before!)}>이전 이력 더 보기</button>}</section>}
    {loadingVersion && <p role="status">버전 원문을 불러오는 중…</p>}
    {versionError && <p className="error" role="alert">{versionError}</p>}
    {selected && <p className="notice">버전 {selected.revision} 원문</p>}
    {editing && <section className="section original-editor" aria-label="원문 편집"><label htmlFor="original-draft">원문 전체 편집</label><textarea id="original-draft" value={draft} disabled={saving} onChange={event => setDraft(event.target.value)} spellCheck={false} rows={20} aria-describedby="original-draft-hint" /><p id="original-draft-hint" className="hint">{draftBytes.toLocaleString("ko-KR")} / {MAX_ORIGINAL_EDIT_BYTES.toLocaleString("ko-KR")} 바이트 · ⌘S 또는 Ctrl+S로 저장</p><div className="original-editor-actions"><button className="primary" disabled={!dirty || saving || draftBytes > MAX_ORIGINAL_EDIT_BYTES} onClick={() => void save()}>{saving ? "저장 중…" : "원문 저장"}</button><button disabled={saving} onClick={() => { if (!dirty || window.confirm("저장하지 않은 초안을 버릴까요?")) { setEditing(false); setBaseline(null); setSaveError(""); } }}>편집 취소</button></div></section>}
    {!editing && showing !== undefined && <>
      {/(\.md|\.markdown)$/i.test(path) && <div className="original-view-switch" role="group" aria-label="원문 보기 방식"><button aria-pressed={!raw} onClick={() => setRaw(false)}>읽기</button><button aria-pressed={raw} onClick={() => setRaw(true)}>원문 텍스트 보기</button></div>}
      {raw || !/(\.md|\.markdown)$/i.test(path) ? <OriginalText content={showing} /> : <DocumentPreview path={path} content={showing} kind="context" suppressGeneratedTitle resolveInternalLink={resolveInternalLink} resolveInternalDownload={resolveInternalDownload} />}
    </>}
    {related.length > 0 && <section className="section" aria-label="연결된 자료"><h3>명시적으로 연결된 자료</h3><div className="compact-list">{related.map(item => <button key={item.id} onClick={() => onNavigate?.(item.id)}><strong>{nodePresentation(item).title}</strong><small>{nodePresentation(item).subtitle}</small></button>)}</div></section>}
  </section>;
}
