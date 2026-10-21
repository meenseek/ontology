import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { editorContent, editorDraft, editorNewlines, failure, OriginalText, contextUrl, originalFilename } from "./Original";

test("browser editing retains original newline convention", () => {
  const crlf = "title\r\nbody\r\n";
  assert.equal(editorNewlines(crlf), "crlf");
  assert.equal(editorContent(editorDraft(crlf).replace("body", "changed"), "crlf"), "title\r\nchanged\r\n");
  assert.equal(editorNewlines("title\nbody\n"), "lf");
  assert.equal(editorNewlines("title\r\nbody\n"), null);
  assert.equal(editorNewlines("title\rbody"), null);
});

test("context URLs retain exact scope, Unicode and query delimiters without changing endpoint", () => {
  const scope = "work/alpha", path = "attachments/한글 &?#%=+\".html";
  for (const kind of ["read", "download", "preview"] as const) {
    const url = new URL(contextUrl(kind, scope, path), "http://127.0.0.1:47831");
    assert.equal(url.origin, "http://127.0.0.1:47831");
    assert.equal(url.pathname, `/api/context/${kind}`);
    assert.equal(url.searchParams.get("scope"), scope);
    assert.equal(url.searchParams.get("path"), path);
    assert.equal(url.searchParams.size, 2);
    assert.equal(url.hash, "");
  }
  const url = new URL(contextUrl("search", scope, "한글 & 원문?", path), "http://local.invalid");
  assert.equal(url.searchParams.get("q"), "한글 & 원문?");
  assert.equal(url.searchParams.get("after"), path);
  assert.equal(url.searchParams.get("limit"), "20");
  assert.equal(new URL(contextUrl("search", "personal", ""), url).searchParams.has("after"), false);
  assert.equal(originalFilename(path), "한글 &?#%=+\".html");
});

test("original text is escaped, never executed or normalized into markup", () => {
  const content = "\uFEFF---\r\ntitle: 원문\r\n---\r\n<script>alert(1)</script><img src=x onerror=alert(1)> & \" ' \n";
  const output = renderToStaticMarkup(<OriginalText content={content} />);
  assert(!output.includes("<script>"));
  assert(!output.includes("<img"));
  assert(output.includes("&lt;script&gt;alert(1)&lt;/script&gt;"));
  assert(output.includes("\uFEFF---\r\ntitle: 원문\r\n---\r\n"));
  assert(output.includes("&amp; &quot; &#x27;"));
  assert.equal(Buffer.from(JSON.parse(JSON.stringify({ content })).content, "utf8").compare(Buffer.from(content, "utf8")), 0);
  assert(renderToStaticMarkup(<OriginalText content="" />).includes('aria-label="원본 텍스트"'));
});

test("all context read helpers display the pending message for HTTP 409", () => {
  const pending = "자료 반영 또는 복구가 진행 중입니다. 완료 후 다시 시도해 주세요.";
  for (const reading of [false, true]) assert.equal(failure({ status: 409, message: "private path" }, reading), pending);
  assert.equal(failure({ status: 404 }), "자료가 없거나 이 범위에서 열 수 없습니다.");
  assert.equal(failure({ status: 400 }, true), "UTF-8 텍스트로 열 수 없는 자료입니다. 원본 다운로드를 이용해 주세요.");
  assert(!failure({ status: 500 }).includes("private path"));
});

// jsdom is an existing, pinned development dependency; it does not render a map.
// @ts-expect-error jsdom intentionally has no additional @types dependency here.
import { JSDOM, VirtualConsole } from "jsdom";
import type { GraphNode, Snapshot } from "./graph";

