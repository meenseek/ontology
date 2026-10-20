import { leaderHitsBox, leadersCross, nodeScreenSize, nodeVisualRadius, summaryAppearance, type LabelLeader } from "./presentation";
import { createRoot } from "react-dom/client";
import { Line, LineBasicMaterial, Mesh, Raycaster, Scene, ShaderMaterial, Sprite, Vector3 } from "three";
import type { Camera } from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import App from "./App";
import { Positions } from "./positions";
import { summaryGlyphTexture } from "./summary-glyph";
import type { Item } from "./Memory";
import type { GraphNode, Scope, Snapshot } from "./graph";
import "./style.css";

type Call = { operation: string; scope: string | null; status: number; requestBytes: number; responseBytes: number };
type Phase = { name: string; calls: Record<string, number>; requestBytes: number; responseBytes: number };
let active: { size: number; calls: Call[]; phases: Phase[]; forbidden: string[] } | null = null;
const output = document.querySelector<HTMLPreElement>("#result")!;
const host = document.querySelector<HTMLDivElement>("#app-check")!;
const encode = (value: string) => new TextEncoder().encode(value).length;
function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const settle = () => new Promise<void>(resolve => setTimeout(resolve, 40));
async function until(condition: () => boolean, message: string) {
  const deadline = performance.now() + 4000;
  while (!condition()) {
    if (performance.now() > deadline) throw new Error(`Timeout: ${message}`);
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  await settle();
}
function button(text: string, parent: ParentNode = host) {
  const element = [...parent.querySelectorAll<HTMLButtonElement>("button")].find(node => (node.textContent?.trim() || node.getAttribute("aria-label")) === text);
  assert(element && !element.disabled, `Enabled button: ${text}`);
  return element;
}
function details(text: string) {
  const summary = [...host.querySelectorAll<HTMLElement>("summary")].find(node => node.textContent?.startsWith(text));
  assert(summary?.parentElement instanceof HTMLDetailsElement, `Details: ${text}`);
  return { summary, element: summary.parentElement };
}
function checkLabelAssociations() {
  const boxes = [...host.querySelectorAll<HTMLElement>('.node-label:not([hidden])')].filter(element => +element.style.opacity > .99).map(element => {
    const box = element.getBoundingClientRect(), length = parseFloat(element.style.getPropertyValue("--leader-length"));
    const x1 = box.left + parseFloat(element.style.getPropertyValue("--leader-x")), y1 = box.top + parseFloat(element.style.getPropertyValue("--leader-y"));
    const angle = parseFloat(element.style.getPropertyValue("--leader-angle"));
    const polyline = element.querySelector("polyline");
    const points = element.classList.contains("routed-label") ? polyline!.getAttribute("points")!.split(" ").map(value => value.split(",").map(Number)) : [];
    const lines: LabelLeader[] = points.length ? points.slice(1).map(([x, y], index) => ({ x1: box.left + points[index][0], y1: box.top + points[index][1], x2: box.left + x, y2: box.top + y, length: Math.hypot(x - points[index][0], y - points[index][1]) })) :
      element.classList.contains("offset-label") ? [{ x1, y1, x2: x1 + length * Math.cos(angle), y2: y1 + length * Math.sin(angle), length }] : [];
    if (points.length) assert(lines.reduce((sum, line) => sum + line.length, 0) <= 240.01, "a rendered title guide has a bounded detour");
    if (lines.length) assert(points.length ? getComputedStyle(polyline!).strokeDasharray !== "none" : getComputedStyle(element, "::before").borderTopStyle === "dashed", "title guides are distinguishable from actual relation lines");
    return { id: element.dataset.nodeId!, left: box.left, right: box.right, top: box.top, bottom: box.bottom, lines };
  });
  for (const [index, box] of boxes.entries()) for (const line of box.lines) {
    assert(boxes.every(other => box.id === other.id || !leaderHitsBox(line, other, 0)), `guide from ${box.id} does not penetrate another rendered name`);
    assert(boxes.slice(index + 1).every(other => other.lines.every(otherLine => !leadersCross(line, otherLine))), `guide from ${box.id} does not cross another rendered guide`);
  }
  return { titles: boxes.length, guides: boxes.filter(box => box.lines.length).length };
}
async function openExplorer() {
  if (!host.querySelector(".map-sidebar")) button("필터·묶음").click();
  await until(() => !!host.querySelector(".map-sidebar"), "explorer opens");
}
function enter(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const prototype = element instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event("input", { bubbles: true }));
  element.dispatchEvent(new Event("change", { bubbles: true }));
}
async function chooseDiagramMode(value: "purpose" | "relationships") {
  const options = details("보기 설정"); options.summary.click();
  await settle();
  const select = host.querySelector<HTMLSelectElement>('select[aria-label="묶음 기준"]')!;
  select.value = value; select.dispatchEvent(new Event("change", { bubbles: true }));
  await settle(); options.summary.click(); await settle();
}
const id = (index: number) => `m_00000000-0000-4000-8000-${index.toString().padStart(12, "0")}`;
const subjectId = "p_00000000-0000-4000-8000-000000000001";
function item(index: number, scope: Scope): Item {
  return { id: id(index), scope, revision: 1, kind: "fact", title: `${scope} 합성 기록 ${index}`, body: `${scope} 합성 본문 ${index}`, subject_id: null, subject_name: null, effective_from: null, effective_until: null, evidence: [], status: "accepted", origin: "user", support: "user-recorded", updated_at: "2026-09-11T00:00:00Z" };
}
function graph(items: Item[], scope: Scope, query: string, focus: string | null, subjectExists: boolean): Snapshot {
  const nodes: GraphNode[] = items.map(value => ({ id: value.id, scope, kind: "memory", label: value.title, revision: String(value.revision), created_at: "2026-09-10T00:00:00Z", content_updated_at: value.updated_at, status: "accepted", temporal: "current", supported: true }));
  if (scope === "personal" && subjectExists) nodes.push({ id: subjectId, scope, kind: "subject", label: "빈 묶음" });
  return { scope, query, focus: { id: focus, found: nodes.some(node => node.id === focus) }, nodes, links: [], matched: nodes.length, totals: { documents: 0, memories: items.length, markers: nodes.length - items.length, links: 0 }, returned: { knowledge: items.length, markers: nodes.length - items.length, links: 0 }, omitted: { nodes: 0, links: 0 }, eligible: { nodes: nodes.length, links: 0 }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
}

async function check(size: number) {
  const originalConfirm = window.confirm;
  let confirmations = 0, permitDiscard = false, failCorrection = false;
  const originalMatchMedia = window.matchMedia;
  const narrowMedia = Object.assign(new EventTarget(), { matches: false, media: "(max-width: 900px)", onchange: null, addListener() {}, removeListener() {} });
  window.matchMedia = query => query === narrowMedia.media ? narrowMedia as MediaQueryList : originalMatchMedia.call(window, query);
  const originalFetch = window.fetch;
  const originalUrl = window.location.href;
  const visibility = Object.getOwnPropertyDescriptor(document, "visibilityState");
  const root = createRoot(host);
  const calls: Call[] = [], phases: Phase[] = [], forbidden: string[] = [];
  active = { size, calls, phases, forbidden };
  const records: Record<Scope, Map<string, Item>> = {
    meenseek: new Map(Array.from({ length: size }, (_, i) => { const value = item(i + 1, "meenseek"); return [value.id, value]; })),
    personal: new Map(Array.from({ length: size }, (_, i) => { const value = item(i + 1, "personal"); return [value.id, value]; })),
  };
  let releaseGraph!: () => void;
  const initialGraph = new Promise<void>(resolve => { releaseGraph = resolve; });
  let firstGraph = true, failSync = false, subjectExists = true, screenVisible = true;
  function phase(name: string, start: number, expected: Record<string, number>) {
    const current = calls.slice(start), counts: Record<string, number> = {};
    for (const call of current) counts[call.operation] = (counts[call.operation] ?? 0) + 1;
    for (const operation of new Set([...Object.keys(expected), ...Object.keys(counts)])) {
      assert((counts[operation] ?? 0) === (expected[operation] ?? 0), `${name}: ${operation} expected ${expected[operation] ?? 0}, got ${counts[operation] ?? 0}`);
    }
    phases.push({ name, calls: counts, requestBytes: current.reduce((sum, call) => sum + call.requestBytes, 0), responseBytes: current.reduce((sum, call) => sum + call.responseBytes, 0) });
  }
  const bodyIs = (text: string) => [...host.querySelectorAll(".memory-body p")].some(p => p.textContent === text);
  const idleGraph = () => host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false";
  async function navigate(scope: Scope, focus: string) {
    // Exercise the public back/forward route event; never read React internals.
    window.history.pushState(null, "", `/?scope=${scope}&focus=${focus}`);
    window.dispatchEvent(new PopStateEvent("popstate"));
    await until(() => idleGraph() && bodyIs(records[scope].get(focus)!.body), `route ${scope}`);
  }
  try {
    window.confirm = () => { confirmations++; return permitDiscard; };
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => screenVisible ? "visible" : "hidden" });
    window.fetch = async (input, options) => {
      const raw = input instanceof Request ? input.url : String(input);
      const url = new URL(raw, window.location.href);
      const method = options?.method ?? (input instanceof Request ? input.method : "GET");
      if (url.origin !== window.location.origin || !["/api/session", "/api/graph", "/api/brain", "/api/sync"].includes(url.pathname)) {
        forbidden.push(raw); throw new Error(`Unexpected request: ${raw}`);
      }
      const body = url.pathname === "/api/brain" && typeof options?.body === "string" ? JSON.parse(options.body) : null;
      const call: Call = { operation: body?.op ?? url.pathname.slice(5), scope: body?.scope ?? url.searchParams.get("scope"), status: 0, requestBytes: typeof options?.body === "string" ? encode(options.body) : 0, responseBytes: 0 };
      calls.push(call); // Count attempts before any wait or abort, not only completed responses.
      const response = (value: unknown, status = 200) => {
        const text = JSON.stringify(value);
        call.status = status; call.responseBytes = encode(text);
        assert(call.responseBytes <= 1048576, `${call.operation}: synthetic response exceeds API byte limit`);
        return new Response(text, { status, headers: { "content-type": "application/json" } });
      };
      if (options?.signal?.aborted) throw new DOMException("Aborted", "AbortError");
      if (url.pathname === "/api/session" && method === "GET") return response({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph" && method === "GET") {
        if (firstGraph) { firstGraph = false; await initialGraph; }
        if (options?.signal?.aborted) throw new DOMException("Aborted", "AbortError");
        const scope = url.searchParams.get("scope") as Scope;
        assert(scope === "meenseek" || scope === "personal", "Graph scope");
        const query = url.searchParams.get("q") ?? "";
        return response(graph(query === "결과 없는 합성 검색" ? [] : [...records[scope].values()], scope, query, url.searchParams.get("focus"), query === "결과 없는 합성 검색" ? false : subjectExists));
      }
      if (url.pathname === "/api/sync" && method === "GET") {
        if (failSync) { failSync = false; return response({ error: "synthetic failure" }, 503); }
        return response({ enabled: false, running: false, last_completed_at: null, error: null, report: null });
      }
      if (url.pathname === "/api/brain" && method === "POST") {
        assert(typeof options?.body === "string", "JSON brain body");
        const scope: Scope = body.scope;
        assert(scope === "meenseek" || scope === "personal", "Brain scope");
        if (body.op === "read") {
          const value = records[scope].get(body.id);
          return response(value ?? { error: "missing" }, value ? 200 : 404);
        }
        if (body.op === "subjects") return response({ items: [], next_after: null });
        if (body.op === "subject-delete") {
          assert(scope === "personal" && body.id === subjectId && subjectExists, "Only the empty fixture subject can be deleted");
          subjectExists = false;
          return response({ id: subjectId, deleted: true, ungrouped: 0 });
        }
        if (body.op === "remember" || body.op === "correct") {
          if (body.op === "correct" && failCorrection) { failCorrection = false; return response({ error: "synthetic conflict" }, 409); }
          const prior = body.op === "correct" ? records[scope].get(body.id) : item(999, scope);
          assert(prior, "Corrected record exists");
          if (body.op === "correct") assert(body.revision === prior.revision, "Correction revision");
          const value: Item = { ...prior, ...body.memory, revision: body.op === "correct" ? prior.revision + 1 : 1 };
          records[scope].set(value.id, value);
          if (body.op === "remember") records.personal.set(value.id, item(999, "personal"));
          return response(value);
        }
      }
      forbidden.push(`${method} ${url.pathname}`);
      throw new Error(`Unsupported synthetic operation: ${method} ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=meenseek`);
    root.render(<App />);
    await until(() => !!host.querySelector(".map-actions"), "App controls");
    button("목록 보기").click();
    releaseGraph();
    await until(() => idleGraph() && host.querySelectorAll(".graph-list > button").length === size, "initial list");
    assert(!button("3D 보기").hasAttribute("aria-pressed"), "the next-view action does not announce a selected view");
    await until(() => host.querySelector(".load-timing")?.textContent?.includes("목록 표시") === true, "list timing");
    assert(/응답 \d+ms · 목록 표시 \d+ms/.test(host.querySelector(".load-timing")?.textContent ?? ""), "Timing names both measured phases");
    assert(host.querySelector<HTMLElement>(".load-timing")?.hidden === true, "development measurements never appear as a user-facing setting");
    const listOrder = () => [...host.querySelectorAll(".graph-list > button strong")].map(value => value.textContent);
    const initialOrder = listOrder();
    const sorter = host.querySelector<HTMLSelectElement>('select[aria-label="표시 항목 정렬"]')!;
    assert(sorter?.value === "updated", "default is recently modified");
    const sortCalls = calls.length;
    for (const value of ["created", "name", "updated"]) {
      sorter.value = value; sorter.dispatchEvent(new Event("change", { bubbles: true })); await settle();
      assert(calls.length === sortCalls, "sorting reuses the loaded snapshot");
      assert(listOrder().length === size, "sorting preserves all displayed records");
    }
    assert(JSON.stringify(listOrder()) === JSON.stringify(initialOrder), "restoring the sort restores the list");
    const viewSettings = details("보기 설정");
    viewSettings.summary.click(); await settle();
    const settingsBox = host.querySelector('.view-options > div')!.getBoundingClientRect();
    assert(settingsBox.left >= 0 && settingsBox.right <= window.innerWidth, "list settings stay inside the viewport when toolbar actions wrap");
    viewSettings.summary.click(); await settle();
    const legend = details("관계·상태 읽기");
    legend.summary.click(); await settle();
    const guide = host.querySelector<HTMLElement>('.legend-guide')!;
    const guideBox = guide.getBoundingClientRect(), mapBox = host.querySelector('.galaxy')!.getBoundingClientRect();
    assert(guideBox.top >= mapBox.top && guideBox.bottom <= legend.summary.getBoundingClientRect().top && guideBox.left >= 0 && guideBox.right <= innerWidth, "relationship guide remains inside the map and leaves its close control visible");
    assert(guide.scrollHeight > guide.clientHeight && getComputedStyle(guide).overflowY === "auto", "long guide is internally scrollable");
    guide.focus(); guide.scrollTop = guide.scrollHeight;
    assert(guide.scrollTop > 0 && guide.scrollTop + guide.clientHeight >= guide.scrollHeight - 1, "the last guidance is reachable");
    guide.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    assert(!legend.element.open && document.activeElement === legend.summary, "Escape closes guidance and returns focus");
    const explorerCalls = calls.length;
    narrowMedia.matches = true; narrowMedia.dispatchEvent(new Event("change")); await settle();
    button("필터·묶음").click(); await settle();
    const explorer = host.querySelector<HTMLElement>('.map-sidebar[role="dialog"]')!;
    const close = button("닫기", explorer);
    assert(explorer?.getAttribute("aria-modal") === "true" && host.querySelector<HTMLElement>("header")?.inert && host.querySelector<HTMLElement>(".galaxy")?.inert, "narrow exploration has one active modal and an inert background");
    assert(document.activeElement === close, "exploration focuses its close action");
    const last = details("검색 방법").summary; last.focus();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    assert(document.activeElement === close, "Tab stays inside the explorer");
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); await settle();
    assert(!host.querySelector(".map-sidebar") && document.activeElement === button("필터·묶음"), "Escape closes exploration and restores the opener");
    assert(calls.length === explorerCalls, "opening and closing exploration reuses loaded data");
    narrowMedia.matches = false; narrowMedia.dispatchEvent(new Event("change")); await settle();
    const first = host.querySelector<HTMLButtonElement>(".graph-list > button")!;
    first.click();
    await until(() => bodyIs(records.meenseek.get(id(1))!.body), "selected memory");
    const selectedRow = host.querySelector<HTMLButtonElement>('.graph-list > button[aria-pressed="true"]')!;
    const selectedStyle = getComputedStyle(selectedRow);
    assert(selectedRow === first && selectedStyle.backgroundColor !== "rgba(0, 0, 0, 0)" && selectedStyle.boxShadow.includes("inset"), "the read item retains both a filled selection and an inner edge marker");
    const otherRow = host.querySelector<HTMLButtonElement>('.graph-list > button[aria-pressed="false"]');
    if (otherRow) assert(selectedStyle.backgroundColor !== getComputedStyle(otherRow).backgroundColor, "selection remains distinguishable from an ordinary row");
    phase("session + graph + selected read; subjects closed", 0, { session: 1, graph: 2, read: 1 });

    button("닫기 ×").click();
    await until(() => !host.querySelector(".management-panel"), "close selection");
    button("기록 남기기").click();
    await until(() => !!host.querySelector(".memory-editor form input"), "new memory editor");
    let start = calls.length;
    const form = host.querySelector<HTMLFormElement>(".memory-editor form")!;
    assert(form.getClientRects().length > 0 && !form.closest("[hidden]"), "New editor is visible");
    enter(form.querySelector<HTMLTextAreaElement>("textarea")!, `새 본문 ${size}`);
    await settle();
    enter(host.querySelector<HTMLInputElement>("#graph-search")!, "찾을 자료");
    host.querySelector<HTMLFormElement>(".map-search")!.requestSubmit();
    await until(() => idleGraph() && window.location.search.includes("q="), "search while writing");
    assert(host.querySelector<HTMLFormElement>(".memory-editor form") === form && form.querySelector("textarea")?.value === `새 본문 ${size}`, "Search preserves the same draft DOM");
    button("검색 해제").click();
    await until(() => idleGraph() && !window.location.search.includes("q="), "clear search while writing");
    assert(host.querySelector<HTMLFormElement>(".memory-editor form") === form && form.querySelector("textarea")?.value === `새 본문 ${size}`, "Clearing search preserves the same draft DOM");
    phase("draft search and clear use only shared graph", start, { graph: 2 });
    await openExplorer();
    const displayFilter = (column: number) => host.querySelector<HTMLSelectElement>(`.filter-row label:nth-child(${column}) select`)!;
    for (const [column, value] of [[1, "memory"], [2, "active"]] as const) {
      displayFilter(column).value = value; displayFilter(column).dispatchEvent(new Event("change", { bubbles: true })); await settle();
    }
    button("닫기", host.querySelector(".map-sidebar")!).click(); await settle();
    start = calls.length; form.requestSubmit();
    await until(() => idleGraph() && bodyIs(`새 본문 ${size}`), "remember response reused");
    phase("remember response handoff", start, { remember: 1, graph: 1 });
    await openExplorer();
    assert(displayFilter(1).value === "memory" && displayFilter(2).value === "active", "Saving a record preserves its kind and state filters");
    for (const column of [1, 2]) { displayFilter(column).value = "all"; displayFilter(column).dispatchEvent(new Event("change", { bubbles: true })); await settle(); }
    button("닫기", host.querySelector(".map-sidebar")!).click(); await settle();

    start = calls.length;
    await navigate("personal", id(999));
    phase("scope popstate rejects previous write seed", start, { graph: 1, read: 1 });
    assert(!bodyIs(`새 본문 ${size}`), "Personal route cannot display the meenseek write response");
    start = calls.length;
    await navigate("meenseek", id(999));
    phase("return route reads its own record", start, { graph: 1, read: 1 });

    button("관리", host.querySelector(".panel-actions")!).click();
    await settle(); button("기록 정정").click();
    await settle();
    const correction = host.querySelector<HTMLFormElement>(".memory-editor form")!;
    enter(correction.querySelector<HTMLTextAreaElement>("textarea")!, `정정 본문 ${size}`);
    enter(correction.querySelector<HTMLInputElement>("input")!, `정정 제목 ${size}`);
    await settle(); start = calls.length;
    const correctionPreserved = () => host.querySelector(".memory-editor form") === correction
      && correction.querySelector("textarea")?.value === `정정 본문 ${size}` && correction.querySelector("input")?.value === `정정 제목 ${size}`;
    for (const query of ["찾을 자료", "결과 없는 합성 검색"]) {
      enter(host.querySelector<HTMLInputElement>("#graph-search")!, query);
      host.querySelector<HTMLFormElement>(".map-search")!.requestSubmit();
      await until(() => idleGraph() && window.location.search.includes("q="), "search while correcting");
      assert(correctionPreserved() && !window.location.search.includes("focus="), "Correction survives search without adding its record to the results");
      if (query === "결과 없는 합성 검색") assert(!!host.querySelector(".map-empty"), "A dirty record does not turn an empty search into a match");
      button(query === "결과 없는 합성 검색" ? "검색·조건 초기화" : "검색 해제").click();
      await until(() => idleGraph() && !window.location.search.includes("q="), "clear search while correcting");
      assert(correctionPreserved(), "Clearing search preserves the same correction editor and fields");
    }
    assert(confirmations === 0, "Search and clear never ask to discard a preserved draft");
    phase("correction search and empty results preserve draft without rereading", start, { graph: 4 });
    start = calls.length;
    const unload = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(unload);
    assert(unload.defaultPrevented, "A correction participates in the shared page-exit guard");
    button("닫기 ×").click(); await settle();
    button("내 지식").click(); await settle();
    host.querySelector<HTMLButtonElement>(".graph-list > button")!.click(); await settle();
    details("보기 설정").summary.click(); await settle(); button("새로고침").click(); await settle();
    assert(Number(confirmations) === 4 && correctionPreserved(), "Declining close, scope, selection and refresh retains the correction");
    phase("declined correction exits perform no reads or writes", start, {});
    failCorrection = true; start = calls.length; correction.requestSubmit();
    await until(() => !!host.querySelector(".memory-editor .error,.brain .error"), "correction conflict");
    assert(correctionPreserved(), "A failed save retains the same correction editor");
    button("현재 기록 다시 읽기").click(); await settle();
    assert(Number(confirmations) === 5 && correctionPreserved(), "Declining conflict recovery retains the dirty draft");
    phase("failed correction and declined reread", start, { correct: 1 });
    start = calls.length; correction.requestSubmit();
    await until(() => idleGraph() && bodyIs(`정정 본문 ${size}`), "correct response reused");
    phase("correct response handoff", start, { correct: 1, graph: 1 });
    if (!details("보기 설정").element.open) details("보기 설정").summary.click(); await settle();
    start = calls.length; button("새로고침").click();
    await until(() => idleGraph() && calls.slice(start).some(call => call.operation === "read") && bodyIs(`정정 본문 ${size}`), "explicit fresh read");
    phase("explicit refresh clears write seed", start, { graph: 1, read: 1 });
    assert(Number(confirmations) === 5, "Successful save clears the dirty guard before explicit refresh");
    button("관리", host.querySelector(".panel-actions")!).click(); await settle(); button("기록 정정").click(); await settle();
    enter(host.querySelector<HTMLTextAreaElement>(".memory-editor form textarea")!, "버릴 정정 초안"); await settle();
    permitDiscard = true; button("닫기 ×").click(); await settle(); permitDiscard = false;
    assert(Number(confirmations) === 6 && !host.querySelector(".management-panel"), "Confirmed close discards the draft");
    const cleanUnload = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(cleanUnload);
    assert(!cleanUnload.defaultPrevented, "Unmounting a discarded editor clears the page-exit guard");
    const originalBody = records.meenseek.get(id(999))!.body;
    await navigate("meenseek", id(999));
    button("관리", host.querySelector(".panel-actions")!).click(); await settle(); button("기록 정정").click(); await settle();
    enter(host.querySelector<HTMLTextAreaElement>(".memory-editor form textarea")!, "다시 선택할 정정 초안"); await settle();
    permitDiscard = true;
    host.querySelector<HTMLButtonElement>('.graph-list > button[aria-pressed="true"]')!.click();
    await until(() => !host.querySelector<HTMLFormElement>(".memory-editor form")?.checkVisibility() && bodyIs(originalBody), "same-record selection discards the correction");
    button("관리", host.querySelector(".panel-actions")!).click(); await settle();
    assert(!host.querySelector<HTMLFormElement>(".memory-editor form")?.checkVisibility(), "Confirmed same-record selection ends editing rather than hiding a dirty draft");
    button("기록 정정").click(); await settle();
    enter(host.querySelector<HTMLTextAreaElement>(".memory-editor form textarea")!, "검색 뒤 새로고침할 초안"); await settle();
    enter(host.querySelector<HTMLInputElement>("#graph-search")!, "찾을 자료"); host.querySelector<HTMLFormElement>(".map-search")!.requestSubmit();
    await until(() => idleGraph() && window.location.search.includes("q="), "retained editor before refresh");
    if (!details("보기 설정").element.open) details("보기 설정").summary.click(); await settle(); button("새로고침").click();
    await until(() => idleGraph() && !host.querySelector(".management-panel"), "confirmed refresh closes the retained unselected editor");
    assert(document.activeElement !== document.body && host.contains(document.activeElement), "Confirmed refresh returns focus from the closed panel to the app");
    button("검색 해제").click(); await until(() => idleGraph() && !window.location.search.includes("q="), "clear query after discarded refresh");
    assert(records.meenseek.get(id(999))!.body === originalBody, "Confirmed exits never write a discarded draft");
    permitDiscard = false;
    await navigate("meenseek", id(999));
    button("관리", host.querySelector(".panel-actions")!).click(); await settle(); button("기록 정정").click(); await settle();
    const orphanedDraft = host.querySelector<HTMLFormElement>(".memory-editor form")!;
    enter(orphanedDraft.querySelector<HTMLTextAreaElement>("textarea")!, "복귀 때 원본이 사라져도 남을 초안"); await settle();
    const priorRecord = records.meenseek.get(id(999))!;
    records.meenseek.delete(id(999)); // A separate client removes the synthetic record while this editor is open.
    start = calls.length;
    narrowMedia.matches = true; narrowMedia.dispatchEvent(new Event("change")); await settle();
    screenVisible = false; document.dispatchEvent(new Event("visibilitychange")); await settle();
    screenVisible = true; document.dispatchEvent(new Event("visibilitychange"));
    await until(() => idleGraph() && !window.location.search.includes("focus="), "resume after external record deletion");
    assert(host.querySelector(".memory-editor form") === orphanedDraft && orphanedDraft.querySelector("textarea")?.value === "복귀 때 원본이 사라져도 남을 초안", "A missing focus response preserves the same unsaved correction");
    const resumeNotice = host.querySelector<HTMLElement>('.management-panel [aria-live="polite"] .notice');
    assert(resumeNotice && resumeNotice.textContent?.includes("초안은 유지") && resumeNotice.checkVisibility() && !resumeNotice.closest("[inert]"), "Missing-record recovery is readable inside the active panel live region");
    if (innerWidth <= 700) {
      const box = resumeNotice.getBoundingClientRect();
      assert(box.left >= 0 && box.right <= innerWidth && box.top >= 0 && box.bottom <= innerHeight, "The retained-draft notice fits the mobile panel viewport");
    }
    const orphanedUnload = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(orphanedUnload);
    assert(orphanedUnload.defaultPrevented, "The retained orphaned draft still blocks accidental page exit");
    phase("visibility resume with missing record retains draft without rereading", start, { graph: 2 });
    records.meenseek.set(priorRecord.id, priorRecord);
    permitDiscard = true; button("닫기 ×").click(); await settle(); permitDiscard = false;
    narrowMedia.matches = false; narrowMedia.dispatchEvent(new Event("change")); await settle();

    start = calls.length; button("기록 남기기").click(); await settle();
    const blankForm = host.querySelector<HTMLFormElement>(".memory-editor form")!;
    enter(blankForm.querySelector<HTMLTextAreaElement>("textarea")!, "다시 시작할 새 기록"); await settle();
    button("기록 남기기").click(); await settle();
    assert(host.querySelector(".memory-editor form") === blankForm, "Declining a new-record restart retains its draft");
    permitDiscard = true; button("기록 남기기").click(); await settle(); permitDiscard = false;
    assert(host.querySelector(".memory-editor form") !== blankForm && host.querySelector<HTMLTextAreaElement>(".memory-editor form textarea")?.value === "", "Confirmed new-record restart creates a clean editor");
    phase("SyncPanel closed", start, {});
    const sync = details("출처 갱신");
    start = calls.length; sync.summary.click();
    await until(() => sync.summary.textContent?.includes("꺼짐") === true, "sync loaded");
    phase("SyncPanel first open", start, { sync: 1 });
    start = calls.length; sync.summary.click(); await settle(); sync.summary.click(); await settle();
    phase("SyncPanel cached reopen", start, {});
    failSync = true; start = calls.length; button("상태 새로고침", sync.element).click();
    await until(() => !!sync.element.querySelector(".error"), "sync failure");
    phase("SyncPanel explicit failure", start, { sync: 1 });
    start = calls.length; sync.summary.click(); await settle(); sync.summary.click(); await settle();
    phase("SyncPanel failure does not retry on reopen", start, {});
    start = calls.length; button("상태 새로고침", sync.element).click();
    await until(() => !sync.element.querySelector(".error") && sync.summary.textContent?.includes("꺼짐") === true, "sync explicit retry");
    phase("SyncPanel explicit retry", start, { sync: 1 });
    button("닫기 ×").click(); await settle();
    start = calls.length;
    window.history.pushState(null, "", `/?scope=personal&focus=${subjectId}`);
    window.dispatchEvent(new PopStateEvent("popstate"));
    await until(() => idleGraph() && host.querySelector(".management-panel h2")?.textContent === "빈 묶음", "empty subject selected");
    button("묶음 삭제").click(); await settle();
    assert(calls.length - start === 1, "Delete confirmation has no network call");
    button("삭제 확인").click();
    await until(() => idleGraph() && !subjectExists && !host.querySelector(".management-panel") && !window.location.search.includes("focus="), "subject deletion refreshed graph");
    phase("empty subject deletion from marker", start, { graph: 2, "subject-delete": 1 });
    assert(!forbidden.length, "No unexpected or forwarded requests");
    return { size, passed: true, phases, calls, totalResponseBytes: calls.reduce((sum, call) => sum + call.responseBytes, 0), forbidden };
  } finally {
    root.unmount(); releaseGraph();
    window.confirm = originalConfirm;
    window.fetch = originalFetch; window.matchMedia = originalMatchMedia;
    window.history.replaceState(null, "", originalUrl);
    if (visibility) Object.defineProperty(document, "visibilityState", visibility);
    else Reflect.deleteProperty(document, "visibilityState");
  }
}

