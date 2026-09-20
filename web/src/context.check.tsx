// Interactive acceptance fixture. Building this page does not execute these checks.
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import App from "./App";
import type { ContextItem } from "./Context";
import "./style.css";

type Pending = { url: URL; signal: AbortSignal | null | undefined; resolve: (response: Response) => void };
function requireValue<T>(value: T | null | undefined, reason: string): T { if (value == null) throw new Error(reason); return value; }
function assert(value: unknown, reason: string): asserts value { if (!value) throw new Error(reason); }
const result = requireValue(document.querySelector<HTMLElement>("#context-check-result"), "result element");
const runButton = requireValue(document.querySelector<HTMLButtonElement>("#context-check-run"), "run button");
const host = requireValue(document.querySelector<HTMLElement>("#context-check-app"), "app host");
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
const metadata = (scope: string, number = 0): ContextItem => ({ scope, path: `note-${number.toString().padStart(2, "0")}.md`, source_path: `${scope}/note-${number.toString().padStart(2, "0")}.md`, content_digest: "a".repeat(64), byte_len: 7 });
// Native browser scheduling works in both Vite development and production builds.
async function step(action: () => void | Promise<void>) {
  let completion: void | Promise<void> = undefined;
  flushSync(() => { completion = action(); });
  await completion;
  await new Promise<void>(resolve => window.setTimeout(resolve, 0));
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
}
const tick = () => step(() => {});
async function click(element: HTMLElement) { await step(async () => { element.click(); }); }
function button(text: string, within: ParentNode = host): HTMLButtonElement {
  return requireValue([...within.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent?.trim() === text && !node.closest("[hidden], [inert]")), `visible button: ${text}`);
}
async function chooseScope(value: string) {
  const select = requireValue(host.querySelector<HTMLSelectElement>("#context-scope"), "scope select");
  await step(async () => { select.value = value; select.dispatchEvent(new Event("change", { bubbles: true })); });
}
async function type(element: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = requireValue(Object.getOwnPropertyDescriptor(prototype, "value")?.set, "native controlled input setter");
  await step(async () => { setter.call(element, value); element.dispatchEvent(new Event("input", { bubbles: true })); });
}
async function settle(pending: Pending, value: unknown, status = 200) { await step(async () => { pending.resolve(json(value, status)); }); }

