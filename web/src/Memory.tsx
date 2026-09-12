import { useEffect, useRef, useState } from "react";
import DocumentPreview from "./DocumentPreview";
import type { GraphNode, Request, Scope } from "./graph";
import { fileName, memoryKindName } from "./presentation";
type Evidence = { entity_id: string; source_id?: string; source_revision: string; content_digest: string; generation: number; kind?: "git" | "vault" | "record"; repository?: string; path?: string; current?: boolean; semantics?: { kind: string; title_from_body?: boolean; effective_from: number | null; effective_until: number | null; applicability?: string } };
type Curation = { review_id: string; source_id: string; applicability: string; reason: string };
type Input = { title_from_body?: boolean; kind: "record" | "fact" | "decision" | "preference" | "idea"; title: string; body: string; subject_id: string | null; effective_from: number | null; effective_until: number | null; evidence: Evidence[] };
export type Item = Input & { id: string; scope: Scope; revision: number; status: string; origin: string; subject_name: string | null; updated_at: string; support: string; curation?: Curation };
type Subject = { id: string; name: string };
type Page<T> = { items: T[]; next_after?: string | null; next_before_revision?: number | null };
type History = { revision: number; status: string; subject_id: string | null; document: Input & { curation?: Curation }; changed_at: string };
type EvidenceSnapshot = { available: boolean; content: string | null; evidence: Evidence };
type Props = { latestNode?: GraphNode | null; managing?: boolean; initialItem?: Item | null; visible: boolean; scope: Scope; csrf: string; request: Request; selectedId: string | null; onBusy: (busy: boolean) => void; onChange: (item: Item | null) => void; onNavigate: (id: string) => void; onMetadataChange: () => void };
const blank = (): Input => ({ kind: "record", title: "", body: "", subject_id: null, effective_from: null, effective_until: null, evidence: [] });
const message = (e: unknown) => e instanceof Error ? e.message : "요청에 실패했습니다.";
const epoch = (value: string) => value ? Math.floor(new Date(value).getTime() / 1000) : null;
const dateInput = (value: number | null) => { if (value === null) return ""; const d = new Date(value * 1000); return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16); };
const statusName = (status: string) => ({ accepted: "저장됨", proposed: "제안", withdrawn: "철회" })[status] ?? status;
const cleanInput = (input: Input): Input => ({ kind: input.kind, title: input.title_from_body ? "" : input.title, body: input.body, subject_id: input.subject_id, effective_from: input.effective_from, effective_until: input.effective_until, evidence: input.evidence.map(({ entity_id, source_revision, content_digest, generation }) => ({ entity_id, source_revision, content_digest, generation })) });
const date = (value: number | null) => value === null ? "제한 없음" : new Date(value * 1000).toLocaleString("ko-KR");
const evidenceName = (evidence: Evidence) => evidence.path ? evidence.kind === "record" ? evidence.path : fileName(evidence.path) : "연결된 출처";
function CurationDetails({ value }: { value?: Curation }) {
  return value ? <section className="section"><h3>적용 범위</h3><p>{value.applicability}</p><details><summary>정리 이유</summary><p>{value.reason}</p></details></section> : null;
}
function EvidenceLocation({ evidence }: { evidence: Evidence }) {
  return <details><summary>출처 위치</summary><dl>
    {evidence.kind && <><dt>출처</dt><dd>{evidence.kind === "record" ? "기록" : evidence.kind === "vault" ? "Vault" : "Git"}</dd></>}
    {evidence.semantics && <><dt>기록 종류</dt><dd>{memoryKindName[evidence.semantics.kind] ?? evidence.semantics.kind}</dd><dt>당시 유효기간</dt><dd>{date(evidence.semantics.effective_from)} ~ {date(evidence.semantics.effective_until)}</dd>{evidence.semantics.applicability && <><dt>당시 적용 범위</dt><dd>{evidence.semantics.applicability}</dd></>}</>}
    {evidence.repository && <><dt>저장소</dt><dd>{evidence.repository}</dd></>}
    <dt>{evidence.kind === "record" ? "기록 제목" : "원문 경로"}</dt><dd>{evidence.path ?? "경로가 기록되지 않았습니다."}</dd>
  </dl></details>;
}
function Validity({ value }: { value: Pick<Input, "effective_from" | "effective_until"> }) {
  return <dl><dt>유효 시작</dt><dd>{date(value.effective_from)}</dd><dt>유효 종료</dt><dd>{date(value.effective_until)}</dd></dl>;
}
export default function Memory({ latestNode, managing = false, initialItem, visible, scope, csrf, request, selectedId, onBusy, onChange, onNavigate, onMetadataChange }: Props) {
  const seed = initialItem?.scope === scope && initialItem.id === selectedId ? initialItem : null;
  const [subjects, setSubjects] = useState<Subject[]>([]), [subjectNext, setSubjectNext] = useState<string | null>(null);
  const subjectsLoaded = useRef(false), subjectsFailed = useRef(false), readFailed = useRef(false);
  const [subjectsRequested, setSubjectsRequested] = useState(false);
  const [subjectsLoading, setSubjectsLoading] = useState(false), [subjectsError, setSubjectsError] = useState("");
  const [subjectRetry, setSubjectRetry] = useState(0), [readRetry, setReadRetry] = useState(0), [readError, setReadError] = useState("");
  const [selected, setSelected] = useState<Item | null>(() => seed), [draft, setDraft] = useState<Input>(() => seed ?? blank()), [editing, setEditing] = useState(!selectedId);
  const changedWhileOpen = !!selected && !!latestNode && latestNode.scope === scope && latestNode.id === selected.id
    && ((latestNode.revision !== undefined && Number(latestNode.revision) > selected.revision)
      || (latestNode.supported === false && (latestNode.revision === undefined || Number(latestNode.revision) >= selected.revision) && selected.evidence.some(e => e.current === true)));
  const [proposal, setProposal] = useState(false), [key, setKey] = useState(() => crypto.randomUUID());
  const [history, setHistory] = useState<History[] | null>(null), [historyNext, setHistoryNext] = useState<number | null>(null);
  const [evidenceQuery, setEvidenceQuery] = useState(""), [choices, setChoices] = useState<Evidence[]>([]);
  const [subjectKey, setSubjectKey] = useState(() => crypto.randomUUID());
  const [subjectName, setSubjectName] = useState("");
  const [evidenceView, setEvidenceView] = useState<string | null>(null);
  const [evidenceSnapshots, setEvidenceSnapshots] = useState<Record<string, EvidenceSnapshot>>({});
  const [busy, setBusy] = useState(false), [loading, setLoading] = useState(!!selectedId && !seed), [error, setError] = useState(""), [notice, setNotice] = useState("");
  const [forgetting, setForgetting] = useState(false);
  const editForm = useRef<HTMLFormElement>(null);
  const mounted = useRef(true), active = useRef<string | null>(selectedId), pending = useRef(false), requests = useRef(new Set<AbortController>());
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; for (const controller of requests.current) controller.abort(); onBusy(false); }; }, []);
  const command = async <T,>(body: object, signal?: AbortSignal): Promise<T> => {
    const controller = new AbortController(); requests.current.add(controller);
    try { return await request<T>("/api/brain", { method: "POST", headers: { "content-type": "application/json", "x-csrf-token": csrf }, body: JSON.stringify({ scope, ...body }), signal: signal ?? controller.signal }); }
    finally { requests.current.delete(controller); }
  };
  useEffect(() => {
    // A supplied write response seeds this mount; resuming never replaces a loaded draft.
    if (!visible || !selectedId || selected || readFailed.current) return;
    const controller = new AbortController(); setLoading(true); setReadError("");
    command<Item>({ op: "read", id: selectedId }, controller.signal).then(value => {
      if (!controller.signal.aborted) { setSelected(value); setDraft(value); }
    }).catch(e => {
      if (!controller.signal.aborted) { readFailed.current = true; setReadError(message(e)); }
    }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [scope, csrf, selectedId, visible, readRetry]);
  useEffect(() => { if (selected && editing && managing) editForm.current?.querySelector("textarea")?.focus(); }, [editing, managing]);
  const needsSubjects = subjectsRequested && editing && (!selected || managing);
  useEffect(() => {
    if (!visible || !needsSubjects || subjectsLoaded.current || subjectsFailed.current) return;
    const controller = new AbortController(); setSubjectsLoading(true); setSubjectsError("");
    command<Page<Subject>>({ op: "subjects" }, controller.signal).then(value => {
      if (!controller.signal.aborted) {
        subjectsLoaded.current = true;
        setSubjects(current => [...current, ...value.items.filter(item => !current.some(existing => existing.id === item.id))]);
        setSubjectNext(value.next_after ?? null);
      }
    }).catch(e => {
      if (!controller.signal.aborted) { subjectsFailed.current = true; setSubjectsError(message(e)); }
    }).finally(() => { if (!controller.signal.aborted) setSubjectsLoading(false); });
    return () => { controller.abort(); setSubjectsLoading(false); };
  }, [scope, csrf, visible, needsSubjects, subjectRetry]);
  async function run(action: () => Promise<void>) {
    if (pending.current) return; pending.current = true; setBusy(true); onBusy(true); setError(""); setNotice("");
    try { await action(); } catch (e) { if (mounted.current) setError(message(e)); }
    finally { pending.current = false; if (mounted.current) { setBusy(false); onBusy(false); } }
  }
  async function choose(id: string) {
    await run(async () => { const value = await command<Item>({ op: "read", id }); if (!mounted.current) return; active.current = id; setSelected(value); setDraft(value); setEditing(false); setHistory(null); setHistoryNext(null); setChoices([]); setForgetting(false); });
  }
  function change(patch: Partial<Input>) { setDraft((v) => ({ ...v, ...patch, ...(patch.title !== undefined ? { title_from_body: false } : {}) })); if (!selected) setKey(crypto.randomUUID()); }
  async function save() {
    await run(async () => {
      const body = selected ? { op: "correct", id: selected.id, revision: selected.revision, memory: cleanInput(draft) } : { op: proposal ? "propose" : "remember", idempotency_key: key, memory: cleanInput(draft) };
      const value = await command<Item>(body); if (!mounted.current) return;
      active.current = value.id; setSelected(value); setDraft(value); setEditing(false); setHistory(null); setHistoryNext(null); setEvidenceView(null); setEvidenceSnapshots({}); setNotice("기록을 저장했습니다."); onChange(value);
    });
  }
  async function mutate(op: "accept" | "withdraw" | "forget") {
    if (!selected) return; const snapshot = selected;
    await run(async () => {
      const value = await command<Item>({ op, id: snapshot.id, revision: snapshot.revision }); if (!mounted.current || active.current !== snapshot.id) return;
      if (op === "forget") { active.current = null; setSelected(null); setDraft(blank()); setEditing(false); setNotice("이 앱의 기록과 이력을 삭제했습니다."); }
      else { setSelected(value); setDraft(value); setNotice(op === "accept" ? "제안을 보관했습니다." : "기록을 철회했습니다."); }
      setForgetting(false); setHistory(null); setHistoryNext(null); setEvidenceView(null); setEvidenceSnapshots({}); onChange(op === "forget" ? null : value);
    });
  }
  async function loadHistory(before?: number) {
    if (!selected || (before === undefined && history !== null)) return;
    await run(async () => { const value = await command<Page<History>>({ op: "history", id: selected.id, before_revision: before ?? null, limit: 10 }); if (!mounted.current) return; setHistory((v) => before !== undefined ? [...(v ?? []), ...value.items] : value.items); setHistoryNext(value.next_before_revision ?? null); });
  }
  async function readEvidence(revision: number, evidence: Evidence) {
    if (!selected) return;
    const cacheKey = `${selected.id}:${revision}:${evidence.entity_id}`;
    if (evidenceView === cacheKey) { setEvidenceView(null); return; }
    if (evidenceSnapshots[cacheKey]) { setEvidenceView(cacheKey); return; }
    await run(async () => {
      const value = await command<EvidenceSnapshot>({ op: "evidence-read", id: selected.id, revision, entity_id: evidence.entity_id });
      if (!mounted.current) return;
      setEvidenceSnapshots(current => ({ ...current, [cacheKey]: value })); setEvidenceView(cacheKey);
    });
  }
  function evidencePreview(revision: number, evidence: Evidence) {
    const cacheKey = `${selected?.id}:${revision}:${evidence.entity_id}`;
    const value = evidenceSnapshots[cacheKey];
    return <><button disabled={busy} aria-expanded={evidenceView === cacheKey} onClick={() => void readEvidence(revision, evidence)}>기록 당시 원문</button>
      {evidenceView === cacheKey && value && <div className="evidence-preview">{value.available
        ? <><p className="hint">이 기록을 남길 때 연결한 원문입니다.</p><DocumentPreview path={value.evidence.path ?? "원문.md"} title={value.evidence.kind === "record" ? value.evidence.path : undefined} generatedTitle={value.evidence.semantics?.title_from_body} kind={value.evidence.kind ?? "git"} content={value.content} /></>
        : <p className="warning">당시 원문은 보존되어 있지 않습니다. 현재 원문으로 대신하지 않습니다.</p>}</div>}</>;
  }
  const subjectStatus = <>
    {subjectsLoading && <p className="hint" role="status">기록 묶음을 불러오는 중…</p>}
    {subjectsError && <p className="error" role="alert">{subjectsError} <button type="button" disabled={subjectsLoading} onClick={() => { subjectsFailed.current = false; setSubjectRetry(value => value + 1); }}>묶음 다시 불러오기</button></p>}
    {subjectNext && <button type="button" disabled={busy} onClick={() => void run(async () => { const value = await command<Page<Subject>>({ op: "subjects", after: subjectNext }); if (!mounted.current) return; setSubjects(current => [...current, ...value.items.filter(item => !current.some(existing => existing.id === item.id))]); setSubjectNext(value.next_after ?? null); })}>묶음 더 보기</button>}
  </>;
  return <section className="brain">
    {readError && <p className="error" role="alert">{readError} <button disabled={loading} onClick={() => { readFailed.current = false; setReadRetry(value => value + 1); }}>기록 다시 불러오기</button></p>}
    <div aria-live="polite">{error && <div className="error" role="alert">{error} {selected && <button disabled={busy} onClick={() => void choose(selected.id)}>현재 기록 다시 읽기</button>}</div>}{notice && <div className="notice">{notice}</div>}</div>
    {loading ? <p role="status">기록을 불러오는 중…</p> : <div className="memory-editor">
        <div id={editing ? "record-management" : undefined} hidden={!!selected && !managing}>
        <form ref={editForm} hidden={!editing} onSubmit={(e) => { e.preventDefault(); void save(); }}>{selected && <h2>기록 정정</h2>}<fieldset disabled={busy}>
          <label className="topic-label">내용<textarea required value={draft.body} maxLength={8192} rows={6} onChange={(e) => change({ body: e.target.value })} /></label>
          <details className="section memory-options"><summary>추가 설정</summary>
          <label className="topic-label">제목 · 비우면 내용에서 자동으로 정합니다<input maxLength={160} value={draft.title} onChange={(e) => change({ title: e.target.value })} /></label>
          <label className="topic-label">종류<select value={draft.kind} onChange={(e) => change({ kind: e.target.value as Input["kind"] })}><option value="record">일반 기록</option><option value="fact">사실</option><option value="decision">결정</option><option value="preference">선호</option><option value="idea">아이디어</option></select></label>
          <label className="topic-label">기록 묶음<select aria-label="기록 묶음" onFocus={() => setSubjectsRequested(true)} value={draft.subject_id ?? ""} onChange={(e) => change({ subject_id: e.target.value || null })}>
            <option value="">묶음 없음</option>
            {selected?.scope === scope && selected.subject_id && !subjects.some((s) => s.id === selected.subject_id) && (
              <option value={selected.subject_id}>{selected.subject_name ?? "현재 기록 묶음"}</option>
            )}
            {subjects.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
          </select></label>
          {subjectStatus}
          <p className="hint">묶음은 선택 사항입니다. 묶지 않아도 저장하고 검색할 수 있습니다.</p>
          <div className="date-fields"><label className="topic-label">유효 시작<input type="datetime-local" value={dateInput(draft.effective_from)} onChange={(e) => change({ effective_from: epoch(e.target.value) })} /></label><label className="topic-label">유효 종료<input type="datetime-local" value={dateInput(draft.effective_until)} onChange={(e) => change({ effective_until: epoch(e.target.value) })} /></label></div>
          <section className="section"><h3>출처 근거 · 최대 10개</h3><p className="hint">연결한 원문은 기록 당시 내용으로 보존합니다. 출처가 바뀌면 재확인 필요 상태로 표시합니다.</p>
            {draft.evidence.map((e) => <div className="evidence-row" key={e.entity_id}><div className="evidence-identity">{evidenceName(e)}{e.repository && ` · ${fileName(e.repository)}`}<EvidenceLocation evidence={e} /></div><button type="button" onClick={() => change({ evidence: draft.evidence.filter((v) => v.entity_id !== e.entity_id) })}>근거 제거</button></div>)}
            <div className="related-search"><input aria-label="출처 근거 검색" maxLength={120} value={evidenceQuery} onChange={(e) => setEvidenceQuery(e.target.value)} placeholder="자료 경로 검색" /><button type="button" onClick={() => void run(async () => { const v = await command<Page<Evidence>>({ op: "evidence", query: evidenceQuery }); setChoices(v.items); })}>근거 찾기</button></div>
            {choices.map((e) => <button className="evidence-choice" type="button" key={e.entity_id} disabled={draft.evidence.length >= 10 && !draft.evidence.some((v) => v.entity_id === e.entity_id)} onClick={() => change({ evidence: [...draft.evidence.filter((v) => v.entity_id !== e.entity_id), e] })} title={[e.repository, e.path].filter(Boolean).join(" · ")}>{e.path ?? evidenceName(e)}{e.repository && ` · ${fileName(e.repository)}`} · {draft.evidence.some((v) => v.entity_id === e.entity_id) ? "현재 근거로 재확인" : "근거 선택"}</button>)}
          </section>
          <details className="section"><summary>새 묶음 만들기</summary><label className="topic-label">묶음 이름<input value={subjectName} maxLength={80} onChange={e => { setSubjectName(e.target.value); setSubjectKey(crypto.randomUUID()); }} /></label><button type="button" disabled={busy || !subjectName.trim()} onClick={() => void run(async () => { const value = await command<Subject>({ op: "subject-create", idempotency_key: subjectKey, name: subjectName }); if (!mounted.current) return; setSubjects(v => [...v.filter(s => s.id !== value.id), value]); onMetadataChange(); setSubjectName(""); setSubjectKey(crypto.randomUUID()); change({ subject_id: value.id }); })}>묶음 만들기</button></details>
          {!selected && <label className="proposal-toggle"><input type="checkbox" checked={proposal} onChange={(e) => { setProposal(e.target.checked); setKey(crypto.randomUUID()); }} /> 아직 확정하지 않은 제안으로 남기기</label>}
          </details>
          {new TextEncoder().encode(draft.body).length > 8192 && <p className="warning">내용은 최대 8,192바이트까지 저장할 수 있습니다. 내용을 줄여주세요.</p>}
          <button className="primary" disabled={new TextEncoder().encode(draft.body).length > 8192}>{busy ? "저장 중…" : selected ? "정정 저장" : proposal ? "제안 저장" : "저장"}</button>
          <p className="hint">내용만 남기면 됩니다. 저장한 내용이 자동으로 검증된 사실이 되지는 않습니다.</p>
        </fieldset></form>
        </div>
        {changedWhileOpen && <p className="warning">열어 둔 기록이나 근거의 변경을 확인했습니다. 아래 내용은 이전에 읽은 상태입니다. <button disabled={busy} onClick={() => void choose(selected!.id)}>현재 기록 다시 읽기</button></p>}
        {selected ? <div hidden={editing && managing}>
          <div className="detail-heading"><p className="source-identity">{memoryKindName[selected.kind] ?? selected.kind} · {statusName(selected.status)}</p>{selected.subject_name && <p className="hint">{selected.subject_name}</p>}</div>
          {selected.effective_from !== null && selected.effective_from > Date.now() / 1000 && <p className="warning">아직 유효 시작 시점이 되지 않았습니다.</p>}
          {selected.effective_until !== null && selected.effective_until <= Date.now() / 1000 && <p className="warning">유효기간이 지난 기록입니다.</p>}
          {selected.status === "withdrawn" && <p className="warning">철회한 기록입니다. 이전 내용과 이력은 남아 있습니다.</p>}
          {selected.status === "proposed" && <p className="warning">아직 확정하지 않은 제안입니다.</p>}
          <div className="memory-body"><DocumentPreview path={selected.title} title={selected.title} generatedTitle={selected.title_from_body} kind="record" content={selected.body} /></div>
          <CurationDetails value={selected.curation} />
          {selected.evidence.length > 0 && <section className="section"><h3>출처 근거</h3>{selected.evidence.map(e => <div className="evidence-source" key={e.entity_id}><h4>{evidenceName(e)}</h4>{evidencePreview(selected.revision, e)} <button disabled={busy} onClick={() => onNavigate(e.entity_id)}>현재 원문 ↗</button><p className={e.current && !changedWhileOpen ? "hint" : "warning"}>{e.current && !changedWhileOpen ? "현재 출처와 일치" : "근거 재확인 필요"}</p><EvidenceLocation evidence={e} /></div>)}</section>}
          {selected.status === "proposed" && <div className="memory-actions"><button className="primary" disabled={busy} onClick={() => void mutate("accept")}>제안 보관</button></div>}
          <div id={!editing ? "record-management" : undefined} hidden={!managing}>
          <div className="memory-actions">{selected.status !== "withdrawn" && <button disabled={busy} onClick={() => setEditing(true)}>기록 정정</button>}</div>
          <details className="section"><summary>기록 상세·변경 이력</summary>
            <p className="hint">‘보관’은 사실 검증을 뜻하지 않습니다. {selected.evidence.length ? "연결한 출처는 위의 근거 상태로 확인합니다." : "외부 근거를 연결하지 않은 기록입니다."}</p>
            <Validity value={selected} />
            <p className="hint">{selected.curation ? "근거와 적용 조건을 독립 검토한 뒤 정리한 기록입니다." : selected.origin === "assistant" ? "제안 경로로 저장했습니다. 원저자를 확인한 것은 아닙니다." : "직접 입력 경로로 저장했습니다. 원저자를 확인한 것은 아닙니다."}</p>
            {history === null && <button disabled={busy} onClick={() => void loadHistory()}>변경 이력 읽기</button>}
            {history && <section className="section">
              <h3>변경 이력 · 과거 내용</h3>
              <p className="hint">각 시점에 저장된 기록입니다. 아래 근거는 현재 출처와의 일치를 뜻하지 않습니다.</p>
              {history.length === 0 && <p className="hint">표시할 변경 이력이 없습니다.</p>}
              {history.map(h => <article className="history" key={h.revision}>
                <p className="source-identity">{memoryKindName[h.document.kind] ?? h.document.kind} · {statusName(h.status)}</p>
                <time>{new Date(h.changed_at).toLocaleString("ko-KR")}</time>
                <DocumentPreview path={h.document.title} title={h.document.title} generatedTitle={h.document.title_from_body} kind="record" content={h.document.body} />
                <CurationDetails value={h.document.curation} />
                <Validity value={h.document} />
                {h.document.evidence.length > 0 && <>
                  <h4>당시 연결한 근거</h4>
                  {h.document.evidence.map(e => <div key={e.entity_id}><p>{evidenceName(e)}</p>{evidencePreview(h.revision, e)}<EvidenceLocation evidence={e} /></div>)}
                </>}
              </article>)}
              {historyNext !== null && <button disabled={busy} onClick={() => void loadHistory(historyNext)}>이력 더 보기</button>}
            </section>}

          </details>
          <details className="section"><summary>철회·삭제</summary>
            <p className="hint">철회하면 현재 적용하지 않는 상태로 표시하며 내용과 이력은 남깁니다.</p>
            <div className="memory-actions">{selected.status !== "withdrawn" && <button disabled={busy} onClick={() => void mutate("withdraw")}>기록 철회</button>}<button disabled={busy} onClick={() => setForgetting(true)}>기록 삭제</button></div>
            {forgetting && <div className="warning"><p>이 앱의 기록 내용, 변경 이력과 근거 연결을 삭제합니다. 원본 문서와 기존 백업은 별도로 남습니다.</p><button disabled={busy} onClick={() => void mutate("forget")}>이 기록 완전히 삭제</button><button disabled={busy} onClick={() => setForgetting(false)}>취소</button></div>}
          </details>
          </div>
        </div> : !editing && !selectedId ? <div className="welcome"><h2>남길 기록을 선택하세요.</h2><p>기록을 직접 보관하거나 제안을 검토할 수 있습니다.</p></div> : null}

    </div>}
  </section>;
}