async function checkDocumentEditing() {
  const originalFetch = window.fetch, originalConfirm = window.confirm, originalUrl = window.location.href, root = createRoot(host);
  const documentId = `e_${"1".repeat(64)}`, targetId = `e_${"2".repeat(64)}`;
  let revision = 1, topics = ["처음 태그"], linked = false, confirmations = 0, permitDiscard = false;
  let failSave = false, failRead = false, graphReads = 0, detailReads = 0, writes = 0;
  const calls: Call[] = [], revisions: number[] = [], history: object[] = [];
  active = { size: 2, calls, phases: [], forbidden: [] };
  const definition = { purpose: "합성 목적", include: "합성 포함", exclude: "합성 제외" };
  const body = "# 합성 문서\n\n현재 문서 본문";
  const json = (call: Call, value: unknown, status = 200) => {
    const text = JSON.stringify(value); call.status = status; call.responseBytes = encode(text);
    return new Response(text, { status, headers: { "content-type": "application/json" } });
  };
  const tag = () => host.querySelector<HTMLTextAreaElement>("#topics")!;
  const select = () => host.querySelector<HTMLSelectElement>('select[aria-label="연결할 자료"]')!;
  const panel = () => host.querySelector<HTMLElement>(".management-panel")!;
  async function open() {
    window.history.pushState(null, "", `/?scope=personal&focus=${documentId}`); window.dispatchEvent(new PopStateEvent("popstate"));
    await until(() => !!host.querySelector(".documents .document-preview"), "standalone document opens");
    button("관리").click(); await settle(); details("분류 수정").summary.click(); await settle();
  }
  try {
    window.confirm = () => { confirmations++; return permitDiscard; };
    window.fetch = async (input, options) => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      const payload = typeof options?.body === "string" ? JSON.parse(options.body) : null;
      const operation = payload?.op ?? (url.pathname === "/api/graph" ? "graph" : url.pathname === "/api/session" ? "session" : url.pathname === "/api/records" ? "candidates" : url.pathname.endsWith("/classification") ? "classification" : url.pathname.endsWith("/links") ? "links" : "detail");
      const call: Call = { operation, scope: payload?.scope ?? url.searchParams.get("scope"), status: 0, requestBytes: typeof options?.body === "string" ? encode(options.body) : 0, responseBytes: 0 };
      calls.push(call);
      if (options?.signal?.aborted) throw new DOMException("Aborted", "AbortError");
      if (url.pathname === "/api/session") return json(call, { csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") {
        graphReads++;
        const scope = url.searchParams.get("scope") as Scope, focus = url.searchParams.get("focus");
        const nodes: GraphNode[] = [documentId, targetId].map((id, index) => ({ id, scope, kind: "document", label: index ? "other.md" : "fixture.md", title: index ? "다른 문서" : "합성 문서", revision: String(index ? 1 : revision), current: true, present: true, status: "ok", source_kind: "git" }));
        const links = linked ? [{ source: documentId, target: targetId, kind: "related" as const, current: true }] : [];
        return json(call, { ...graph([], scope, url.searchParams.get("q") ?? "", focus, false), nodes, links, totals: { documents: 2, memories: 0, markers: 0, links: links.length }, returned: { knowledge: 2, markers: 0, links: links.length }, matched: 2 });
      }
      if (url.pathname === `/api/records/${documentId}`) {
        detailReads++;
        if (failRead) { failRead = false; return json(call, { error: "synthetic detail failure" }, 503); }
        return json(call, { id: documentId, scope: "personal", revision, current: true, areas: [], topics,
          source: { kind: "git", repository: "/synthetic/documents", path: "fixture.md", status: "ok", last_success_at: "2026-01-01T00:00:00Z" }, projection: { content: body, present: true },
          related: linked ? [{ id: targetId, path: "other.md" }] : [], history });
      }
      if (url.pathname === `/api/records/${documentId}/classification` || url.pathname === `/api/records/${documentId}/links`) {
        assert(options?.method === "POST" && options.headers && payload.revision === revision, "each mutation uses the latest confirmed document revision");
        revisions.push(payload.revision); writes++;
        if (failSave) { failSave = false; return json(call, { error: "synthetic conflict" }, 409); }
        if (operation === "classification") topics = payload.topics;
        else { assert(payload.target_id === targetId, "link target stays in the fixture scope"); linked = !payload.remove; }
        revision++; history.unshift({ id: revision, revision, kind: operation === "classification" ? "classification" : payload.remove ? "link-remove" : "link-add", previous: {}, confirmed: { areas: [], topics }, confirmed_at: "2026-01-01T00:00:00Z" });
        return json(call, { ok: true });
      }
      if (url.pathname === "/api/records") return json(call, { items: [{ id: targetId, path: "other.md" }], total: 1, limit: 100, areas: [] });
      if (url.pathname === "/api/brain" && payload.op === "document-subject") return json(call, { revision: 1, subject_id: null, subject_revision: null, reason: null, review_needed: false, current_source: { source_revision: "a".repeat(40), content_digest: "b".repeat(64) } });
      if (url.pathname === "/api/brain" && payload.op === "subjects") return json(call, { items: [{ id: subjectId, name: "합성 목적", revision: 1, definition }], next_after: null });
      throw new Error(`Unexpected standalone document request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal&focus=${documentId}`); root.render(<App />);
    await until(() => !!host.querySelector(".documents .document-preview"), "initial standalone detail");
    button("관리").click(); await settle(); details("분류 수정").summary.click(); await settle();
    enter(tag(), "보호할 태그 초안"); await settle(); const draft = tag();
    button("읽기로 돌아가기").click(); await settle(); button("관리").click(); await settle();
    assert(tag() === draft && tag().value === "보호할 태그 초안", "reading/management keeps the actual draft element");
    button("닫기 ×").click(); await settle();
    assert(Number(confirmations) === 1 && panel() && tag().value === "보호할 태그 초안", "close cancellation preserves a tag draft");
    button("다시 불러오기", host.querySelector(".provenance")!).click(); await settle();
    assert(Number(confirmations) === 2 && Number(detailReads) === 1 && tag().value === "보호할 태그 초안", "local reload cancellation makes no read and preserves the draft");
    button("meenseek 사업").click(); await settle();
    assert(Number(confirmations) === 3 && new URLSearchParams(window.location.search).get("scope") === "personal", "scope cancellation protects the standalone draft");
    const unload = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(unload);
    assert(unload.defaultPrevented, "standalone draft participates in beforeunload protection");
    window.history.pushState(null, "", "/?scope=meenseek"); window.dispatchEvent(new PopStateEvent("popstate")); await settle();
    assert(Number(confirmations) === 4 && tag().value === "보호할 태그 초안" && new URLSearchParams(window.location.search).get("scope") === "personal", "back/forward cancellation restores the route and draft");
    failSave = true; button("분류 확인 저장").click();
    await until(() => !!host.querySelector(".documents .error"), "classification conflict");
    assert(tag().value === "보호할 태그 초안" && Number(writes) === 1 && Number(detailReads) === 1, "failed save retains the original revision and draft without refetch");
    button("분류 확인 저장").click();
    await until(() => Number(detailReads) === 2 && host.querySelector(".classification-summary")?.textContent === "보호할 태그 초안" && !panel()?.querySelector<HTMLButtonElement>('button[aria-label="관리 패널 닫기"]')?.disabled, "first successful classification refresh");
    assert(Number(graphReads) === 2 && tag() === draft && !host.querySelector(".documents .error"), "successful save refetches detail and graph once without remounting the editor");
    enter(tag(), "두 번째 저장"); await settle(); button("분류 확인 저장").click();
    await until(() => Number(revision) === 3 && Number(detailReads) === 3 && host.querySelector(".classification-summary")?.textContent === "두 번째 저장", "second classification uses the refreshed revision");
    button("분류·연결").click(); await settle(); button("목적 소속 관리").click();
    await until(() => !!host.querySelector(".document-purpose textarea"), "concurrent purpose draft");
    enter(host.querySelector<HTMLTextAreaElement>(".document-purpose textarea")!, "보존할 소속 이유"); enter(tag(), "연결 저장 동안 유지할 태그"); await settle();
    details("자료 연결 수정").summary.click(); await settle(); button("찾기", host.querySelector(".related-search")!).click();
    await until(() => select().options.length === 2, "link candidates"); select().value = targetId; select().dispatchEvent(new Event("change", { bubbles: true })); await settle();
    failRead = true; button("연결 추가").click();
    await until(() => !!host.querySelector(".documents .warning")?.textContent?.includes("저장은 완료"), "saved link with failed follow-up read");
    assert(linked && Number(writes) === 4 && tag().value === "연결 저장 동안 유지할 태그" && button("소속 저장").disabled === false, "committed link retains the other two drafts");
    const classificationSave = [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === "분류 확인 저장")!;
    assert(classificationSave.disabled, "a stale document revision cannot be used for another mutation");
    const beforeRetry = confirmations;
    button("다시 불러오기", host.querySelector(".documents .error")!).click();
    await until(() => !host.querySelector(".documents .warning") && !classificationSave.disabled, "follow-up detail retry");
    assert(Number(confirmations) === beforeRetry && tag().value === "연결 저장 동안 유지할 태그" && host.querySelector<HTMLTextAreaElement>(".document-purpose textarea")?.value === "보존할 소속 이유", "read retry preserves both drafts without prompting to discard");
    button("연결 해제").click(); await until(() => !linked && Number(detailReads) === 6 && !panel().querySelector(".documents .related"), "link removal refreshes related data");
    assert(tag().value === "연결 저장 동안 유지할 태그", "link removal does not reset classification input");
    button("분류 확인 저장").click(); await until(() => Number(revision) === 6 && Number(detailReads) === 7 && !classificationSave.disabled, "classification after link uses current revision");
    button("닫기 ×").click(); await settle(); assert(Number(confirmations) === beforeRetry + 1 && panel(), "saving document metadata cannot clear the separate purpose draft guard");
    button("취소", host.querySelector(".document-purpose")!).click(); await settle(); button("닫기 ×").click();
    await until(() => !panel(), "all saved/cancelled drafts can close");
    await open(); assert(tag().value === "연결 저장 동안 유지할 태그", "reopening reads the latest committed tags");
    enter(tag(), "명시적으로 버릴 초안"); await settle(); permitDiscard = true;
    button("다시 불러오기", host.querySelector(".provenance")!).click();
    await until(() => !!tag() && tag().value === "연결 저장 동안 유지할 태그", "accepted local discard rereads the record");
    permitDiscard = false;
    details("자료 연결 수정").summary.click(); await settle(); button("찾기", host.querySelector(".related-search")!).click();
    await until(() => select().options.length === 2, "link candidates after reload"); select().value = targetId; select().dispatchEvent(new Event("change", { bubbles: true })); await settle();
    const beforeLinkGuard = confirmations; button("닫기 ×").click(); await settle();
    assert(Number(confirmations) === beforeLinkGuard + 1 && select().value === targetId, "an unsaved link selection is also protected");
    select().value = ""; select().dispatchEvent(new Event("change", { bubbles: true })); await settle();
    enter(tag(), "재선택으로 버릴 태그"); await settle();
    button("분류·연결").click(); await settle(); button("목적 소속 관리").click();
    await until(() => !!host.querySelector(".document-purpose textarea"), "purpose draft before same-document selection");
    enter(host.querySelector<HTMLTextAreaElement>(".document-purpose textarea")!, "재선택으로 버릴 소속"); await settle();
    const listToggle = [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === "목록 보기");
    listToggle?.click(); await settle();
    const oldTag = tag(), beforeReselect = confirmations;
    permitDiscard = true; host.querySelector<HTMLButtonElement>('.graph-list > button[aria-pressed="true"]')!.click();
    await until(() => !!tag() && tag() !== oldTag && tag().value === "연결 저장 동안 유지할 태그", "same-document selection actually discards its drafts");
    assert(Number(confirmations) === beforeReselect + 1 && !host.querySelector(".document-purpose form"), "same-identity discard resets both document and purpose editors");
    const cleanUnload = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(cleanUnload);
    assert(!cleanUnload.defaultPrevented, "the discarded same-document editor has no remaining dirty owner");
    button("관리").click(); await settle(); details("분류 수정").summary.click(); await settle();
    enter(tag(), "다시 보호할 태그"); await settle(); permitDiscard = false;
    button("닫기 ×").click(); await settle();
    assert(Number(confirmations) === beforeReselect + 2 && tag().value === "다시 보호할 태그", "a fresh draft regains protection after same-document discard");
    enter(tag(), "연결 저장 동안 유지할 태그"); await settle(); button("닫기 ×").click(); await until(() => !panel(), "restoring the draft clears its guard");
    assert(revisions.join(",") === "1,1,2,3,4,5", "consecutive writes use refreshed CAS revisions including an initial conflict");
    active = null;
    return { documentEditing: true, confirmations, revisions, graphReads, detailReads, writes, requestBytes: calls.reduce((sum, call) => sum + call.requestBytes, 0), responseBytes: calls.reduce((sum, call) => sum + call.responseBytes, 0), calls };
  } finally { root.unmount(); window.fetch = originalFetch; window.confirm = originalConfirm; window.history.replaceState(null, "", originalUrl); }
}

async function checkPurposeDefinitionRefresh() {
  const originalFetch = window.fetch, originalUrl = window.location.href, originalConfirm = window.confirm, root = createRoot(host);
  const node: GraphNode = { id: subjectId, scope: "personal", kind: "subject", label: "합성 목적", revision: "1",
    definition: { purpose: "현재 기준", include: "포함", exclude: "제외" } };
  const snapshot: Snapshot = { ...graph([], "personal", "", subjectId, false), nodes: [node],
    totals: { documents: 0, memories: 0, markers: 1, links: 0 }, returned: { knowledge: 0, markers: 1, links: 0 } };
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  let confirmations = 0, permit = false, graphCalls = 0;
  try {
    window.confirm = () => { confirmations++; return permit; };
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") { graphCalls++; return json(snapshot); }
      throw new Error(`Unexpected purpose refresh request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal&focus=${subjectId}`);
    root.render(<App />); await until(() => !!host.querySelector(".management-panel"), "purpose marker opens");
    button("목적 정의 편집").click(); await settle();
    const draft = host.querySelector<HTMLTextAreaElement>(".management-panel textarea")!;
    enter(draft, "새로고침 전 초안"); await settle();
    details("보기 설정").summary.click(); await settle();
    const before = graphCalls; button("새로고침").click(); await settle();
    assert(confirmations === 1 && graphCalls === before && draft === host.querySelector(".management-panel textarea") && draft.value === "새로고침 전 초안", "declined refresh retains the exact dirty editor");
    permit = true; button("새로고침").click();
    await until(() => !host.querySelector(".management-panel textarea") && graphCalls > before, "confirmed refresh actually resets the definition editor");
    button("목적 정의 편집").click(); await settle();
    const fresh = host.querySelector<HTMLTextAreaElement>(".management-panel textarea")!;
    assert(fresh !== draft && fresh.value === "현재 기준", "new editor starts from confirmed graph criteria");
    enter(fresh, "새 편집 초안"); await settle(); permit = false;
    button("닫기 ×").click(); await settle();
    assert(Number(confirmations) === 3 && !!host.querySelector(".management-panel") && String(fresh.value) === "새 편집 초안", "new dirty draft restores navigation protection after refresh");
    return { purposeDefinitionRefresh: true, canceledRefreshPreservesDraft: true, confirmedRefreshResetsEditor: true, newDraftGuard: true };
  } finally { root.unmount(); window.fetch = originalFetch; window.confirm = originalConfirm; window.history.replaceState(null, "", originalUrl); }
}

async function checkSourceReferenceDirection() {
  const originalFetch = window.fetch, originalUrl = window.location.href, root = createRoot(host);
  const originals: GraphNode[] = [1, 2, 3].map(index => ({ id: `c_00000000-0000-4000-8000-00000000030${index}`, material_id: `00000000-0000-4000-8000-00000000030${index}`,
    scope: "personal", kind: "document", label: `원문 ${index}`, title: `원문 ${index}`, revision: "1", source_kind: "original", context_scope: index === 3 ? "profile" : "personal", context_path: `${index === 3 ? "rules" : "references"}/${index}.md`, status: "ok", present: true, current: true }));
  const links: Snapshot["links"] = [{ source: originals[0].id, target: originals[1].id, kind: "reference", current: true }, { source: originals[2].id, target: originals[0].id, kind: "reference", current: true }, { source: originals[0].id, target: originals[2].id, kind: "reference", current: true }];
  const snapshot: Snapshot = { ...graph([], "personal", "", originals[0].id, false), nodes: originals, links, totals: { documents: 3, memories: 0, markers: 0, links: 3 }, returned: { knowledge: 3, markers: 0, links: 3 }, matched: 3 };
  let brainCalls = 0, graphCalls = 0, originalReads = 0;
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  try {
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") { graphCalls++; return json(snapshot); }
      if (url.pathname === "/api/brain") { brainCalls++; throw new Error("reference reading cannot fetch purpose/evidence"); }
      if (url.pathname === "/api/context/history") return json({ items: [], next_before: null });
      if (url.pathname === "/api/context/read") {
        originalReads++; const path = url.searchParams.get("path")!; const index = path.split('/').at(-1)!.split('.')[0];
        assert(url.searchParams.get("scope") === (index === "3" ? "profile" : "personal"), "reference opens the target's exact native scope");
        const content = `# 원문 ${index}\n\n${index === "1" ? `${Array.from({ length: 30 }, (_, i) => `합성 본문 ${i + 1}. 긴 원문을 읽은 뒤 연결된 자료로 이동합니다.`).join("\n\n")}\n\n[본문에서 대상 열기](2.md#part) [다른 범위 원문 열기](../../profile/rules/3.md)` : "대상 본문"}`;
        return json({ metadata: { scope: url.searchParams.get("scope"), path, source_path: "synthetic", revision: 1, content_digest: "synthetic", byte_len: encode(content) }, title: `원문 ${index}`, content });
      }
      throw new Error(`Unexpected reference reading request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal&focus=${originals[0].id}`);
    root.render(<App />);
    await until(() => !!host.querySelector(".original-detail .document-internal-link"), "outgoing authored link resolves in actual preview");
    const context = host.querySelector<HTMLDetailsElement>(".document-context")!;
    assert(!context.open, "document classification and links start collapsed after the body");
    assert(!!(host.querySelector('.original-detail')!.compareDocumentPosition(context) & Node.DOCUMENT_POSITION_FOLLOWING), "title and body precede classification in the reading order");
    assert(!host.querySelector('.original-detail [aria-label="연결된 자료"]'), "original reference does not become an explicit semantic related item");
    if (innerWidth > 900) {
      const legend = details("관계·상태 읽기"); legend.summary.click(); await settle();
      const guide = host.querySelector<HTMLElement>('.legend-guide')!;
      guide.focus(); guide.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); await settle();
      assert(!legend.element.open && !!host.querySelector('.management-panel') && document.activeElement === legend.summary, "guide Escape preserves the selected detail and its local focus");
      legend.summary.click(); await settle();
      legend.summary.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); await settle();
      assert(!legend.element.open && !!host.querySelector('.management-panel') && document.activeElement === legend.summary, "summary Escape also closes only its open guide");
    }
    button("분류·연결").click(); await settle();
    assert(context.open && document.activeElement === context.querySelector("summary"), "the header reaches classification with keyboard focus and no eager request");
    const before = graphCalls; details("원문 링크").summary.click(); await settle();
    const referenceButtons = [...host.querySelectorAll<HTMLButtonElement>(".source-references button")];
    assert(referenceButtons.length === 3, "selected source retains outgoing and incoming references");
    assert(referenceButtons.some(value => value.textContent?.includes("원문 링크 →") && value.textContent?.includes("원문 2")), "outgoing endpoint label");
    assert(referenceButtons.some(value => value.textContent?.includes("← 이 원문을 참조한 자료") && value.textContent?.includes("원문 3")), "incoming endpoint label");
    assert(brainCalls === 0 && graphCalls === before && originalReads === 1, "disclosure reuses returned link metadata without eager API reads");
    host.querySelector<HTMLButtonElement>(".original-detail .document-internal-link")!.click();
    await until(() => host.querySelector(".original-detail h1")?.textContent === "원문 2", "authored outgoing link opens its exact target");
    assert(new URL(window.location.href).searchParams.get("focus") === originals[1].id && brainCalls === 0, "inline reference navigation preserves exact target and lazy purpose reads");
    button("닫기 ×").click(); await until(() => !host.querySelector(".management-panel"), "return before filtering references");
    const listToggle = [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === "목록 보기");
    if (listToggle) { listToggle.click(); await settle(); }
    await openExplorer();
    const sourceFilter = host.querySelector<HTMLSelectElement>(".topic-label select")!;
    sourceFilter.value = "personal"; sourceFilter.dispatchEvent(new Event("change", { bubbles: true })); await settle();
    button("닫기", host.querySelector(".map-sidebar")!).click(); await settle();
    [...host.querySelectorAll<HTMLButtonElement>(".graph-list > button")].find(node => node.querySelector("strong")?.textContent === "원문 1")!.click();
    await until(() => host.querySelectorAll(".original-detail .document-internal-link").length === 2, "return to both authored outgoing references");
    const crossScopeReference = button("다른 범위 원문 열기", host.querySelector(".original-detail")!);
    crossScopeReference.scrollIntoView({ block: "center" }); await settle();
    assert(host.querySelector<HTMLElement>(".management-panel")!.scrollTop > 100, "outside-filter navigation follows a link after reading and scrolling a long original");
    crossScopeReference.click();
    await until(() => host.querySelector(".original-detail h1")?.textContent === "원문 3", "cross-native-scope reference opens its exact current target");
    assert(new URL(window.location.href).searchParams.get("focus") === originals[2].id && brainCalls === 0, "cross-scope source reference does not promote incoming evidence or change app scope");
    const outsideNotice = host.querySelector<HTMLElement>(".management-panel .notice")!;
    assert(outsideNotice?.textContent?.includes("필터 밖") && !outsideNotice.closest("[inert]") && host.querySelectorAll(".graph-list > button").length === 2, "following a reference outside the filter shows its notice in the active detail without changing the visible map set");
    const noticeBounds = outsideNotice.getBoundingClientRect();
    assert(noticeBounds.top >= 0 && noticeBounds.bottom <= innerHeight && noticeBounds.left >= 0 && noticeBounds.right <= innerWidth && outsideNotice.contains(document.elementFromPoint((noticeBounds.left + noticeBounds.right) / 2, (noticeBounds.top + noticeBounds.bottom) / 2)), "outside-filter guidance is visible and uncovered in the current viewport");
    assert(!host.querySelector(".map-status .notice"), "outside-filter guidance is not duplicated behind the active detail");
    button("닫기 ×").click(); await until(() => !host.querySelector(".management-panel"), "close out-of-filter reference");
    await openExplorer();
    assert(host.querySelector<HTMLSelectElement>(".topic-label select")?.value === "personal", "out-of-filter navigation preserves the original source condition");
    button("닫기", host.querySelector(".map-sidebar")!).click(); await settle();
    [...host.querySelectorAll<HTMLButtonElement>(".graph-list > button")].find(node => node.querySelector("strong")?.textContent === "원문 1")!.click();
    await until(() => host.querySelectorAll(".original-detail .document-internal-link").length === 2, "return to the filtered reference before explicitly clearing");
    button("다른 범위 원문 열기", host.querySelector(".original-detail")!).click();
    await until(() => host.querySelector(".original-detail h1")?.textContent === "원문 3", "reopen outside-filter detail");
    const clear = button("필터 해제", host.querySelector(".management-panel")!); clear.focus(); clear.click(); await settle();
    assert(!host.querySelector(".management-panel .notice") && host.querySelectorAll(".graph-list > button").length === 3 && new URL(location.href).searchParams.get("focus") === originals[2].id, "explicit clear includes the selected target while preserving its detail");
    assert(document.activeElement === button("닫기 ×"), "keyboard clear keeps focus in the active detail after removing its action");
    return { sourceReferenceDirection: true, outgoing: 2, incoming: 1, inlineNavigation: true, fragmentLookup: true, crossNativeScope: true, outsideFilterPreserved: true, outsideFilterVisible: true, explicitFilterClear: true, eagerBrainCalls: 0 };
  } finally { root.unmount(); window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl); }
}

