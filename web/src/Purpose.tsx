import { useEffect, useRef, useState } from "react";
import type { GraphNode, Request, Scope, SubjectDefinition } from "./graph";

export type Subject = { id: string; name: string; revision: number; definition: SubjectDefinition | null };
type Page<T> = { items: T[]; next_after?: string | null; next_before_revision?: number | null };
type Identity = { material_id?: string; entity_id?: string };
type Membership = { revision: number; subject_id: string | null; subject_revision: number | null; current_subject_revision?: number;
  subject_name?: string | null; definition?: SubjectDefinition | null; reason: string | null; review_needed: boolean;
  grouping?: { mode: string; state: string; suggestions: { candidate_ids?: string[]; candidate_names?: Record<string, string> }; reason: string | null } | null;
  current_source: { source_revision: string; content_digest: string } };
type History = { revision: number; subject_name: string | null; reason: string; changed_at: string; definition: SubjectDefinition | null };
type Props = { node: GraphNode; scope: Scope; csrf: string; request: Request; visible: boolean; disabled: boolean;
  onBusy: (value: boolean) => void; onDirtyChange: (value: boolean) => void; onChange: () => void };
export const blankDefinition = (): SubjectDefinition => ({ purpose: "", include: "", exclude: "" });
const bytes = (value: string) => new TextEncoder().encode(value).length;
export const completeDefinition = (value: SubjectDefinition) => Object.entries(value).every(([key, text]) => !!text.trim() && bytes(text) <= (key === "purpose" ? 1024 : 2048));
export function DefinitionFields({ value, onChange }: { value: SubjectDefinition; onChange: (value: SubjectDefinition) => void }) {
  return <>{([['purpose', '목적'], ['include', '포함 기준'], ['exclude', '제외 기준']] as const).map(([key, label]) =>
    <label className="topic-label" key={key}>{label}<textarea rows={2} maxLength={key === "purpose" ? 1024 : 2048} value={value[key]} onChange={event => onChange({ ...value, [key]: event.target.value })} /></label>)}</>;
}
function Definition({ value }: { value?: SubjectDefinition | null }) {
  return value ? <dl><dt>목적</dt><dd>{value.purpose}</dd><dt>포함 기준</dt><dd>{value.include}</dd><dt>제외 기준</dt><dd>{value.exclude}</dd></dl> : <p className="hint">아직 목적과 포함·제외 기준이 정의되지 않았습니다.</p>;
}
const message = (error: unknown) => error instanceof Error ? error.message : "요청에 실패했습니다.";
function useCommand({ scope, csrf, request, onBusy }: Pick<Props, "scope" | "csrf" | "request" | "onBusy">) {
  const mounted = useRef(true), controllers = useRef(new Set<AbortController>());
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; for (const c of controllers.current) c.abort(); onBusy(false); }; }, []);
  return { mounted, command: async <T,>(body: object): Promise<T> => {
    const controller = new AbortController(); controllers.current.add(controller);
    try { return await request<T>("/api/brain", { method: "POST", headers: { "content-type": "application/json", "x-csrf-token": csrf }, body: JSON.stringify({ scope, ...body }), signal: controller.signal }); }
    finally { controllers.current.delete(controller); }
  } };
}
export function SubjectPurpose(props: Props) {
  const { node, disabled, onBusy, onDirtyChange, onChange } = props;
  const { command, mounted } = useCommand(props);
  const [current, setCurrent] = useState<Subject>(() => ({ id: node.id, name: node.label, revision: Number(node.revision ?? 0), definition: node.definition ?? null }));
  const [editing, setEditing] = useState(false), [name, setName] = useState(current.name), [definition, setDefinition] = useState(current.definition ?? blankDefinition());
  const [error, setError] = useState(""); const pending = useRef(false);
  const dirty = editing && (name !== current.name || JSON.stringify(definition) !== JSON.stringify(current.definition ?? blankDefinition()));
  useEffect(() => { onDirtyChange(dirty); return () => onDirtyChange(false); }, [dirty]);
  useEffect(() => {
    if (editing || Number(node.revision ?? 0) < current.revision) return;
    const value = { id: node.id, name: node.label, revision: Number(node.revision ?? 0), definition: node.definition ?? null };
    setCurrent(value); setName(value.name); setDefinition(value.definition ?? blankDefinition());
  }, [node.id, node.label, node.revision, node.definition, editing]);
  async function save() {
    if (pending.current) return;
    pending.current = true; onBusy(true); setError("");
    try {
      const value = await command<Subject>({ op: "subject-define", id: current.id, revision: current.revision, name: name.trim(), definition });
      if (!mounted.current) return;
      setCurrent(value); setName(value.name); setDefinition(value.definition!); setEditing(false); onDirtyChange(false); onChange();
    } catch (error) { if (mounted.current) setError(message(error)); }
    finally { pending.current = false; if (mounted.current) onBusy(false); }
  }
  return <section className="section"><h3>목적 정의</h3><p className="hint">전체 소속 {node.purpose_total ?? 0}개 · 이름을 바꿔도 같은 묶음을 유지합니다.</p>
    {editing ? <form onSubmit={event => { event.preventDefault(); void save(); }}><fieldset disabled={disabled}>
      <label className="topic-label">묶음 이름<input value={name} maxLength={80} onChange={event => setName(event.target.value)} /></label>
      <DefinitionFields value={definition} onChange={setDefinition} /><p className="hint">기준 변경은 기존 소속을 옮기지 않습니다. 연결된 문서는 재확인 상태로 남습니다.</p>
      <button disabled={!name.trim() || !completeDefinition(definition)}>정의 저장</button><button type="button" onClick={() => { setName(current.name); setDefinition(current.definition ?? blankDefinition()); setEditing(false); setError(""); }}>취소</button>
    </fieldset></form> : <><Definition value={current.definition} /><button disabled={disabled} onClick={() => { setName(current.name); setDefinition(current.definition ?? blankDefinition()); setEditing(true); }}>목적 정의 편집</button></>}
    {error && <p className="error" role="alert">{error}</p>}
  </section>;
}
export function DocumentPurpose(props: Props) {
  const { node, disabled, visible, onBusy, onDirtyChange, onChange } = props;
  const { command, mounted } = useCommand(props);
  const identity: Identity = node.material_id ? { material_id: node.material_id } : { entity_id: node.id };
  const [value, setValue] = useState<Membership | null>(null), [editing, setEditing] = useState(false);
  const [subject, setSubject] = useState(""), [reason, setReason] = useState("");
  const [subjects, setSubjects] = useState<Subject[]>([]), [next, setNext] = useState<string | null>(null), [query, setQuery] = useState("");
  const [history, setHistory] = useState<History[] | null>(null), [before, setBefore] = useState<number | null>(null), [error, setError] = useState("");
  const pending = useRef(false);
  const dirty = editing && !!value && (subject !== (value.subject_id ?? "") || reason !== (value.reason ?? ""));
  useEffect(() => { onDirtyChange(dirty); return () => onDirtyChange(false); }, [dirty]);
  async function run(action: () => Promise<void>) {
    if (pending.current || !visible || disabled) return;
    pending.current = true; onBusy(true); setError("");
    try { await action(); } catch (error) { if (mounted.current) setError(message(error)); }
    finally { pending.current = false; if (mounted.current) onBusy(false); }
  }
  async function read() {
    const current = await command<Membership>({ op: "document-subject", document: identity });
    if (!mounted.current) return;
    setValue(current); setSubject(current.subject_id ?? ""); setReason(current.reason ?? ""); setSubjects([]); setNext(null); setEditing(true);
  }
  async function choices(after?: string) {
    const page = await command<Page<Subject>>({ op: "subjects", query, ...(after ? { after } : {}), limit: 20 });
    if (!mounted.current) return;
    setSubjects(items => after ? [...items, ...page.items.filter(item => !items.some(old => old.id === item.id))] : page.items); setNext(page.next_after ?? null);
  }
  const chosen = subjects.find(item => item.id === subject);
  const subjectRevision = chosen?.revision ?? (value?.subject_id === subject ? value.current_subject_revision : undefined);
  async function save() {
    if (!value) return;
    const current = await command<Membership>({ op: "document-subject-set", document: { identity, revision: value.revision,
      ...value.current_source, subject_id: subject || null, subject_revision: subject ? subjectRevision : null, reason } });
    if (!mounted.current) return;
    setValue(current); setEditing(false); setHistory(null); onDirtyChange(false); onChange();
  }
  async function past(cursor?: number) {
    const page = await command<Page<History>>({ op: "document-subject-history", document: identity, ...(cursor ? { before_revision: cursor } : {}), limit: 20 });
    if (!mounted.current) return;
    setHistory(items => cursor ? [...(items ?? []), ...page.items] : page.items); setBefore(page.next_before_revision ?? null);
  }
  async function retry() {
    if (!value || dirty) return;
    const current = await command<Membership>({ op: "document-grouping-retry", document: { identity, revision: value.revision, ...value.current_source } });
    if (!mounted.current) return;
    setValue(current); onChange();
  }
  const groupingLabel: Record<string, string> = { pending: "분류 대기", processing: "분류 중", assigned: "자동 배정", suggested: "후보 검토 필요", unmatched: "적합한 목적 없음", error: "수동 검토 필요", manual: "수동 소속", off: "자동 분류 제외", ineligible: "현재 원문 확인 필요" };
  return <section className="section document-purpose"><h3>목적 소속</h3>
    <p>{value ? value.subject_name ?? "미분류" : node.subject_name ?? "미분류"}</p>
    {(value?.review_needed || node.classification_review_needed) && <p className="warning">원문 또는 목적 정의가 바뀌었습니다. 현재 내용을 읽고 소속을 재확인해 주세요.</p>}
    {value?.grouping && <div className="hint"><p>{groupingLabel[value.grouping.state] ?? "분류 상태 확인 필요"}</p>
      {value.grouping.reason && <p>{value.grouping.reason}</p>}
      {!!value.grouping.suggestions.candidate_ids?.length && <p>후보: {value.grouping.suggestions.candidate_ids.map(id => value.grouping?.suggestions.candidate_names?.[id] ?? "목적 그룹").join(", ")}</p>}
    </div>}
    {value && !editing && <><Definition value={value.definition} />{value.reason && <p className="hint">분류 이유: {value.reason}</p>}</>}
    {!editing ? <button disabled={disabled} onClick={() => void run(read)}>목적 소속 관리</button> : value && <form onSubmit={event => { event.preventDefault(); void run(save); }}><fieldset disabled={disabled}>
      <Definition value={chosen?.definition ?? (value.subject_id === subject ? value.definition : null)} />
      <label className="topic-label">목적 묶음<select value={subject} onFocus={() => { if (!subjects.length) void run(() => choices()); }} onChange={event => setSubject(event.target.value)}>
        <option value="">미분류</option>{value.subject_id && !subjects.some(item => item.id === value.subject_id) && <option value={value.subject_id}>{value.subject_name ?? "현재 묶음"}</option>}
        {subjects.map(item => <option key={item.id} value={item.id}>{item.name}{!item.definition ? " · 정의 없음" : ""}</option>)}
      </select></label>
      <div className="related-search"><input aria-label="목적 묶음 검색" maxLength={120} value={query} onChange={event => setQuery(event.target.value)} /><button type="button" onClick={() => void run(() => choices())}>묶음 찾기</button></div>
      {next && <button type="button" onClick={() => void run(() => choices(next))}>묶음 더 보기</button>}
      <label className="topic-label">분류 이유<textarea rows={3} value={reason} maxLength={2048} onChange={event => setReason(event.target.value)} /></label>
      <button disabled={!reason.trim() || bytes(reason) > 2048 || (!!subject && subjectRevision === undefined)}>소속 저장</button>
      <button type="button" onClick={() => { setEditing(false); setError(""); }}>취소</button><button type="button" onClick={() => void run(read)}>현재 소속 다시 읽기</button>
    </fieldset></form>}
    <button disabled={disabled} onClick={() => void run(() => past())}>소속 이력</button>
    {value && <button disabled={disabled || dirty || ["pending", "processing"].includes(value.grouping?.state ?? "")} onClick={() => void run(retry)}>목적 자동 재검토</button>}
    {history && <><ol>{history.map(item => <li key={item.revision}><strong>{item.subject_name ?? "미분류"}</strong> · {new Date(item.changed_at).toLocaleString("ko-KR")}<p>{item.reason}</p><details><summary>당시 목적 정의</summary><Definition value={item.definition} /></details></li>)}</ol>{before && <button disabled={disabled} onClick={() => void run(() => past(before))}>이력 더 보기</button>}</>}
    {error && <p className="error" role="alert">{error}</p>}
  </section>;
}
