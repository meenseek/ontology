import { useCallback, useEffect, useRef, useState } from "react";
import { OriginalText, contextUrl, editorContent, editorDraft, editorNewlines, failure, originalFilename, type ContextItem } from "./Original";
import DocumentPreview, { resolveRelativeContextFilePath, resolveRelativeContextPath } from "./DocumentPreview";
import type { GraphNode, Request } from "./graph";
import { nodePresentation } from "./presentation";
import { attachmentMediaKind } from "./AttachmentPreview";

type Original = { metadata: ContextItem & { revision: number; origin_kind: "native" | "imported-file"; source_digest: string | null }; content: string; title: string | null };
type History = { items: { revision: number; content_digest: string; byte_len: number; recorded_at: number; change_kind: "manual" | "core" | "import" }[]; next_before: number | null };
type Version = { revision: number; content_digest: string; content: string; title: string | null };
type EditReceipt = { revision: number; content_digest: string; changed: boolean };
const MAX_ORIGINAL_EDIT_BYTES = 1024 * 1024;
const noRelated: GraphNode[] = [];
// Match only returned outgoing reference targets; this grants no new read scope.
function referenceTarget(scope: string, path: string, href: string): string | null {
  const raw = href.split("#", 1)[0];
  if (!raw || raw.includes("?") || raw.split("/")[0].includes(":")) return null;
  let decoded: string;
  try { decoded = decodeURIComponent(raw); } catch { return null; }
  if (decoded.startsWith("/") || /[\\\u0000-\u001f\u007f]/.test(decoded)) return null;
  const parts = `${scope}/${path}`.split("/"); parts.pop();
  for (const part of decoded.split("/")) {
    if (!part) return null;
    if (part === ".") continue;
    if (part === "..") { if (parts.pop() === undefined) return null; }
    else parts.push(part);
  }
  return parts.join("/");
}
const url = (kind: "history" | "version", scope: string, path: string, number?: number) => {
  const params = new URLSearchParams({ scope, path });
  if (number !== undefined) params.set(kind === "history" ? "before" : "revision", String(number));
  return `/api/context/${kind}?${params}`;
};