async function checkPurposeAndOriginalBusy() {
  const originalFetch = window.fetch, originalUrl = window.location.href, root = createRoot(host);
  const material = "00000000-0000-4000-8000-000000000201", nodeId = `c_${material}`;
  const node: GraphNode = { id: nodeId, material_id: material, scope: "personal", kind: "document", label: "합성 원문", revision: "1",
    source_kind: "original", context_scope: "personal", context_path: "notes/busy.md", status: "ok", present: true, current: true };
  const snapshot: Snapshot = { ...graph([], "personal", "", nodeId, false), nodes: [node],
    totals: { documents: 1, memories: 0, markers: 0, links: 0 }, returned: { knowledge: 1, markers: 0, links: 0 }, matched: 1 };
  let content = "# 합성 원문\n\n기존 내용", revision = 1, digest = "initial", purposeCalls = 0, editCalls = 0;
  let releasePurpose!: () => void, releaseEdit!: () => void;
  const purposeWait = new Promise<void>(resolve => { releasePurpose = resolve; });
  const editWait = new Promise<void>(resolve => { releaseEdit = resolve; });
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  try {
    window.fetch = async (input, options) => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") return json(snapshot);
      if (url.pathname === "/api/context/read") return json({ metadata: { scope: "personal", path: "notes/busy.md", source_path: "synthetic", revision, origin_kind: "native", source_digest: null, content_digest: digest, byte_len: encode(content) }, content, title: "합성 원문" });
      if (url.pathname === "/api/context/history") return json({ items: [], next_before: null });
      const body = typeof options?.body === "string" ? JSON.parse(options.body) : null;
      if (url.pathname === "/api/brain" && body.op === "document-subject") {
        purposeCalls++; assert(body.scope === "personal" && body.document.material_id === material, "purpose request identity");
        await purposeWait;
        return json({ revision: 0, subject_id: null, subject_revision: null, reason: null, review_needed: false,
          current_source: { source_revision: String(revision), content_digest: digest } });
      }
      if (url.pathname === "/api/context/edit") {
        editCalls++; assert(body.expected_revision === revision && body.expected_digest === digest, "original retains its source CAS");
        await editWait; content = body.content; revision++; digest = "saved";
        return json({ changed: true, revision, content_digest: digest });
      }
      throw new Error(`Unexpected overlapping request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal&focus=${nodeId}`);
    root.render(<App />);
    await until(() => !!host.querySelector(".original-detail .document-preview"), "busy original selected");
    assert(purposeCalls === 0, "purpose panel has no eager request");
    button("편집").click(); await settle();
    enter(host.querySelector<HTMLTextAreaElement>("#original-draft")!, "# 합성 원문\n\n저장할 변경"); await settle();
    button("분류·연결").click(); await settle();
    assert(host.querySelector<HTMLTextAreaElement>("#original-draft")?.value.includes("저장할 변경"), "classification navigation preserves the original draft");
    button("목적 소속 관리").click(); await until(() => purposeCalls === 1, "purpose request held");
    const close = () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(value => value.textContent?.trim() === "닫기 ×")!;
    assert(close().disabled, "purpose request blocks navigation");
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true, cancelable: true }));
    await until(() => editCalls === 1, "original keyboard save held alongside purpose request");
    releasePurpose(); await until(() => !!host.querySelector(".document-purpose form"), "purpose request completed first");
    assert(close().disabled && host.querySelector<HTMLTextAreaElement>("#original-draft")?.disabled, "finishing one owner cannot clear the other owner's busy");
    releaseEdit(); await until(() => host.querySelector(".original-detail .notice")?.textContent?.includes("다시 확인") === true, "original save independently verified");
    assert(!close().disabled && content.includes("저장할 변경"), "navigation unlocks only after both owners finish");
    button("취소", host.querySelector(".document-purpose")!).click(); await settle();
    close().click(); await until(() => !host.querySelector(".management-panel"), "panel closes after both requests finish");
    return { purposeAndOriginalBusy: true, overlappingRequests: 2, sourceCAS: true, ownershipPreserved: true };
  } finally { releasePurpose(); releaseEdit(); root.unmount(); window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl); }
}

