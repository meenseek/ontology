import { createRoot } from "react-dom/client";
import { Mesh, Scene, ShaderMaterial, Vector3 } from "three";
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
function enter(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const prototype = element instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event("input", { bubbles: true }));
  element.dispatchEvent(new Event("change", { bubbles: true }));
}
async function chooseDiagramMode(value: "purpose" | "relationships") {
  button("필터·묶음").click();
  await until(() => !!host.querySelector('select[aria-label="묶음 기준"]'), "diagram mode selector");
  const select = host.querySelector<HTMLSelectElement>('select[aria-label="묶음 기준"]')!;
  select.value = value;
  select.dispatchEvent(new Event("change", { bubbles: true }));
  await settle();
  button("필터·묶음").click();
  await settle();
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
  let firstGraph = true, failSync = false, subjectExists = true;
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
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
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
        return response(graph([...records[scope].values()], scope, url.searchParams.get("q") ?? "", url.searchParams.get("focus"), subjectExists));
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
    await until(() => host.querySelector(".load-timing")?.textContent?.includes("목록 표시") === true, "list timing");
    assert(/응답 \d+ms · 목록 표시 \d+ms/.test(host.querySelector(".load-timing")?.textContent ?? ""), "Timing names both measured phases");
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
    const first = host.querySelector<HTMLButtonElement>(".graph-list > button")!;
    first.click();
    await until(() => bodyIs(records.meenseek.get(id(1))!.body), "selected memory");
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
    start = calls.length; form.requestSubmit();
    await until(() => idleGraph() && bodyIs(`새 본문 ${size}`), "remember response reused");
    phase("remember response handoff", start, { remember: 1, graph: 1 });

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
    await settle(); start = calls.length; correction.requestSubmit();
    await until(() => idleGraph() && bodyIs(`정정 본문 ${size}`), "correct response reused");
    phase("correct response handoff", start, { correct: 1, graph: 1 });
    details("보기 설정").summary.click(); await settle();
    start = calls.length; button("새로고침").click();
    await until(() => idleGraph() && calls.slice(start).some(call => call.operation === "read") && bodyIs(`정정 본문 ${size}`), "explicit fresh read");
    phase("explicit refresh clears write seed", start, { graph: 1, read: 1 });

    button("닫기 ×").click(); await settle();
    start = calls.length; button("기록 남기기").click(); await settle();
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
    window.fetch = originalFetch;
    window.history.replaceState(null, "", originalUrl);
    if (visibility) Object.defineProperty(document, "visibilityState", visibility);
    else Reflect.deleteProperty(document, "visibilityState");
  }
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
        return json({ metadata: { scope: url.searchParams.get("scope"), path, source_path: "synthetic", revision: 1, content_digest: "synthetic", byte_len: 120 }, title: `원문 ${index}`,
          content: `# 원문 ${index}\n\n${index === "1" ? "[본문에서 대상 열기](2.md#part) [다른 범위 원문 열기](../../profile/rules/3.md)" : "대상 본문"}` });
      }
      throw new Error(`Unexpected reference reading request: ${url.pathname}`);
    };
    window.history.replaceState(null, "", `${window.location.pathname}?scope=personal&focus=${originals[0].id}`);
    root.render(<App />);
    await until(() => !!host.querySelector(".original-detail .document-internal-link"), "outgoing authored link resolves in actual preview");
    assert(!host.querySelector('.original-detail [aria-label="연결된 자료"]'), "original reference does not become an explicit semantic related item");
    const before = graphCalls; details("원문 링크").summary.click(); await settle();
    const referenceButtons = [...host.querySelectorAll<HTMLButtonElement>(".source-references button")];
    assert(referenceButtons.length === 3, "selected source retains outgoing and incoming references");
    assert(referenceButtons.some(value => value.textContent?.includes("원문 링크 →") && value.textContent?.includes("원문 2")), "outgoing endpoint label");
    assert(referenceButtons.some(value => value.textContent?.includes("← 이 원문을 참조한 자료") && value.textContent?.includes("원문 3")), "incoming endpoint label");
    assert(brainCalls === 0 && graphCalls === before && originalReads === 1, "disclosure reuses returned link metadata without eager API reads");
    host.querySelector<HTMLButtonElement>(".original-detail .document-internal-link")!.click();
    await until(() => host.querySelector(".original-detail h1")?.textContent === "원문 2", "authored outgoing link opens its exact target");
    assert(new URL(window.location.href).searchParams.get("focus") === originals[1].id && brainCalls === 0, "inline reference navigation preserves exact target and lazy purpose reads");
    window.history.pushState(null, "", `${window.location.pathname}?scope=personal&focus=${originals[0].id}`);
    window.dispatchEvent(new PopStateEvent("popstate"));
    await until(() => host.querySelectorAll(".original-detail .document-internal-link").length === 2, "return to both authored outgoing references");
    button("다른 범위 원문 열기", host.querySelector(".original-detail")!).click();
    await until(() => host.querySelector(".original-detail h1")?.textContent === "원문 3", "cross-native-scope reference opens its exact current target");
    assert(new URL(window.location.href).searchParams.get("focus") === originals[2].id && brainCalls === 0, "cross-scope source reference does not promote incoming evidence or change app scope");
    return { sourceReferenceDirection: true, outgoing: 2, incoming: 1, inlineNavigation: true, fragmentLookup: true, crossNativeScope: true, eagerBrainCalls: 0 };
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
    id, scope: "personal", kind: "document", label: "지원 현황", title: "지원 현황",
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
    const singleton = folderButtons().find(node => node.querySelector("strong")?.textContent === "personal · writing/2025");
    assert(singleton, "Single-document folder remains selectable");
    singleton.click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 1, "single original in selected folder");
    assert(source.value === "personal" && state.value === "active" && folderButtons().length === 4, "Folder click preserves source and state filters and other folder options");
    const beforeStructure = graphCalls;
    await until(() => !!host.querySelector('.map-sidebar input[type="checkbox"]'), "folder overlay control is rendered");
    const structure = [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find(input => input.parentElement?.textContent?.includes("폴더 연결 표시"))!;
    structure.click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 2, "document plus its structural parent");
    assert(host.querySelector(".count-breakdown")?.textContent?.includes("문서 1 · 기록 0 · 관계 0 · 부모·소속 1"), "Parent links and folders do not inflate knowledge or semantic relation counts");
    const folderRow = [...host.querySelectorAll<HTMLButtonElement>(".graph-list > button")].find(node => node.querySelector(".node-symbol.folder"))!;
    folderRow.click();
    await settle();
    assert(!new URL(window.location.href).searchParams.has("focus") && graphCalls === beforeStructure, "Folder navigation stays local without synthetic backend focus or extra requests");
    structure.click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 1, "hide structural overlay");
    assert(graphCalls === beforeStructure, "Both structural toggles reuse the returned snapshot");
    structure.click();
    await until(() => host.querySelectorAll(".graph-list > button").length === 2, "structural overlay before document selection");
    [...host.querySelectorAll<HTMLButtonElement>(".graph-list > button")].find(node => node.querySelector(".node-symbol.document"))!.click();
    await until(() => !!host.querySelector(".original-detail .document-preview"), "original detail preview");
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
    return { originalFolders: true, listedFolders: 4, visibleOriginals: 1, titleCount: 1, draftGuard: true, localFolderStructure: true };
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
  const summary = () => [...host.querySelectorAll<HTMLElement>('.node-label[role="button"]')].find(node => !node.hidden && node.textContent?.includes("관계 묶음 · 121개"));
  const folderSummary = () => [...host.querySelectorAll<HTMLElement>('.node-label[role="button"]')].find(node => !node.hidden && node.textContent?.includes("폴더 묶음 · 40개"));
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
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes("폴더 묶음 · 40개 · 1/4") === true, "folder opens a compact structural page without counting its marker");
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
  const summary = (group: number) => [...host.querySelectorAll<HTMLElement>('.node-label[role="button"]')].find(node => !node.hidden && node.textContent?.includes(`${names[group]} · ${group ? 23 : 120}개`));
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
    summary(0)!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes(`${names[0]} · 120개 · 1/10`) === true && !positions!.layoutMoving, "keyboard opens a 12-item purpose page without counting its subject marker");
    button("다음", host.querySelector(".graph-core-actions")!).click();
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes("2/10") === true && !positions!.layoutMoving, "purpose members page locally");
    button("묶음 접기").click();
    await until(() => !!summary(0) && !!summary(1) && !positions!.layoutMoving, "purpose summaries restore their compact overview");
    assert(graphCalls === firstCalls, "purpose expansion and paging use no extra graph requests");
    responseSnapshot = { ...snapshot, links: memberships };
    details("보기 설정").summary.click(); button("새로고침").click();
    await until(() => graphCalls === firstCalls + 1 && host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false" && !!summary(0) && !!summary(1) && !positions!.layoutMoving, "removing references retains both purpose summaries");
    assert(positions!.structureEpoch === epoch, "reference-only refresh keeps the purpose topology epoch");
    await chooseDiagramMode("relationships");
    await until(() => [...host.querySelectorAll<HTMLElement>('.node-label[role="button"]')].some(node => !node.hidden && node.textContent?.includes("관계 묶음")), "relationship mode preserves adjacency-based summaries");
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
  const hasSummary = () => [...host.querySelectorAll<HTMLElement>('.node-label[role="button"]')].some(node => !node.hidden && node.textContent?.includes("근접 묶음"));
  try {
    Positions.prototype.install = function (this: Positions, ...args: Parameters<Positions["install"]>) {
      positions = this; return originalInstall.apply(this, args);
    };
    Positions.prototype.packOverview = function (this: Positions, ...args: Parameters<Positions["packOverview"]>) {
      const packed = originalPack.apply(this, args);
      if (packed) overviewPacks++;
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
    await until(() => !!host.querySelector("canvas") && !!positions && !positions.layoutMoving && !hasSummary() && performance.now() - openedAt > 650, "initial view shows ungrouped stars");
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
    return { firstProximityCollapse: true, documents: 12, markers: 1, zoomPacks: overviewPacks - initialPacks, coordinatesPreserved: true };
  } finally {
    root.unmount(); Positions.prototype.install = originalInstall; Positions.prototype.packOverview = originalPack;
    window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
  }
}

async function checkSummaryOverview() {
  const originalFetch = window.fetch, originalUrl = window.location.href, root = createRoot(host);
  const originalPack = Positions.prototype.packOverview;
  let positions: Positions | undefined, overviewPacks = 0;
  const counts = [22, 40, 75, 120], nodes: GraphNode[] = [], links: Snapshot["links"] = [];
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
  const labels = () => [...host.querySelectorAll<HTMLElement>('.node-label[role="button"]')].filter(node => !node.hidden);
  const summary = (count: number) => labels().find(node => node.textContent?.includes(`관계 묶음 · ${count}개`));
  try {
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
    await until(() => counts.every(count => !!summary(count)) && labels().some(node => node.textContent?.includes("근접 묶음")), "four differently sized relation summaries and proximity cohorts remain readable");
    const initialCalls = graphCalls;
    const placements = () => counts.map(count => {
      const node = summary(count);
      return node ? `${count}:${Math.round(parseFloat(node.style.left) * 10)}:${Math.round(parseFloat(node.style.top) * 10)}` : `${count}:hidden`;
    }).join("|");
    let lastPlacement = placements(), stableSince = performance.now();
    await until(() => {
      const current = placements();
      if (current !== lastPlacement) { lastPlacement = current; stableSince = performance.now(); }
      return !!positions && !positions.layoutMoving && counts.every(count => !!summary(count)) && performance.now() - stableSince > 350;
    }, "initial compact overview settles");
    const coordinates = () => JSON.stringify(nodes.map(node => positions!.layoutTarget(node.id)));
    const worldBeforeZoom = coordinates(), packsBeforeZoom = overviewPacks;
    const beforeZoom = lastPlacement, canvas = host.querySelector("canvas")!;
    const rect = canvas.getBoundingClientRect();
    canvas.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: 100,
      clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }));
    let zoomMoved = false;
    await until(() => {
      const current = placements();
      if (current !== beforeZoom) zoomMoved = true;
      if (current !== lastPlacement) { lastPlacement = current; stableSince = performance.now(); }
      return zoomMoved && counts.every(count => !!summary(count)) && performance.now() - stableSince > 300;
    }, "zoom-out settles with all differently sized summaries readable");
    assert(coordinates() === worldBeforeZoom && overviewPacks === packsBeforeZoom, "Wheel zoom changes the camera without repacking stars");
    for (const deltaY of [-1200, -1200, 1200, 1200, -100]) {
      const started = performance.now();
      canvas.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY,
        clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }));
      await until(() => performance.now() - started > 650 && !positions!.layoutMoving, "camera zoom settles across proximity detail bands");
      assert(coordinates() === worldBeforeZoom && overviewPacks === packsBeforeZoom, "Crossing proximity detail bands preserves every world coordinate");
    }
    await until(() => counts.every(count => !!summary(count)), "return zoom keeps all relation summaries available");
    const zoomPacks = overviewPacks - packsBeforeZoom;
    const background = () => JSON.stringify(nodes.filter(node => !node.id.startsWith("group-120-")).map(node => positions!.layoutTarget(node.id)));
    const backgroundBeforeExpansion = background();
    const assertLocalEdges = () => {
      for (const link of links.filter(link => link.source === "group-120-0")) {
        const a = positions!.layoutTarget(link.source)!, b = positions!.layoutTarget(link.target)!;
        assert(Math.hypot(a.x - b.x, a.y - b.y) < 200, "A settled core page cannot eject a star beyond its local neighborhood");
      }
    };
    summary(120)!.click();
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes("관계 묶음 · 120개 · 1/10") === true, "large summary opens its compact page");
    await until(() => !positions!.layoutMoving, "local core expansion settles");
    assertLocalEdges();
    assert(background() === backgroundBeforeExpansion, "Opening a core leaves every unrelated star in place");
    button("다음", host.querySelector(".graph-core-actions")!).click();
    await until(() => host.querySelector(".graph-core-actions")?.textContent?.includes("2/10") === true, "large summary pages");
    await until(() => !positions!.layoutMoving, "local core page settles");
    assertLocalEdges();
    assert(background() === backgroundBeforeExpansion, "Changing the core page leaves unrelated stars in place");
    const proximity = labels().find(node => node.textContent?.includes("근접 묶음"));
    assert(proximity, "a proximity summary is visible alongside the open relation core");
    proximity.click();
    await until(() => host.querySelector(".graph-core-close")?.textContent?.startsWith("근접 묶음") === true && !positions!.layoutMoving,
      "a spatial reveal settles while its relation core stays open");
    assertLocalEdges();
    assert(background() === backgroundBeforeExpansion, "A spatial reveal preserves the unrelated world coordinates");
    host.querySelector<HTMLButtonElement>(".graph-core-close")!.click();
    await until(() => !!host.querySelector(".graph-core-actions") && !positions!.layoutMoving, "spatial close restores the open relation page");
    assertLocalEdges();
    button("묶음 접기").click();
    const closedAt = performance.now();
    await until(() => !positions!.layoutMoving && performance.now() - closedAt > 1000 && counts.every(count => !!summary(count)), "closing restores all overview summaries");
    assert(background() === backgroundBeforeExpansion && overviewPacks === packsBeforeZoom, "Closing restores the same overview without a new whole-map pack");
    assert(graphCalls === initialCalls, "overview packing and expansion reuse one response");
    details("보기 설정").summary.click(); button("새로고침").click();
    await until(() => graphCalls === initialCalls + 1 && host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false" && counts.every(count => !!summary(count)), "refresh retains all overview summaries");
    return { summaryOverview: true, documents: nodes.length, relationCounts: counts, proximityCohorts: true, zoomPreservesWorldCoordinates: true, zoomPacks, localExpansion: true, spatialRevealWithOpenCore: true, closingPreservesOverview: true, refresh: true };
  } finally {
    root.unmount(); Positions.prototype.packOverview = originalPack; window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
  }
}