runButton.addEventListener("click", async () => {
  if (runButton.disabled) return;
  runButton.disabled = true; result.dataset.state = "running"; result.textContent = "합성 자료로 실제 App·Context·기록 편집기를 확인하는 중…";
  const originalFetch = window.fetch, originalUrl = window.location.href;
  const root = createRoot(host), calls: URL[] = [], pending: Pending[] = [];
  let size = 1, delaySearch = false, delayRead = false, failRead = false, failScopes = true;
  window.history.replaceState(null, "", "?scope=personal&q=context-fixture");
  window.fetch = async (input, options) => {
    const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
    calls.push(url);
    if (url.pathname === "/api/session") return json({ csrf: "synthetic-token", areas: [] });
    if (url.pathname === "/api/graph") return json({ scope: url.searchParams.get("scope"), query: url.searchParams.get("q"), nodes: [], links: [], totals: { nodes: 0, links: 0 }, truncated: false });
    if (url.pathname === "/api/brain") return json({ items: [], sources: [], next_after: null, total: 0 });
    if (url.pathname === "/api/sync") return json({ enabled: false, running: false, last_completed_at: null, error: null, report: null });
    if (url.pathname === "/api/context/scopes") {
      if (failScopes) { failScopes = false; return json({ error: "synthetic failure" }, 503); }
      return json({ scopes: ["personal", "work/fixture"] });
    }
    if (url.pathname === "/api/context" && delaySearch || url.pathname === "/api/context/read" && delayRead) {
      // Intentionally ignore cancellation: component guards must reject stale resolutions too.
      return new Promise<Response>(resolve => pending.push({ url, signal: options?.signal, resolve }));
    }
    const scope = url.searchParams.get("scope") ?? "personal";
    if (url.pathname === "/api/context") return json({ items: Array.from({ length: Math.min(20, size) }, (_, number) => metadata(scope, number)), next_after: size > 20 ? "note-19.md" : null, limit: 20 });
    if (url.pathname === "/api/context/read") {
      if (failRead) { failRead = false; return json({ error: "synthetic failure" }, 503); }
      return json({ metadata: { ...metadata(scope), source_digest: "b".repeat(64) }, content: "<script>synthetic()</script>\r\n원본 자료" });
    }
    throw new Error(`Unexpected request in synthetic check: ${url.pathname}`);
  };
  const contextCalls = () => calls.filter(url => url.pathname.startsWith("/api/context"));
  const bodyCalls = () => calls.filter(url => url.pathname === "/api/context/read" || url.pathname === "/api/context/download");
  const take = (path: string) => requireValue(pending.find(request => request.url.pathname === path), `delayed ${path}`);
  const remove = (request: Pending) => pending.splice(pending.indexOf(request), 1);
  try {
    await step(async () => { root.render(<App />); }); await tick();
    await click(button("기록 남기기")); await tick();
    const editor = requireValue(host.querySelector<HTMLTextAreaElement>(".management-panel textarea"), "actual record editor textarea");
    await type(editor, "저장하지 않은 기록 초안 · 유지 확인");
    const draft = editor.value;
    const management = requireValue(editor.closest<HTMLElement>(".management-panel"), "record panel");
    // This opener is available inside the record panel on narrow and wide screens.
    await click(button("자료 보관함", management)); await tick();
    assert(contextCalls().length === 1 && bodyCalls().length === 0, "failed discovery made one metadata request and no body request");
    await tick(); assert(contextCalls().length === 1, "no automatic discovery retry");
    await click(button("범위 다시 불러오기")); await tick();
    assert(contextCalls().length === 2 && bodyCalls().length === 0, "one explicit discovery retry");
    assert(host.querySelector<HTMLSelectElement>("#context-scope")?.value === "", "scope must initially remain unselected");
    await chooseScope("personal"); await tick();
    assert(bodyCalls().length === 0, "metadata selection does not prefetch any body");
    failRead = true;
    await click(button("note-00.md7 바이트")); await tick();
    const beforeRetry = bodyCalls().length;
    await tick(); assert(bodyCalls().length === beforeRetry, "read failure never automatically retries");
    await click(button("원문 다시 불러오기")); await tick();
    assert(bodyCalls().length === beforeRetry + 1, "one explicit selected read retry");
    assert(host.querySelector(".context-text")?.textContent === "<script>synthetic()</script>\r\n원본 자료", "original text is preserved");
    assert(!host.querySelector(".context-text script"), "source content cannot execute as HTML");
    delaySearch = true;
    await chooseScope("work/fixture");
    const oldList = take("/api/context"); remove(oldList);
    await chooseScope("personal");
    assert(oldList.signal?.aborted, "scope switch aborts old list");
    const freshList = take("/api/context"); remove(freshList);
    await settle(freshList, { items: [metadata("personal")], next_after: null, limit: 20 });
    await settle(oldList, { items: [{ ...metadata("work/fixture"), path: "stale-list.md" }], next_after: null, limit: 20 });
    assert(!host.textContent?.includes("stale-list.md"), "late previous-scope list is ignored");
    delaySearch = false; delayRead = true;
    await click(button("note-00.md7 바이트"));
    const oldRead = take("/api/context/read"); remove(oldRead);
    await chooseScope("work/fixture"); await tick();
    assert(oldRead.signal?.aborted, "scope switch aborts selected body");
    assert(!host.querySelector(".context-text"), "scope switch immediately clears old content");
    await settle(oldRead, { metadata: { ...metadata("personal"), source_digest: "b".repeat(64) }, content: "STALE BODY" });
    assert(!host.textContent?.includes("STALE BODY"), "late previous-scope body is ignored");
    await click(button("note-00.md7 바이트"));
    const closingRead = take("/api/context/read"); remove(closingRead);
    await click(requireValue(host.querySelector<HTMLElement>('[aria-label="자료 보관함 닫기"]'), "library close"));
    assert(closingRead.signal?.aborted, "close cancels pending body");
    await settle(closingRead, { metadata: { ...metadata("work/fixture"), source_digest: "b".repeat(64) }, content: "CLOSED BODY" });
    assert(!host.querySelector(".context-panel") && !host.textContent?.includes("CLOSED BODY"), "close prevents stale content updates");
    assert(editor.isConnected && editor.value === draft && !management.hidden, "actual record editor and its unsaved draft survive open/close");
    delayRead = false;
    for (const count of [0, 1, 25]) {
      size = count; const before = contextCalls().length, beforeBodies = bodyCalls().length;
      await click(button("자료 보관함", management)); await tick();
      assert(host.querySelector<HTMLSelectElement>("#context-scope")?.value === "", "reopening requires a fresh explicit scope");
      await chooseScope("personal"); await tick();
      assert(host.querySelectorAll(".context-list li").length === Math.min(count, 20), "metadata page remains bounded for zero/one/many");
      assert(contextCalls().length - before === 2 && bodyCalls().length === beforeBodies, "one discovery plus one list request, no body fanout");
      await click(requireValue(host.querySelector<HTMLElement>('[aria-label="자료 보관함 닫기"]'), "library close"));
    }
    assert(editor.isConnected && editor.value === draft, "repeated library visits preserve the original draft");
    result.dataset.state = "passed"; result.textContent = "통과: 실제 컴포넌트 범위 선택·호출 상한·재시도·지연 응답 차단·닫기 취소·안전한 원문·기록 초안 유지";
  } catch (error) {
    result.dataset.state = "failed"; result.textContent = `실패: ${error instanceof Error ? error.message : "unknown"}`;
  } finally {
    await step(async () => { root.unmount(); });
    window.fetch = originalFetch; window.history.replaceState(null, "", originalUrl);
    runButton.disabled = false;
  }
});