async function checkOriginalFolders() {
  const originalFetch = window.fetch;
  const originalConfirm = window.confirm;
  const originalUrl = window.location.href;
  const root = createRoot(host);
  const original = (id: string, context_scope: string, context_path: string): GraphNode => ({
    id, scope: "personal", kind: "document", label: id === "folder-c" ? "지원 현황" : "다른 지원 자료", title: id === "folder-c" ? "지원 현황" : "다른 지원 자료",
    revision: "1", status: "ok", present: true, current: true, source_kind: "original", context_scope, context_path,
  });
  const nodes = [
    original("folder-a", "personal", "writing/2026/a.md"),
    { ...original("folder-b", "personal", "writing/2026/b.md"), status: "failed", present: false },
    original("folder-c", "personal", "writing/2025/c.md"),
    original("folder-d", "work/common", "writing/2026/d.md"),
    { id: "unrelated-memory", scope: "personal", kind: "memory", label: "독립 기록", status: "accepted", temporal: "current", supported: true } satisfies GraphNode,
  ];
  const snapshot: Snapshot = {
    scope: "personal", query: "", focus: { id: null, found: false }, nodes, links: [], matched: nodes.length,
    totals: { documents: 4, memories: 1, markers: 0, links: 0 }, returned: { knowledge: 5, markers: 0, links: 0 },
    omitted: { nodes: 0, links: 0 }, eligible: { nodes: 5, links: 0 },
    limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false,
  };
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  let graphCalls = 0;
  const folderButtons = () => [...host.querySelectorAll<HTMLButtonElement>(".folder-list .compact-list > button")];
  try {
    window.fetch = async (input, options) => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") {
        graphCalls++;
        assert(url.searchParams.get("scope") === "personal" && !url.searchParams.get("q") && [null, "folder-c"].includes(url.searchParams.get("focus")), "Graph request stays in the personal scope and selected original");
        return json(snapshot);
      }
      if (url.pathname === "/api/context/read") {
        assert(url.searchParams.get("scope") === "personal" && url.searchParams.get("path") === "writing/2025/c.md", "Read the selected personal original");
        return json({
          metadata: { scope: "personal", path: "writing/2025/c.md", source_path: "synthetic", revision: 1, origin_kind: "native", source_digest: null, content_digest: "synthetic", byte_len: 30 },
          content: "지원\n현황\n====\n\n본문", title: "지원 현황",
        });
      }
      if (url.pathname === "/api/context/history") {
        assert(url.searchParams.get("scope") === "personal" && url.searchParams.get("path") === "writing/2025/c.md", "History belongs to the selected personal original");
        return json({ items: [], next_before: null });
      }
      throw new Error(`Unexpected original check request: ${options?.method ?? "GET"} ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal`);
    root.render(<App />);
    await until(() => !!host.querySelector(".map-actions"), "original App controls");
    button("목록 보기").click();
    button("필터·묶음").click();
    await until(() => folderButtons().length === 7, "scope roots and original folder ancestry");
    assert(host.querySelector(".cluster-list .hint")?.textContent === "0개 묶음 · 단독 항목 5개", "Sharing a folder cannot turn isolated knowledge into a relation community");
    const source = host.querySelector<HTMLSelectElement>(".topic-label select")!;
    source.value = "personal";
    source.dispatchEvent(new Event("change", { bubbles: true }));
    await until(() => folderButtons().length === 4, "personal folder ancestry only");
    const folders = folderButtons().map(node => [node.querySelector("strong")?.textContent, node.querySelector("small")?.textContent]);
    assert(JSON.stringify(folders) === JSON.stringify([
      ["personal · 최상위", "하위 원문 3개"],
      ["personal · writing", "하위 원문 3개"],
      ["personal · writing/2025", "하위 원문 1개"],
      ["personal · writing/2026", "하위 원문 2개"],
    ]), "Folder names, order and document counts match the visible originals");
    assert(host.querySelector(".cluster-list .hint")?.textContent === "0개 묶음 · 단독 항목 3개", "Filtered sidebar counts knowledge separately from structural ancestry");
    const state = host.querySelector<HTMLSelectElement>(".filter-row label:nth-child(2) select")!;
    state.value = "active";
    state.dispatchEvent(new Event("change", { bubbles: true }));
    await until(() => folderButtons().find(node => node.querySelector("strong")?.textContent === "personal · writing/2026")?.querySelector("small")?.textContent === "하위 원문 1개", "partially filtered folder count");
    details("상위 폴더").summary.click(); await settle();
    const singleton = folderButtons().find(node => node.querySelector("strong")?.textContent === "personal · writing/2025");
    assert(singleton, "Single-document folder remains selectable");
    singleton.click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 1, "single original in selected folder");
    await openExplorer();
    assert(host.querySelector<HTMLSelectElement>(".topic-label select")?.value === "personal" && host.querySelector<HTMLSelectElement>(".filter-row label:nth-child(2) select")?.value === "active" && folderButtons().length === 4, "Folder click preserves source and state filters and other folder options after returning to exploration");
    const beforeStructure = graphCalls;
    await until(() => !!host.querySelector('.map-sidebar input[type="checkbox"]'), "folder overlay control is rendered");
    const structure = () => [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find(input => input.parentElement?.textContent?.includes("폴더 연결 표시"))!;
    await openExplorer(); structure().click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 2, "document plus its structural parent");
    assert(host.querySelector(".count-breakdown")?.textContent?.includes("문서 1 · 기록 0 · 관계 0 · 부모·소속 1"), "Parent links and folders do not inflate knowledge or semantic relation counts");
    const folderRow = [...host.querySelectorAll<HTMLButtonElement>(".graph-list > button")].find(node => node.querySelector(".node-symbol.folder"))!;
    folderRow.click();
    await settle();
    assert(!new URL(window.location.href).searchParams.has("focus") && graphCalls === beforeStructure, "Folder navigation stays local without synthetic backend focus or extra requests");
    await openExplorer(); structure().click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 1, "hide structural overlay");
    assert(graphCalls === beforeStructure, "Both structural toggles reuse the returned snapshot");
    await openExplorer(); structure().click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 2, "structural overlay before document selection");
    [...host.querySelectorAll<HTMLButtonElement>(".graph-list > button")].find(node => node.querySelector(".node-symbol.document"))!.click();
    await until(() => !!host.querySelector(".original-detail .document-preview"), "original detail preview");
    assert(host.querySelector<HTMLSelectElement>(".topic-label select")?.value === "personal" && host.querySelector<HTMLSelectElement>(".filter-row label:nth-child(2) select")?.value === "active", "Selecting an in-scope original preserves source and state filters");
    assert(!host.querySelector('.original-detail [aria-label="연결된 자료"]'), "Derived folder membership cannot appear as explicitly related material in document details");
    assert(host.querySelectorAll(".original-detail h1").length === 1, "Multiline authored H1 is displayed once in the actual detail path");
    assert(host.querySelector(".original-detail h1")?.textContent?.replace(/\s+/g, " ").trim() === "지원 현황", "The displayed H1 is the selected original's title");
    assert([...host.querySelectorAll(".original-detail .document-preview p")].some(node => node.textContent === "본문"), "Original body remains visible");
    button("편집").click();
    await until(() => !!host.querySelector("#original-draft"), "original editor");
    const draft = "지원\n현황\n====\n\n본문\n저장하지 않은 초안";
    enter(host.querySelector<HTMLTextAreaElement>("#original-draft")!, draft);
    await settle();
    const grouping = host.querySelector<HTMLSelectElement>('select[aria-label="묶음 기준"]')!;
    assert(grouping.value === "purpose", "App starts with stored purposes");
    const beforeView = graphCalls;
    for (const mode of ["relationships", "purpose"]) {
      grouping.value = mode; grouping.dispatchEvent(new Event("change", { bubbles: true })); await settle();
      assert(host.querySelector<HTMLTextAreaElement>("#original-draft")?.value === draft, "display-only grouping switches preserve the draft");
    }
    assert(graphCalls === beforeView, "view switches reuse the current snapshot without API writes or reads");
    button("읽기").click();
    await until(() => !!host.querySelector(".original-detail .notice"), "unsaved draft in reading mode");
    const kind = host.querySelector<HTMLSelectElement>(".filter-row label:first-child select")!;
    kind.value = "subject";
    kind.dispatchEvent(new Event("change", { bubbles: true }));
    await until(() => !!host.querySelector(".map-empty"), "empty filtered result with original still open");
    let prompts = 0;
    const promptCount = () => prompts;
    window.confirm = () => { prompts++; return false; };
    button("기록 남기기", host.querySelector(".map-empty")!).click();
    await settle();
    assert(promptCount() === 1 && !!host.querySelector(".original-detail"), "Empty-result record entry respects draft discard refusal");
    button("기록 남기기", host.querySelector(".app-header")!).click();
    await settle();
    assert(promptCount() === 2 && !!host.querySelector(".original-detail"), "Header record entry uses the same draft guard");
    button("편집").click();
    await until(() => host.querySelector<HTMLTextAreaElement>("#original-draft")?.value === draft, "draft survives both cancelled entries");
    button("읽기").click();
    await settle();
    window.confirm = () => { prompts++; return true; };
    button("기록 남기기", host.querySelector(".map-empty")!).click();
    await until(() => host.querySelector(".management-panel")?.getAttribute("aria-label") === "기록 남기기", "confirmed record entry");
    assert(promptCount() === 3 && !host.querySelector(".original-detail") && !new URL(window.location.href).searchParams.has("focus"), "Confirmed entry discards the original and clears its selection");
    button("검색·조건 초기화", host.querySelector(".map-empty")!).click();
    await until(() => !host.querySelector(".map-empty") && host.querySelector<HTMLSelectElement>(".filter-row label:first-child select")?.value === "all", "empty results provide a guarded return to the full scope");
    host.querySelector<HTMLButtonElement>('.management-panel button[aria-label="관리 패널 닫기"]')!.click();
    await until(() => !host.querySelector(".management-panel"), "close blank record panel");
    await openExplorer();
    const sourceFilter = host.querySelector<HTMLSelectElement>(".topic-label select")!;
    const stateFilter = host.querySelector<HTMLSelectElement>(".filter-row label:nth-child(2) select")!;
    const kindFilter = host.querySelector<HTMLSelectElement>(".filter-row label:first-child select")!;
    for (const [select, value] of [[sourceFilter, "personal"], [stateFilter, "active"], [kindFilter, "knowledge"]] as const) {
      select.value = value; select.dispatchEvent(new Event("change", { bubbles: true })); await settle();
    }
    button("닫기", host.querySelector(".map-sidebar")!).click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 2, "three display filters applied");
    const filteredOriginal = () => [...host.querySelectorAll<HTMLButtonElement>(".graph-list > button")].find(node => node.querySelector("strong")?.textContent === "지원 현황")!;
    const filtersRetained = () => host.querySelector<HTMLSelectElement>(".topic-label select")?.value === "personal" &&
      host.querySelector<HTMLSelectElement>(".filter-row label:nth-child(2) select")?.value === "active" &&
      host.querySelector<HTMLSelectElement>(".filter-row label:first-child select")?.value === "knowledge";
    for (const exit of ["close", "back"] as const) {
      filteredOriginal().click();
      await until(() => !!host.querySelector(".original-detail .document-preview"), "read filtered original");
      assert(host.querySelectorAll(".graph-list > button").length === 2, "Opening detail retains the exact filtered visible set");
      if (exit === "close") host.querySelector<HTMLButtonElement>('.management-panel button[aria-label="관리 패널 닫기"]')!.click();
      else window.history.back();
      await until(() => !host.querySelector(".management-panel"), "return from filtered detail");
      await openExplorer();
      assert(filtersRetained() && host.querySelectorAll(".graph-list > button").length === 2, "Close and browser Back preserve the filtered exploration context");
      button("닫기", host.querySelector(".map-sidebar")!).click(); await settle();
    }
    return { originalFolders: true, listedFolders: 4, visibleOriginals: 1, titleCount: 1, draftGuard: true, localFolderStructure: true, selectionFiltersRetained: true, browserBackFiltersRetained: true };
  } finally {
    root.unmount();
    window.fetch = originalFetch;
    window.confirm = originalConfirm;
    window.history.replaceState(null, "", originalUrl);
  }
}

async function checkNativeConstellation() {
  const originalFetch = window.fetch, originalUrl = window.location.href;
  const root = createRoot(host);
  const nodes: GraphNode[] = ["hub", ...Array.from({ length: 120 }, (_, index) => `page-${index}`)].map((id, index) => ({
    id, scope: "personal", kind: "document", label: id, title: index ? `합성 자료 ${index}` : "합성 상위 자료",
    status: "ok", present: true, current: true, source_kind: "original", context_scope: "personal",
    context_path: index ? `knowledge/pages-${index % 3}/${id}.md` : "knowledge/index.md",
  }));
  const links = nodes.slice(1).map(node => ({ source: "hub", target: node.id, kind: "related" as const, current: true }));
  nodes.push(...Array.from({ length: 40 }, (_, index): GraphNode => ({
    id: `unrelated-${index}`, scope: "personal", kind: "document", label: `독립 자료 ${index}`,
    status: "ok", present: true, current: true, source_kind: "original", context_scope: "personal", context_path: `notes/item-${index}.md`,
  })));
  const snapshot: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links,
    matched: nodes.length, totals: { documents: nodes.length, memories: 0, markers: 0, links: links.length },
    returned: { knowledge: nodes.length, markers: 0, links: links.length }, omitted: { nodes: 0, links: 0 },
    eligible: { nodes: nodes.length, links: links.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  let graphCalls = 0;
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  const summary = () => [...host.querySelectorAll<HTMLElement>('.graph-group-target')].find(node => !node.hidden && node.getAttribute("aria-label")?.includes("관계 묶음 · 합성 상위 자료 · 121개"));
  const folderSummary = () => [...host.querySelectorAll<HTMLElement>('.graph-group-target')].find(node => !node.hidden && node.getAttribute("aria-label")?.includes("폴더 묶음 · 40개"));
  try {
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") {
        assert(url.searchParams.get("scope") === "personal" && !url.searchParams.get("focus"), "Structure and core expansion cannot create a synthetic backend focus");
        graphCalls++; return json(snapshot);
      }
      if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
      throw new Error(`Unexpected constellation request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal`);
    root.render(<App />);
    await until(() => host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "native graph loaded");
    await chooseDiagramMode("relationships");
    await until(() => !!summary(), "large native overview summary across three folders");
    button("필터·묶음").click();
    const beforeStructure = graphCalls;
    await until(() => !!host.querySelector('.map-sidebar input[type="checkbox"]'), "folder overlay control is rendered");
    const structure = [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find(input => input.parentElement?.textContent?.includes("폴더 연결 표시"))!;
    structure.click();
    await until(() => !!summary() && (host.querySelector(".count-breakdown")?.textContent?.includes("부모·소속") === true), "folder overlay retains native overview summary");
    button("필터·묶음").click();
    await until(() => !!folderSummary(), "unrelated siblings have a distinctly named folder summary");
    assert(graphCalls === beforeStructure, "The structural overlay reuses the existing bounded response");
    summary()!.click();
    await until(() => !!host.querySelector(".graph-core-actions") && (host.querySelector(".graph-core-actions")?.textContent?.includes("1/10") === true), "core opens its first compact page");
    button("다음", host.querySelector(".graph-core-actions")!).click();
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes("2/10") === true, "core opens the next page");
    button("묶음 접기").click();
    await until(() => !!summary(), "core returns to its overview summary");
    folderSummary()!.click();
    await until(() => host.querySelector(".graph-core-heading")?.textContent?.includes("폴더 묶음 · 40개") === true &&
      host.querySelector(".graph-core-page")?.textContent === "1/4", "folder opens a compact structural page without counting its marker");
    button("다음", host.querySelector(".graph-core-actions")!).click();
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes("2/4") === true, "folder pages its original children");
    button("묶음 접기").click();
    await until(() => !!folderSummary(), "folder returns to its structural summary");
    const settings = [...host.querySelectorAll<HTMLElement>("summary")].find(node => node.textContent === "보기 설정")!;
    settings.click();
    button("새로고침").click();
    await until(() => graphCalls === beforeStructure + 1 && !!summary() && !!folderSummary() && host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "same-route refresh preserves folder overlay and both distinct summaries");
    return { nativeConstellation: true, documents: 161, semanticLinks: 120, unrelatedFolderChildren: 40, pageSize: 12, parentOverlay: true, refresh: true };
  } finally {
    root.unmount(); window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
  }
}