export default function OriginalDetail({ scope, path, request, related = noRelated, references = noRelated, onNavigate, csrf, onBusy, onChange, onDirtyChange }: { scope: string; path: string; request: Request; related?: GraphNode[]; references?: GraphNode[]; onNavigate?: (id: string) => void; csrf?: string; onBusy?: (busy: boolean) => void; onChange?: () => void; onDirtyChange?: (dirty: boolean) => void }) {
  const [original, setOriginal] = useState<Original | null>(null), [readError, setReadError] = useState("");
  const [history, setHistory] = useState<History | null>(null), [historyError, setHistoryError] = useState("");
  const [selected, setSelected] = useState<Version | null>(null), [versionError, setVersionError] = useState("");
  const [loadingVersion, setLoadingVersion] = useState(false);
  const [editing, setEditing] = useState(false), [draft, setDraft] = useState(""), [baseline, setBaseline] = useState<Original | null>(null);
  const [saving, setSaving] = useState(false), [saveError, setSaveError] = useState(""), [saveNotice, setSaveNotice] = useState("");
  const versionRequest = useRef<AbortController | null>(null), olderRequest = useRef<AbortController | null>(null), historyRequest = useRef<AbortController | null>(null);
  const [refresh, setRefresh] = useState(0);
  const newlines = editorNewlines(baseline?.content ?? original?.content ?? "");
  const dirty = baseline !== null && draft !== editorDraft(baseline.content);
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
    setOriginal(null); setReadError(""); setHistory(null); setHistoryError(""); setSelected(null); setVersionError(""); setLoadingVersion(false); setEditing(false); setBaseline(null); setDraft(""); setSaveError(""); setSaveNotice("");
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
    if (baseline) return;
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
  function edit() {
    if (!original || !editAllowed || selected || loadingVersion || saving) return;
    if (!baseline) { setBaseline(original); setDraft(editorDraft(original.content)); setSaveError(""); }
    setEditing(true); setSaveNotice("");
  }
  function read() {
    if (saving) return;
    setEditing(false);
    if (!dirty) { setBaseline(null); setSaveError(""); }
  }
  function cancel() {
    if (saving || dirty && !window.confirm("저장하지 않은 변경을 버릴까요?")) return;
    setEditing(false); setBaseline(null); setDraft(""); setSaveError("");
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
  const title = (selected ? selected.title : original?.title) || originalFilename(path);
  const markdown = /(\.md|\.markdown)$/i.test(path);
  const resolveInternalLink = useCallback((href: string) => {
    const target = resolveRelativeContextPath(path, href);
    const canonical = referenceTarget(scope, path, href);
    const match = references.find(item => canonical === `${item.context_scope}/${item.context_path}`)
      ?? (target && related.find(item => item.context_scope === scope && item.context_path === target));
    return match && onNavigate ? () => onNavigate(match.id) : undefined;
  }, [path, related, references, scope, onNavigate]);
  const resolveInternalDownload = useCallback((href: string) => {
    const target = resolveRelativeContextFilePath(path, href);
    return target && !/\.(md|markdown)$/i.test(target) ? contextUrl("download", scope, target) : undefined;
  }, [path, scope]);
  const resolveInternalMedia = useCallback((href: string) => {
    // Historical text does not imply a historical attachment revision.
    if (selected) return;
    const target = resolveRelativeContextFilePath(path, href);
    const kind = target && attachmentMediaKind(target);
    return target && kind ? { kind, url: contextUrl("preview", scope, target) } : undefined;
  }, [path, scope, selected]);
  const controls = <>
    {scope === "profile" && <p className="hint">공통 운영 규칙은 검토된 변경 절차로 수정합니다.</p>}
    <a href={contextUrl("download", scope, path)} download={originalFilename(path)}>현재 원본 다운로드</a>
    {markdown && showing !== undefined && <div className="original-view-switch" role="group" aria-label="문서 보기 방식"><button aria-pressed={!editing} disabled={saving} onClick={read}>읽기</button><button aria-pressed={editing} disabled={!editAllowed || !!selected || loadingVersion || saving} onClick={edit}>편집</button></div>}
    {original && newlines === null && /\.(md|markdown)$/i.test(path) && <p className="hint">줄바꿈 형식이 섞인 원문은 바이트 보존을 위해 CLI에서 확인하세요.</p>}
    {original && original.metadata.byte_len > MAX_ORIGINAL_EDIT_BYTES && markdown && <p className="hint">이 원문은 앱에서 편집할 수 있는 크기를 넘었습니다. 원본 다운로드로 확인하세요.</p>}
    {!editing && dirty && <p className="notice" role="status">저장하지 않은 변경이 있습니다. 편집으로 돌아가 이어서 작성할 수 있습니다.</p>}
    {saveNotice && <p className="notice" role="status">{saveNotice}</p>}
    {saveError && <p className="error" role="alert">{saveError}</p>}
    {readError && <p className="error" role="alert">{readError} <button onClick={() => setRefresh(value => value + 1)}>다시 불러오기</button></p>}
    {!original && !readError && <p role="status">원문을 불러오는 중…</p>}
    {loadingVersion && <p role="status">버전 원문을 불러오는 중…</p>}
    {versionError && <p className="error" role="alert">{versionError}</p>}
    {selected && <p className="notice">버전 {selected.revision} 원문 · 이전 버전은 읽기만 가능합니다. 첨부 링크는 현재 원본을 다운로드합니다.</p>}
  </>;
  const editor = <section className="section original-editor" aria-label="원문 편집"><label htmlFor="original-draft">원문 전체</label><textarea id="original-draft" value={draft} disabled={saving} onChange={event => setDraft(event.target.value)} spellCheck={false} rows={20} aria-describedby="original-draft-hint" /><p id="original-draft-hint" className="hint">{draftBytes.toLocaleString("ko-KR")} / {MAX_ORIGINAL_EDIT_BYTES.toLocaleString("ko-KR")} 바이트 · ⌘S 또는 Ctrl+S로 저장</p><div className="original-editor-actions"><button className="primary" disabled={!dirty || saving || draftBytes > MAX_ORIGINAL_EDIT_BYTES} onClick={() => void save()}>{saving ? "저장 중…" : "저장"}</button><button disabled={saving} onClick={cancel}>변경 취소</button></div></section>;
  return <section className="documents original-detail" aria-label="원문과 이력">
    {markdown && showing !== undefined ? <DocumentPreview path={path} content={showing} kind="context" title={title} preferTitle afterTitle={controls} bodyOverride={editing ? editor : undefined} resolveInternalLink={resolveInternalLink} resolveInternalDownload={resolveInternalDownload} resolveInternalMedia={resolveInternalMedia} /> : <><h2>{title}</h2>{controls}{showing !== undefined && <OriginalText content={showing} />}</>}
    {historyError && <p className="error" role="alert">이력 확인 실패: {historyError} <button onClick={reloadHistory}>다시 확인</button></p>}
    {history && <details className="section" aria-label="원문 이력"><summary>변경 이력</summary><div className="compact-list">{history.items.map(item => <button key={item.revision} disabled={baseline !== null} aria-pressed={(selected?.revision ?? original?.metadata.revision) === item.revision} onClick={() => void version(item.revision)}>버전 {item.revision} · {new Date(item.recorded_at * 1000).toLocaleString("ko-KR")}</button>)}</div>{history.next_before !== null && <button disabled={baseline !== null} onClick={() => void older(history.next_before!)}>이전 이력 더 보기</button>}</details>}
    {related.length > 0 && <section className="section" aria-label="연결된 자료"><h3>명시적으로 연결된 자료</h3><div className="compact-list">{related.map(item => <button key={item.id} onClick={() => onNavigate?.(item.id)}><strong>{nodePresentation(item).title}</strong><small>{nodePresentation(item).subtitle}</small></button>)}</div></section>}
  </section>;
}
