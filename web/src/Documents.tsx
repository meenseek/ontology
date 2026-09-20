import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import type { Request, Scope } from "./graph";
import { fileName } from "./presentation";
import DocumentPreview from "./DocumentPreview";
type SourceKind = "git" | "vault" | "context";
type Area = { id: string; label: string };
type RecordItem = {
  id: string;
  path: string;
  kind: SourceKind;
  areas: string[];
  topics: string[];
  excerpt: string;
  status: string;
  present: boolean;
};
type Listing = {
  items: RecordItem[];
  total: number;
  limit: number;
  areas: { area: string; count: number }[];
};
type Detail = {
  scope: Scope;
  id: string;
  revision: number;
  current: boolean;
  areas: string[];
  topics: string[];
  source: {
    kind: SourceKind;
    repository: string;
    path: string;
    status: string;
    last_success_at: string | null;
  };
  projection: {
    content: string | null;
    present: boolean;
  };
  related: { id: string; path: string }[];
  history: {
    id: number;
    kind: string;
    revision: number;
  current: boolean;
    previous: unknown;
    confirmed: unknown;
    confirmed_at: string;
  }[];
};

type Props = { visible: boolean; managing: boolean; scope: Scope; id: string; csrf: string; allAreas: Area[]; request: Request; onBusy: (busy: boolean) => void; onChange: () => void; onNavigate: (id: string) => void };
const date = (value: string | null) => value ? new Date(value).toLocaleString("ko-KR") : "아직 확인되지 않음";
const message = (error: unknown) => error instanceof Error ? error.message : "요청을 완료하지 못했습니다.";
export default function Documents({ visible, managing, scope, id, csrf, allAreas, request, onBusy, onChange, onNavigate }: Props) {
  const [detail, setDetail] = useState<Detail | null>(null), [areas, setAreas] = useState<string[]>([]), [topics, setTopics] = useState("");
  const [relatedQuery, setRelatedQuery] = useState(""), [candidates, setCandidates] = useState<RecordItem[]>([]), [target, setTarget] = useState("");
  const [saving, setSaving] = useState(false), [error, setError] = useState(""), [notice, setNotice] = useState("");
  const [loading, setLoading] = useState(true), [candidateTotal, setCandidateTotal] = useState<number | null>(null);
  const [refresh, setRefresh] = useState(0);
  const mounted = useRef(true), pending = useRef(false), requests = useRef(new Set<AbortController>());
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; for (const request of requests.current) request.abort(); onBusy(false); };
  }, []);
  useEffect(() => {
    // A loaded editor keeps its original revision and unsaved fields on resume.
    // Explicit refresh/remount still reads a fresh record after a save or conflict.
    if (!visible || detail || error) return;
    const controller = new AbortController(); setLoading(true);
    request<Detail>(`/api/records/${id}?scope=${scope}`, { signal: controller.signal }).then(v => {
      if (!controller.signal.aborted) { setDetail(v); setAreas(v.areas); setTopics(v.topics.join("\n")); }
    }).catch(e => { if (!controller.signal.aborted) setError(message(e)); }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [scope, id, request, visible, refresh]);
  function reload() { setLoading(true); setDetail(null); setError(""); setRefresh(value => value + 1); }
  const areaName = (id: string) => allAreas.find(area => area.id === id)?.label ?? id;
  function historyValue(value: unknown): string {
    if (!value || typeof value !== "object") return "기록 없음";
    const data = value as { areas?: string[]; topics?: string[]; target_id?: string; linked?: boolean };
    if (Array.isArray(data.areas) && Array.isArray(data.topics)) return [data.areas.length ? `분야: ${data.areas.map(areaName).join(" · ")}` : "", data.topics.length ? `태그: ${data.topics.join(" · ")}` : ""].filter(Boolean).join(" / ") || "분류 없음";
    return data.linked === true ? "관련 자료로 연결" : data.linked === false ? "연결되지 않음" : "연결 기록";
  }
  function historyTarget(value: unknown): string | null {
    return value && typeof value === "object" && "target_id" in value && typeof value.target_id === "string" ? value.target_id : null;
  }
  const choose = (next: string) => { if (!saving) onNavigate(next); };
  async function run(action: (signal: AbortSignal) => Promise<void>) {
    if (pending.current) return; pending.current = true; setSaving(true); onBusy(true); setError(""); setNotice("");
    const controller = new AbortController(); requests.current.add(controller);
    try { await action(controller.signal); } catch (e) { if (mounted.current) setError(message(e)); }
    finally { requests.current.delete(controller); pending.current = false; if (mounted.current) { setSaving(false); onBusy(false); } }
  }
  async function save(kind: "classification" | "links", body: object) {
    if (!detail) return;
    await run(async signal => {
      await request(`/api/records/${id}/${kind}?scope=${scope}`, { method: "POST", headers: { "content-type": "application/json", "x-csrf-token": csrf }, body: JSON.stringify({ revision: detail.revision, ...body }), signal });
      if (mounted.current) { setNotice(kind === "classification" ? "분류 확인을 저장했습니다." : "자료 연결을 변경했습니다."); onChange(); }
    });
  }
  async function findRelated(event: FormEvent) {
    event.preventDefault();
    await run(async signal => {
      const result = await request<Listing>(`/api/records?scope=${scope}&q=${encodeURIComponent(relatedQuery)}`, { signal });
      if (mounted.current) { setCandidates(result.items.filter(item => item.id !== id)); setCandidateTotal(result.total); setTarget(""); }
    });
  }
  return <section className="documents" aria-busy={loading}>
    {error && <p className="error" role="alert">{error}<button disabled={saving} onClick={reload}>다시 불러오기</button></p>}
    {notice && <p className="notice">{notice}</p>}
    {loading ? <p role="status">자료를 불러오는 중…</p> : detail && <>
      <p className="source-identity">{detail.source.kind === "vault" ? "Vault" : detail.source.kind === "context" ? "Context" : "Git"} 문서</p>
      {detail.source.status === "failed" && <p className="error">최근 출처 확인에 실패했습니다. 아래 내용은 마지막으로 성공한 기록입니다.</p>}
      {!detail.projection.present && <p className="warning">등록한 경로의 부재를 확인했습니다. 마지막 원문과 사용자의 확인 기록은 보존되어 있습니다.</p>}
      {detail.source.status === "ok" && detail.projection.present && !detail.current && <p className="warning">갱신 대기 · 아래 내용은 마지막으로 성공한 기록입니다.</p>}
      <DocumentPreview path={detail.source.path} content={detail.projection.content} kind={detail.source.kind} />
      {detail.related.length > 0 && <section className="section">
        <h3>관련 자료 <span>{detail.related.length}개 연결</span></h3>
        <ul className="related">{detail.related.map(item => <li key={item.id}><button disabled={saving} onClick={() => choose(item.id)}><strong>{fileName(item.path)}</strong><span className="node-location">{item.path}</span></button></li>)}</ul>
      </section>}
      {(detail.areas.length > 0 || detail.topics.length > 0) && <p className="classification-summary">{[...detail.areas.map(areaName), ...detail.topics].join(" · ")}</p>}
      <div id="record-management" className="record-management" hidden={!managing}>
      <section className="section provenance">
        <h3>출처</h3>
        <dl>
          <dt>위치</dt><dd>{detail.source.repository}/{detail.source.path}</dd>
          <dt>최근 확인</dt><dd>{date(detail.source.last_success_at)}</dd>
        </dl>
        <p className="hint">최근 확인은 원본을 읽은 시점이며, 내용 수정일은 아닙니다.</p>
        <button disabled={saving} onClick={reload}>다시 불러오기</button>
      </section>
      <details className="section">
        <summary>분류 수정</summary>
        <p className="hint">문서의 분야와 태그를 확인합니다. 원문 내용은 바꾸지 않습니다.</p>
        <form onSubmit={event => { event.preventDefault(); void save("classification", { areas, topics: topics.split("\n").map(v => v.trim()).filter(Boolean) }); }}>
          <fieldset disabled={saving}>
            <legend>{scope === "meenseek" ? "분야 · 여러 개 선택 가능" : "개인 자료의 문서 태그"}</legend>
            {scope === "meenseek" && <div className="area-options">{allAreas.map(area => <label key={area.id}><input type="checkbox" checked={areas.includes(area.id)} onChange={event => setAreas(current => event.target.checked ? [...current, area.id] : current.filter(id => id !== area.id))} />{area.label}</label>)}</div>}
            <label className="topic-label" htmlFor="topics">문서 태그 · 한 줄에 하나, 최대 10개</label>
            <textarea id="topics" value={topics} onChange={event => setTopics(event.target.value)} rows={3} maxLength={810} placeholder="태그가 없으면 비워두세요." />
            <button className="primary" type="submit">{saving ? "저장 중…" : "분류 확인 저장"}</button>
          </fieldset>
        </form>
      </details>
      <details className="section">
        <summary>자료 연결 수정</summary>
        <p className="hint">같은 범위의 문서 사이에 관련 자료 관계를 추가하거나 해제합니다.</p>
        {detail.related.length > 0 && <ul className="related">{detail.related.map(item => <li key={item.id}><span>{item.path}</span><button disabled={saving} aria-label={`${item.path} 연결 해제`} onClick={() => void save("links", { target_id: item.id, remove: true })}>연결 해제</button></li>)}</ul>}
        <form className="related-search" onSubmit={findRelated}><input aria-label="관련 자료 검색" maxLength={120} value={relatedQuery} onChange={event => setRelatedQuery(event.target.value)} placeholder="연결할 자료 검색" /><button disabled={saving}>찾기</button></form>
        {candidateTotal !== null && <p className="hint">검색 결과 {candidateTotal}개 · 앞의 100개까지 연결 후보로 표시합니다. 경로나 검색어를 좁혀 찾을 수 있습니다.</p>}
        <div className="link-controls"><select aria-label="연결할 자료" value={target} disabled={saving} onChange={event => setTarget(event.target.value)}><option value="">연결할 자료 선택</option>{candidates.map(item => <option key={item.id} value={item.id}>{item.path}</option>)}</select><button disabled={!target || saving} onClick={() => void save("links", { target_id: target, remove: false })}>연결 추가</button></div>
      </details>
      {detail.history.length > 0 && <details className="section">
        <summary>분류·연결 변경 이력</summary>
        <p className="hint">최근 변경 30건까지 표시합니다.</p>
        {detail.history.map(history => { const targetId = historyTarget(history.confirmed); return <div className="history" key={history.id}><b>{history.kind === "classification" ? "분류 확인" : history.kind === "link-add" ? "관련 자료 추가" : "관련 자료 해제"}</b><time>{date(history.confirmed_at)}</time><p>{historyValue(history.previous)} → {historyValue(history.confirmed)}</p>{targetId && <button disabled={saving} onClick={() => choose(targetId)}>연결 대상 보기</button>}</div>; })}
      </details>}
      </div>
    </>}
  </section>;
}