async function checkPurposeDiagram() {
  const originalFetch = window.fetch, originalUrl = window.location.href, root = createRoot(host);
  const originalPack = Positions.prototype.packOverview;
  let positions: Positions | undefined, graphCalls = 0;
  const subjects = [subjectId, "p_00000000-0000-4000-8000-000000000002"], names = ["합성 연구 목적", "합성 작성 목적"];
  const members: GraphNode[] = [120, 23].flatMap((count, group) => Array.from({ length: count }, (_, index) => ({
    id: `purpose-${group}-${String(index).padStart(3, "0")}`, scope: "personal", kind: "document", label: `합성 자료 ${group}-${index}`,
    subject_id: subjects[group], subject_name: names[group], status: "ok", present: true, current: true,
  })));
  const nodes: GraphNode[] = [...subjects.map((id, i): GraphNode => ({ id, scope: "personal", kind: "subject", label: names[i] })), ...members,
    { id: "reference-hub", scope: "personal", kind: "document", label: "분류되지 않은 참조 중심", status: "ok", present: true, current: true }];
  const memberships: Snapshot["links"] = members.map(node => ({ source: node.id, target: node.subject_id!, kind: "subject", current: true }));
  const references: Snapshot["links"] = members.map(node => ({ source: "reference-hub", target: node.id, kind: "reference", current: true }));
  const snapshot: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links: [...memberships, ...references],
    matched: nodes.length, totals: { documents: members.length + 1, memories: 0, markers: 2, links: memberships.length + references.length },
    returned: { knowledge: members.length + 1, markers: 2, links: memberships.length + references.length }, omitted: { nodes: 0, links: 0 },
    eligible: { nodes: nodes.length, links: memberships.length + references.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  let responseSnapshot = snapshot;
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  const summary = (group: number) => [...host.querySelectorAll<HTMLElement>('.graph-group-target')].find(node => !node.hidden && node.getAttribute("aria-label")?.includes(`${names[group]} · ${group ? 23 : 120}개`));
  try {
    Positions.prototype.packOverview = function (this: Positions, ...args: Parameters<Positions["packOverview"]>) {
      positions = this; return originalPack.apply(this, args);
    };
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") { graphCalls++; assert(!url.searchParams.get("focus"), "purpose summaries reuse the full response"); return json(responseSnapshot); }
      if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
      throw new Error(`Unexpected purpose diagram request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal`);
    root.render(<App />);
    await until(() => !!summary(0) && !!summary(1) && !!positions && !positions.layoutMoving, "default purpose diagram uses persisted memberships despite a cross-purpose reference hub");
    const firstCalls = graphCalls;
    const epoch = positions!.structureEpoch;
    summary(0)!.focus(); summary(0)!.click();
    await until(() => host.querySelector(".graph-core-heading")?.textContent?.includes(`${names[0]} · 120개`) === true &&
      host.querySelector(".graph-core-page")?.textContent === "1/10" && !positions!.layoutMoving, "native button activation opens a 12-item purpose page without counting its subject marker");
    await until(() => [...host.querySelectorAll<HTMLElement>('.node-label:not([hidden])')].filter(label => label.dataset.nodeId?.startsWith("purpose-0-") && +label.style.opacity > .99).length >= 12,
      "every disclosed member retains its title in the first purpose page");
    checkLabelAssociations();
    if (innerWidth <= 700) {
      const previous = [...host.querySelectorAll<HTMLButtonElement>(".graph-core-navigation button")].find(control => control.textContent === "이전")!;
      assert(previous.disabled, "the first purpose page has no previous page");
      const controls = [previous, host.querySelector<HTMLElement>(".graph-core-page")!,
        button("다음", host.querySelector(".graph-core-actions")!)];
      const boxes = controls.map(control => control.getBoundingClientRect()), center = boxes[0].y + boxes[0].height / 2;
      assert(boxes.every(box => Math.abs(box.y + box.height / 2 - center) < .5 && box.left >= 0 && box.right <= innerWidth),
        "mobile previous, page and next stay together in a visible row");
      assert(boxes[0].height >= 44 && boxes[2].height >= 44, "mobile paging has usable touch targets");
    }
    button("다음", host.querySelector(".graph-core-actions")!).click();
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes("2/10") === true && !positions!.layoutMoving, "purpose members page locally");
    button("묶음 접기").click();
    await until(() => !!summary(0) && !!summary(1) && !positions!.layoutMoving, "purpose summaries restore their compact overview");
    summary(1)!.focus(); summary(1)!.click();
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes(`${names[1]} · 23개`) === true && !positions!.layoutMoving,
      "a small opened purpose retains its literal name and knowledge count without paging controls");
    checkLabelAssociations();
    button("묶음 접기").click();
    await until(() => !!summary(0) && !!summary(1) && !positions!.layoutMoving, "the small purpose returns to the same named overview");
    assert(graphCalls === firstCalls, "purpose expansion and paging use no extra graph requests");
    await openExplorer();
    const filteredPurpose = [...host.querySelectorAll<HTMLButtonElement>(".cluster-list button")].find(control => control.querySelector("strong")?.textContent === names[1]);
    assert(filteredPurpose, "the selected purpose has an actual sidebar entry");
    filteredPurpose.click();
    await until(() => !host.querySelector(".map-sidebar") && host.querySelector(".graph-core-heading")?.textContent?.includes(`${names[1]} · 23개`) === true && !positions!.layoutMoving,
      "choosing a purpose in the sidebar opens its members through the existing core disclosure");
    assert([...host.querySelectorAll<HTMLElement>('.node-label:not([hidden])')].some(label => label.dataset.nodeId?.startsWith("purpose-1-")), "the filtered map contains member titles, not just the core marker");
    button("묶음 접기").click(); await settle();
    assert(!host.querySelector(".graph-core-actions"), "explicit collapse remains collapsed under the same filter");
    button("묶음 해제").click();
    await until(() => !!summary(0) && !!summary(1) && !positions!.layoutMoving, "clearing the group filter restores the full overview");
    assert(graphCalls === firstCalls, "group filtering and disclosure reuse the current snapshot");
    responseSnapshot = { ...snapshot, links: memberships };
    details("보기 설정").summary.click(); button("새로고침").click();
    await until(() => graphCalls === firstCalls + 1 && host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false" && !!summary(0) && !!summary(1) && !positions!.layoutMoving, "removing references retains both purpose summaries");
    assert(positions!.structureEpoch === epoch, "reference-only refresh keeps the purpose topology epoch");
    await chooseDiagramMode("relationships");
    await until(() => [...host.querySelectorAll<HTMLElement>('.graph-group-target')].some(node => !node.hidden && node.textContent?.includes("관계 묶음")), "relationship mode preserves adjacency-based summaries");
    await chooseDiagramMode("purpose");
    await until(() => !!summary(0) && !!summary(1), "returning to purpose mode restores stored names and counts");
    assert(graphCalls === firstCalls + 1, "diagram switches reuse the existing source response");
    assert(snapshot.links.length === memberships.length + references.length, "diagram projection never mutates original reference data");
    return { purposeDiagram: true, memberCounts: [120, 23], crossPurposeReferences: references.length, pageSize: 12, pageCount: 10, referenceRefreshRetainsMembership: true, keyboard: true, relationshipRoundTrip: true };
  } finally {
    root.unmount(); Positions.prototype.packOverview = originalPack; window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
  }
}

async function checkFirstProximityCollapse() {
  const originalFetch = window.fetch, originalUrl = window.location.href, root = createRoot(host);
  const originalInstall = Positions.prototype.install, originalPack = Positions.prototype.packOverview;
  const nodes: GraphNode[] = Array.from({ length: 12 }, (_, index) => ({ id: `ungrouped-${index}`, scope: "personal", kind: "document", label: `합성 독립 자료 ${index}`, status: "ok", present: true, current: true }));
  // A structural marker keeps two visible discs even when every document collapses.
  // Otherwise packOverview's one-disc early return would conceal the regression.
  nodes.push({ id: "ungrouped-marker", scope: "personal", kind: "topic", label: "합성 분류 표식" });
  const snapshot: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links: [],
    matched: nodes.length, totals: { documents: 12, memories: 0, markers: 1, links: 0 },
    returned: { knowledge: 12, markers: 1, links: 0 }, omitted: { nodes: 0, links: 0 }, eligible: { nodes: nodes.length, links: 0 },
    limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  let positions: Positions | undefined, overviewPacks = 0;
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  const packedFrames: unknown[] = [];
  const hasSummary = () => [...host.querySelectorAll<HTMLElement>('.graph-group-target')].some(node => !node.hidden && node.textContent?.includes("근접 묶음"));
  try {
    Positions.prototype.install = function (this: Positions, ...args: Parameters<Positions["install"]>) {
      positions = this; return originalInstall.apply(this, args);
    };
    Positions.prototype.packOverview = function (this: Positions, ...args: Parameters<Positions["packOverview"]>) {
      const packed = originalPack.apply(this, args);
      if (packed) {
        overviewPacks++;
        packedFrames.push({ epoch: this.structureEpoch, nodes: args[0].length, canvas: host.querySelector("canvas")?.getBoundingClientRect().toJSON() });
      }
      return packed;
    };
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") return json(snapshot);
      if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
      throw new Error(`Unexpected first collapse request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal`);
    root.render(<App />);
    const openedAt = performance.now();
    await until(() => !!host.querySelector("canvas") && !!positions && overviewPacks === 1 && !positions.layoutMoving && !hasSummary() && performance.now() - openedAt > 650, "initial ungrouped stars compact once and settle").catch(error => {
      throw new Error(`${error.message}; ${JSON.stringify({ canvas: !!host.querySelector("canvas"), positions: !!positions, overviewPacks, moving: positions?.layoutMoving, hasSummary: hasSummary(), packedFrames })}`);
    });
    const coordinates = () => JSON.stringify(nodes.map(node => positions!.layoutTarget(node.id)));
    const initial = coordinates(), initialPacks = overviewPacks, canvas = host.querySelector("canvas")!;
    const rect = canvas.getBoundingClientRect();
    for (const deltaY of [3000, -3000]) {
      const zoomedAt = performance.now();
      canvas.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY,
        clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }));
      await until(() => hasSummary() === (deltaY > 0) && !positions!.layoutMoving && performance.now() - zoomedAt > 650, "zoom forms and dissolves the first proximity summary");
      assert(coordinates() === initial && overviewPacks === initialPacks, "The first proximity collapse also preserves all world coordinates without packing");
    }
    return { firstProximityCollapse: true, documents: 12, markers: 1, initialPacks, zoomPacks: overviewPacks - initialPacks, coordinatesPreserved: true };
  } finally {
    root.unmount(); Positions.prototype.install = originalInstall; Positions.prototype.packOverview = originalPack;
    window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
  }
}

