import { createRoot } from "react-dom/client";
import App from "./App";
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
  const element = [...parent.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent?.trim() === text);
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
const id = (index: number) => `m_00000000-0000-4000-8000-${index.toString().padStart(12, "0")}`;
const subjectId = "p_00000000-0000-4000-8000-000000000001";
function item(index: number, scope: Scope): Item {
  return { id: id(index), scope, revision: 1, kind: "fact", title: `${scope} 합성 기록 ${index}`, body: `${scope} 합성 본문 ${index}`, subject_id: null, subject_name: null, effective_from: null, effective_until: null, evidence: [], status: "accepted", origin: "user", support: "user-recorded", updated_at: "2026-09-11T00:00:00Z" };
}
function graph(items: Item[], scope: Scope, query: string, focus: string | null, subjectExists: boolean): Snapshot {
  const nodes: GraphNode[] = items.map(value => ({ id: value.id, scope, kind: "memory", label: value.title, revision: String(value.revision), status: "accepted", temporal: "current", supported: true }));
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

async function run() {
  const results: unknown[] = [];
  try {
    assert(import.meta.env.DEV, "Run this fixture through the development server");
    for (const size of [1, 20]) results.push(await check(size));
    output.textContent = `PASS\n${JSON.stringify({ passed: true, results }, null, 2)}`;
  } catch (error) {
    output.textContent = `FAIL\n${JSON.stringify({ passed: false, error: error instanceof Error ? error.message : String(error), results, active }, null, 2)}`;
  }
}
void run();
