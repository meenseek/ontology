import { Component, Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import Documents from "./Documents";
import Memory from "./Memory";
import { nodePresentation } from "./presentation";
import { graphUrl, kindName, knowledge, linkName, parseLocation, reconcile, sameGraphLocation, stateName, visibleGraph } from "./graph";
import type { Filters, Model, Scope, Snapshot } from "./graph";
const Graph = lazy(() => import("./Graph.tsx"));
type Session = { csrf: string; areas: { id: string; label: string }[] };
class ApiError extends Error {
  constructor(public status: number) {
    super(status === 409 ? "다른 변경이 먼저 저장되었습니다. 다시 읽은 뒤 확인해 주세요." : status === 403 ? "세션을 확인할 수 없습니다. 페이지를 새로고침해 주세요." : status === 404 ? "자료가 없거나 이 범위에 속하지 않습니다." : "요청을 완료하지 못했습니다. 입력과 서버 상태를 확인해 주세요.");
  }
}
async function request<T>(url: string, options?: RequestInit): Promise<T> {
  const response = await fetch(url, { credentials: "same-origin", ...options });
  if (!response.ok) throw new ApiError(response.status);
  return response.json() as Promise<T>;
}
const message = (e: unknown) => e instanceof Error ? e.message : "요청에 실패했습니다.";
class GraphBoundary extends Component<{ children: ReactNode; onFailure: () => void }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch() { this.props.onFailure(); }
  render() { return this.state.failed ? null : this.props.children; }
}
const initialFilters: Filters = { kind: "all", state: "all", cluster: null };
export default function App() {
  const [route, setRoute] = useState(() => parseLocation(window.location.search));
  const routeRef = useRef(route); routeRef.current = route;
  const [session, setSession] = useState<Session | null>(null);
  const [stored, setStored] = useState<{ snapshot: Snapshot; model: Model } | null>(null);
  const previous = useRef<Model | undefined>(undefined);
  const data = stored?.snapshot.scope === route.scope && stored.snapshot.query === route.q ? stored : null;
  const [input, setInput] = useState(route.q), [refresh, setRefresh] = useState(0), [panelEpoch, setPanelEpoch] = useState(0);
  const [filters, setFilters] = useState<Filters>(initialFilters);
  const [panel, setPanel] = useState<"node" | "manage" | null>(route.focus ? "node" : null);
  const [busy, setBusy] = useState(false), [loading, setLoading] = useState(true), [error, setError] = useState("");
  const [notice, setNotice] = useState(""), [listMode, setListMode] = useState(false), [webglFailed, setWebglFailed] = useState(false);
  const [visible, setVisible] = useState(document.visibilityState !== "hidden");
  const [reduced, setReduced] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  const [rotate, setRotate] = useState(false), [fit, setFit] = useState(0);
  const rotating = rotate && !reduced && !route.focus;
  const [explore, setExplore] = useState(false);
  const panelElement = useRef<HTMLElement>(null), closeButton = useRef<HTMLButtonElement>(null), manageButton = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null), filterButton = useRef<HTMLButtonElement>(null);
  const [narrow, setNarrow] = useState(() => window.matchMedia("(max-width: 900px)").matches);
  const selected = data?.model.nodes.find(n => n.id === route.focus) ?? null;
  const shown = useMemo(() => data ? visibleGraph(data.model, filters) : { nodes: [], links: [] }, [data, filters]);
  const fallback = listMode || webglFailed;
  const failed = useCallback(() => { setWebglFailed(true); }, []);
  const syncRoute = (next: typeof route, replace = false) => {
    const current = routeRef.current;
    if ((!data || loading) && next.scope === current.scope && next.q === current.q && next.focus !== current.focus) setRefresh(v => v + 1);
    window.history[replace ? "replaceState" : "pushState"](null, "", graphUrl(next.scope, next.q, next.focus));
    routeRef.current = next; setRoute(next);
  };
  useEffect(() => {
    const visibility = () => setVisible(document.visibilityState !== "hidden");
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const motion = () => setReduced(media.matches);
    const pop = () => { const next = parseLocation(window.location.search); routeRef.current = next; setRoute(next); setInput(next.q); setFilters(initialFilters); setPanel(next.focus ? "node" : null); setRefresh(v => v + 1); setPanelEpoch(v => v + 1); };
    document.addEventListener("visibilitychange", visibility); media.addEventListener("change", motion); window.addEventListener("popstate", pop);
    return () => { document.removeEventListener("visibilitychange", visibility); media.removeEventListener("change", motion); window.removeEventListener("popstate", pop); };
  }, []);
  useEffect(() => {
    if (!visible || session) return;
    const controller = new AbortController();
    request<Session>("/api/session", { signal: controller.signal }).then(v => { if (!controller.signal.aborted) setSession(v); }).catch(e => { if (!controller.signal.aborted) { setError(message(e)); setLoading(false); } });
    return () => controller.abort();
  }, [visible, session, refresh]);
  useEffect(() => {
    if (!visible || !session) return;
    const controller = new AbortController(); const current = routeRef.current;
    setLoading(true); setError("");
    const params = new URLSearchParams({ scope: current.scope, q: current.q });
    if (current.focus) params.set("focus", current.focus);
    request<Snapshot>(`/api/graph?${params}`, { signal: controller.signal }).then(snapshot => {
      if (controller.signal.aborted || !sameGraphLocation(current, routeRef.current)) return;
      const model = reconcile(snapshot, previous.current); previous.current = model;
      setStored({ snapshot, model });
      setFilters(f => f.cluster && !model.clusters.some(c => c.id === f.cluster) ? { ...f, cluster: null } : f);
      const focus = routeRef.current.focus;
      if (focus && !model.nodes.some(n => n.id === focus)) {
        setNotice("초점을 지정한 자료가 없거나 이 범위에서 찾을 수 없습니다.");
        syncRoute({ ...routeRef.current, focus: null }, true); setPanel(null);
      }
    }).catch(e => { if (!controller.signal.aborted && sameGraphLocation(current, routeRef.current)) setError(message(e)); }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [visible, session, route.scope, route.q, refresh]);
  const reload = () => { setRefresh(v => v + 1); setPanelEpoch(v => v + 1); setNotice(""); };
  function changeScope(scope: Scope) {
    if (busy || scope === route.scope) return;
    previous.current = undefined; setStored(null); setInput(""); setFilters(initialFilters); setPanel(null); setNotice(""); setError("");
    syncRoute({ scope, q: "", focus: null });
  }
  function choose(id: string) {
    if (busy) return;
    const node = data?.model.nodes.find(n => n.id === id);
    setNotice(""); setFilters(f => ({ ...initialFilters, cluster: f.cluster === node?.cluster ? f.cluster : null }));
    syncRoute({ ...route, focus: id }); openPanel("node");
    if (!node) setRefresh(v => v + 1);
  }
  function changed(id: string | null) {
    syncRoute({ ...routeRef.current, focus: id }, true);
    setPanel(id ? "node" : "manage"); setFilters(initialFilters); reload();
  }
  function openPanel(next: "node" | "manage") {
    if (!panel) opener.current = document.activeElement instanceof HTMLElement && document.activeElement.matches("button, a[href], input, select, textarea, summary, [tabindex]") ? document.activeElement : manageButton.current;
    setPanel(next);
  }
  function fitView(cluster = filters.cluster) {
    if (busy) return;
    setFilters(f => ({ ...f, cluster }));
    closePanel(); setFit(v => v + 1);
  }
  function closePanel() {
    if (busy) return;
    setPanel(null); syncRoute({ ...routeRef.current, focus: null });
    requestAnimationFrame(() => (opener.current?.isConnected ? opener.current : manageButton.current)?.focus());
  }
  useEffect(() => {
    const media = window.matchMedia("(max-width: 900px)");
    const resize = () => setNarrow(media.matches);
    media.addEventListener("change", resize); return () => media.removeEventListener("change", resize);
  }, []);
  useEffect(() => { if (panel) closeButton.current?.focus(); }, [!!panel, !!session]);
  useEffect(() => {
    if (!panel) return;
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) { event.preventDefault(); closePanel(); }
      if (event.key !== "Tab" || !narrow) return;
      const controls = [...(panelElement.current?.querySelectorAll<HTMLElement>('button:not(:disabled), a[href], input:not(:disabled), textarea:not(:disabled), select:not(:disabled), summary, [tabindex="0"]') ?? [])].filter(el => {
        if (!el.getClientRects().length || getComputedStyle(el).visibility !== "visible") return false;
        for (let ancestor: HTMLElement | null = el; ancestor; ancestor = ancestor.parentElement) {
          if (ancestor.hidden || ancestor.inert || getComputedStyle(ancestor).opacity === "0") return false;
          if (ancestor instanceof HTMLDetailsElement && !ancestor.open) {
            const summary = [...ancestor.children].find(child => child.tagName === "SUMMARY");
            if (!summary?.contains(el)) return false;
          }
        }
        return true;
      });
      const first = controls[0], last = controls.at(-1);
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    };
    document.addEventListener("keydown", keydown); return () => document.removeEventListener("keydown", keydown);
  }, [panel, narrow, busy]);
  const cluster = data?.model.clusters.find(c => c.id === filters.cluster);
  return <div className="app">
    <header className="app-header" inert={!!panel && narrow}>
      <a className="wordmark" href="/">ontology<span> / meenseek</span></a>
      <div className="scope-switch" role="group" aria-label="탐색 범위">
        <button aria-pressed={route.scope === "meenseek"} disabled={busy} onClick={() => changeScope("meenseek")}>meenseek</button>
        <button aria-pressed={route.scope === "personal"} disabled={busy} onClick={() => changeScope("personal")}>개인</button>
      </div>
      <form className="map-search" role="search" onSubmit={e => { e.preventDefault(); if (busy) return; const same = route.q === input; syncRoute({ scope: route.scope, q: input, focus: null }); setPanel(null); setFilters(initialFilters); setNotice(""); if (same) setRefresh(v => v + 1); }}>
        <label className="sr-only" htmlFor="graph-search">현재 범위의 문서·기억 검색</label>
        <input id="graph-search" value={input} onChange={e => setInput(e.target.value)} maxLength={120} placeholder="문서·기억 검색" />
        <button type="submit" disabled={busy || !session}>검색</button>
      </form>
      <button ref={manageButton} className="manage-button" disabled={!session || busy} onClick={() => { syncRoute({ ...route, focus: null }); openPanel("manage"); }}>기억 쓰기</button>
    </header>
    <main className={`map-workspace ${panel ? "panel-open" : ""} ${explore ? "explorer-open" : ""}`}>
      {explore && <aside className="map-sidebar" aria-label="지식 탐색" inert={!!panel && narrow}>
        <div className="sidebar-heading"><h2>탐색 조건</h2><button className="quiet" aria-label="탐색 조건 닫기" onClick={() => { setExplore(false); filterButton.current?.focus(); }}>닫기</button></div>
        <div className="filter-row">
          <label>표시 종류<select value={filters.kind} disabled={busy} onChange={e => setFilters(f => ({ ...f, kind: e.target.value as Filters["kind"] }))}><option value="all">모두</option><option value="knowledge">문서와 기억</option>{Object.entries(kindName).map(([id, name]) => <option key={id} value={id}>{name}</option>)}</select></label>
          <label>표시 상태<select value={filters.state} disabled={busy} onChange={e => setFilters(f => ({ ...f, state: e.target.value as Filters["state"] }))}><option value="all">모든 상태</option><option value="active">현재 사용 가능</option><option value="proposed">제안</option><option value="withdrawn">철회</option><option value="attention">확인 필요·제외</option></select></label>
        </div>
        <section className="cluster-list"><h2>관계에서 찾은 군집</h2>
          <button className="cluster" aria-pressed={!filters.cluster} disabled={busy} onClick={() => fitView(null)}>전체 보기</button>
          <div className="compact-list">{data?.model.clusters.filter(c => c.members.length > 1).map(c => <button className="cluster" key={c.id} disabled={busy} aria-pressed={filters.cluster === c.id} onClick={() => fitView(c.id)}><i style={{ background: c.color }} /><strong>{c.label}</strong><small>문서·기억 {c.knowledge} · 분류 표식 {c.members.length - c.knowledge}</small></button>)}</div>
          <p className="hint">{data?.model.clusters.filter(c => c.members.length > 1).length ?? 0}개 군집 · 단독 항목 {data?.model.clusters.filter(c => c.members.length === 1).length ?? 0}개</p>
          <p className="hint">표시된 현재 관계로 계산합니다. 군집은 자동 분류가 아닙니다.</p>
        </section>
        <details className="section"><summary>검색 방법</summary><p className="hint">현재 범위에서 제목·내용·경로·문서 태그의 문자열을 찾습니다. 표시 조건은 조회한 결과 안에서 적용합니다.</p></details>
      </aside>}
      <section className="galaxy" aria-label="지식 지도" aria-busy={loading} inert={!!panel && narrow}>
        <div className="map-toolbar"><div className="map-title"><h1>{cluster ? cluster.label : route.q ? `“${route.q}” 검색` : "지식 지도"}</h1><p className="count-breakdown">문서 {data?.snapshot.totals.documents ?? "—"} · 기억 {data?.snapshot.totals.memories ?? "—"} · 관계 {data?.snapshot.totals.links ?? "—"}</p></div>
          <div className="map-actions">
            <button ref={filterButton} disabled={busy} aria-expanded={explore} onClick={() => setExplore(v => !v)}>필터·군집</button>
            <button disabled={busy} aria-pressed={fallback} onClick={() => { if (webglFailed) { setWebglFailed(false); setListMode(false); } else setListMode(v => !v); }}>{fallback ? "3D 보기" : "목록 보기"}</button>
            <details className="view-options"><summary>보기 설정</summary><div>
              <button disabled={busy || loading} aria-busy={loading} onClick={reload}>{loading ? "새로고침 중…" : "새로고침"}</button>
              {!fallback && <><button disabled={busy} onClick={() => fitView()}>전체 맞춤</button><button disabled={reduced || !!route.focus} aria-pressed={rotating} title={route.focus ? "자료 선택을 해제하면 설정한 지도 회전이 재개됩니다." : undefined} onClick={() => setRotate(v => !v)}>{reduced ? "동작 줄이기 · 지도 회전 멈춤" : route.focus ? "자료 선택 중 · 지도 회전 멈춤" : rotating ? "지도 회전 멈춤" : "지도 천천히 회전"}</button></>}
            </div></details>
          </div>
        </div>
        <div className="map-status" aria-live="polite">
          {error && <p className="error" role="alert">{error} <button onClick={reload}>다시 불러오기</button></p>}
          {notice && <p className="notice">{notice}</p>}
          {loading && !data && <p className="loading" role="status">지식을 불러오는 중…</p>}
          {webglFailed && <p className="warning">3D 화면을 표시할 수 없어 같은 조회 결과를 목록으로 보여드립니다.</p>}
          {(route.q || cluster) && <div className="active-filters">{route.q && <button className="quiet" disabled={busy} onClick={() => { setInput(""); syncRoute({ scope: route.scope, q: "", focus: null }); setFilters(initialFilters); setPanel(null); }}>검색 해제</button>}{cluster && <button className="quiet" disabled={busy} onClick={() => fitView(null)}>군집 해제</button>}</div>}
          {data && <p className="data-limits">표시: 문서·기억 {shown.nodes.filter(knowledge).length} · 분류 표식 {shown.nodes.filter(n => !knowledge(n)).length} · 관계 {shown.links.length}{route.q && ` · 검색 일치 ${data.snapshot.matched}개`}</p>}
          {data && <details className="limits-detail"><summary>{data.snapshot.truncated ? "반환 한도로 일부 생략 · 조회 범위" : "조회 범위"}</summary><p>범위 전체: 문서 {data.snapshot.totals.documents}개 · 기억 {data.snapshot.totals.memories}개 · 분류 표식 {data.snapshot.totals.markers}개 · 관계 {data.snapshot.totals.links}개. 조회 결과: 문서·기억 {data.snapshot.returned.knowledge}개 · 표식 {data.snapshot.returned.markers}개 · 관계 {data.snapshot.returned.links}개. 한 번에 점 {data.snapshot.limits.nodes}개, 관계 {data.snapshot.limits.links}개, {data.snapshot.limits.response_bytes.toLocaleString()}바이트까지 반환합니다. 현재 조회에서 점 {data.snapshot.omitted.nodes}개·관계 {data.snapshot.omitted.links}개가 제외되었습니다{route.q ? " (검색 범위 밖 포함)" : ""}.{data.snapshot.limits.byte_limited && " 응답 크기 상한도 적용했습니다."} 범위 검색과 자료 초점 URL로 상한 밖 자료를 찾을 수 있습니다.</p></details>}
          {data && shown.nodes.length > 0 && !shown.links.length && <p className="hint no-relations">{data.snapshot.totals.links === 0 ? "아직 등록된 관계가 없습니다." : "현재 표시한 항목 사이에는 조회된 관계가 없습니다."}</p>}
          {selected && !shown.nodes.some(n => n.id === selected.id) && <p className="notice">선택한 자료가 현재 필터 밖에 있습니다. <button onClick={() => setFilters(initialFilters)}>필터 해제</button></p>}
        </div>
        {data && !loading && !shown.nodes.length ? <div className="map-empty"><h2>표시할 항목이 없습니다.</h2><p>{route.q || filters.kind !== "all" || filters.state !== "all" ? "검색어와 표시 조건을 바꿔보세요." : "등록한 문서와 보관한 기억이 이곳에 나타납니다."}</p><button disabled={!session || busy} onClick={() => openPanel("manage")}>기억 쓰기</button></div> : data && (fallback ? <div className="graph-list" aria-label="지식 지도 목록">{shown.nodes.map(n => { const label = nodePresentation(n); return <button key={n.id} disabled={busy} aria-pressed={route.focus === n.id} onClick={() => choose(n.id)}><span className={`node-symbol ${n.kind}`} aria-hidden="true">{knowledge(n) ? "·" : "○"}</span><span><small>{kindName[n.kind]} · {stateName(n)}</small><strong>{label.title}</strong>{label.subtitle && <span className="node-location">{label.subtitle}</span>}</span>{n.changed && <em>변경</em>}</button>; })}</div> : <GraphBoundary onFailure={failed}><Suspense fallback={<p className="loading">3D 화면 준비 중…</p>}><Graph nodes={shown.nodes} links={shown.links} selected={route.focus} rotate={rotating} reduced={reduced} visible={visible} fit={fit} disabled={busy} onSelect={choose} onFailure={failed} /></Suspense></GraphBoundary>)}
        <div className="map-legend"><span><i className="legend-star document" />문서</span><span><i className="legend-star memory" />기억</span><span>○ 분류 표식</span><details><summary>관계·상태 읽기</summary><p>가는 원은 선택, 바깥 점선 원은 이전 조회 이후의 기록·관계 변경입니다. 흐린 점은 제안·철회·유효기간·출처 확인 상태를 살펴보세요. {Object.values(linkName).join(" · ")} 관계만 선으로 표시하며 선택하면 연결된 선을 강조합니다. 과거 출처 근거는 갈색의 가는 선이며 군집에서 제외합니다. ‘보관’은 사실 검증을 뜻하지 않습니다. 출처 확인 시각만 바뀌면 변경으로 표시하지 않습니다.</p></details></div>
      </section>
      {panel && session && <aside ref={panelElement} className="management-panel" role="dialog" aria-modal={narrow || undefined} aria-label={panel === "manage" ? "기억 쓰기" : "선택한 자료 상세"}>
        <div className="panel-heading"><span>{panel === "manage" ? "기억 쓰기" : selected ? kindName[selected.kind] : "자료 상세"}</span><button ref={closeButton} disabled={busy} aria-label="관리 패널 닫기" onClick={closePanel}>닫기 ×</button></div>
        {panel === "manage" ? <><Memory visible={visible} key={`${route.scope}:manage:${panelEpoch}`} scope={route.scope} csrf={session.csrf} request={request} selectedId={null} onBusy={setBusy} onChange={changed} onNavigate={choose} onMetadataChange={() => setRefresh(v => v + 1)} /><SyncPanel key={panelEpoch} visible={visible} /></> : !selected ? <p className="empty">선택한 자료를 불러오는 중…</p> : <>
          {selected.changed && <p className="notice">이전 조회 이후 기록이나 관계가 변경되었습니다.</p>}
          {selected.kind === "document" ? <Documents visible={visible} key={`${route.scope}:${selected.id}:${panelEpoch}`} scope={route.scope} id={selected.id} csrf={session.csrf} allAreas={session.areas} request={request} onBusy={setBusy} onChange={reload} onNavigate={choose} /> : selected.kind === "memory" ? <Memory visible={visible} key={`${route.scope}:${selected.id}:${panelEpoch}`} scope={route.scope} csrf={session.csrf} request={request} selectedId={selected.id} onBusy={setBusy} onChange={changed} onNavigate={choose} onMetadataChange={() => setRefresh(v => v + 1)} /> : <><h2>{selected.label}</h2><p className="hint">{kindName[selected.kind]} 표식입니다.</p><button onClick={() => fitView(selected.cluster)}>이 군집 보기</button><div className="compact-list">{data?.model.links.filter(l => l.source === selected.id || l.target === selected.id).map(l => { const other = data.model.nodes.find(n => n.id === (l.source === selected.id ? l.target : l.source)); return other && <button key={`${l.kind}:${other.id}`} disabled={busy} onClick={() => choose(other.id)}><span>{linkName[l.kind]}</span><strong>{nodePresentation(other).title}</strong>{nodePresentation(other).subtitle && <small>{nodePresentation(other).subtitle}</small>}</button>; })}</div></>}
          <details className="section node-permalink"><summary>자료 링크</summary><p><a href={graphUrl(route.scope, route.q, selected.id)}>이 자료의 초점 URL</a></p><code>{selected.id}</code></details>
        </>}
      </aside>}
    </main>
  </div>;
}