async function checkSummaryOverview() {
  const originalFetch = window.fetch, originalUrl = window.location.href, root = createRoot(host);
  const originalPack = Positions.prototype.packOverview;
  const originalRender = Scene.prototype.onBeforeRender, wrappedLights = new WeakSet<Mesh>();
  let lightPhase: "opening" | "closing" | null = null;
  const cloudCrossfade = { opening: false, closing: false };
  const cloudSamples = new Map<string, { opacity: number; at: number }>();
  const glyphSamples = new Map<string, { pixels: number; opacity: number; at: number; x: number; y: number }>();
  const glyphCrossfade = { opening: false, closing: false }, wrappedGlyphs = new WeakSet<Sprite>();
  let spatialCloudId: string | undefined, spatialCloudCrossfade = false;
  let positions: Positions | undefined, overviewPacks = 0;
  let observedCamera: Camera | undefined;
  const counts = [20, 21, 22, 40, 75, 120], nodes: GraphNode[] = [], links: Snapshot["links"] = [];
  for (const count of counts) for (let index = 0; index < count; index++) {
    const id = `group-${count}-${index}`;
    nodes.push({ id, scope: "personal", kind: "document", label: `합성 ${count}개 묶음 자료 ${index}`, status: "ok", present: true, current: true });
    if (index) links.push({ source: `group-${count}-0`, target: id, kind: "related", current: true });
  }
  nodes.push(...Array.from({ length: 300 }, (_, index): GraphNode => ({ id: `isolated-${index}`, scope: "personal", kind: "document", label: `합성 독립 자료 ${index}`, status: "ok", present: true, current: true })));
  const snapshot: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links,
    matched: nodes.length, totals: { documents: nodes.length, memories: 0, markers: 0, links: links.length },
    returned: { knowledge: nodes.length, markers: 0, links: links.length }, omitted: { nodes: 0, links: 0 }, eligible: { nodes: nodes.length, links: links.length },
    limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  let graphCalls = 0;
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  const labels = () => [...host.querySelectorAll<HTMLElement>('.graph-group-target')].filter(node => !node.hidden);
  const summary = (count: number) => labels().find(node => node.getAttribute("aria-label")?.includes(`관계 묶음 · 합성 ${count}개 묶음 자료 0 · ${count}개`));
  const overviewReady = () => counts.every(count => !!summary(count) && (glyphSamples.get(`group-${count}-0`)?.opacity ?? 0) > .99);
  const pointAt = (count: number | null) => {
    // Synthetic PointerEvent page coordinates do not include the fixture's scroll offset.
    // Keep this standalone check at its origin so the renderer receives native-equivalent coordinates.
    window.scrollTo(0, 0);
    const canvas = host.querySelector("canvas")!, rect = canvas.getBoundingClientRect();
    const sample = count === null ? undefined : glyphSamples.get(`group-${count}-0`);
    const at = sample ? { x: sample.x, y: sample.y } : { x: -2, y: -2 };
    canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse",
      clientX: rect.left + (at.x + 1) * rect.width / 2, clientY: rect.top + (1 - at.y) * rect.height / 2 }));
  };
  const revealSummary = async (count: number) => {
    pointAt(count);
    await until(() => !!summary(count), `pointing reveals the ${count}-item summary name`);
    return summary(count)!;
  };
  try {
    Scene.prototype.onBeforeRender = function (renderer, scene, camera, geometry, material, group) {
      observedCamera = camera;
      scene.traverse(object => {
        if (object instanceof Sprite && ["summary-glyph", "retiring-summary-glyph"].includes(object.name) && !wrappedGlyphs.has(object)) {
          wrappedGlyphs.add(object);
          const drawGlyph = object.onBeforeRender;
          object.onBeforeRender = function (...args) {
            drawGlyph.apply(this, args);
            const id = object.parent?.userData.nodeId, opacity = object.material.opacity;
            const point = object.getWorldPosition(new Vector3()).project(args[2]);
            glyphSamples.set(id, { pixels: object.scale.x * args[0].domElement.clientHeight * args[2].projectionMatrix.elements[5] / 2, opacity, at: performance.now(), x: point.x, y: point.y });
            if (lightPhase && id === "group-120-0" && opacity > 0 && opacity < 1) glyphCrossfade[lightPhase] = true;
          };
        }
        if (!(object instanceof Mesh) || object.name !== "stellar-light" || wrappedLights.has(object)) return;
        wrappedLights.add(object);
        const draw = object.onBeforeRender;
        object.onBeforeRender = function (...args) {
          draw.apply(this, args);
          const uniforms = (object.material as ShaderMaterial).uniforms;
          if (uniforms.uCloud.value === 1) {
            const id = object.parent?.userData.nodeId;
            cloudSamples.set(id, { opacity: uniforms.uOpacity.value, at: performance.now() });
            if (uniforms.uOpacity.value > 0 && uniforms.uOpacity.value < 1) {
              if (lightPhase && id === "group-120-0") cloudCrossfade[lightPhase] = true;
              if (id === spatialCloudId) spatialCloudCrossfade = true;
            }
          }
        };
      });
      return originalRender.call(this, renderer, scene, camera, geometry, material, group);
    };
    Positions.prototype.packOverview = function (this: Positions, ...args: Parameters<Positions["packOverview"]>) {
      positions = this;
      const packed = originalPack.apply(this, args);
      if (packed) overviewPacks++;
      return packed;
    };
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") { graphCalls++; assert(!url.searchParams.get("focus"), "summary gestures do not request individual originals"); return json(snapshot); }
      if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
      throw new Error(`Unexpected summary request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal`);
    root.render(<App />);
    await until(() => host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "summary graph loaded");
    await chooseDiagramMode("relationships");
    await until(() => overviewReady() && labels().some(node => node.textContent?.includes("근접 묶음")), "four differently sized relation images and proximity cohorts remain available");
    const initialCalls = graphCalls;
    const placements = () => counts.map(count => {
      const node = summary(count);
      const glyph = glyphSamples.get(`group-${count}-0`);
      return node && glyph ? `${count}:${glyph.x.toFixed(4)}:${glyph.y.toFixed(4)}` : `${count}:hidden`;
    }).join("|") + `|camera:${observedCamera?.position.toArray().map(value => value.toFixed(3)).join(",")}`;
    let lastPlacement = placements(), stableSince = performance.now();
    await until(() => {
      const current = placements();
      if (current !== lastPlacement) { lastPlacement = current; stableSince = performance.now(); }
      return !!positions && !positions.layoutMoving && overviewReady() && performance.now() - stableSince > 350;
    }, "initial compact overview settles").catch(error => {
      throw new Error(`${error.message}; ${JSON.stringify({ moving: positions?.layoutMoving, packs: overviewPacks, placements: placements(), lastPlacement, stableFor: performance.now() - stableSince, visible: labels().map(label => label.children[0].textContent) })}`);
    });
    const coordinates = () => JSON.stringify(nodes.map(node => positions!.layoutTarget(node.id)));
    const worldBeforeZoom = coordinates(), packsBeforeZoom = overviewPacks;
    const assertMobileKeys = () => {
      if (innerWidth > 700) return;
      const pinned = [...host.querySelectorAll<HTMLElement>(".graph-group-target.core-name")];
      assert(pinned.length === counts.length && pinned.every(target => target.offsetWidth > 1 && target.offsetHeight >= 38), "all six core names and counts are readable without pointing");
      const navigation = host.querySelector<HTMLElement>(".graph-group-navigation")!;
      assert(navigation.getBoundingClientRect().height <= 72 && getComputedStyle(navigation).overflowX === "auto", "mobile names occupy one compact scrollable row");
      const markers = [...host.querySelectorAll<HTMLElement>(".graph-group-marker")];
      assert(markers.length === pinned.length && new Set(markers.map(marker => marker.textContent)).size === pinned.length, "each mobile core has one distinct visual key");
      const mapRect = host.querySelector("canvas")!.getBoundingClientRect();
      for (const target of pinned) {
        const marker = markers.find(marker => marker.dataset.nodeId === target.dataset.nodeId)!;
        const sample = glyphSamples.get(target.dataset.nodeId!)!;
        const box = marker.getBoundingClientRect();
        assert(marker && !marker.hidden && marker.textContent === target.querySelector(".group-key")?.textContent, "the name and its actual rendered core share the same key");
        const x = mapRect.left + (sample.x + 1) * mapRect.width / 2, y = mapRect.top + (1 - sample.y) * mapRect.height / 2;
        assert(Math.hypot(box.x + box.width / 2 - x, box.y + box.height / 2 - y) < 100, "a key stays beside its own projected core");
        assert([navigation, host.querySelector<HTMLElement>(".graph-minimap")!].every(chrome => {
          const other = chrome.getBoundingClientRect();
          return box.right <= other.left || box.left >= other.right || box.bottom <= other.top || box.top >= other.bottom;
        }), "core keys remain visible outside the name row and minimap");
      }
      assert([...host.querySelectorAll<HTMLElement>(".node-label:not([hidden])")].every(label => parseFloat(label.style.getPropertyValue("--leader-length")) <= 34.01), "resting mobile names use short leaders");
    };
    assertMobileKeys();
    const keyboardGroups = labels().length;
    for (const target of labels()) {
      const id = target.dataset.nodeId, text = target.textContent;
      target.focus(); pointAt(null); await settle();
      assert(document.activeElement === target && target.offsetWidth > 1 && target.dataset.nodeId === id && target.textContent === text, "every group, including deferred proximity names, has a stable visible keyboard target");
      if (innerWidth <= 700) {
        const box = target.getBoundingClientRect();
        assert(box.left >= 0 && box.right <= innerWidth && box.top >= 0 && box.bottom <= innerHeight && target.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)), "Tab focus reveals the entire name without clipping or covered controls");
      }
      target.blur(); await settle();
    }
    assert(coordinates() === worldBeforeZoom && overviewPacks === packsBeforeZoom, "adaptive names and focus never reposition stars");
    const beforeZoom = lastPlacement, canvas = host.querySelector("canvas")!;
    const rect = canvas.getBoundingClientRect();
    canvas.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: 100,
      clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }));
    let zoomMoved = false;
    await until(() => {
      const current = placements();
      if (current !== beforeZoom) zoomMoved = true;
      if (current !== lastPlacement) { lastPlacement = current; stableSince = performance.now(); }
      return zoomMoved && overviewReady() && performance.now() - stableSince > 300;
    }, "zoom-out settles with all differently sized summary images available");
    assert(coordinates() === worldBeforeZoom && overviewPacks === packsBeforeZoom, "Wheel zoom changes the camera without repacking stars");
    for (const deltaY of [-1200, -1200, 1200, 1200, -100]) {
      const started = performance.now();
      canvas.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY,
        clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }));
      await until(() => performance.now() - started > 650 && !positions!.layoutMoving, "camera zoom settles across proximity detail bands");
      assert(coordinates() === worldBeforeZoom && overviewPacks === packsBeforeZoom, "Crossing proximity detail bands preserves every world coordinate");
    }
    await until(overviewReady, "return zoom keeps all relation images available").catch(error => {
      throw new Error(`${error.message}; ${JSON.stringify({ placements: placements(), visible: labels().map(label => label.children[0].textContent), packs: overviewPacks, moving: positions?.layoutMoving })}`);
    });
    const zoomPacks = overviewPacks - packsBeforeZoom;
    const background = () => JSON.stringify(nodes.filter(node => !node.id.startsWith("group-120-")).map(node => positions!.layoutTarget(node.id)));
    const backgroundBeforeExpansion = background();
    const assertLocalEdges = () => {
      for (const link of links.filter(link => link.source === "group-120-0")) {
        const a = positions!.layoutTarget(link.source)!, b = positions!.layoutTarget(link.target)!;
        assert(Math.hypot(a.x - b.x, a.y - b.y) < 200, `A settled core page cannot eject a star beyond its local neighborhood: ${link.target} (${Math.hypot(a.x-b.x,a.y-b.y)}, ${JSON.stringify({a,b})})`);
      }
    };
    lightPhase = "opening";
    const overviewCamera = observedCamera!.position.clone(), overviewRotation = observedCamera!.quaternion.clone();
    const hubProjections = () => counts.map(count => {
      const at = positions!.layoutTarget(`group-${count}-0`)!, projected = new Vector3(at.x, at.y, at.z).project(observedCamera!);
      return { count, x: projected.x, y: projected.y, z: projected.z };
    });
    const overviewProjections = hubProjections();
    (await revealSummary(120)).click();
    await until(() => host.querySelector(".graph-core-heading")?.textContent?.includes("관계 묶음 · 합성 120개 묶음 자료 0 · 120개") === true &&
      host.querySelector(".graph-core-page")?.textContent === "1/10", "large summary opens its compact page");
    await until(() => { const value = cloudSamples.get("group-120-0")?.opacity; return value !== undefined && value > .3 && value < .85; }, "the real summary cloud is midway through opening");
    const beforeClose = cloudSamples.get("group-120-0")!;
    const glyphBeforeClose = glyphSamples.get("group-120-0")!;
    assert(Math.abs(glyphBeforeClose.pixels - summaryAppearance(120).pixels) < .001, "opening retains the full count-sized glyph instead of jumping to 26px");
    button("묶음 접기").click();
    await until(() => cloudSamples.get("group-120-0")!.at > beforeClose.at, "closing retargets the rendered cloud before opening finishes");
    const afterClose = cloudSamples.get("group-120-0")!;
    const glyphAfterClose = glyphSamples.get("group-120-0")!;
    assert(Math.abs(glyphAfterClose.opacity - glyphBeforeClose.opacity) <= (glyphAfterClose.at - glyphBeforeClose.at) / 650 * 2 + .03, "reversing opening retargets the observed glyph opacity");
    assert(Math.abs(afterClose.opacity - beforeClose.opacity) <= (afterClose.at - beforeClose.at) / 650 * 2 + .03,
      "reversing opening preserves the observed cloud opacity within the 650ms motion bound");
    await until(() => cloudSamples.get("group-120-0")!.at - afterClose.at > 80, "the interrupted close renders several frames");
    const beforeReopen = cloudSamples.get("group-120-0")!;
    (await revealSummary(120)).click();
    await until(() => !!host.querySelector(".graph-core-actions") && cloudSamples.get("group-120-0")!.at > beforeReopen.at, "reopening retains a rendered cloud during the reverse");
    const afterReopen = cloudSamples.get("group-120-0")!;
    assert(Math.abs(afterReopen.opacity - beforeReopen.opacity) <= (afterReopen.at - beforeReopen.at) / 650 * 2 + .03,
      "reversing closing preserves the observed cloud opacity within the 650ms motion bound");
    await until(() => !positions!.layoutMoving, "local core expansion settles");
    assert(cloudCrossfade.opening, "the actual summary light dissolves into its individual stars while opening");
    lightPhase = null;
    assertLocalEdges();
    assert(background() === backgroundBeforeExpansion, "Opening a core leaves every unrelated star in place");
    button("다음", host.querySelector(".graph-core-actions")!).click();
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes("2/10") === true, "large summary pages");
    await until(() => !positions!.layoutMoving, "local core page settles");
    assertLocalEdges();
    assert(background() === backgroundBeforeExpansion, "Changing the core page leaves unrelated stars in place");
    const proximity = labels().find(node => node.textContent?.includes("근접 묶음"));
    assert(proximity, "a proximity summary is visible alongside the open relation core");
    spatialCloudId = proximity.dataset.nodeId;
    proximity.click();
    await until(() => host.querySelector(".graph-core-close")?.textContent?.startsWith("근접 묶음") === true && !positions!.layoutMoving,
      "a spatial reveal settles while its relation core stays open");
    assert(spatialCloudCrossfade, "a proximity summary also dissolves its actual cloud instead of removing it in one frame");
    assertLocalEdges();
    assert(background() === backgroundBeforeExpansion, "A spatial reveal preserves the unrelated world coordinates");
    host.querySelector<HTMLButtonElement>(".graph-core-close")!.click();
    await until(() => !!host.querySelector(".graph-core-actions") && !positions!.layoutMoving, "spatial close restores the open relation page");
    assertLocalEdges();
    lightPhase = "closing";
    button("묶음 접기").click();
    const closedAt = performance.now();
    pointAt(null);
    await until(() => !positions!.layoutMoving && performance.now() - closedAt > 1000 && overviewReady(), "closing restores all overview images").catch(error => {
      throw new Error(`${error.message}; ${JSON.stringify({ moving: positions?.layoutMoving, packs: overviewPacks, placements: placements(), cameraDelta: observedCamera!.position.distanceTo(overviewCamera), rotationDelta: observedCamera!.quaternion.angleTo(overviewRotation), backgroundSame: background() === backgroundBeforeExpansion, overviewProjections, closedProjections: hubProjections(), visible: labels().map(label => label.children[0].textContent) })}`);
    });
    assert(observedCamera!.position.distanceTo(overviewCamera) < .01 && observedCamera!.quaternion.angleTo(overviewRotation) < .001, "a rapid reopen keeps the original overview camera as the return destination");
    assert(cloudCrossfade.closing, "the actual summary light regathers as returning stars dissolve");
    assert(glyphCrossfade.opening && glyphCrossfade.closing, "the count-sized summary image shares the light's continuous opening and closing fade");
    lightPhase = null;
    assert(background() === backgroundBeforeExpansion && overviewPacks === packsBeforeZoom, "Closing restores the same overview without a new whole-map pack");
    assertMobileKeys();
    assert(graphCalls === initialCalls, "overview packing and expansion reuse one response");
    details("보기 설정").summary.click(); button("새로고침").click();
    await until(() => graphCalls === initialCalls + 1 && host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false" && overviewReady(), "refresh retains all overview images");
    return { summaryOverview: true, documents: nodes.length, relationCounts: counts, cloudCrossfade, glyphCrossfade, glyphBeforeClose, glyphAfterClose,
      reversedCloudOpacity: { beforeClose, afterClose, beforeReopen, afterReopen }, spatialCloudCrossfade,
      proximityCohorts: true, keyboardGroups, adaptiveNames: innerWidth <= 700, zoomPreservesWorldCoordinates: true, zoomPacks, localExpansion: true, spatialRevealWithOpenCore: true, closingPreservesOverview: true, refresh: true };
  } finally {
    root.unmount(); Positions.prototype.packOverview = originalPack; Scene.prototype.onBeforeRender = originalRender;
    window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
  }
}

async function checkSummaryContentRefresh() {
  const originalFetch = window.fetch, originalPack = Positions.prototype.packOverview, originalRender = Scene.prototype.onBeforeRender;
  const originalUrl = window.location.href, root = createRoot(host);
  const counts = [22, 40, 75, 120], nodes: GraphNode[] = [], links: Snapshot["links"] = [];
  for (const count of counts) for (let index = 0; index < count; index++) {
    const id = `refresh-${count}-${index}`;
    nodes.push({ id, scope: "personal", kind: "document", label: `갱신 합성 ${count}개 자료 ${index}`, status: "ok", present: true, current: true });
    if (index) links.push({ source: `refresh-${count}-0`, target: id, kind: "related", current: true });
  }
  const snapshot: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links,
    matched: nodes.length, totals: { documents: nodes.length, memories: 0, markers: 0, links: links.length },
    returned: { knowledge: nodes.length, markers: 0, links: links.length }, omitted: { nodes: 0, links: 0 }, eligible: { nodes: nodes.length, links: links.length },
    limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  let responseSnapshot = snapshot, graphCalls = 0, overviewPacks = 0, lastPack = 0, refreshedAt = 0;
  let positions: Positions | undefined, camera: Camera | undefined;
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  const settled = () => !!positions && !positions.layoutMoving && performance.now() - Math.max(lastPack, refreshedAt) > 350;
  const refresh = async () => {
    const before = graphCalls;
    button("새로고침").click();
    await until(() => graphCalls === before + 1 && host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "same-topology refresh completes");
    refreshedAt = performance.now();
  };
  try {
    Scene.prototype.onBeforeRender = function (renderer, scene, frameCamera, geometry, material, group) {
      camera = frameCamera;
      originalRender.call(this, renderer, scene, frameCamera, geometry, material, group);
    };
    // Observe the real packing boundary while leaving its implementation intact.
    Positions.prototype.packOverview = function (this: Positions, ...args: Parameters<Positions["packOverview"]>) {
      positions = this;
      const packed = originalPack.apply(this, args);
      if (packed) { overviewPacks++; lastPack = performance.now(); }
      return packed;
    };
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") { graphCalls++; return json(responseSnapshot); }
      if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
      throw new Error(`Unexpected content refresh request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal`);
    root.render(<App />);
    await until(() => host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "refresh graph loaded");
    await chooseDiagramMode("relationships");
    await until(settled, "stationary overview finishes packing before metadata changes");
    const structureEpoch = positions!.structureEpoch;
    details("보기 설정").summary.click();
    const pose = () => camera!.position.toArray().map(value => value.toFixed(2)).join(",");
    const beforeGrowth = overviewPacks, poseBefore = pose();
    responseSnapshot = { ...snapshot, nodes: nodes.map(node => ({ ...node, revision: "content-only-update" })) };
    await refresh();
    await until(settled, "larger change rings settle");
    // Graph clears the actual body/ring footprint; names find their own space without moving stars.
    const clearance = (gap: number) => {
      const rect = host.querySelector("canvas")!.getBoundingClientRect();
      const rings = counts.map(count => {
        const at = positions!.layoutTarget(`refresh-${count}-0`)!, point = new Vector3(at.x, at.y, at.z).project(camera!);
        return { x: (point.x + 1) * rect.width / 2, y: (1 - point.y) * rect.height / 2, radius: nodeVisualRadius(1, false, true, count) };
      });
      return rings.every((a, index) => rings.slice(index + 1).every(b => Math.hypot(a.x - b.x, a.y - b.y) >= a.radius + b.radius + gap - .01));
    };
    const growthRepacked = overviewPacks > beforeGrowth;
    // A spread first view may already have room for larger rings; then clearance needs no repack.
    assert(growthRepacked || clearance(6), "growing change rings either trigger clearance or already have Graph's footprint clearance");
    assert(clearance(0), "larger change rings do not overlap after clearance");
    assert(pose() === poseBefore, "a content refresh keeps the overview camera");
    assert(positions!.structureEpoch === structureEpoch, "content changes preserve the topology epoch");
    const afterGrowth = overviewPacks;
    await refresh();
    await until(settled, "unchanged response clears change rings");
    assert(overviewPacks === afterGrowth, "adequate clearance does not repack after rings shrink");
    assert(pose() === poseBefore, "shrinking rings keep the overview camera");
    return { summaryContentRefresh: true, relationCounts: counts, growingRings: true, growthRepacked, shrinkingRingsPreserveLayout: true };
  } finally {
    root.unmount(); Positions.prototype.packOverview = originalPack; Scene.prototype.onBeforeRender = originalRender; window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
  }
}

function checkSummaryGlyphMotion() {
  const glyph = summaryGlyphTexture("synthetic-hover", Array.from({ length: 120 }, (_, index) => ({ id: `synthetic-star-${index}`, kind: "document" })));
  try {
    // Read a separate CPU canvas so repeated probes do not switch the glyph's
    // raster backend and change edge antialiasing during the comparison.
    const capture = document.createElement("canvas"); capture.width = capture.height = 128;
    const context = capture.getContext("2d", { willReadFrequently: true })!;
    const pixels = () => { context.clearRect(0, 0, 128, 128); context.drawImage(glyph.texture.image as HTMLCanvasElement, 0, 0); return context.getImageData(0, 0, 128, 128).data; };
    const resting = pixels(), same = (data: Uint8ClampedArray) => data.every((value, index) => value === resting[index]);
    const initialVersion = glyph.texture.version;
    for (let frame = 0; frame < 120; frame++) glyph.updateMotion(false, frame / 60, false);
    assert(glyph.texture.version === initialVersion, "idle summaries upload no animation frames");
    for (let frame = 0; frame < 120; frame++) glyph.updateMotion(true, 2 + frame / 60, false);
    const redraws = glyph.texture.version - initialVersion;
    assert(!same(pixels()) && redraws > 1 && redraws <= 61, "hover animates the retained drawing at no more than 30fps");
    glyph.updateMotion(true, 4, true);
    assert(same(pixels()), "reduced motion immediately restores the resting drawing");
    const reducedVersion = glyph.texture.version;
    for (let frame = 0; frame < 60; frame++) glyph.updateMotion(true, 4, true);
    assert(glyph.texture.version === reducedVersion, "a frozen motion clock performs no uploads");
    for (let frame = 0; frame < 60; frame++) glyph.updateMotion(true, 4 + frame / 60, false);
    for (let frame = 0; frame < 120; frame++) glyph.updateMotion(false, 5 + frame / 60, false);
    assert(same(pixels()), "pointer exit settles back to the original glyph");
    const settledVersion = glyph.texture.version;
    glyph.updateMotion(false, 8, false);
    assert(glyph.texture.version === settledVersion, "settled summaries stop repainting");
    return { summaryGlyphMotion: true, members: 120, redraws, idleUploads: 0, reducedMotion: true, exitRestoresDrawing: true };
  } finally { glyph.texture.dispose(); }
}