async function checkSummaryContentRefresh() {
  const originalFetch = window.fetch, originalPack = Positions.prototype.packOverview;
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
  let positions: Positions | undefined;
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  const settled = () => !!positions && !positions.layoutMoving && performance.now() - Math.max(lastPack, refreshedAt) > 350;
  const refresh = async () => {
    const before = graphCalls;
    button("새로고침").click();
    await until(() => graphCalls === before + 1 && host.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "same-topology refresh completes");
    refreshedAt = performance.now();
  };
  try {
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
    const beforeGrowth = overviewPacks;
    responseSnapshot = { ...snapshot, nodes: nodes.map(node => ({ ...node, revision: "content-only-update" })) };
    await refresh();
    await until(() => overviewPacks > beforeGrowth, "growing change rings trigger clearance without zoom or topology changes");
    await until(settled, "larger change rings finish packing");
    assert(positions!.structureEpoch === structureEpoch, "content changes preserve the topology epoch");
    const afterGrowth = overviewPacks;
    await refresh();
    await until(settled, "unchanged response clears change rings");
    assert(overviewPacks === afterGrowth, "adequate clearance does not repack after rings shrink");
    return { summaryContentRefresh: true, relationCounts: counts, growingRings: true, shrinkingRingsPreserveLayout: true };
  } finally {
    root.unmount(); Positions.prototype.packOverview = originalPack; window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
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
  const originalRender = Scene.prototype.onBeforeRender, originalInstall = Positions.prototype.install;
  const originalControlsUpdate = OrbitControls.prototype.update;
  let cameraDistance = 0;
  const nodes: GraphNode[] = Array.from({ length: 5 }, (_, index) => ({ id: `motion-${index}`, scope: "personal", kind: "document", label: `합성 모션 자료 ${index}`, status: "ok", present: true, current: true }));
  const links: Snapshot["links"] = [1, 2].map(index => ({ source: nodes[0].id, target: nodes[index].id, kind: "related", current: true }));
  const snapshot: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links,
    matched: nodes.length, totals: { documents: nodes.length, memories: 0, markers: 0, links: links.length },
    returned: { knowledge: nodes.length, markers: 0, links: links.length }, omitted: { nodes: 0, links: 0 },
    eligible: { nodes: nodes.length, links: links.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  let model: Parameters<Positions["install"]>[0] | undefined, positions: Positions | undefined, camera: Camera | undefined;
  const sampled = new Map<string, { rotation: number; shimmer: number; wobble: number; z: number }>(), wrapped = new WeakSet<Mesh>(), worldPosition = new Vector3();
  const json = (value: unknown) => new Response(JSON.stringify(value), { headers: { "content-type": "application/json" } });
  try {
    OrbitControls.prototype.update = function (this: OrbitControls, ...args: Parameters<OrbitControls["update"]>) {
      const result = originalControlsUpdate.apply(this, args);
      cameraDistance = this.object.position.distanceTo(this.target);
      return result;
    };
    Positions.prototype.install = function (this: Positions, ...args: Parameters<Positions["install"]>) {
      positions = this; model = args[0]; return originalInstall.apply(this, args);
    };
    Scene.prototype.onBeforeRender = function (renderer, scene, frameCamera, geometry, material, group) {
      camera = frameCamera;
      scene.traverse(object => {
        if (!(object instanceof Mesh) || !(object.material instanceof ShaderMaterial) || !object.material.uniforms.uRotation || wrapped.has(object)) return;
        wrapped.add(object);
        const draw = object.onBeforeRender;
        object.onBeforeRender = function (...args) {
          draw.apply(this, args);
          const uniforms = (object.material as ShaderMaterial).uniforms, id = object.parent?.userData.nodeId;
          if (id) sampled.set(id, { rotation: uniforms.uRotation.value, shimmer: uniforms.uShimmer.value, wobble: uniforms.uWobble.value.length(), z: object.getWorldPosition(worldPosition).z });
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
    root.render(<App />);
    await until(() => sampled.size === nodes.length && !!positions && !positions.layoutMoving && !!camera, "actual Graph renders each luminous star");
    assert(new Set([...sampled.values()].map(star => star.z.toFixed(6))).size > 1, "the rendered stars retain depth instead of flattening their world positions");
    const initial = new Map(sampled);
    await until(() => [...sampled].some(([id, value]) => Math.abs(value.rotation - initial.get(id)!.rotation) > .01 && Math.abs(value.shimmer - initial.get(id)!.shimmer) > .001), "the mounted renderer advances stellar surface and shimmer motion");
    const node = model!.nodes.find(node => sampled.has(node.id))!, canvas = host.querySelector("canvas")!, rect = canvas.getBoundingClientRect();
    const at = new Vector3(node.x, node.y, node.z).project(camera!);
    canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse", clientX: rect.left + (at.x + 1) * rect.width / 2, clientY: rect.top + (1 - at.y) * rect.height / 2 }));
    await until(() => (sampled.get(node.id)?.wobble ?? 0) > .01, "pointer proximity drives wobble in the actual star draw");
    canvas.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, relatedTarget: host }));
    await until(() => sampled.get(node.id)?.wobble === 0, "pointer exit restores the star without leaving a wobble behind");
    details("보기 설정").summary.click();
    const before = camera!.position.clone();
    button("지도 천천히 회전").click();
    await until(() => camera!.position.distanceTo(before) > .01, "slow 3D camera rotation remains available with pointer rotation disabled");
    button("지도 회전 멈춤").click();
    details("보기 설정").summary.click();
    let restingDirection = camera!.getWorldDirection(new Vector3()), restingSamples = 0;
    await until(() => {
      const current = camera!.getWorldDirection(new Vector3());
      const resting = current.distanceTo(restingDirection) < .000001;
      restingDirection = current;
      restingSamples = resting ? restingSamples + 1 : 0;
      return restingSamples >= 4;
    }, "the existing damped rotation settles before isolated zoom observations");
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
    button("전체 맞춤").click();
    await until(() => camera!.getWorldDirection(new Vector3()).distanceTo(new Vector3(0, 0, -1)) < .00001, "the existing full fit resets the camera orientation");
    assert([...sampled.values()].some(star => star.z !== 0), "minimap navigation preserves stellar depth");
    return { renderedMotion: true, stars: nodes.length, depth: true, surfaceAndShimmer: true, hoverWobble: true, slow3DRotation: true, minimapControls: true, zoomRoundTrip: true, centerPreservesZoom: true, fullFit: true };
  } finally {
    root.unmount(); Scene.prototype.onBeforeRender = originalRender; Positions.prototype.install = originalInstall;
    OrbitControls.prototype.update = originalControlsUpdate;
    window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
  }
}

async function run() {
  const results: unknown[] = [];
  try {
    assert(import.meta.env.DEV, "Run this fixture through the development server");
    results.push(checkSummaryGlyphMotion());
    for (const size of [1, 20]) results.push(await check(size));
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