type SyncStatus = { enabled: boolean; running: boolean; last_completed_at: number | null; error: string | null; report: { ok: boolean; sources: { index: number; kind: string; documents: number; ok: boolean }[] } | null };
function SyncPanel({ visible }: { visible: boolean }) {
  const [value, setValue] = useState<SyncStatus | null>(null), [error, setError] = useState(""), [refresh, setRefresh] = useState(0);
  useEffect(() => { if (!visible) return; const controller = new AbortController(); request<SyncStatus>("/api/sync", { signal: controller.signal }).then(v => { if (!controller.signal.aborted) setValue(v); }).catch(e => { if (!controller.signal.aborted) setError(message(e)); }); return () => controller.abort(); }, [visible, refresh]);
  return <details className="section"><summary>출처 갱신 · {value ? value.enabled ? value.running ? "확인 중" : value.error || value.report?.ok === false ? "확인 필요" : "실행 중" : "꺼짐" : "불러오는 중"}</summary><p className="hint">앱 실행 중 지정된 원문만 갱신합니다. 기억 내용은 자동으로 바꾸지 않습니다.</p>{error && <p className="error">{error}</p>}{value?.last_completed_at && <p className="hint">최근 확인: {new Date(value.last_completed_at * 1000).toLocaleString("ko-KR")}</p>}{value?.report?.sources.map(s => <p className={s.ok ? "hint" : "warning"} key={s.index}>설정 {s.index + 1} · {s.kind} · {s.documents}개 · {s.ok ? "확인 완료" : "확인 실패"}</p>)}<button onClick={() => setRefresh(v => v + 1)}>갱신 상태 다시 읽기</button></details>;
}