async function checkRenderedMotion() {
  const originalFetch = window.fetch, originalUrl = window.location.href, root = createRoot(host);
  const originalMatchMedia = window.matchMedia;
  let reduceMotion = false;
  const motionPreference = Object.defineProperties(new EventTarget(), {
    matches: { get: () => reduceMotion }, media: { value: "(prefers-reduced-motion: reduce)" },
  }) as MediaQueryList;
  const originalRender = Scene.prototype.onBeforeRender, originalInstall = Positions.prototype.install, originalPack = Positions.prototype.packOverview;
  const originalControlsUpdate = OrbitControls.prototype.update;
  let cameraDistance = 0, overviewPacked = false, autoRotation = false;
  const nodes: GraphNode[] = Array.from({ length: 5 }, (_, index) => ({ id: `motion-${index}`, scope: "personal", kind: "document", title: `합성 모션 자료 ${index}`, label: `synthetic/rendering/긴 원문 경로/${index}.md`, repository: "synthetic-workspace", subject_id: "p_motion", subject_name: "합성 모션 목적", purpose_total: 5, status: "ok", present: true, current: true }));
  const links: Snapshot["links"] = [1, 2, 3, 4].map(index => ({ source: nodes[0].id, target: nodes[index].id, kind: "related", current: true }));
  const snapshot: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links,
    matched: nodes.length, totals: { documents: nodes.length, memories: 0, markers: 0, links: links.length },
    returned: { knowledge: nodes.length, markers: 0, links: links.length }, omitted: { nodes: 0, links: 0 },
    eligible: { nodes: nodes.length, links: links.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  let model: Parameters<Positions["install"]>[0] | undefined, positions: Positions | undefined, camera: Camera | undefined;
  const sampled = new Map<string, { rotation: number; shimmer: number; wobble: number; depth: number; z: number }>(), wrapped = new WeakSet<Mesh>(), worldPosition = new Vector3();
  let edgeOpacities: number[] = [];
  const lightSamples = new Map<string, { time: number; depth: number; opacity: number; z: number; pickable: boolean }>();
  let renderCost = { calls: 0, triangles: 0, geometries: 0, textures: 0 };
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  try {
    OrbitControls.prototype.update = function (this: OrbitControls, ...args: Parameters<OrbitControls["update"]>) {
      const result = originalControlsUpdate.apply(this, args);
      cameraDistance = this.object.position.distanceTo(this.target);
      autoRotation = this.autoRotate;
      return result;
    };
    Positions.prototype.install = function (this: Positions, ...args: Parameters<Positions["install"]>) {
      positions = this; model = args[0]; return originalInstall.apply(this, args);
    };
    Positions.prototype.packOverview = function (this: Positions, ...args: Parameters<Positions["packOverview"]>) {
      const packed = originalPack.apply(this, args);
      if (packed) overviewPacked = true;
      return packed;
    };
    Scene.prototype.onBeforeRender = function (renderer, scene, frameCamera, geometry, material, group) {
      camera = frameCamera;
      edgeOpacities = [];
      renderCost = { calls: renderer.info.render.calls, triangles: renderer.info.render.triangles,
        geometries: renderer.info.memory.geometries, textures: renderer.info.memory.textures };
      scene.traverse(object => {
        if (object instanceof Line && object.material instanceof LineBasicMaterial) edgeOpacities.push(object.material.opacity);
        if (object instanceof Mesh && object.name === "stellar-light" && object.material instanceof ShaderMaterial && !wrapped.has(object)) {
          wrapped.add(object);
          const drawLight = object.onBeforeRender;
          object.onBeforeRender = function (...args) {
            drawLight.apply(this, args);
            const uniforms = (object.material as ShaderMaterial).uniforms, id = object.parent?.userData.nodeId;
            const hits: Parameters<Mesh["raycast"]>[1] = [];
            object.raycast(new Raycaster(), hits);
            if (id) lightSamples.set(id, { time: uniforms.uTime.value, depth: uniforms.uDepth.value,
              opacity: uniforms.uOpacity.value, z: object.getWorldPosition(worldPosition).z, pickable: hits.length > 0 });
          };
        }
        if (!(object instanceof Mesh) || !(object.material instanceof ShaderMaterial) || !object.material.uniforms.uRotation || wrapped.has(object)) return;
        wrapped.add(object);
        const draw = object.onBeforeRender;
        object.onBeforeRender = function (...args) {
          draw.apply(this, args);
          const uniforms = (object.material as ShaderMaterial).uniforms, id = object.parent?.userData.nodeId;
          if (id) sampled.set(id, { rotation: uniforms.uRotation.value, shimmer: uniforms.uShimmer.value, wobble: uniforms.uWobble.value.length(), depth: uniforms.uDepth.value, z: object.getWorldPosition(worldPosition).z });
        };
      });
      return originalRender.call(this, renderer, scene, frameCamera, geometry, material, group);
    };
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
      if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
      if (url.pathname === "/api/graph") return json(snapshot);
      if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
      throw new Error(`Unexpected motion request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal`);
    window.matchMedia = query => query === motionPreference.media ? motionPreference : originalMatchMedia.call(window, query);
    root.render(<App />);
    await until(() => sampled.size === nodes.length && !!positions && overviewPacked && !positions.layoutMoving && !!camera, "actual Graph renders each luminous star after its compact initial layout settles");
    let initialPose = "", initialPoseSince = performance.now();
    await until(() => {
      const pose = JSON.stringify([camera!.position.toArray(), model!.nodes.map(node => [node.id, node.x, node.y, node.z])]);
      if (positions!.layoutMoving || positions!.settling || pose !== initialPose) initialPoseSince = performance.now();
      initialPose = pose;
      return performance.now() - initialPoseSince >= 250;
    }, "the connected layout and framing settle before label observations");
    const labelLayer = host.querySelector<HTMLElement>(".node-labels")!, labelOrigin = labelLayer.getBoundingClientRect();
    const renderSize = host.querySelector("canvas")!.getBoundingClientRect();
    const labelBoxes = [...labelLayer.querySelectorAll<HTMLElement>(".node-label")].filter(label => !label.hidden).map(label => ({ id: label.dataset.nodeId!, box: label.getBoundingClientRect() }));
    assert(labelBoxes.length > 0 && [...labelLayer.querySelectorAll<HTMLElement>(".node-label:not([hidden])")].every(label => label.classList.contains("compact")), "resting labels display compact literal titles");
    const chromeBoxes = [...host.querySelectorAll<HTMLElement>(".graph-minimap, .graph-core-actions, .graph-core-close, .graph-instructions")].filter(element => element.offsetWidth && element.offsetHeight).map(element => element.getBoundingClientRect());
    const overlap = (a: DOMRect, b: DOMRect) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
    assert(labelBoxes.every(({ box }) => !chromeBoxes.some(chrome => overlap(box, chrome))), "mounted labels clear the actual minimap and navigation areas");
    for (const { id, box } of labelBoxes) for (const other of model!.nodes) {
      if (other.id === id) continue;
      const point = new Vector3(other.x, other.y, other.z).applyMatrix4(camera!.matrixWorldInverse);
      const pixels = nodeScreenSize(other.kind, -point.z, renderSize.height, camera!.projectionMatrix.elements[5]);
      const radius = nodeVisualRadius(pixels, false, other.changed);
      point.applyMatrix4(camera!.projectionMatrix);
      if (point.z < -1 || point.z > 1) continue;
      const x = labelOrigin.left + (point.x + 1) * renderSize.width / 2, y = labelOrigin.top + (1 - point.y) * renderSize.height / 2;
      const clearance = Math.hypot(x - Math.max(box.left, Math.min(box.right, x)), y - Math.max(box.top, Math.min(box.bottom, y)));
      assert(clearance >= radius + 3.8, `mounted label ${id} clears star ${other.id}: ${clearance.toFixed(3)} >= ${(radius + 3.8).toFixed(3)}`);
    }
    const restingLayout = JSON.stringify(model!.nodes.map(node => [node.id, node.x, node.y, node.z]));
    await until(() => edgeOpacities.filter(value => value > .2).length === 2 && edgeOpacities.filter(value => value < .1).length === 2, "the actual hub retains two outline edges and its remaining genuine edges at low emphasis");
    details("보기 설정").summary.click();
    const allConnections = [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find(input => input.parentElement?.textContent?.includes("연결선 모두 보기"));
    assert(allConnections && !allConnections.checked, "full connections have an explicit unchecked view setting");
    allConnections.click();
    await until(() => edgeOpacities.length === links.length && edgeOpacities.every(value => value > .2), "all-connections restores each rendered edge");
    assert(JSON.stringify(model!.nodes.map(node => [node.id, node.x, node.y, node.z])) === restingLayout, "edge emphasis changes preserve every real world position");
    allConnections.click();
    await until(() => edgeOpacities.filter(value => value > .2).length === 2 && edgeOpacities.filter(value => value < .1).length === 2, "returning to the overview restores the outline without losing connections");
    details("보기 설정").summary.click();
    assert(new Set([...sampled.values()].map(star => star.z.toFixed(6))).size > 1, "the rendered stars retain depth instead of flattening their world positions");
    assert([...sampled.values()].every(star => Number.isFinite(star.depth) && star.depth >= 0 && star.depth <= 1), "each actual draw uploads a bounded depth light value");
    assert(lightSamples.size === nodes.length && [...lightSamples].every(([id, value]) => !value.pickable &&
      value.z === sampled.get(id)!.z && value.depth === sampled.get(id)!.depth),
      "one non-pickable light follows each real star at its unchanged world depth");
    const initialLight = new Map(lightSamples);
    await until(() => [...lightSamples].every(([id, value]) => value.time > initialLight.get(id)!.time),
      "the diffuse light advances with the mounted renderer's existing clock");
    const initial = new Map(sampled);
    await until(() => [...sampled].some(([id, value]) => Math.abs(value.rotation - initial.get(id)!.rotation) > .01 && Math.abs(value.shimmer - initial.get(id)!.shimmer) > .001), "the mounted renderer advances stellar surface and shimmer motion");
    const node = model!.nodes.find(node => node.id === nodes[0].id)!, canvas = host.querySelector("canvas")!, rect = canvas.getBoundingClientRect();
    const at = new Vector3(node.x, node.y, node.z).project(camera!);
    const titleBox = () => host.querySelector<HTMLElement>(`.node-label:not([hidden])[data-node-id="${node.id}"] strong`)?.getBoundingClientRect();
    const restingTitle = titleBox();
    assert(restingTitle, "the pointed star already has a resting title");
    canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse", clientX: rect.left + (at.x + 1) * rect.width / 2, clientY: rect.top + (1 - at.y) * rect.height / 2 }));
    await until(() => (sampled.get(node.id)?.wobble ?? 0) > .01, "pointer proximity drives wobble in the actual star draw");
    await until(() => !!host.querySelector(".graph-tooltip"), "mouse pointing exposes the actual hover guidance");
    const pointedTitle = titleBox();
    assert(pointedTitle && Math.hypot(pointedTitle.left - restingTitle.left, pointedTitle.top - restingTitle.top) < 1,
      `pointing keeps the visible title at its resting position: ${pointedTitle ? Math.hypot(pointedTitle.left - restingTitle.left, pointedTitle.top - restingTitle.top).toFixed(1) : "missing"}px`);
    assert(host.querySelector(".graph-tooltip span")?.textContent?.includes(node.label) &&
      [...host.querySelectorAll<HTMLElement>(`.node-label[data-node-id="${node.id}"] > span, .node-label[data-node-id="${node.id}"] > small`)].every(element => element.offsetHeight === 0),
      "the full source path and state remain in the guidance without duplicating them beside the anchored title");
    for (const pointerType of ["mouse", "pen"]) {
      const minimap = host.querySelector<HTMLButtonElement>(".minimap-disclosure")!;
      minimap.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "touch", pointerId: 19, button: 0 }));
      minimap.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "touch", pointerId: 19, button: 0 }));
      minimap.click();
      await settle();
      assert(!host.querySelector(".graph-tooltip"), "touch clears guidance even while the renderer retains its last pointed star");
      assert([...host.querySelectorAll<HTMLElement>(".node-label:not([hidden])")].every(label => !label.classList.contains("hovered")), "touch does not retain a detailed hovered title");
      await until(() => edgeOpacities.filter(value => value < .1).length === 2, "touch restores the resting connection emphasis");
      canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType, clientX: rect.left + (at.x + 1) * rect.width / 2, clientY: rect.top + (1 - at.y) * rect.height / 2 }));
      await until(() => host.querySelector(".graph-tooltip strong")?.textContent === node.title &&
        !!host.querySelector(`.node-label.hovered[data-node-id="${node.id}"]`) &&
        edgeOpacities.length === links.length && edgeOpacities.every(value => value > .7),
        `${pointerType} restores the anchored title, guidance and every actual connection on the same star after touch without leaving its hit area`);
      const restoredTitle = titleBox();
      assert(restoredTitle && Math.hypot(restoredTitle.left - restingTitle.left, restoredTitle.top - restingTitle.top) < 1,
        `${pointerType} keeps the title anchored after touch`);
    }
    canvas.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, relatedTarget: host }));
    await until(() => sampled.get(node.id)?.wobble === 0, "pointer exit restores the star without leaving a wobble behind");
    details("보기 설정").summary.click();
    const before = camera!.position.clone();
    const rotation = button("3D 회전 시작");
    assert(rotation.getAttribute("aria-pressed") === "false" && rotation.title.includes("느린 3D"), "minimap rotation exposes its mode and pace");
    rotation.click();
    await until(() => camera!.position.distanceTo(before) > .01, "slow 3D camera rotation remains available with pointer rotation disabled");
    // Exercise the root hold guard; native pan is checked separately with real pointer capture.
    host.querySelector<HTMLElement>(".graph-canvas")!.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse", pointerId: 9, button: 0, buttons: 1, clientX: rect.left + 5, clientY: rect.top + 5 }));
    await until(() => !autoRotation, "the root pointer hold guard pauses automatic rotation");
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "mouse", pointerId: 9, button: 0, buttons: 0, clientX: rect.left + 5, clientY: rect.top + 5 }));
    await until(() => autoRotation, "release resumes the requested rotation without leaving a stale pointer");
    const navigation = host.querySelector<HTMLCanvasElement>('.graph-minimap canvas')!;
    navigation.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    navigation.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", repeat: true, bubbles: true }));
    await until(() => !autoRotation, "held and repeating minimap keys pause automatic rotation");
    window.dispatchEvent(new KeyboardEvent("keyup", { key: "ArrowRight", bubbles: true }));
    await until(() => autoRotation, "key release resumes automatic rotation");
    assert(button("지도 회전 멈춤").getAttribute("aria-pressed") === "true", "settings and minimap share the same rotation state");
    button("3D 회전 멈춤").click();
    await until(() => host.querySelector('[aria-label="3D 회전 시작"]')?.getAttribute("aria-pressed") === "false", "minimap stop commits the shared rotation state");
    assert(button("지도 천천히 회전").getAttribute("aria-pressed") === "false", "stopping at the minimap also updates settings");
    details("보기 설정").summary.click();
    let restingDirection = camera!.getWorldDirection(new Vector3()), restingSamples = 0;
    await until(() => {
      const current = camera!.getWorldDirection(new Vector3());
      const resting = current.distanceTo(restingDirection) < .000001;
      restingDirection = current;
      restingSamples = resting ? restingSamples + 1 : 0;
      return restingSamples >= 4;
    }, "the existing damped rotation settles before isolated zoom observations");
    assert(JSON.stringify(model!.nodes.map(node => [node.id, node.x, node.y, node.z])) === restingLayout, "slow camera rotation preserves the compact world layout without repacking");
    const disclosure = host.querySelector<HTMLButtonElement>(".minimap-disclosure")!;
    if (disclosure.getAttribute("aria-expanded") === "false") disclosure.click();
    await until(() => disclosure.getAttribute("aria-expanded") === "true", "open mobile minimap before measuring controls");
    const minimap = host.querySelector<HTMLCanvasElement>('.graph-minimap canvas')!;
    const controls = [...host.querySelectorAll<HTMLButtonElement>('.minimap-actions button')];
    assert(controls.length === 4 && controls.every(control => control.getAttribute("aria-label") && control.title), "four minimap controls expose accessible names and tooltips");
    const boxes = controls.map(control => control.getBoundingClientRect());
    assert(boxes.every((box, index) => box.width >= 32 && box.height >= 32 && (!index || box.left - boxes[index - 1].right >= 3.9)), "minimap icon targets have usable size and separation");
    assert(host.querySelectorAll('button[aria-label="전체 맞춤"]').length === 1, "full fit has one minimap entry");
    const zoomBefore = camera!.position.clone(), direction = camera!.getWorldDirection(new Vector3());
    button("확대").click();
    await until(() => camera!.position.distanceTo(zoomBefore) > 1, "minimap plus changes the mounted camera");
    await new Promise(resolve => setTimeout(resolve, 200));
    assert(camera!.getWorldDirection(new Vector3()).distanceTo(direction) < .00001, "zoom retains the rotated 3D direction");
    button("축소").click();
    await until(() => camera!.position.distanceTo(zoomBefore) < .01, "inverse minimap zoom restores the live camera pose");
    minimap.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await until(() => camera!.position.distanceTo(zoomBefore) > 1, "minimap keyboard pans the live camera");
    const panned = camera!.position.clone(), pannedDistance = cameraDistance;
    button("지도 가운데로 이동").click();
    await until(() => camera!.position.distanceTo(panned) > 1, "center returns from a panned view");
    assert(Math.abs(cameraDistance - pannedDistance) < .00001, "center preserves the actual camera-to-target zoom distance");
    const lightBeforeZoom = new Map([...sampled].map(([id, star]) => [id, star.depth]));
    for (let step = 0; step < 8; step++) button("축소").click();
    await until(() => [...sampled].every(([id, star]) => star.depth < lightBeforeZoom.get(id)!) && new Set([...sampled.values()].map(star => star.depth.toFixed(6))).size > 1,
      "shared material uploads distinct actual camera depths after zooming into the distant size floor");
    const fullFit = button("전체 맞춤"); fullFit.focus(); fullFit.click();
    await until(() => camera!.getWorldDirection(new Vector3()).distanceTo(new Vector3(0, 0, -1)) < .00001, "the existing full fit resets the camera orientation");
    assert(document.activeElement === fullFit, "full fit preserves the initiating control's focus after panel close callbacks");
    assert([...sampled.values()].some(star => star.z !== 0), "minimap navigation preserves stellar depth");
    let fittedPosition = camera!.position.clone(), fittedLayout = "", fittedSince = performance.now();
    await until(() => {
      const layout = JSON.stringify(model!.nodes.map(node => [node.id, node.x, node.y, node.z]));
      if (positions!.layoutMoving || camera!.position.distanceTo(fittedPosition) > .000001 || layout !== fittedLayout) fittedSince = performance.now();
      fittedPosition = camera!.position.clone(); fittedLayout = layout;
      return performance.now() - fittedSince >= 250;
    }, "full fit and its overview clearance settle before disclosure observations");
    const minimapDisclosure = () => host.querySelector<HTMLButtonElement>(".minimap-disclosure")!;
    const poseBeforeCollapse = camera!.position.clone(), layoutBeforeCollapse = JSON.stringify(model!.nodes.map(node => [node.id, node.x, node.y, node.z]));
    const openHeight = host.querySelector('.graph-minimap')!.getBoundingClientRect().height;
    minimapDisclosure().click();
    await until(() => minimapDisclosure().getAttribute("aria-expanded") === "false", "minimap collapse exposes its accessible expansion control");
    assert(minimap.closest<HTMLElement>('[hidden]')?.hidden, "collapsed navigation leaves the tab order and drawing area");
    assert(host.querySelector('.graph-minimap')!.getBoundingClientRect().height < openHeight / 2, "collapse frees graph space");
    assert(button("3D 회전 시작").getBoundingClientRect().height >= 32, "rotation remains reachable while collapsed");
    await until(() => {
      const chrome = host.querySelector('.graph-minimap')!.getBoundingClientRect();
      return [...host.querySelectorAll<HTMLElement>('.node-label:not([hidden])')].every(label => {
        const box = label.getBoundingClientRect();
        return box.right <= chrome.left || box.left >= chrome.right || box.bottom <= chrome.top || box.top >= chrome.bottom;
      });
    }, "labels reflow around the collapsed minimap");
    minimapDisclosure().click();
    await until(() => minimapDisclosure().getAttribute("aria-expanded") === "true" && !minimap.closest('[hidden]'), "minimap expands and restores navigation");
    assert(camera!.position.distanceTo(poseBeforeCollapse) < .00001, "disclosure preserves the settled camera pose");
    assert(JSON.stringify(model!.nodes.map(node => [node.id, node.x, node.y, node.z])) === layoutBeforeCollapse, "disclosure preserves the settled world layout");
    reduceMotion = true; motionPreference.dispatchEvent(new Event("change"));
    await until(() => host.querySelector<HTMLButtonElement>('[aria-label="3D 회전 시작"]')?.disabled === true, "the mounted App receives reduced motion from its media preference");
    const frozenLight = new Map(lightSamples), frozenStars = new Map(sampled);
    await new Promise(resolve => setTimeout(resolve, 180));
    assert([...lightSamples].every(([id, value]) => value.time === frozenLight.get(id)!.time) &&
      [...sampled].every(([id, value]) => value.rotation === frozenStars.get(id)!.rotation && value.shimmer === frozenStars.get(id)!.shimmer),
      "reduced motion freezes both the real star draw and its diffuse light clock");
    reduceMotion = false; motionPreference.dispatchEvent(new Event("change"));
    await until(() => [...lightSamples].every(([id, value]) => value.time > frozenLight.get(id)!.time), "restoring motion resumes the shared light clock");
    return { renderedMotion: true, cosmicLight: true, reducedLightMotion: true, renderCost, depthLight: true, connectionOutline: true, allConnections: true, labelsClearStarsAndChrome: true, stars: nodes.length, initialCompacted: overviewPacked, depth: true, surfaceAndShimmer: true, hoverWobble: true, touchHoverCleared: true, sameStarPointerRestored: ["mouse", "pen"], slow3DRotation: true, rotationPreservesLayout: true, minimapControls: true, minimapDisclosure: true, zoomRoundTrip: true, centerPreservesZoom: true, fullFit: true };
  } finally {
    root.unmount(); Scene.prototype.onBeforeRender = originalRender; Positions.prototype.install = originalInstall;
    Positions.prototype.packOverview = originalPack;
    OrbitControls.prototype.update = originalControlsUpdate;
    window.fetch = originalFetch; window.matchMedia = originalMatchMedia; window.history.replaceState(null, "", originalUrl);
  }
}

