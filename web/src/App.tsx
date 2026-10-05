import { Component, Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import Documents from "./Documents";
import OriginalDetail from "./OriginalDetail";
import Memory from "./Memory";
import { DocumentPurpose, SubjectPurpose } from "./Purpose";
import type { Item as MemoryItem } from "./Memory";
import { Positions } from "./positions";
import { nodePresentation, starColor } from "./presentation";
import { graphUrl, isNativeOriginal, kindName, knowledge, linkName, nativeFolders, parseLocation, reconcile, sameGraphLocation, searchResults, stateName, visibleClusterOptions, visibleGraph } from "./graph";
import type { Filters, GraphNode, GraphView, Model, Scope, Snapshot } from "./graph";
const Graph = lazy(() => import("./Graph.tsx"));
type Session = { csrf: string; areas: { id: string; label: string }[] };
type GraphTiming = { location: ReturnType<typeof parseLocation>; responseMs: number; displayMs: number; list: boolean };
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
function SubjectDelete({ scope, id, label, csrf, disabled, onBusy, onDeleted }: { scope: Scope; id: string; label: string; csrf: string; disabled: boolean; onBusy: (value: boolean) => void; onDeleted: (ungrouped: number, documents: number) => void }) {
  const [confirming, setConfirming] = useState(false), [error, setError] = useState("");
  const pending = useRef(false);
  async function remove() {
    if (pending.current) return;
    pending.current = true;
    onBusy(true); setError("");
    try {
      const result = await request<{ id: string; deleted: boolean; ungrouped: number; ungrouped_documents?: number }>("/api/brain", { method: "POST", headers: { "content-type": "application/json", "x-csrf-token": csrf }, body: JSON.stringify({ op: "subject-delete", scope, id }) });
      onDeleted(result.ungrouped, result.ungrouped_documents ?? 0);
    } catch (failure) {
      setError(message(failure));
    } finally { pending.current = false; onBusy(false); }
  }
  return <section className="section">
    <button disabled={disabled} onClick={() => { setConfirming(true); setError(""); }}>묶음 삭제</button>
    {confirming && <div className="subject-delete-confirm"><p>‘{label}’ 묶음을 삭제할까요? 연결된 문서와 기록은 남기고 묶음 연결만 해제합니다.</p><button disabled={disabled} onClick={() => void remove()}>삭제 확인</button><button disabled={disabled} onClick={() => { setConfirming(false); setError(""); }}>취소</button></div>}
    {error && <p className="error" role="alert">{error}</p>}
  </section>;
}
const initialFilters: Filters = { kind: "all", state: "all", cluster: null, sourceScope: null };
export default function App() {
  const [route, setRoute] = useState(() => parseLocation(window.location.search));
  const positions = useMemo(() => new Positions(), [route.scope, route.q]);
  const routeRef = useRef(route); routeRef.current = route;
  const [session, setSession] = useState<Session | null>(null);
  const [stored, setStored] = useState<{ snapshot: Snapshot; model: Model } | null>(null);
  const previous = useRef<Model | undefined>(undefined);
  const pendingTiming = useRef<{ location: typeof route; startedAt: number; responseMs: number } | null>(null);
  const [timing, setTiming] = useState<GraphTiming | null>(null);
  const data = stored?.snapshot.scope === route.scope && stored.snapshot.query === route.q ? stored : null;
  const [input, setInput] = useState(route.q), [refresh, setRefresh] = useState(0), [panelEpoch, setPanelEpoch] = useState(0);
  const [filters, setFilters] = useState<Filters>(initialFilters);
  const [showFolders, setShowFolders] = useState(false);
  const showFoldersRef = useRef(false);
  const [view, setView] = useState<GraphView>("purpose");
  const viewRef = useRef<GraphView>("purpose");
  const [panel, setPanel] = useState<"node" | "manage" | null>(route.focus ? "node" : null);
  const [managing, setManaging] = useState(false);
  // One write response is handed to its new detail view; this is not a cross-document cache.
  const [savedMemory, setSavedMemory] = useState<MemoryItem | null>(null);
  const [busy, setBusy] = useState(false), [loading, setLoading] = useState(true), [error, setError] = useState("");
  const busyRef = useRef(false);
  const originalBusy = useRef(false), purposeBusy = useRef(false);
  const updateBusy = useCallback((value: boolean) => { originalBusy.current = value; busyRef.current = value || purposeBusy.current; setBusy(busyRef.current); }, []);
  const updatePurposeBusy = useCallback((value: boolean) => { purposeBusy.current = value; busyRef.current = value || originalBusy.current; setBusy(busyRef.current); }, []);
  const dirtyRef = useRef(false);
  const originalDirty = useRef(false), purposeDirty = useRef(false);
  const updateDirty = useCallback((value: boolean) => { originalDirty.current = value; dirtyRef.current = value || purposeDirty.current; }, []);
  const updatePurposeDirty = useCallback((value: boolean) => { purposeDirty.current = value; dirtyRef.current = value || originalDirty.current; }, []);
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
  const relatedOriginals: GraphNode[] = selected ? data?.model.links.flatMap(link => {
    if (link.kind === "parent" || link.kind === "reference") return [];
    const id = link.source === selected.id ? link.target : link.target === selected.id ? link.source : null;
    const node = data.model.nodes.find(candidate => candidate.id === id);
    return node ? [node] : [];
  }) ?? [] : [];
  const references = selected ? data?.model.links.flatMap(link => {
    if (link.kind !== "reference") return [];
    const outgoing = link.source === selected.id;
    const id = outgoing ? link.target : link.target === selected.id ? link.source : null;
    const node = data.model.nodes.find(candidate => candidate.id === id);
    return node ? [{ node, outgoing }] : [];
  }) ?? [] : [];
  const memorySeed = savedMemory?.scope === route.scope && savedMemory.id === route.focus ? savedMemory : null;
  const shown = useMemo(() => data ? visibleGraph(data.model, filters) : { nodes: [], links: [] }, [data, filters]);
  const fallback = listMode || webglFailed;
  const displayed = fallback && route.q ? searchResults(shown.nodes, route.q) : shown.nodes;
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
    const pop = () => { if (busyRef.current || dirtyRef.current && !window.confirm("저장하지 않은 초안을 버리고 이동할까요?")) { window.history.pushState(null, "", graphUrl(routeRef.current.scope, routeRef.current.q, routeRef.current.focus)); return; } updatePurposeDirty(false); updateDirty(false); const next = parseLocation(window.location.search); routeRef.current = next; setRoute(next); setInput(next.q); setFilters(initialFilters); setPanel(next.focus ? "node" : null); setManaging(false); setSavedMemory(null); setRefresh(v => v + 1); setPanelEpoch(v => v + 1); };
    const beforeUnload = (event: BeforeUnloadEvent) => { if (dirtyRef.current || busyRef.current) event.preventDefault(); };
    document.addEventListener("visibilitychange", visibility); media.addEventListener("change", motion); window.addEventListener("popstate", pop); window.addEventListener("beforeunload", beforeUnload);
    return () => { document.removeEventListener("visibilitychange", visibility); media.removeEventListener("change", motion); window.removeEventListener("popstate", pop); window.removeEventListener("beforeunload", beforeUnload); };
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
    const startedAt = performance.now();
    pendingTiming.current = null;
    setLoading(true); setError(""); setTiming(null);
    const params = new URLSearchParams({ scope: current.scope, q: current.q });
    if (current.focus) params.set("focus", current.focus);
    request<Snapshot>(`/api/graph?${params}`, { signal: controller.signal }).then(snapshot => {
      if (controller.signal.aborted || !sameGraphLocation(current, routeRef.current)) return;
      const responseMs = performance.now() - startedAt;
      const model = reconcile(snapshot, previous.current, showFoldersRef.current, viewRef.current);
      positions.install(model, true);
      previous.current = { ...model, nodes: model.nodes.map(node => ({ ...node })) };
      pendingTiming.current = { location: current, startedAt, responseMs };
      setStored({ snapshot, model });
      setFilters(f => f.cluster && !model.clusters.some(c => c.id === f.cluster) ? { ...f, cluster: null } : f);
      const focus = routeRef.current.focus;
      if (focus && !model.nodes.some(n => n.id === focus)) {
        setNotice("초점을 지정한 자료가 없거나 이 범위에서 찾을 수 없습니다.");
        syncRoute({ ...routeRef.current, focus: null }, true); setPanel(null);
      }
    }).catch(e => { if (!controller.signal.aborted && sameGraphLocation(current, routeRef.current)) setError(message(e)); }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [visible, session, route.scope, route.q, route.focus, refresh, positions]);
  useEffect(() => {
    const pending = pendingTiming.current;
    if (!visible || !pending || loading || !data || !sameGraphLocation(pending.location, routeRef.current)) return;
    let secondFrame = 0;
    const firstFrame = requestAnimationFrame(() => {
      secondFrame = requestAnimationFrame(() => {
        if (pendingTiming.current !== pending || !sameGraphLocation(pending.location, routeRef.current)) return;
        setTiming({ location: pending.location, responseMs: pending.responseMs, displayMs: performance.now() - pending.startedAt, list: fallback });
        pendingTiming.current = null;
      });
    });
    return () => { cancelAnimationFrame(firstFrame); cancelAnimationFrame(secondFrame); };
  }, [data, loading, route.scope, route.q, route.focus, fallback, visible]);
  function confirmDiscard() {
    if (busyRef.current) return false;
    if (!dirtyRef.current) return true;
    if (!window.confirm("저장하지 않은 초안을 버리고 이동할까요?")) return false;
    updatePurposeDirty(false); updateDirty(false); return true;
  }
  const reload = () => { if (busy || !confirmDiscard()) return; setSavedMemory(null); setRefresh(v => v + 1); setPanelEpoch(v => v + 1); setNotice(""); };
  function search(query: string) {
    if (busy || !confirmDiscard()) return;
    const next = query.trim(), same = route.q === next;
    setInput(next); syncRoute({ scope: route.scope, q: next, focus: null });
    if (panel !== "manage") setPanel(null);
    setFilters(initialFilters); setNotice(""); if (same) setRefresh(v => v + 1);
  }
  function changeScope(scope: Scope) {
    if (busy || scope === route.scope || !confirmDiscard()) return;
    previous.current = undefined; setStored(null); setInput(""); setFilters(initialFilters); setPanel(null); setManaging(false); setSavedMemory(null); setNotice(""); setError("");
    syncRoute({ scope, q: "", focus: null });
  }
  function choose(id: string) {
    if (busy || !confirmDiscard()) return;
    const node = data?.model.nodes.find(n => n.id === id);
    if (node?.kind === "folder") { fitFolder(id); return; }
    setNotice(""); setManaging(false); setSavedMemory(null); setFilters(f => ({ ...initialFilters, cluster: f.cluster === node?.cluster ? f.cluster : null, folder: shown.nodes.some(value => value.id === id) ? f.folder : null }));
    syncRoute({ ...route, focus: id }); openPanel("node");
    if (!node) setRefresh(v => v + 1);
  }
  function changed(item: MemoryItem | null) {
    const id = item?.id ?? null;
    syncRoute({ ...routeRef.current, focus: id }, true);
    setSavedMemory(item); setManaging(false); setPanel(id ? "node" : "manage"); setFilters(initialFilters);
    setRefresh(v => v + 1); setPanelEpoch(v => v + 1); setNotice("");
  }
  function subjectDeleted(ungrouped: number, documents: number) {
    setStored(null); setInput(""); setFilters(initialFilters); setPanel(null);
    setNotice(ungrouped || documents ? `문서 ${documents}개·기록 ${ungrouped}개의 소속을 해제하고 묶음을 삭제했습니다.` : "빈 묶음을 삭제했습니다.");
    syncRoute({ scope: route.scope, q: "", focus: null });
  }
  function groupingChanged(item: MemoryItem, contentChanged: boolean) {
    setSavedMemory(item);
    if (contentChanged) setRefresh(v => v + 1);
  }
  function openPanel(next: "node" | "manage") {
    if (!panel) opener.current = document.activeElement instanceof HTMLElement && document.activeElement.matches("button, a[href], input, select, textarea, summary, [tabindex]") ? document.activeElement : manageButton.current;
    setPanel(next); setManaging(false);
  }
  function startRecord() {
    if (busy || !confirmDiscard()) return;
    syncRoute({ ...route, focus: null }); openPanel("manage");
  }
  function fitView(cluster = filters.cluster) {
    if (busy || !confirmDiscard()) return;
    setFilters(f => ({ ...f, cluster, folder: null }));
    closePanel(); setFit(v => v + 1);
  }
  function fitFolder(folder: string | null) {
    if (busy || !confirmDiscard()) return;
    setFilters(f => ({ ...f, folder, cluster: null }));
    closePanel(); setFit(v => v + 1);
  }
  function toggleFolders(value: boolean) {
    showFoldersRef.current = value; setShowFolders(value);
    if (data) {
      const model = reconcile(data.snapshot, data.model, value, viewRef.current);
      positions.install(model, true);
      previous.current = { ...model, nodes: model.nodes.map(node => ({ ...node })) };
      setStored({ snapshot: data.snapshot, model });
    }
    setFilters(f => f.kind === "folder" && !value ? { ...f, kind: "all" } : f);
    setFit(v => v + 1);
  }
  function changeView(value: GraphView) {
    if (busy || value === viewRef.current) return;
    viewRef.current = value; setView(value); setFilters(f => ({ ...f, cluster: null }));
    if (data) {
      const model = reconcile(data.snapshot, data.model, showFoldersRef.current, value);
      positions.install(model, true);
      previous.current = { ...model, nodes: model.nodes.map(node => ({ ...node })) };
      setStored({ snapshot: data.snapshot, model });
    }
  }
  function closePanel(): boolean {
    if (busy || !confirmDiscard()) return false;
    setPanel(null); setManaging(false); setSavedMemory(null); syncRoute({ ...routeRef.current, focus: null });
    requestAnimationFrame(() => (opener.current?.isConnected ? opener.current : manageButton.current)?.focus());
    return true;
  }
  function toggleManagement() {
    setManaging(value => !value);
    requestAnimationFrame(() => {
      if (managing) panelElement.current?.scrollTo({ top: 0 });
      else panelElement.current?.querySelector("#record-management")?.scrollIntoView({ block: "start" });
    });
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
  const clusterOptions = useMemo(() => data ? visibleClusterOptions(data.model, filters) : { listed: [], standalone: 0 }, [data, filters]);
  const listedClusters = clusterOptions.listed;
  const folders = useMemo(() => data ? nativeFolders(visibleGraph(data.model, { ...filters, cluster: null, folder: null }).nodes) : [], [data, filters]);
  const folder = data && filters.folder ? nativeFolders(data.model.nodes).find(value => value.id === filters.folder) : null;
  return <div className="app">
    <header className="app-header" inert={!!panel && narrow}>
      <a className="wordmark" href="/">개인 온톨로지</a>
      <div className="scope-switch" role="group" aria-label="탐색 범위">
        <button aria-pressed={route.scope === "personal"} disabled={busy} onClick={() => changeScope("personal")}>내 지식</button>
        <button aria-pressed={route.scope === "meenseek"} disabled={busy} onClick={() => changeScope("meenseek")}>meenseek 사업</button>
      </div>
      <form className="map-search" role="search" onSubmit={e => { e.preventDefault(); search(input); }}>
        <label className="sr-only" htmlFor="graph-search">현재 범위의 지식 검색</label>
        <input id="graph-search" value={input} onChange={e => setInput(e.target.value)} maxLength={120} placeholder="지식 검색" />
        <button type="submit" disabled={busy || !session}>검색</button>
      </form>
      <button ref={manageButton} className="manage-button" disabled={!session || busy} onClick={startRecord}>기록 남기기</button>
    </header>
    <main className={`map-workspace ${panel ? "panel-open" : ""} ${explore ? "explorer-open" : ""}`}>
      {explore && <aside className="map-sidebar" aria-label="지식 탐색" inert={!!panel && narrow}>
        <div className="sidebar-heading"><h2>탐색 조건</h2><button className="quiet" aria-label="탐색 조건 닫기" onClick={() => { setExplore(false); filterButton.current?.focus(); }}>닫기</button></div>
        <div className="filter-row">
          <label>표시 종류<select value={filters.kind} disabled={busy} onChange={e => setFilters(f => ({ ...f, kind: e.target.value as Filters["kind"] }))}><option value="all">모두</option><option value="knowledge">문서와 기록</option>{Object.entries(kindName).filter(([id]) => id !== "folder" || showFolders).map(([id, name]) => <option key={id} value={id}>{name}</option>)}</select></label>
          <label>표시 상태<select value={filters.state} disabled={busy} onChange={e => setFilters(f => ({ ...f, state: e.target.value as Filters["state"] }))}><option value="all">모든 상태</option><option value="active">현재 사용 가능</option><option value="proposed">제안</option><option value="withdrawn">철회</option><option value="attention">확인 필요·제외</option></select></label>
        </div>
        {route.scope === "personal" && <label className="topic-label">원문 출처 범위<select value={filters.sourceScope ?? ""} onChange={event => setFilters(value => ({ ...value, sourceScope: event.target.value || null, cluster: null, folder: null }))}><option value="">모든 출처</option>{[...new Set(data?.model.nodes.flatMap(node => node.context_scope ? [node.context_scope] : []) ?? [])].sort().map(value => <option key={value} value={value}>{value}</option>)}</select></label>}
        <label className="topic-label"><input type="checkbox" checked={showFolders} disabled={busy} onChange={event => toggleFolders(event.target.checked)} /> 폴더 연결 표시</label>
        <section className="cluster-list"><h2>{view === "purpose" ? "목적 묶음" : "관계 군집"}</h2>
          <button className="cluster" aria-pressed={!filters.cluster && !filters.folder} disabled={busy} onClick={() => fitView(null)}>전체 보기</button>
          <div className="compact-list">{listedClusters.map(c => <button className="cluster" key={c.id} disabled={busy} aria-pressed={filters.cluster === c.id} onClick={() => fitView(c.id)}><i style={{ background: c.color }} /><strong>{c.label}</strong><small>{c.purpose ? `전체 소속 ${c.totalKnowledge ?? c.knowledge} · 표시 ${c.knowledge}` : `문서·기록 ${c.knowledge}`} · 분류 표식 {c.members.length - c.knowledge}</small></button>)}</div>
          <p className="hint">{listedClusters.length}개 묶음 · 단독 항목 {clusterOptions.standalone}개</p>
          <p className="hint">{view === "purpose" ? "저장된 목적 소속을 표시합니다. 미분류는 개별 항목으로 남으며 검색·관계 변화로 소속이 바뀌지 않습니다." : "현재 관계로 계산한 군집입니다. 검색 범위와 관계 변화에 따라 달라질 수 있습니다."} 폴더 소속은 별도 구조입니다.</p>
        </section>
        <section className="folder-list"><h2>상위 폴더</h2><div className="compact-list">{folders.map(value => <button className="cluster" key={value.id} disabled={busy} aria-pressed={filters.folder === value.id} onClick={() => fitFolder(value.id)}><strong>{value.label}</strong><small>하위 원문 {value.members.length}개</small></button>)}</div><p className="hint">조회된 원문 경로에서 계산한 부모·소속 구조입니다. 폴더를 누르면 하위 자료를 봅니다.</p></section>
        <details className="section"><summary>검색 방법</summary><p className="hint">단어를 띄어 쓰면 순서와 관계없이 모든 단어가 포함된 항목을 찾습니다. 경로와 검색 가능한 원문 내용도 함께 찾습니다. 첨부 파일은 연결된 원문 안에서 내려받습니다. 폴더 연결은 경로에서 계산하며 내용의 유사성을 뜻하지 않습니다.</p></details>
      </aside>}
      <section className="galaxy" aria-label="지식 지도" aria-busy={loading} inert={!!panel && narrow}>
        <div className="map-toolbar"><div className="map-title"><h1>{folder ? folder.label : cluster ? cluster.label : route.q ? `“${route.q}” 검색` : "지식 지도"}</h1><p className="count-breakdown">표시 중 · 문서 {data ? displayed.filter(n => n.kind === "document").length : "—"} · 기록 {data ? displayed.filter(n => n.kind === "memory").length : "—"} · 관계 {data ? shown.links.filter(link => link.kind !== "parent").length : "—"}{showFolders && data && <> · 부모·소속 {shown.links.filter(link => link.kind === "parent").length}</>}</p>{timing && data && !loading && sameGraphLocation(timing.location, route) && <p className="count-breakdown load-timing" title="응답은 자료 요청부터 JSON 수신까지, 목록 표시는 요청부터 목록의 첫 화면이 그려질 때까지입니다. 앱 실행·접속 시간은 제외합니다.">최근 조회 · 응답 {Math.round(timing.responseMs)}ms{timing.list && fallback && <> · 목록 표시 {Math.round(timing.displayMs)}ms</>}</p>}</div>
          <div className="map-actions"><label>묶음 기준<select aria-label="묶음 기준" value={view} disabled={busy} onChange={event => changeView(event.target.value as GraphView)}><option value="purpose">목적</option><option value="relationships">관계</option></select></label>
            <button ref={filterButton} disabled={busy} aria-expanded={explore} onClick={() => setExplore(v => !v)}>필터·묶음</button>
            <button disabled={busy} aria-pressed={fallback} onClick={() => { if (webglFailed) { setWebglFailed(false); setListMode(false); } else setListMode(v => !v); }}>{fallback ? "3D 보기" : "목록 보기"}</button>
            <details className="view-options"><summary>보기 설정</summary><div>
              <button disabled={busy || !data} onClick={() => { positions.reset(); setFit(v => v + 1); }}>배치 초기화</button>
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
          {(route.q || cluster || folder) && <div className="active-filters">{route.q && <button className="quiet" disabled={busy} onClick={() => search("")}>검색 해제</button>}{cluster && <button className="quiet" disabled={busy} onClick={() => fitView(null)}>묶음 해제</button>}{folder && <button className="quiet" disabled={busy} onClick={() => fitFolder(null)}>폴더 해제</button>}</div>}
          {route.q && data && <p className="hint">일치 구절은 원문의 일부입니다. 적용 여부는 현재 내용과 근거에서 확인하세요.</p>}
          {data?.snapshot.truncated && <p className="warning">자료가 많아 일부만 표시합니다. 검색으로 범위를 좁히면 다른 자료를 찾을 수 있습니다.</p>}
          {data && shown.nodes.length > 0 && !shown.links.length && <p className="hint no-relations">{data.snapshot.totals.links === 0 ? "아직 등록된 관계가 없습니다." : "현재 표시한 항목 사이에는 조회된 관계가 없습니다."}</p>}
          {selected && !shown.nodes.some(n => n.id === selected.id) && <p className="notice">선택한 자료가 현재 필터 밖에 있습니다. <button onClick={() => setFilters(initialFilters)}>필터 해제</button></p>}
        </div>
        {data && !loading && !displayed.length ? <div className="map-empty"><h2>표시할 항목이 없습니다.</h2><p>{route.q || filters.kind !== "all" || filters.state !== "all" ? "검색어와 표시 조건을 바꿔보세요." : "등록한 문서와 저장한 기록이 이곳에 나타납니다."}</p><button disabled={!session || busy} onClick={startRecord}>기록 남기기</button></div> : data && (fallback ? <div className="graph-list" aria-label="지식 지도 목록">{displayed.map(n => { const label = nodePresentation(n); return <button key={n.id} disabled={busy} aria-pressed={route.focus === n.id} onClick={() => choose(n.id)}><span className={`node-symbol ${n.kind}`} style={{ color: knowledge(n) ? starColor(n) : n.taxonomyColor ?? n.color }} aria-hidden="true">{knowledge(n) ? "·" : "○"}</span><span><small>{kindName[n.kind]} · {stateName(n)}</small><strong>{label.title}</strong>{label.subtitle && <span className="node-location">{label.subtitle}</span>}{route.q && n.historical_match && <small className="warning">이전 내용에서 일치 · 현재 상태를 확인하세요</small>}{route.q && n.excerpt && <span className="search-excerpt">{n.excerpt}</span>}</span>{n.changed && <em>변경</em>}</button>; })}</div> : <GraphBoundary onFailure={failed}><Suspense fallback={<p className="loading">3D 화면 준비 중…</p>}><Graph key={`${route.scope}:${route.q}`} positions={positions} snapshot={data.model} nodes={shown.nodes} links={shown.links} selected={route.focus} rotate={rotating} reduced={reduced} visible={visible} fit={fit} disabled={busy} onSelect={choose} onClearSelection={closePanel} onFailure={failed} /></Suspense></GraphBoundary>)}
        <div className="map-legend"><span><i className="legend-star document" />문서</span><span><i className="legend-star memory" />기록</span><span>○ 분류 표식{showFolders ? " · 폴더" : ""}</span><details><summary>관계·상태 읽기</summary><p>목적 보기에서는 저장된 목적 소속과 폴더 연결을 그립니다. 목적 이름이 붙은 별은 같은 목적의 문서·기록 19개 이상을 접어 표시하며, 참조 관계가 바뀌어도 같은 소속을 유지합니다. 관계 보기에서는 원문 링크·근거·분류 등의 전체 관계를 그립니다. 원문의 들어오는·나가는 링크는 두 보기 모두 자료 상세에서 읽을 수 있습니다. ‘관계 묶음’은 한 별에 현재 연결된 이웃이 19개 이상인 묶음입니다. ‘근접 묶음’은 멀리서 볼 때 화면상 가까운 관계 없는 문서·기록 6개 이상을 잠시 합쳐 보인 것입니다. 근접 묶음은 내용의 유사성이나 관계를 뜻하지 않으며, 확대하거나 누르면 개별 별이 보입니다. 묶음 크기는 항목 수가 10·30·50·100개 이상인 구간에 따라 커집니다. 접힌 지도는 보이는 별과 묶음의 실제 크기에 맞춰 간격을 줄이며, 펼치면 구성원을 다시 봅니다. 가는 원은 선택, 바깥 점선 원은 이전 조회 이후의 기록·관계 변경입니다. 흐린 점은 제안·철회·유효기간·출처 확인 상태를 살펴보세요. 같은 분류 표식 하나에 속한 별은 표식과 색을 공유하고, 여러 분류에 속한 별은 고유색을 유지합니다. {Object.values(linkName).join(" · ")} 관계를 선으로 표시하며 선택하면 연결된 선을 강조합니다. 부모·소속은 조회된 경로에서 계산하며 관계 군집을 바꾸지 않습니다. ‘폴더 묶음’은 같은 폴더 바로 아래에서 다른 관계가 없는 원문 19개 이상을 접어 표시합니다. 다른 관계가 있는 원문과 하위 폴더는 계속 보이며, 묶음을 누르면 개별 부모 연결을 확인합니다. 폴더를 누르면 하위 자료를 봅니다. 접힌 묶음의 폴더 연결은 개별 항목을 펼쳐 확인할 수 있습니다. 과거 출처 근거는 갈색의 가는 선이며 군집에서 제외합니다. 단독 항목은 연결된 자료 곁에 배치될 수 있습니다. ‘저장’은 사실 검증을 뜻하지 않습니다. 출처 확인 시각만 바뀌면 변경으로 표시하지 않습니다.</p></details></div>
      </section>
      {panel && session && <aside ref={panelElement} className="management-panel" role="dialog" aria-modal={narrow || undefined} aria-label={panel === "manage" ? "기록 남기기" : "선택한 자료 상세"}>
        <div className="panel-heading"><span>{panel === "manage" ? "기록 남기기" : selected ? kindName[selected.kind] : memorySeed ? "기록" : "자료 상세"}</span><div className="panel-actions">{panel === "node" && (memorySeed || selected && knowledge(selected) && !isNativeOriginal(selected)) && <button disabled={busy} aria-expanded={managing} onClick={toggleManagement}>{managing ? "읽기로 돌아가기" : "관리"}</button>}<button ref={closeButton} disabled={busy} aria-label="관리 패널 닫기" onClick={closePanel}>닫기 ×</button></div></div>
        {panel === "manage" ? <><Memory visible={visible} managing key={`${route.scope}:manage:${panelEpoch}`} scope={route.scope} csrf={session.csrf} request={request} selectedId={null} onBusy={updateBusy} onChange={changed} onNavigate={choose} onMetadataChange={() => setRefresh(v => v + 1)} /><SyncPanel key={panelEpoch} visible={visible} /></> : !selected && !memorySeed ? <p className="empty">선택한 자료를 불러오는 중…</p> : <>
          {selected?.changed && <p className="notice">이전 조회 이후 기록이나 관계가 변경되었습니다.</p>}
          {selected?.kind === "document" && <DocumentPurpose key={`${route.scope}:${selected.id}:${panelEpoch}:purpose`} node={selected} scope={route.scope} csrf={session.csrf} request={request} visible={visible} disabled={busy} onBusy={updatePurposeBusy} onDirtyChange={updatePurposeDirty} onChange={() => setRefresh(v => v + 1)} />}
          {references.length > 0 && <details className="section source-references"><summary>원문 링크 · {references.length}개</summary><p className="hint">현재 조회된 자료 사이의 본문 링크입니다. 같은 목적이나 주장의 근거를 뜻하지 않습니다.</p><div className="compact-list">{references.map(({ node, outgoing }) => <button key={`${outgoing}:${node.id}`} disabled={busy} onClick={() => choose(node.id)}><span>{outgoing ? "원문 링크 →" : "← 이 원문을 참조한 자료"}</span><strong>{nodePresentation(node).title}</strong>{nodePresentation(node).subtitle && <small>{nodePresentation(node).subtitle}</small>}</button>)}</div></details>}
          {selected?.kind === "document" ? isNativeOriginal(selected) ? <OriginalDetail key={selected.id} scope={selected.context_scope!} path={selected.context_path!} request={request} related={relatedOriginals} references={references.filter(item => item.outgoing).map(item => item.node)} onNavigate={choose} csrf={session.csrf} onBusy={updateBusy} onChange={() => setRefresh(v => v + 1)} onDirtyChange={updateDirty} /> : <Documents visible={visible} managing={managing} key={`${route.scope}:${selected.id}:${panelEpoch}`} scope={route.scope} id={selected.id} csrf={session.csrf} allAreas={session.areas} request={request} onBusy={updateBusy} onChange={reload} onNavigate={choose} contextScope={selected.context_scope} contextPath={selected.context_path} onDirtyChange={updateDirty} /> : selected?.kind === "memory" || memorySeed ? <Memory latestNode={selected} visible={visible} managing={managing} initialItem={memorySeed} key={`${route.scope}:${route.focus}:${panelEpoch}`} scope={route.scope} csrf={session.csrf} request={request} selectedId={route.focus} onBusy={updateBusy} onChange={changed} onBackgroundChange={groupingChanged} onNavigate={choose} onMetadataChange={() => setRefresh(v => v + 1)} /> : selected && <><h2>{selected.label}</h2><p className="hint">{kindName[selected.kind]} 표식입니다.</p><button onClick={() => fitView(selected.cluster)}>이 묶음 보기</button>{selected.kind === "subject" && <SubjectPurpose key={`${route.scope}:${selected.id}:${panelEpoch}:definition`} node={selected} scope={route.scope} csrf={session.csrf} request={request} visible={visible} disabled={busy} onBusy={updatePurposeBusy} onDirtyChange={updatePurposeDirty} onChange={() => setRefresh(v => v + 1)} />}{selected.kind === "subject" && <SubjectDelete key={selected.id} scope={route.scope} id={selected.id} label={selected.label} csrf={session.csrf} disabled={busy} onBusy={updateBusy} onDeleted={subjectDeleted} />}<div className="compact-list">{data?.model.links.filter(l => l.source === selected.id || l.target === selected.id).map(l => { const other = data.model.nodes.find(n => n.id === (l.source === selected.id ? l.target : l.source)); return other && <button key={`${l.kind}:${other.id}`} disabled={busy} onClick={() => choose(other.id)}><span>{linkName[l.kind]}</span><strong>{nodePresentation(other).title}</strong>{nodePresentation(other).subtitle && <small>{nodePresentation(other).subtitle}</small>}</button>; })}</div></>}
          {managing && <p className="section node-permalink"><a href={graphUrl(route.scope, route.q, route.focus)}>이 자료 링크</a></p>}
        </>}
      </aside>}
    </main>
  </div>;
}

type SyncStatus = { enabled: boolean; running: boolean; last_completed_at: number | null; error: string | null; report: { ok: boolean; sources: { index: number; kind: string; documents: number; ok: boolean }[] } | null };
function SyncPanel({ visible }: { visible: boolean }) {
  const [value, setValue] = useState<SyncStatus | null>(null), [error, setError] = useState(""), [refresh, setRefresh] = useState(0);
  const [opened, setOpened] = useState(false), [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!visible || !opened || value || error) return;
    const controller = new AbortController(); setLoading(true);
    request<SyncStatus>("/api/sync", { signal: controller.signal }).then(v => { if (!controller.signal.aborted) setValue(v); }).catch(e => { if (!controller.signal.aborted) setError(message(e)); }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [visible, opened, refresh]);
  return <details className="section" onToggle={event => setOpened(event.currentTarget.open)}><summary>출처 갱신{value && ` · ${value.enabled ? value.running ? "확인 중" : value.error || value.report?.ok === false ? "확인 필요" : "실행 중" : "꺼짐"}`}</summary><p className="hint">앱 실행 중 지정된 원문만 갱신합니다. 기록 내용은 자동으로 바꾸지 않습니다.</p>{error && <p className="error">{error}</p>}{value?.last_completed_at && <p className="hint">최근 확인: {new Date(value.last_completed_at * 1000).toLocaleString("ko-KR")}</p>}<button disabled={loading} onClick={() => { setValue(null); setError(""); setRefresh(v => v + 1); }}>{loading ? "불러오는 중…" : "상태 새로고침"}</button></details>;
}
