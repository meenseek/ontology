import { useEffect, useRef, useState } from "react";
import type { Request, Scope } from "./graph";
import { fileName, memoryKindName } from "./presentation";
type Evidence = { entity_id: string; source_id?: string; source_revision: string; content_digest: string; generation: number; kind?: "git" | "vault"; repository?: string; path?: string; current?: boolean };
type Input = { kind: "fact" | "decision" | "preference" | "idea"; title: string; body: string; subject_id: string | null; effective_from: number | null; effective_until: number | null; evidence: Evidence[] };
type Item = Input & { id: string; scope: Scope; revision: number; status: string; origin: string; subject_name: string | null; updated_at: string; support: string };
type Subject = { id: string; name: string };
type Page<T> = { items: T[]; next_after?: string | null; next_before_revision?: number | null };
type History = { revision: number; status: string; subject_id: string | null; document: Input; changed_at: string };
type Context = { items: Item[]; status: string; truncated: boolean; instruction: string };
type Props = { visible: boolean; scope: Scope; csrf: string; request: Request; selectedId: string | null; onBusy: (busy: boolean) => void; onChange: (id: string | null) => void; onNavigate: (id: string) => void; onMetadataChange: () => void };
const blank = (): Input => ({ kind: "fact", title: "", body: "", subject_id: null, effective_from: null, effective_until: null, evidence: [] });
const message = (e: unknown) => e instanceof Error ? e.message : "요청에 실패했습니다.";
const epoch = (value: string) => value ? Math.floor(new Date(value).getTime() / 1000) : null;
const dateInput = (value: number | null) => { if (value === null) return ""; const d = new Date(value * 1000); return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16); };
const statusName = (status: string) => ({ accepted: "보관", proposed: "제안", withdrawn: "철회" })[status] ?? status;
const cleanInput = (input: Input): Input => ({ kind: input.kind, title: input.title, body: input.body, subject_id: input.subject_id, effective_from: input.effective_from, effective_until: input.effective_until, evidence: input.evidence.map(({ entity_id, source_revision, content_digest, generation }) => ({ entity_id, source_revision, content_digest, generation })) });
export default function Memory({ visible, scope, csrf, request, selectedId, onBusy, onChange, onNavigate, onMetadataChange }: Props) {
  const [subjects, setSubjects] = useState<Subject[]>([]), [subjectNext, setSubjectNext] = useState<string | null>(null);
  const [subject, setSubject] = useState("");
  const subjectsLoaded = useRef(false);
  const [selected, setSelected] = useState<Item | null>(null), [draft, setDraft] = useState<Input>(blank), [editing, setEditing] = useState(!selectedId);
  const [proposal, setProposal] = useState(false), [key, setKey] = useState(() => crypto.randomUUID());
  const [history, setHistory] = useState<History[] | null>(null), [historyNext, setHistoryNext] = useState<number | null>(null);
  const [evidenceQuery, setEvidenceQuery] = useState(""), [choices, setChoices] = useState<Evidence[]>([]);
  const [subjectKey, setSubjectKey] = useState(() => crypto.randomUUID());
  const [subjectName, setSubjectName] = useState(""), [contextQuery, setContextQuery] = useState(""), [context, setContext] = useState<Context | null>(null);
  const [busy, setBusy] = useState(false), [loading, setLoading] = useState(!!selectedId), [error, setError] = useState(""), [notice, setNotice] = useState("");
  const [forgetting, setForgetting] = useState(false);
  const mounted = useRef(true), active = useRef<string | null>(selectedId), pending = useRef(false), requests = useRef(new Set<AbortController>());
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; for (const controller of requests.current) controller.abort(); onBusy(false); }; }, []);
  const command = async <T,>(body: object, signal?: AbortSignal): Promise<T> => {
    const controller = new AbortController(); requests.current.add(controller);
    try { return await request<T>("/api/brain", { method: "POST", headers: { "content-type": "application/json", "x-csrf-token": csrf }, body: JSON.stringify({ scope, ...body }), signal: signal ?? controller.signal }); }
    finally { requests.current.delete(controller); }
  };
  useEffect(() => {
    if (!visible) return;
    const controller = new AbortController();
    if (!subjectsLoaded.current) command<Page<Subject>>({ op: "subjects" }, controller.signal).then((v) => { if (!controller.signal.aborted) { subjectsLoaded.current = true; setSubjects(v.items); setSubjectNext(v.next_after ?? null); } }).catch((e) => { if (!controller.signal.aborted) setError(message(e)); });
    // Resume unfinished reads without replacing a loaded editor's draft or revision.
    if (selectedId && !selected) command<Item>({ op: "read", id: selectedId }, controller.signal).then(v => { if (!controller.signal.aborted) { setSelected(v); setDraft(v); setLoading(false); } }).catch(e => { if (!controller.signal.aborted) { setError(message(e)); setLoading(false); } });
    return () => controller.abort();
  }, [scope, csrf, selectedId, visible]);
  async function run(action: () => Promise<void>) {
    if (pending.current) return; pending.current = true; setBusy(true); onBusy(true); setError(""); setNotice("");
    try { await action(); } catch (e) { if (mounted.current) setError(message(e)); }
    finally { pending.current = false; if (mounted.current) { setBusy(false); onBusy(false); } }
  }
  async function choose(id: string) {
    await run(async () => { const value = await command<Item>({ op: "read", id }); if (!mounted.current) return; active.current = id; setSelected(value); setDraft(value); setEditing(false); setHistory(null); setChoices([]); setForgetting(false); });
  }
  function change(patch: Partial<Input>) { setDraft((v) => ({ ...v, ...patch })); if (!selected) setKey(crypto.randomUUID()); }
  async function save() {
    await run(async () => {
      const body = selected ? { op: "correct", id: selected.id, revision: selected.revision, memory: cleanInput(draft) } : { op: proposal ? "propose" : "remember", idempotency_key: key, memory: cleanInput(draft) };
      const value = await command<Item>(body); if (!mounted.current) return;
      active.current = value.id; setSelected(value); setDraft(value); setEditing(false); setHistory(null); setContext(null); setNotice("기억을 저장했습니다."); onChange(value.id);
    });
  }
  async function mutate(op: "accept" | "withdraw" | "forget") {
    if (!selected) return; const snapshot = selected;
    await run(async () => {
      const value = await command<Item>({ op, id: snapshot.id, revision: snapshot.revision }); if (!mounted.current || active.current !== snapshot.id) return;
      if (op === "forget") { active.current = null; setSelected(null); setDraft(blank()); setEditing(false); setNotice("이 앱의 기억과 이력을 삭제했습니다."); }
      else { setSelected(value); setDraft(value); setNotice(op === "accept" ? "제안을 보관했습니다." : "기억을 철회했습니다."); }
      setForgetting(false); setHistory(null); setContext(null); onChange(op === "forget" ? null : value.id);
    });
  }
  async function loadHistory(before?: number) {
    if (!selected) return;
    await run(async () => { const value = await command<Page<History>>({ op: "history", id: selected.id, before_revision: before ?? null, limit: 10 }); if (!mounted.current) return; setHistory((v) => before ? [...(v ?? []), ...value.items] : value.items); setHistoryNext(value.next_before_revision ?? null); });
  }
  return <section className="brain">
    <div aria-live="polite">{error && <div className="error" role="alert">{error} {selected && <button disabled={busy} onClick={() => void choose(selected.id)}>현재 기억 다시 읽기</button>}</div>}{notice && <div className="notice">{notice}</div>}</div>
    {loading ? <p role="status">기억을 불러오는 중…</p> : <div className="memory-editor">
        {editing ? <form onSubmit={(e) => { e.preventDefault(); void save(); }}><h2>{selected ? "기억 정정" : "새 기억"}</h2><fieldset disabled={busy}>
          <label className="topic-label">제목<input required maxLength={160} value={draft.title} onChange={(e) => change({ title: e.target.value })} /></label>
          <label className="topic-label">내용<textarea required value={draft.body} maxLength={8192} rows={6} onChange={(e) => change({ body: e.target.value })} /></label>
          <label className="topic-label">종류<select value={draft.kind} onChange={(e) => change({ kind: e.target.value as Input["kind"] })}><option value="fact">사실</option><option value="decision">결정</option><option value="preference">선호</option><option value="idea">아이디어</option></select></label>
          <details className="section memory-options"><summary>추가 설정 · 묶음, 유효기간, 출처</summary>
          <label className="topic-label">기억 묶음<select value={draft.subject_id ?? ""} onChange={(e) => change({ subject_id: e.target.value || null })}>
            <option value="">묶음 없음</option>
            {selected?.scope === scope && selected.subject_id && !subjects.some((s) => s.id === selected.subject_id) && (
              <option value={selected.subject_id}>{selected.subject_name ?? selected.subject_id}</option>
            )}
            {subjects.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
          </select></label>
          <p className="hint">기억 묶음은 하나만 선택하거나 비워 둘 수 있으며, 문서 태그와는 별도 분류입니다.</p>
          <div className="date-fields"><label className="topic-label">유효 시작<input type="datetime-local" value={dateInput(draft.effective_from)} onChange={(e) => change({ effective_from: epoch(e.target.value) })} /></label><label className="topic-label">유효 종료 · 해당 시점부터 제외<input type="datetime-local" value={dateInput(draft.effective_until)} onChange={(e) => change({ effective_until: epoch(e.target.value) })} /></label></div>
          <section className="section"><h3>출처 근거 · 최대 10개</h3><p className="hint">출처가 변경되거나 읽기에 실패하면 그 근거를 연결한 기억은 맥락 조회에서 제외됩니다. 근거를 다시 선택하고 정정 저장하면 현재 출처로 재확인합니다.</p>
            {draft.evidence.map((e) => <div className="evidence-row" key={e.entity_id}><span>{e.kind} · {e.repository} · {e.path}</span><button type="button" onClick={() => change({ evidence: draft.evidence.filter((v) => v.entity_id !== e.entity_id) })}>근거 제거</button></div>)}
            <div className="related-search"><input aria-label="출처 근거 검색" maxLength={120} value={evidenceQuery} onChange={(e) => setEvidenceQuery(e.target.value)} placeholder="자료 경로 검색" /><button type="button" onClick={() => void run(async () => { const v = await command<Page<Evidence>>({ op: "evidence", query: evidenceQuery }); setChoices(v.items); })}>근거 찾기</button></div>
            {choices.map((e) => <button className="evidence-choice" type="button" key={e.entity_id} disabled={draft.evidence.length >= 10 && !draft.evidence.some((v) => v.entity_id === e.entity_id)} onClick={() => change({ evidence: [...draft.evidence.filter((v) => v.entity_id !== e.entity_id), e] })}>{e.kind} · {e.repository} · {e.path} · {draft.evidence.some((v) => v.entity_id === e.entity_id) ? "현재 근거로 재확인" : "근거 선택"}</button>)}
          </section>
          {!selected && <label className="proposal-toggle"><input type="checkbox" checked={proposal} onChange={(e) => { setProposal(e.target.checked); setKey(crypto.randomUUID()); }} /> 제안으로 남기기 · 사용자가 보관하기 전까지 맥락 조회에서 제외</label>}
          </details>
          {new TextEncoder().encode(draft.body).length > 8192 && <p className="warning">내용은 최대 8,192바이트까지 저장할 수 있습니다. 내용을 줄여주세요.</p>}
          <button className="primary" disabled={new TextEncoder().encode(draft.body).length > 8192}>{busy ? "저장 중…" : selected ? "정정 저장" : proposal ? "제안 저장" : "기억 보관"}</button>
          <p className="hint">‘보관’은 남기기로 했다는 뜻이며 사실 검증을 뜻하지 않습니다.</p>
        </fieldset></form> : selected ? <>
          <div className="detail-heading"><p className="source-identity">{memoryKindName[selected.kind] ?? selected.kind} · {statusName(selected.status)}</p><h2>{selected.title}</h2>{selected.subject_name && <p className="hint">{selected.subject_name}</p>}</div>
          {selected.effective_from !== null && selected.effective_from > Date.now() / 1000 && <p className="warning">미래에 유효한 기억 · 아직 맥락 조회에 포함하지 않습니다.</p>}
          {selected.effective_until !== null && selected.effective_until <= Date.now() / 1000 && <p className="warning">만료된 기억 · 맥락 조회에서 제외합니다.</p>}
          {selected.status === "withdrawn" && <p className="warning">철회한 기억입니다. 맥락 조회에서 제외되며 이력은 남습니다.</p>}
          {selected.status === "proposed" && <p className="hint">보관하기 전까지 맥락 조회에서 제외하는 제안입니다.</p>}
          <pre className="source-text memory-body">{selected.body}</pre>
          {selected.evidence.length > 0 && <section className="section"><h3>출처 근거</h3>{selected.evidence.map(e => <div className="evidence-source" key={e.entity_id}><button disabled={busy} onClick={() => onNavigate(e.entity_id)}>{fileName(e.path ?? e.entity_id)} ↗</button><p className={e.current ? "hint" : "warning"}>{e.current ? "현재 출처와 일치" : "근거 재확인 필요 · 맥락 조회에서 제외"}</p><details><summary>근거 상세</summary><dl><dt>출처</dt><dd>{e.kind}</dd><dt>로컬 위치</dt><dd>{e.repository}</dd><dt>원문 경로</dt><dd>{e.path}</dd><dt>근거 버전</dt><dd>{e.source_revision}</dd><dt>내용 SHA-256</dt><dd>{e.content_digest}</dd></dl></details></div>)}</section>}
          <div className="memory-actions">{selected.status === "proposed" && <button className="primary" disabled={busy} onClick={() => void mutate("accept")}>제안 보관</button>}{selected.status !== "withdrawn" && <button disabled={busy} onClick={() => setEditing(true)}>기억 정정</button>}</div>
          <details className="section"><summary>기억 상세·변경 이력</summary>
            <p className="hint">‘보관’은 사실 검증을 뜻하지 않습니다. {selected.evidence.length ? "연결한 출처는 위의 근거 상태로 확인합니다." : "외부 근거를 연결하지 않은 기억입니다."}</p>
            <dl><dt>유효 시작</dt><dd>{selected.effective_from === null ? "제한 없음" : new Date(selected.effective_from * 1000).toLocaleString("ko-KR")}</dd><dt>유효 종료</dt><dd>{selected.effective_until === null ? "제한 없음" : new Date(selected.effective_until * 1000).toLocaleString("ko-KR")}</dd><dt>시작</dt><dd>{selected.origin === "assistant" ? "제안" : "사용자 기록"}</dd><dt>버전</dt><dd>r{selected.revision}</dd><dt>기억 ID</dt><dd>{selected.id}</dd></dl>
            <button disabled={busy} onClick={() => void loadHistory()}>변경 이력 읽기</button>
            {history && <section className="section"><h3>변경 이력 · 과거 내용</h3>{history.map(h => <div className="history" key={h.revision}><b>r{h.revision} · {statusName(h.status)}</b><time>{new Date(h.changed_at).toLocaleString("ko-KR")}</time><pre>{JSON.stringify(h.document, null, 2)}</pre></div>)}{historyNext && <button disabled={busy} onClick={() => void loadHistory(historyNext)}>이력 더 보기</button>}</section>}
          </details>
          <details className="section"><summary>철회·삭제</summary>
            <p className="hint">철회하면 맥락 조회에서 제외하며 내용과 이력은 남깁니다.</p>
            <div className="memory-actions">{selected.status !== "withdrawn" && <button disabled={busy} onClick={() => void mutate("withdraw")}>기억 철회</button>}<button disabled={busy} onClick={() => setForgetting(true)}>기억 삭제</button></div>
            {forgetting && <div className="warning"><p>이 앱의 기억 내용, 변경 이력과 근거 연결을 삭제합니다. 원본 문서와 기존 백업은 별도로 남습니다.</p><button disabled={busy} onClick={() => void mutate("forget")}>이 기억 완전히 삭제</button><button disabled={busy} onClick={() => setForgetting(false)}>취소</button></div>}
          </details>
        </> : <div className="welcome"><h2>남길 기억을 선택하세요.</h2><p>기억을 직접 보관하거나 제안을 검토할 수 있습니다.</p></div>}

    </div>}
        <details className="section"><summary>기억 묶음 만들기</summary><form onSubmit={(e) => { e.preventDefault(); void run(async () => { const value = await command<Subject>({ op: "subject-create", idempotency_key: subjectKey, name: subjectName }); setSubjects((v) => [...v.filter(s => s.id !== value.id), value]); onMetadataChange(); setSubjectName(""); setSubjectKey(crypto.randomUUID()); change({ subject_id: value.id }); }); }}><label className="facet-filter">새 묶음 이름<input required disabled={busy} value={subjectName} maxLength={80} onChange={(e) => { setSubjectName(e.target.value); setSubjectKey(crypto.randomUUID()); }} /></label><button disabled={busy || !subjectName.trim()}>묶음 만들기</button></form></details>
        {subjectNext && <button disabled={busy} onClick={() => void run(async () => { const v = await command<Page<Subject>>({ op: "subjects", after: subjectNext }); setSubjects((old) => [...old, ...v.items]); setSubjectNext(v.next_after ?? null); })}>묶음 더 보기</button>}

<details className="section context-retrieval"><summary>맥락 조회</summary>
<label className="facet-filter">맥락 조회 묶음<select disabled={busy} value={subject} onChange={e => { setSubject(e.target.value); setContext(null); }}><option value="">전체 묶음</option>{subjects.map(s => <option key={s.id} value={s.id}>{s.name}</option>)}</select></label>
    <section><p className="hint">현재 범위{subject ? "와 선택한 묶음" : ""}에서 보관한 유효 기억을 찾습니다. 한국어·영어 단어의 문자열 일치로 조회하며, 원문 자료 검색과 구분됩니다.</p><form className="related-search" onSubmit={(e) => { e.preventDefault(); void run(async () => { const v = await command<Context>({ op: "recall", query: contextQuery, subject_id: subject || null }); if (mounted.current) setContext(v); }); }}><input required aria-label="맥락 조회 검색어" disabled={busy} maxLength={120} value={contextQuery} onChange={(e) => { setContextQuery(e.target.value); setContext(null); }} /><button disabled={busy}>맥락 찾기</button></form>{context && <><p>{context.status === "insufficient-evidence" ? "관련된 유효 기억이 없습니다. 근거가 부족합니다." : `${context.items.length}개 기억을 찾았습니다.`}{context.truncated && " 출력 한도로 일부 결과를 생략했습니다."}</p>{context.items.map((m) => <div className="context-card" key={m.id}><button disabled={busy} onClick={() => onNavigate(m.id)}>{m.title} ↗</button><pre className="source-text">{m.body}</pre><p className="hint">{memoryKindName[m.kind]} · {m.evidence.length ? `출처 근거 ${m.evidence.length}개` : "외부 근거 없는 기억"}</p></div>)}<details><summary>에이전트용 JSON</summary><pre className="source-text">{JSON.stringify(context, null, 2)}</pre></details></>}</section></details>
  </section>;
}