async function checkHeaderLayout() {
  const originalFetch = window.fetch, originalUrl = window.location.href, originalZoom = host.style.zoom;
  const root = createRoot(host), samples: { name: string; controls: number; height: number }[] = [];
  try {
    window.history.replaceState(null, "", "/?scope=personal");
    window.fetch = async input => {
      const url = new URL(input instanceof Request ? input.url : String(input), location.href);
      const value = url.pathname === "/api/session" ? { csrf: "synthetic-only", areas: [] }
        : url.pathname === "/api/graph" ? graph([item(1, "personal")], "personal", "", null, false)
        : { enabled: false, running: false, error: null, report: null };
      return new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
    };
    root.render(<App />);
    await until(() => !!host.querySelector(".app-navigation") && host.querySelector<HTMLButtonElement>(".map-search button")?.disabled === false, "header is ready");
    const header = host.querySelector<HTMLElement>(".app-header")!, nav = header.querySelector<HTMLElement>(".app-navigation")!;
    assert(!nav.querySelector('a[href="/career"]'), "retired workspace is absent from primary navigation");
    assert(!header.querySelector(".scope-switch a"), "workflow navigation is separate from scope selection");
    const inspect = async (name: string) => {
      header.scrollIntoView({ block: "start" }); await settle();
      const controls = [...header.querySelectorAll<HTMLElement>("a, button, input")];
      const boxes = controls.map(control => control.getBoundingClientRect());
      for (const [index, control] of controls.entries()) {
        const box = boxes[index];
        assert(box.width > 1 && box.left >= -.5 && box.right <= innerWidth + .5, `${name}: every header control stays inside the width`);
        assert(box.top >= -.5 && box.bottom <= innerHeight + .5, `${name}: header controls can be reached in the viewport`);
        assert(boxes.every((other, otherIndex) => index === otherIndex || box.right <= other.left + .5 || box.left >= other.right - .5 || box.bottom <= other.top + .5 || box.top >= other.bottom - .5), `${name}: header hit areas do not overlap`);
        const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
        assert(hit && control.contains(hit), `${name}: the visible control receives its own pointer target`);
        control.focus(); assert(document.activeElement === control, `${name}: every control remains keyboard reachable`);
        const focused = control.getBoundingClientRect();
        assert(Math.abs(focused.width - box.width) < .5 && Math.abs(focused.height - box.height) < .5, `${name}: focus does not resize the control`);
      }
      samples.push({ name, controls: controls.length, height: header.getBoundingClientRect().height });
    };
    await inspect("normal");
    if (innerWidth <= 700) { host.style.zoom = "2"; await inspect("mobile-css-zoom-200-percent"); }
    return { headerLayout: true, viewport: { width: innerWidth, height: innerHeight }, samples };
  } finally {
    root.unmount(); window.fetch = originalFetch; host.style.zoom = originalZoom;
    window.history.replaceState(null, "", originalUrl);
  }
}


// Only the newly affected map/list/marker transitions; existing editing suites are separate.
async function checkResidualMap(mouseOnly = false) {
  const originalUrl = window.location.href;
  const originalRender = Scene.prototype.onBeforeRender, originalPack = Positions.prototype.packOverview;
  const root = createRoot(host), samples = new Map<string, { x: number; y: number; z: number }>();
  let positions: Positions | undefined, apiCalls = 0;
  const names = ["합성 연구와 실행 목적", "합성 개발과 품질 목적"], subjects = [subjectId, "p_00000000-0000-4000-8000-000000000002"];
  const members: GraphNode[] = [225, 39].flatMap((count, group) => Array.from({ length: count }, (_, i) => ({
    id: `residual-${group}-${String(i).padStart(3, "0")}`, scope: "personal", kind: "document", label: `합성 자료 ${group}-${i}`,
    subject_id: subjects[group], subject_name: names[group], status: "ok", present: true, current: true,
  })));
  const nodes: GraphNode[] = [...subjects.map((id, i): GraphNode => ({ id, scope: "personal", kind: "subject", label: names[i], purpose_total: i ? 39 : 225 })), ...members];
  const links: Snapshot["links"] = members.map(node => ({ source: node.id, target: node.subject_id!, kind: "subject", current: true }));
  const snapshot: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links,
    matched: nodes.length, totals: { documents: members.length, memories: 0, markers: 2, links: links.length },
    returned: { knowledge: members.length, markers: 2, links: links.length }, omitted: { nodes: 0, links: 0 },
    eligible: { nodes: nodes.length, links: links.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  const separate = (a: DOMRect, b: DOMRect) => a.right <= b.left || a.left >= b.right || a.bottom <= b.top || a.top >= b.bottom;
  const chromeClear = () => {
    const help = host.querySelector(".graph-quick-help")!.getBoundingClientRect(), mini = host.querySelector(".graph-minimap")!.getBoundingClientRect();
    assert(separate(help, mini), "the actual canvas keeps minimap and sentence guidance separate");
    assert(help.left >= 0 && help.right <= innerWidth && mini.right <= innerWidth, "map controls remain inside the screen");
  };
  const frameIsReadable = (group: number) => {
    if (!positions || positions.layoutMoving) return false;
    const box = host.querySelector(".graph-canvas")!.getBoundingClientRect();
    const actual = [...samples].filter(([id]) => id.startsWith(`residual-${group}-`));
    const obstacles = [...host.querySelectorAll<HTMLElement>(".graph-minimap, .graph-quick-help, .graph-core-actions, .map-sidebar")]
      .filter(element => element.offsetWidth && element.offsetHeight).map(element => element.getBoundingClientRect());
    return actual.length === 12 && actual.every(([, at]) => {
      const x = box.left + (at.x + 1) * box.width / 2, y = box.top + (1 - at.y) * box.height / 2;
      return at.z > -1 && at.z < 1 && x >= box.left + 10 && x <= box.right - 10 && y >= box.top + 10 && y <= box.bottom - 10 &&
        obstacles.every(other => x + 10 <= other.left || x - 10 >= other.right || y + 10 <= other.top || y - 10 >= other.bottom);
    });
  };
  // Match the application viewport; fixture prose/results stay available in the DOM.
  document.querySelector<HTMLParagraphElement>("body > p")!.hidden = true;
  output.hidden = true; host.style.position = "fixed"; host.style.inset = "0";
  Positions.prototype.packOverview = function (this: Positions, ...args: Parameters<Positions["packOverview"]>) {
    positions = this; return originalPack.apply(this, args);
  };
  Scene.prototype.onBeforeRender = function (...args) {
    samples.clear();
    this.traverse(object => {
      const id = object.userData.nodeId;
      if (typeof id !== "string" || typeof object.userData.opacity !== "function" || object.userData.opacity() < .99 || object.userData.retiring?.()) return;
      const p = object.getWorldPosition(new Vector3()).project(args[2]); samples.set(id, { x: p.x, y: p.y, z: p.z });
    });
    originalRender.apply(this, args);
  };
  window.fetch = async (input, init) => {
    const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
    assert(!init?.method || init.method === "GET", "the visual fixture never writes real or synthetic records");
    apiCalls++;
    if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
    if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
    if (url.pathname === "/api/graph") return json({ ...snapshot, focus: { id: url.searchParams.get("focus"), found: !!url.searchParams.get("focus") } });
    throw new Error(`Unexpected residual-map request: ${url.pathname}`);
  };
  window.history.replaceState(null, "", `${window.location.pathname}?scope=personal`);
  root.render(<App />);
  await until(() => !!positions && !!host.querySelector(".graph-minimap") && !positions.layoutMoving, "purpose overview is rendered");
  chromeClear();
  if (mouseOnly) {
    await until(() => samples.has(subjects[0]) && !positions!.layoutMoving, "the actual summary is projected");
    const canvas = host.querySelector<HTMLCanvasElement>(".graph-canvas canvas")!, rect = canvas.getBoundingClientRect();
    const at = samples.get(subjects[0])!;
    const eventAt = { clientX: rect.left + (at.x + 1) * rect.width / 2, clientY: rect.top + (1 - at.y) * rect.height / 2 };
    canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse", ...eventAt }));
    await until(() => host.querySelector(".graph-tooltip")?.getAttribute("data-node-id") === subjects[0], "renderer hover state is visible");
    await new Promise(resolve => setTimeout(resolve, 550));
    const current = samples.get(subjects[0])!;
    window.history.replaceState(null, "", originalUrl);
    output.textContent = `AWAITING_MOUSE\n${JSON.stringify({ x: rect.left + (current.x + 1) * rect.width / 2, y: rect.top + (1 - current.y) * rect.height / 2, hoverMilliseconds: 550 })}`;
    await new Promise<void>(resolve => canvas.addEventListener("pointerup", function activated(event) {
      if (!event.isTrusted || event.pointerType !== "mouse" || event.button !== 0) return;
      canvas.removeEventListener("pointerup", activated); resolve();
    }));
    await until(() => host.querySelector(".graph-core-page")?.textContent === "1/19" && !host.querySelector(".management-panel") && frameIsReadable(0), "native mouse release expands the hovered summary once");
    window.history.replaceState(null, "", originalUrl);
    return { width: innerWidth, height: innerHeight, hoverMilliseconds: 550, hover: "scripted pointer movement with actual renderer state", activation: "trusted browser mouse release", page: "1/19", apiCalls };
  }
  const quick = host.querySelector(".graph-quick-help .pointer-instructions")!.textContent;
  assert(quick?.includes("묶음을 누르면 펼쳐집니다."), "quick guidance uses complete gesture sentences");
  button("목록 보기").click(); await openExplorer();
  if (innerWidth > 900) {
    const sidebar = host.querySelector(".map-sidebar")!.getBoundingClientRect();
    for (const title of [...host.querySelectorAll(".graph-list strong")].slice(0, 3))
      assert(title.getBoundingClientRect().left >= sidebar.right, "list titles remain outside the open filter panel");
  } else assert(host.querySelector(".galaxy")!.hasAttribute("inert"), "mobile filters are a modal choice rather than a covered list");
  host.querySelector<HTMLButtonElement>(".map-sidebar .sidebar-heading button")!.click(); await settle();
  const phases: unknown[] = [];
  for (const group of [1, 0]) {
    const row = [...host.querySelectorAll<HTMLButtonElement>(".graph-list > button")].find(row => row.querySelector("strong")?.textContent === names[group])!;
    row.click(); await until(() => host.querySelector(".management-panel h2")?.textContent === names[group], "purpose detail selection");
    button("3D 보기").click();
    await until(() => host.querySelector(".graph-core-page")?.textContent === `1/${group ? 4 : 19}` && frameIsReadable(group), "selected purpose opens a readable 12-member page");
    if (innerWidth <= 900) {
      assert(host.querySelector(".galaxy")!.hasAttribute("inert"), "mobile purpose detail owns its modal screen");
      button("닫기 ×").click(); await settle();
      const target = host.querySelector<HTMLButtonElement>(`.graph-group-target[data-node-id="${subjects[group]}"]`)!;
      target.click(); await until(() => !!host.querySelector(".graph-core-page") && frameIsReadable(group), "mobile returns to the same purpose grouping");
    }
    chromeClear();
    button("다음", host.querySelector(".graph-core-actions")!).click();
    await until(() => host.querySelector(".graph-core-page")?.textContent === `2/${group ? 4 : 19}` && frameIsReadable(group), "page two remains in the reading frame");
    if (innerWidth > 900) {
      assert(host.querySelector(".management-panel h2")?.textContent === names[group], "paging preserves the selected purpose detail");
      await openExplorer();
      await until(() => frameIsReadable(group), "selected purpose refits after filter obstruction");
      const sidebar = host.querySelector(".map-sidebar")!.getBoundingClientRect();
      assert(host.querySelector(".graph-core-actions")!.getBoundingClientRect().left >= sidebar.right, "the open filter leaves every page control readable");
      const canvas = host.querySelector<HTMLCanvasElement>(".graph-canvas canvas")!, box = canvas.getBoundingClientRect();
      const sample = [...samples].find(([id]) => id.startsWith(`residual-${group}-`))![1];
      canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse", clientX: box.left + (sample.x + 1) * box.width / 2, clientY: box.top + (1 - sample.y) * box.height / 2 }));
      await until(() => !!host.querySelector(".graph-tooltip"), "pointed member guidance is rendered");
      const tooltip = host.querySelector(".graph-tooltip")!.getBoundingClientRect();
      assert(tooltip.left >= sidebar.right && [...host.querySelectorAll(".graph-minimap, .graph-quick-help")].every(element => separate(tooltip, element.getBoundingClientRect())), "hover guidance clears the filter, minimap and quick instructions");
      host.querySelector<HTMLButtonElement>(".map-sidebar .sidebar-heading button")!.click(); await settle();
    }
    phases.push({ group: names[group], page: host.querySelector(".graph-core-page")!.textContent, visibleMembers: [...samples.keys()].filter(id => id.startsWith(`residual-${group}-`)).length, labels: checkLabelAssociations() });
    if (group !== 0) { button("묶음 접기").click(); await settle(); button("목록 보기").click(); await settle(); }
  }
  // Restore the fixture entry for a development refresh while retaining the rendered App state.
  window.history.replaceState(null, "", originalUrl);
  // The final synthetic screen is deliberately retained for visual inspection.
  return { width: innerWidth, height: innerHeight, phases, apiCalls, retained: "purpose page two; synthetic APIs only" };
}

async function run() {
  const results: unknown[] = [];
  try {
    assert(import.meta.env.DEV, "Run this fixture through the development server");
    if (["residual", "residual-mouse"].includes(new URLSearchParams(window.location.search).get("scenario") ?? "")) {
      results.push(await checkResidualMap(new URLSearchParams(window.location.search).get("scenario") === "residual-mouse"));
      output.textContent = `PASS\n${JSON.stringify({ passed: true, results }, null, 2)}`;
      return;
    }
    if (new URLSearchParams(window.location.search).get("scenario") === "references") {
      results.push(await checkSourceReferenceDirection());
      output.textContent = `PASS\n${JSON.stringify({ passed: true, results }, null, 2)}`;
      return;
    }
    if (new URLSearchParams(window.location.search).get("scenario") === "filters") {
      const filters = await checkOriginalFolders();
      output.textContent = `PASS\n${JSON.stringify({ passed: true, filters }, null, 2)}`;
      return;
    }
    if (new URLSearchParams(window.location.search).get("scenario") === "header") {
      results.push(await checkHeaderLayout());
      output.textContent = `PASS\n${JSON.stringify({ passed: true, results }, null, 2)}`;
      return;
    }
    if (new URLSearchParams(window.location.search).get("scenario") === "motion") {
      results.push(await checkRenderedMotion());
      output.textContent = `PASS\n${JSON.stringify({ passed: true, results }, null, 2)}`;
      return;
    }
    if (new URLSearchParams(window.location.search).get("scenario") === "records") {
      for (const size of [1, 20]) results.push(await check(size));
      output.textContent = `PASS\n${JSON.stringify({ passed: true, results }, null, 2)}`;
      return;
    }
    if (new URLSearchParams(window.location.search).get("scenario") === "documents") {
      results.push(await checkDocumentEditing());
      output.textContent = `PASS\n${JSON.stringify({ passed: true, results }, null, 2)}`;
      return;
    }
    results.push(checkSummaryGlyphMotion());
    for (const size of [1, 20]) results.push(await check(size));
    results.push(await checkDocumentEditing());
    results.push(await checkOriginalFolders());
    results.push(await checkPurposeAndOriginalBusy());
    results.push(await checkPurposeDefinitionRefresh());
    results.push(await checkSourceReferenceDirection());
    results.push(await checkNativeConstellation());
    results.push(await checkPurposeDiagram());
    results.push(await checkFirstProximityCollapse());
    results.push(await checkSummaryOverview());
    results.push(await checkSummaryContentRefresh());
    results.push(await checkRenderedMotion());
    output.textContent = `PASS\n${JSON.stringify({ passed: true, results }, null, 2)}`;
  } catch (error) {
    output.textContent = `FAIL\n${JSON.stringify({ passed: false, error: error instanceof Error ? error.message : String(error), results, active }, null, 2)}`;
  }
}
void run();