for (const width of [1440, 390]) test(`original draft survives missing graph and recovers conflict at ${width}px`, { timeout: 15000 }, async () => {
  const errors: string[] = [], saved = new Map<string, PropertyDescriptor | undefined>();
  const virtualConsole = new VirtualConsole();
  virtualConsole.on("jsdomError", (error: Error) => errors.push(error.message));
  const dom = new JSDOM('<div id="root"></div>', { virtualConsole, url: "http://localhost/?scope=personal&focus=e_1111111111111111111111111111111111111111111111111111111111111111", pretendToBeVisual: true });
  const w = dom.window as Window & typeof globalThis;
  const bind = (name: string, value: unknown) => { saved.set(name, Object.getOwnPropertyDescriptor(globalThis, name)); Object.defineProperty(globalThis, name, { configurable: true, writable: true, value }); };
  for (const name of ["window", "document", "navigator", "location", "history", "HTMLElement", "HTMLInputElement", "HTMLTextAreaElement", "HTMLSelectElement", "Event", "DOMParser", "Node", "MutationObserver"]) bind(name, name === "window" ? w : Reflect.get(w, name));
  bind("getComputedStyle", w.getComputedStyle.bind(w)); bind("innerWidth", width); bind("innerHeight", 844);
  bind("requestAnimationFrame", w.requestAnimationFrame.bind(w)); bind("cancelAnimationFrame", w.cancelAnimationFrame.bind(w));
  w.matchMedia = query => ({ media: query, matches: query === "(max-width: 900px)" && width <= 900, addEventListener() {}, removeEventListener() {} } as unknown as MediaQueryList);
  w.HTMLElement.prototype.scrollIntoView = () => {};
  w.HTMLElement.prototype.scrollTo = () => {};
  let permitDiscard = false, confirmations = 0, visible = true;
  w.confirm = () => { confirmations++; return permitDiscard; };
  Object.defineProperty(w.document, "visibilityState", { configurable: true, get: () => visible ? "visible" : "hidden" });
  let content = "# 합성 원문\n\n기존 내용", revision = 1, present = true, sourcePath = "synthetic/original.md", graphCalls = 0, writes = 0, readFailure = false;
  const draft = "# 합성 원문\n\n보존할 초안", digest = () => `revision-${revision}`;
  let currentId = "e_1111111111111111111111111111111111111111111111111111111111111111";
  const node = (): GraphNode => ({ id: currentId, scope: "personal", kind: "document", label: "합성 원문", context_scope: "personal", context_path: sourcePath, source_kind: "original", status: "ok", present: true, current: true });
  const snapshot = (): Snapshot => ({ scope: "personal", query: "", focus: { id: "e_1111111111111111111111111111111111111111111111111111111111111111", found: present }, nodes: present ? [node()] : [], links: [], matched: present ? 1 : 0, totals: { documents: +present, memories: 0, markers: 0, links: 0 }, returned: { knowledge: +present, markers: 0, links: 0 }, omitted: { nodes: 0, links: 0 }, eligible: { nodes: +present, links: 0 }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false });
  const original = () => ({ content, title: "합성 원문", metadata: { scope: "personal", path: "synthetic/original.md", source_path: "synthetic", revision, content_digest: digest(), source_digest: null, origin_kind: "native", byte_len: new TextEncoder().encode(content).length } });
  const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
  let release!: () => void;
  const first = new Promise<void>(resolve => { release = resolve; });
  bind("fetch", async (input: RequestInfo | URL, options?: RequestInit) => {
    const url = new URL(String(input), w.location.href);
    assert.equal(url.origin, "http://localhost", "synthetic requests never leave this test");
    if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
    if (url.pathname === "/api/graph") { if (++graphCalls === 1) await first; return json(snapshot()); }
    if (url.pathname === "/api/context/history") return json({ items: [], next_before: null });
    if (url.pathname === "/api/context/read") {
      assert.equal(url.searchParams.get("path"), "synthetic/original.md", "dirty editor retains its exact source identity");
      return readFailure ? json({}, 409) : json(original());
    }
    if (url.pathname === "/api/context/edit") {
      assert.equal(options?.method, "POST"); writes++;
      const body = JSON.parse(String(options?.body));
      if (body.expected_revision !== revision || body.expected_digest !== digest()) return json({}, 409);
      content = body.content; revision++; return json({ revision, content_digest: digest(), changed: true });
    }
    assert.fail(`Forbidden synthetic request: ${url.pathname}`);
  });
  const { createRoot } = await import("react-dom/client");
  const { default: App } = await import("./App");
  const root = createRoot(w.document.querySelector("#root")!);
  const until = async (predicate: () => boolean, message: string) => { const end = Date.now() + 3000; while (!predicate()) { assert.ok(Date.now() < end, message); await new Promise(resolve => setTimeout(resolve, 10)); } await new Promise(resolve => setTimeout(resolve, 20)); };
  const button = (name: string) => { const button = [...w.document.querySelectorAll("button")].find(b => b.textContent === name || b.getAttribute("aria-label") === name); assert.ok(button, name); return button; };
  const textarea = () => w.document.querySelector<HTMLTextAreaElement>("#original-draft");
  const enter = (value: string) => { Object.getOwnPropertyDescriptor(w.HTMLTextAreaElement.prototype, "value")!.set!.call(textarea(), value); textarea()!.dispatchEvent(new w.Event("input", { bubbles: true })); };
  const refreshGraph = async () => { const before = graphCalls; visible = false; w.document.dispatchEvent(new w.Event("visibilitychange")); await new Promise(resolve => setTimeout(resolve, 30)); visible = true; w.document.dispatchEvent(new w.Event("visibilitychange")); await until(() => graphCalls > before && w.document.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "visibility refresh finishes"); };
  try {
    root.render(<App />);
    await until(() => !!w.document.querySelector(".map-actions"), "App mounts");
    button("목록 보기").click(); release();
    await until(() => !!w.document.querySelector(".original-view-switch"), "source loads");
    button("편집").click(); await until(() => !!textarea(), "editor opens"); enter(draft);
    await until(() => !button("저장").disabled, "draft is dirty");
    present = false;
    for (let i = 0; i < 2; i++) { await refreshGraph(); assert.equal(textarea()?.value, draft, "repeated missing graph never unmounts the draft"); assert.match(w.document.querySelector(".management-panel")!.textContent!, /초안은 유지했습니다/); }
    assert.equal(w.location.search.includes("focus=e_1111111111111111111111111111111111111111111111111111111111111111"), true);
    assert.ok(!w.document.querySelector(".management-panel")!.textContent!.includes("현재 필터 밖"), "a missing source never offers an ineffective filter reset");
    present = true; sourcePath = "synthetic/moved.md"; await refreshGraph();
    assert.equal(textarea()?.value, draft, "a returned node with a changed path cannot reset the editor");
    sourcePath = "synthetic/original.md"; await refreshGraph(); assert.equal(textarea()?.value, draft);
    revision = 2; content = "# 합성 원문\n\n다른 사람이 저장한 내용"; readFailure = true;
    button("저장").click(); await until(() => !!w.document.querySelector(".original-conflict") && !button("변경 취소").disabled, "409 recovery is available");
    assert.equal(textarea()?.value, draft); assert.equal(button("저장").disabled, true);
    assert.ok(w.document.querySelector('.original-conflict[role="alert"].error'), "a blocked save retains its error alert while offering recovery");
    w.document.dispatchEvent(new w.KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true }));
    await new Promise(resolve => setTimeout(resolve, 30)); assert.equal(writes, 1, "keyboard retry cannot send stale CAS");
    enter("# 합성 원문\n\n기존 내용"); present = false; await refreshGraph();
    assert.equal(textarea()?.value, "# 합성 원문\n\n기존 내용", "an unresolved conflict remains protected even if draft matches the old baseline");
    present = true; enter(draft); await refreshGraph();
    readFailure = false; button("최신 원문 다시 확인").click();
    await until(() => w.document.querySelector(".original-conflict")?.textContent?.includes("버전 2") === true, "latest read can be retried without losing draft");
    button("읽기").click(); await until(() => !textarea(), "latest reading mode");
    assert.match(w.document.querySelector(".original-detail")!.textContent!, /다른 사람이 저장한 내용/);
    button("편집").click(); await until(() => !!textarea(), "return to draft"); assert.equal(textarea()!.value, draft);
    enter(`${draft}\n\n다른 사람이 저장한 내용`); await new Promise(resolve => setTimeout(resolve, 30));
    button("비교 완료 · 편집 계속").click(); await until(() => !button("저장").disabled, "explicit comparison updates baseline");
    button("저장").click(); await until(() => !textarea() && w.document.querySelector(".original-detail")?.textContent?.includes("저장하고 다시 확인") === true, "new CAS saves and readback verifies");
    assert.equal(writes, 2); assert.equal(content, `${draft}\n\n다른 사람이 저장한 내용`); assert.equal(confirmations, 0);
    button("편집").click(); await until(() => !!textarea(), "editor opens again"); enter("남길 초안"); await until(() => !button("저장").disabled, "new dirty state");
    present = false; await refreshGraph();
    button("관리 패널 닫기").click(); await new Promise(resolve => setTimeout(resolve, 30)); assert.equal(textarea()?.value, "남길 초안", "declining navigation preserves draft");
    permitDiscard = true; button("변경 취소").click(); await until(() => !w.document.querySelector(".management-panel"), "explicit discard releases missing selection");
    assert.equal(w.location.search.includes("focus="), false); assert.equal(confirmations, 2);
    await until(() => w.document.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "missing selection cleanup settles");
    currentId = "e_" + "2".repeat(64); present = true;
    w.history.pushState(null, "", `/?scope=personal&focus=${currentId}`); w.dispatchEvent(new w.PopStateEvent("popstate"));
    await until(() => !!w.document.querySelector(".original-view-switch") && w.document.querySelector(".galaxy")?.getAttribute("aria-busy") === "false", "a focus absent from the previous graph opens from its own response");
    assert.equal(new URLSearchParams(w.location.search).get("focus"), currentId);
    assert.ok(w.document.querySelector(".management-panel"), "old graph cannot dismiss a newly requested selection");
    assert.ok(!w.document.querySelector(".management-panel")!.textContent!.includes("초점을 지정한 자료가 없거나"), "new navigation clears the previous missing-source notice");
    assert.deepEqual(errors, []);
  } finally {
    root.unmount(); await new Promise(resolve => setTimeout(resolve, 30)); w.close();
    for (const [name, descriptor] of saved) { if (descriptor) Object.defineProperty(globalThis, name, descriptor); else Reflect.deleteProperty(globalThis, name); }
  }
});
