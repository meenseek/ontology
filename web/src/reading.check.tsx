import { useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import Memory from "./Memory";
import type { Item } from "./Memory";
import Documents from "./Documents";
import OriginalDetail from "./OriginalDetail";
import DocumentPreview from "./DocumentPreview";
import { DocumentPurpose, SubjectPurpose } from "./Purpose";
import type { Request, Scope } from "./graph";
import "./style.css";

// Development-only: exercise the shipped components and effects, not copies of their logic.
const output = document.querySelector<HTMLPreElement>("#result")!;
const host = document.querySelector<HTMLDivElement>("#fixture")!;
const encoder = new TextEncoder();
const memoryId = "fixture-memory", documentId = "fixture-document";
const quiet = () => undefined;
let networkAttempts = 0;
globalThis.fetch = async () => { networkAttempts++; throw new Error("실제 fetch는 이 합성 fixture에서 차단됩니다."); };

type Outcome = "pending" | "resolved" | "failed" | "aborted";
type Call = { op: string; state: Outcome; requestBytes: number; responseBytes: number; signal: boolean };
type Mode = "fail" | "hold";
type Result = { name: string; passed: boolean; checks: number; reason?: string; counts: Record<string, number>; responseBytes: number; requestBytes: number; maxPending: number; calls: Call[] };
const results: Result[] = [];
let checks = 0;
function assert(value: unknown, reason: string): asserts value { checks++; if (!value) throw new Error(reason); }
const delay = (ms = 0) => new Promise<void>(resolve => window.setTimeout(resolve, ms));
async function waitFor(predicate: () => boolean, reason: string) {
  const deadline = performance.now() + 4000;
  while (!predicate()) { if (performance.now() > deadline) throw new Error(`대기 시간 초과: ${reason}`); await delay(10); }
  await delay();
}
async function settle() { await delay(25); await delay(25); }

function memory(size: number): Item {
  return {
    id: memoryId, scope: "personal", revision: 1, kind: "fact", status: "accepted", origin: "user",
    title: "합성 기록", body: `기록 본문${" · 추가 문장".repeat(size)}`, subject_id: null, subject_name: null,
    effective_from: null, effective_until: null, updated_at: "2026-01-01T00:00:00Z", support: "supported",
    evidence: Array.from({ length: Math.min(size, 10) }, (_, index) => ({
      entity_id: `fixture-source-${index}`, source_revision: "a".repeat(40), content_digest: "b".repeat(64),
      generation: 1, kind: "git", repository: "/synthetic/repository", path: `notes/source-${index}.md`, current: true,
    })),
  };
}
function documentData(size: number) {
  return {
    id: documentId, scope: "personal", revision: 1, areas: [], topics: ["합성 태그"],
    source: { kind: "git", repository: "/synthetic/repository", path: "notes/fixture.md", status: "ok", last_success_at: "2026-01-01T00:00:00Z" },
    projection: { content: `# 합성 문서\n\n본문${" · 추가 문장".repeat(size)}`, present: true },
    related: Array.from({ length: size }, (_, index) => ({ id: `fixture-related-${index}`, path: `notes/related-${index}.md` })),
    history: Array.from({ length: size }, (_, index) => ({ id: index + 1, revision: index + 1, kind: "classification", previous: { areas: [], topics: [] }, confirmed: { areas: [], topics: ["합성 태그"] }, confirmed_at: "2026-01-01T00:00:00Z" })),
  };
}

class Stub {
  calls: Call[] = [];
  maxPending = 0;
  changed: (Item | null)[] = [];
  documentChanges = 0;
  item: Item;
  reply?: (op: string, body: Record<string, unknown>) => unknown;
  private modes = new Map<string, Mode[]>();
  private releases: (() => void)[] = [];
  constructor(readonly size: number) { this.item = memory(size); }
  next(op: string, ...modes: Mode[]) { this.modes.set(op, modes); }
  count(op: string) { return this.calls.filter(call => call.op === op).length; }
  release() { this.releases.splice(0).forEach(release => release()); }
  private response(op: string, body: Record<string, unknown>): unknown {
    const custom = this.reply?.(op, body);
    if (custom !== undefined) return custom;
    if (op === "read") return this.item;
    if (op === "evidence-read") return { available: true, content: `# 당시 원문 ${body.revision}\n\n보존한 내용`, evidence: this.item.evidence.find(e => e.entity_id === body.entity_id) };
    if (op === "document-read") return documentData(this.size);
    if (op === "subjects") return { items: Array.from({ length: this.size }, (_, index) => ({ id: `fixture-subject-${index}`, name: `합성 묶음 ${index}` })), next_after: null };
    if (op === "history") {
      const start = typeof body.before_revision === "number" ? this.size - body.before_revision + 1 : 0;
      const end = Math.min(start + 10, this.size);
      return { items: Array.from({ length: end - start }, (_, index) => ({ revision: this.size - start - index, status: "accepted", subject_id: null, document: { ...memory(this.size), title: `과거 제목 ${start + index}`, body: `보존할 과거 본문 ${start + index}` }, changed_at: "2026-01-01T00:00:00Z" })), next_before_revision: end < this.size ? this.size - end + 1 : null };
    }
    if (op === "correct" || op === "remember" || op === "propose") {
      const input = body.memory as Partial<Item>;
      this.item = { ...this.item, ...input, evidence: this.item.evidence, revision: this.item.revision + 1, status: op === "propose" ? "proposed" : "accepted" };
      return this.item;
    }
    if (op === "grouping-set") {
      const subjectId = body.subject_id as string | null;
      const mode = body.mode as "manual" | "auto" | "off";
      this.item = { ...this.item, subject_id: subjectId, subject_name: subjectId === null ? null : "합성 묶음 0", revision: this.item.revision + 1,
        grouping: { mode, state: mode === "manual" ? "manual" : mode === "auto" ? "pending" : "off", suggestions: {}, reason: null } };
      return this.item;
    }
    throw new Error(`예상하지 않은 합성 요청: ${op}`);
  }
  request: Request = <T,>(url: string, options?: RequestInit): Promise<T> => {
    const body = typeof options?.body === "string" ? JSON.parse(options.body) as Record<string, unknown> : {};
    const context = url.startsWith("/api/context/") ? new URL(url, "http://synthetic.invalid") : null;
    if (context) context.searchParams.forEach((value, key) => { body[key] = value; });
    const op = context ? `context-${context.pathname.split("/").at(-1)}` : url === "/api/brain" ? String(body.op) : url === `/api/records/${documentId}?scope=personal` && !options?.method ? "document-read" : `unexpected:${url}`;
    const call: Call = { op, state: "pending", requestBytes: encoder.encode(url + (typeof options?.body === "string" ? options.body : "")).length, responseBytes: 0, signal: !!options?.signal };
    this.calls.push(call);
    this.maxPending = Math.max(this.maxPending, this.calls.filter(item => item.state === "pending").length);
    const mode = this.modes.get(op)?.shift();
    return new Promise<T>((resolve, reject) => {
      const signal = options?.signal;
      let timer: number | undefined;
      const finish = (state: Outcome) => { call.state = state; signal?.removeEventListener("abort", abort); if (timer !== undefined) clearTimeout(timer); };
      const abort = () => { if (call.state !== "pending") return; finish("aborted"); reject(new DOMException("합성 요청 취소", "AbortError")); };
      const deliver = () => {
        if (call.state !== "pending") return;
        if (signal?.aborted) { abort(); return; }
        try {
          if (mode === "fail") throw new Error(`${op} 합성 실패`);
          const encoded = JSON.stringify(this.response(op, body));
          call.responseBytes = encoder.encode(encoded).length;
          finish("resolved"); resolve(JSON.parse(encoded) as T);
        } catch (error) { finish("failed"); reject(error); }
      };
      signal?.addEventListener("abort", abort, { once: true });
      if (signal?.aborted) abort();
      else if (mode === "hold") this.releases.push(deliver);
      else timer = window.setTimeout(deliver, 0);
    });
  };
}

function Scene({ stub, kind, seed, creating = false, scope = "personal" }: { stub: Stub; kind: "memory" | "documents"; seed?: Item | null; creating?: boolean; scope?: Scope }) {
  const [managing, setManaging] = useState(false), [visible, setVisible] = useState(true), [initialItem, setInitialItem] = useState(seed);
  const [sourceChanged, setSourceChanged] = useState(false);
  return <>
    <div className="fixture-controls">
      <button id="fixture-managing" onClick={() => setManaging(value => !value)}>관리 전환</button>
      <button id="fixture-visible" onClick={() => setVisible(value => !value)}>탭 가시성 전환</button>
      <button id="fixture-source-changed" onClick={() => setSourceChanged(true)}>외부 근거 변경</button>
      <button id="fixture-seed" onClick={() => setInitialItem({ ...stub.item, title: "나중에 전달한 seed" })}>seed prop 변경</button>
    </div>
    <div id="fixture-surface" data-visible={visible} data-managing={managing}>
      {kind === "memory" ? <Memory latestNode={sourceChanged ? { id: memoryId, scope: "personal", kind: "memory", label: "합성 기록", revision: String(initialItem?.revision ?? 1), supported: false } : null} visible={visible} managing={managing} initialItem={initialItem} scope={scope} csrf="synthetic-only" request={stub.request} selectedId={creating ? null : memoryId} onBusy={quiet} onChange={value => stub.changed.push(value)} onNavigate={quiet} onMetadataChange={quiet} /> : <Documents visible={visible} managing={managing} scope={scope} id={documentId} csrf="synthetic-only" allAreas={[]} request={stub.request} onBusy={quiet} onChange={() => { stub.documentChanges++; }} onNavigate={quiet} />}
    </div>
  </>;
}
function isShown(element: HTMLElement) {
  if (element.closest("[hidden]")) return false;
  for (let parent = element.parentElement; parent; parent = parent.parentElement) {
    if (parent instanceof HTMLDetailsElement && !parent.open && !parent.querySelector(":scope > summary")?.contains(element)) return false;
  }
  return element.getClientRects().length > 0 && getComputedStyle(element).visibility !== "hidden";
}
function element<T extends HTMLElement>(selector: string, text?: string): T {
  const found = [...host.querySelectorAll<T>(selector)].filter(item => isShown(item) && (text === undefined || item.textContent?.trim() === text));
  assert(found.length === 1, `${selector} ${text ?? ""}: 표시된 요소 1개 필요, 실제 ${found.length}`);
  return found[0];
}
async function click(selector: string, text?: string) { flushSync(() => element(selector, text).click()); await settle(); }
const button = (text: string) => click("#fixture-surface button", text);
const toggle = (text: string) => click("#fixture-surface summary", text);
async function type(selector: string, value: string) {
  const control = element<HTMLInputElement | HTMLTextAreaElement>(selector);
  const prototype = control instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  flushSync(() => { Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(control, value); control.dispatchEvent(new Event("input", { bubbles: true })); });
  await settle();
}
function counts(stub: Stub, expected: Record<string, number>, phase: string) {
  const total = Object.values(expected).reduce((sum, count) => sum + count, 0);
  assert(stub.calls.length === total, `${phase}: 총 호출 ${stub.calls.length}, 기대 ${total}`);
  for (const [op, count] of Object.entries(expected)) assert(stub.count(op) === count, `${phase}: ${op}=${stub.count(op)}, 기대 ${count}`);
}
function publish(done = false) {
  const passed = results.every(result => result.passed) && networkAttempts === 0;
  const status = done ? passed ? "PASS" : "FAIL" : "RUNNING";
  output.dataset.status = status.toLowerCase();
  output.textContent = `${status} · ${results.filter(result => result.passed).length}/${results.length} 시나리오 · 실제 fetch ${networkAttempts}회\n${JSON.stringify(results, null, 2)}`;
}
async function scenario(name: string, size: number, kind: "memory" | "documents", run: (stub: Stub) => Promise<void>, setup: { seed?: Item | null; creating?: boolean; scope?: Scope; configure?: (stub: Stub) => void; modes?: [string, Mode[]] } = {}) {
  const stub = new Stub(size), startChecks = checks;
  if (setup.scope) stub.item.scope = setup.scope;
  setup.configure?.(stub);
  if (setup.modes) stub.next(setup.modes[0], ...setup.modes[1]);
  const root = createRoot(host);
  let reason: string | undefined;
  try {
    flushSync(() => root.render(<Scene stub={stub} kind={kind} seed={setup.seed} creating={setup.creating} scope={setup.scope} />));
    await settle(); await run(stub); await settle();
    assert(stub.calls.every(call => call.signal), "모든 요청에 AbortSignal 필요");
    assert(stub.calls.every(call => call.state !== "pending"), "완료 시 미결 요청 없음");
    assert(networkAttempts === 0, "실제 fetch 호출 금지");
  } catch (error) { reason = error instanceof Error ? error.message : String(error); }
  finally { flushSync(() => root.unmount()); stub.release(); await settle(); }
  results.push({ name, passed: reason === undefined, checks: checks - startChecks, reason, counts: Object.fromEntries([...new Set(stub.calls.map(call => call.op))].map(op => [op, stub.count(op)])), responseBytes: stub.calls.reduce((sum, call) => sum + call.responseBytes, 0), requestBytes: stub.calls.reduce((sum, call) => sum + call.requestBytes, 0), maxPending: stub.maxPending, calls: stub.calls });
  publish();
}

async function originalEditing() {
  const stub = new Stub(0), startChecks = checks, root = createRoot(host);
  const saved = "---\r\ntitle: 합성 원문\r\n---\r\n\r\n# 합성 원문\r\n\r\n기존 내용\r\n\r\n[사진](../assets/photo.png)\r\n";
  let content = saved, revision = 2, digest = "initial", conflict = false, changed = 0, dirty = false;
  let sourceScope = "personal", bytes: number | null = null;
  const confirm = window.confirm;
  let reason: string | undefined;
  const render = () => flushSync(() => root.render(<div id="fixture-surface"><OriginalDetail scope={sourceScope} path="notes/original.md" request={stub.request} csrf="synthetic-only" onDirtyChange={value => { dirty = value; }} onChange={() => { changed++; }} /></div>));
  stub.reply = (op, body) => {
    assert(body.scope === sourceScope && body.path === "notes/original.md", "선택한 범위·원문만 요청");
    if (op === "context-read") return { metadata: { scope: sourceScope, path: body.path, source_path: "synthetic", revision, origin_kind: "native", source_digest: null, content_digest: digest, byte_len: bytes ?? encoder.encode(content).length }, title: "합성 원문", content };
    if (op === "context-history") return { items: [revision, 1].map(value => ({ revision: value, content_digest: "synthetic", byte_len: 1, recorded_at: 1, change_kind: "manual" })), next_before: null };
    if (op === "context-version") return { revision: Number(body.revision), content_digest: "historical", title: "과거 원문", content: "# 과거 원문\n\n이전 내용\n\n[사진](../assets/photo.png)" };
    if (op === "context-edit") {
      assert(body.expected_revision === revision && body.expected_digest === digest, "저장은 편집 시작 시 버전과 내용을 확인");
      if (conflict) throw { status: 409 };
      content = String(body.content); revision++; digest = `saved-${revision}`;
      return { revision, content_digest: digest, changed: true };
    }
    throw new Error(`예상하지 않은 원문 요청: ${op}`);
  };
  try {
    render(); await waitFor(() => !!host.querySelector(".document-preview"), "원문 읽기");
    assert([...host.querySelectorAll(".original-view-switch button")].map(item => item.textContent).join("/") === "읽기/편집", "공통 상세의 두 모드");
    assert(!host.textContent?.match(/SHA-256|기술 정보|온톨로지에서 작성/), "원문 상세에 진단 정보 없음");
    assert(element<HTMLButtonElement>(".attachment-actions button", "사진 미리보기").getAttribute("aria-expanded") === "false", "현재 원문의 일반 첨부 링크도 필요한 시점 미리보기 제공");
    assert(!host.querySelector("img, video"), "읽기만 하면 첨부 요청 요소 없음");
    const titleBox = () => {
      const title = element<HTMLElement>(".document-header h1"), box = title.getBoundingClientRect(), style = getComputedStyle(title);
      return [box.x, box.y, box.width, box.height, style.fontSize, style.lineHeight].join("/");
    };
    const readingTitle = titleBox();
    await button("편집");
    assert(titleBox() === readingTitle && host.querySelectorAll(".original-detail h1").length === 1, "읽기·편집에서 제목의 위치·크기와 단일 표시 유지");
    assert(element<HTMLTextAreaElement>("#original-draft").value === saved.replace(/\r\n/g, "\n"), "편집기에서 frontmatter를 포함한 전체 원문을 읽고 수정");
    const textarea = element<HTMLTextAreaElement>("#original-draft");
    textarea.focus();
    const draft = textarea.value.replace("기존 내용", "수정 내용");
    await type("#original-draft", draft);
    assert(textarea === host.querySelector("#original-draft") && document.activeElement === textarea, "입력 중 편집기가 재생성되거나 초점을 잃지 않음");
    assert(dirty, "초안을 바꾸면 이동 보호 활성화");
    await button("읽기");
    assert(dirty && host.querySelector(".document-preview")?.textContent?.includes("기존 내용"), "읽기는 저장된 본문을 표시하며 초안을 보존");
    await button("편집"); assert(element<HTMLTextAreaElement>("#original-draft").value === draft, "편집 복귀 시 같은 초안 유지");
    counts(stub, { "context-read": 1, "context-history": 1 }, "모드 전환에 추가 조회 없음");
    stub.next("context-edit", "hold"); await button("저장");
    assert(element<HTMLButtonElement>(".original-view-switch button", "읽기").disabled, "저장 중 모드 이동 방지");
    assert(element<HTMLTextAreaElement>("#original-draft").disabled, "저장 중 입력 변경 방지");
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true })); await settle();
    assert(stub.count("context-edit") === 1, "저장 중 단축키가 중복 쓰기를 만들지 않음");
    stub.release(); await waitFor(() => changed === 1, "저장 후 다시 읽어 검증");
    assert(content === draft.replace(/\n/g, "\r\n") && !dirty, "줄바꿈 보존과 저장 후 초안 보호 해제");
    assert(host.querySelector(".document-preview")?.textContent?.includes("수정 내용"), "저장한 본문으로 읽기 복귀");
    await button("편집"); await type("#original-draft", draft + "실패할 변경");
    conflict = true; await button("저장");
    assert(element<HTMLTextAreaElement>("#original-draft").value.endsWith("실패할 변경") && dirty, "충돌 후 초안 보존");
    assert(host.querySelector(".error")?.textContent?.includes("초안은 유지"), "충돌의 다음 행동 안내");
    await button("읽기"); await button("편집");
    assert(element<HTMLTextAreaElement>("#original-draft").value.endsWith("실패할 변경"), "실패한 초안도 모드 전환에 보존");
    window.confirm = () => false; await button("변경 취소"); assert(dirty, "취소 확인 거부 시 초안 유지");
    window.confirm = () => true; await button("변경 취소"); assert(!dirty && !host.querySelector("#original-draft"), "명시적 취소 시 초안 해제");
    await toggle("변경 이력"); await click('.original-detail details button[aria-pressed="false"]');
    await waitFor(() => host.querySelector(".document-preview h1")?.textContent === "과거 원문", "과거 원문 조회");
    assert(element<HTMLButtonElement>(".original-view-switch button", "편집").disabled, "과거 버전 편집 금지");
    assert(!host.querySelector(".attachment-actions"), "이전 원문의 첨부를 당시 이미지로 잘못 표시하지 않음");
    assert(host.textContent?.includes("첨부 링크는 현재 원본"), "이전 원문 다운로드의 현재 버전 안내");
    for (const state of ["profile", "mixed", "large"] as const) {
      sourceScope = state === "profile" ? "profile" : "personal";
      content = state === "mixed" ? "# 합성 원문\r\n본문\n" : saved;
      bytes = state === "large" ? 1024 * 1024 + 1 : null;
      // Remount exactly as App does when selecting a different original.
      flushSync(() => root.render(null)); render();
      await waitFor(() => !!host.querySelector(".document-preview"), `${state} 원문 읽기`);
      assert(element<HTMLButtonElement>(".original-view-switch button", "편집").disabled, `${state} 원문의 기존 편집 제한 유지`);
    }
    assert(stub.count("context-edit") === 2 && changed === 1, "성공·충돌 외에는 원문 쓰기 없음");
    assert(networkAttempts === 0, "원문 검사도 실제 fetch 차단");
  } catch (error) { reason = error instanceof Error ? error.message : String(error); }
  finally { window.confirm = confirm; flushSync(() => root.unmount()); stub.release(); await settle(); }
  results.push({ name: "OriginalDetail 두 모드·초안 보존·저장·충돌·읽기 전용", passed: reason === undefined, checks: checks - startChecks, reason, counts: Object.fromEntries([...new Set(stub.calls.map(call => call.op))].map(op => [op, stub.count(op)])), responseBytes: stub.calls.reduce((sum, call) => sum + call.responseBytes, 0), requestBytes: stub.calls.reduce((sum, call) => sum + call.requestBytes, 0), maxPending: stub.maxPending, calls: stub.calls });
  publish();
}

async function mediaPreviewControls() {
  const root = createRoot(host), startChecks = checks;
  let reason: string | undefined;
  try {
    flushSync(() => root.render(<div id="fixture-surface"><DocumentPreview path="fixture.md" kind="context" content="![사진](photo.png)\n\n[영상](video.mp4)\n\n[**![사진](photo.png)**](video.mp4)" resolveInternalDownload={href => `about:blank#${href}`} resolveInternalMedia={href => ({ kind: href.endsWith("png") ? "image" : "video", url: `about:blank#${href}` })} /></div>));
    assert(!host.querySelector("img, video"), "초기 미디어 요소와 자동 요청 없음");
    assert(!host.querySelector("button button, button a, a button, a a"), "중첩된 첨부도 컨트롤을 서로 감싸지 않음");
    const photo = host.querySelector<HTMLElement>(".attachment-preview")!;
    flushSync(() => photo.querySelector<HTMLButtonElement>("button")!.click());
    const image = photo.querySelector<HTMLImageElement>("img")!;
    assert(image?.alt === "사진" && image.src === "about:blank#photo.png", "열 때만 사진 요소 생성과 대체 설명 유지");
    flushSync(() => image.dispatchEvent(new Event("error")));
    assert(photo.querySelector('[role="alert"]') && !photo.querySelector("img"), "미디어 실패 시 내려받기·재시도 안내");
    flushSync(() => photo.querySelector<HTMLButtonElement>(".attachment-error button")!.click());
    assert(photo.querySelector("img"), "재시도에서 새 미디어 요소 생성");
    flushSync(() => photo.querySelector<HTMLButtonElement>(".attachment-actions button")!.click());
    assert(!photo.querySelector("img") && photo.querySelector("[hidden]"), "닫기에서 사진과 요청 요소 제거");
    const videoContainer = host.querySelectorAll<HTMLElement>(".attachment-preview")[1];
    flushSync(() => videoContainer.querySelector<HTMLButtonElement>("button")!.click());
    const video = videoContainer.querySelector<HTMLVideoElement>("video")!;
    assert(video.controls && video.playsInline && video.preload === "none" && !video.autoplay, "영상은 사용자가 조작하는 기본 재생 컨트롤과 필요한 시점 로딩");
    flushSync(() => videoContainer.querySelector<HTMLButtonElement>("button")!.click());
    assert(!videoContainer.querySelector("video"), "닫기에서 영상 제거와 재생 종료");
    assert(video.paused && !video.getAttribute("src"), "제거된 영상도 명시적으로 정지하고 미디어 요청 해제");
    assert(document.documentElement.scrollWidth <= window.innerWidth, "작은 화면에서도 가로 넘침 없음");
  } catch (error) { reason = error instanceof Error ? error.message : String(error); }
  finally { flushSync(() => root.unmount()); await settle(); }
  results.push({ name: "미디어 UI 열기·실패·재시도·닫기 (합성 URL, 재생 검증은 별도)", passed: reason === undefined, checks: checks - startChecks, reason, counts: {}, responseBytes: 0, requestBytes: 0, maxPending: 0, calls: [] });
  publish();
}

async function purposeManagement() {
  const stub = new Stub(0), root = createRoot(host), startChecks = checks;
  const material = "00000000-0000-4000-8000-000000000101";
  const node = { id: `c_${material}`, material_id: material, scope: "personal" as const, kind: "document" as const, label: "합성 원문" };
  let groupRevision = 1, membershipRevision = 1, busy = false, dirty = false, changed = 0, failure: string | undefined, autoState = "suggested";
  const definition = () => ({ purpose: `현재 목적 ${groupRevision}`, include: "합성 포함", exclude: "합성 제외" });
  const membership = () => ({ revision: membershipRevision, subject_id: "fixture-purpose", subject_revision: groupRevision,
    current_subject_revision: groupRevision, subject_name: "합성 목적", definition: definition(), reason: "확인한 목적", review_needed: autoState !== "assigned",
    grouping: { mode: "auto", state: autoState, suggestions: { candidate_ids: ["fixture-purpose"], candidate_names: { "fixture-purpose": "합성 후보" } }, reason: "분류 후보를 확인해 주세요." },
    current_source: { source_revision: "1", content_digest: "a".repeat(64) } });
  const render = () => flushSync(() => root.render(<div id="fixture-surface"><DocumentPurpose node={node} scope="personal" csrf="synthetic-only" request={stub.request}
    visible disabled={false} onBusy={value => { busy = value; }} onDirtyChange={value => { dirty = value; }} onChange={() => { changed++; }} /></div>));
  stub.reply = (op, body) => {
    assert(body.scope === "personal", "purpose stays in the requested app scope");
    if (op === "subjects") {
      assert(body.limit === 20, "selector request is bounded");
      return { items: [{ id: "fixture-purpose", name: "합성 목적", revision: groupRevision, definition: definition() }], next_after: null };
    }
    const identity = ["document-subject-set", "document-grouping-retry"].includes(op) ? (body.document as { identity: object }).identity : body.document;
    assert(JSON.stringify(identity) === JSON.stringify({ material_id: material }), "native purpose APIs use canonical material, never a display alias");
    if (op === "document-subject") return membership();
    if (op === "document-subject-set") {
      const value = body.document as { revision: number; subject_revision: number; content_digest: string; reason: string };
      assert(value.revision === membershipRevision && value.content_digest === "a".repeat(64), "assignment retains the reviewed source and CAS revision");
      if (value.subject_revision !== groupRevision) throw new Error("목적 정의가 먼저 변경되었습니다. 다시 읽어주세요.");
      membershipRevision++; return { ...membership(), reason: value.reason };
    }
    if (op === "document-grouping-retry") {
      const value = body.document as { revision: number; source_revision: string; content_digest: string };
      assert(value.revision === membershipRevision && value.source_revision === "1" && value.content_digest === "a".repeat(64), "automatic retry binds the current source and membership without deleting it");
      autoState = "pending"; return membership();
    }
    if (op === "document-subject-history") {
      assert(body.limit === 20, "history is paged");
      const count = body.before_revision ? 1 : 20, first = body.before_revision ? 1 : 21;
      return { items: Array.from({ length: count }, (_, i) => ({ revision: first - i, subject_name: "당시 이름", reason: `당시 이유 ${first - i}`, changed_at: "2026-01-01T00:00:00Z", definition: { ...definition(), purpose: "당시 목적" } })), next_before_revision: body.before_revision ? null : 2 };
    }
    throw new Error(`Unexpected purpose fixture: ${op}`);
  };
  try {
    render(); await settle(); counts(stub, {}, "purpose detail makes no eager API reads");
    await button("목적 소속 관리");
    assert(host.textContent?.includes("후보 검토 필요") && host.textContent?.includes("합성 후보"), "automatic candidate state and names are visible after explicit read");
    element<HTMLSelectElement>(".document-purpose select").focus(); await settle();
    counts(stub, { "document-subject": 1, subjects: 1 }, "explicit read and selector only");
    await type(".document-purpose textarea", "재확인 이유"); assert(dirty, "purpose draft reports dirty");
    assert([...host.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === "목적 자동 재검토")?.disabled, "automatic retry cannot discard a manual draft");
    groupRevision = 2;
    await button("소속 저장"); assert(!!host.querySelector(".error") && dirty, "conflict preserves the draft");
    await button("현재 소속 다시 읽기");
    element<HTMLSelectElement>(".document-purpose select").blur();
    element<HTMLSelectElement>(".document-purpose select").focus(); await settle();
    assert(host.textContent?.includes("현재 목적 2"), "reread invalidates the stale selector definition");
    await type(".document-purpose textarea", "새 정의로 재확인"); await button("소속 저장");
    assert(changed === 1 && !dirty && !busy, "fresh criteria can be confirmed once with busy and draft cleared");
    await button("소속 이력"); assert(host.querySelectorAll(".document-purpose li").length === 20, "only first history page rendered");
    await button("이력 더 보기"); assert(host.querySelectorAll(".document-purpose li").length === 21, "explicit next history page");
    counts(stub, { "document-subject": 2, subjects: 2, "document-subject-set": 2, "document-subject-history": 2 }, "bounded explicit purpose workflow");
    await button("목적 자동 재검토");
    assert(host.textContent?.includes("분류 대기") && [...host.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent === "목적 자동 재검토")?.disabled, "one explicit automatic retry is visible and cannot be repeated while pending");
    stub.next("document-subject", "hold"); await button("목적 소속 관리"); assert(busy, "held purpose request owns busy");
    flushSync(() => root.render(<div id="fixture-surface" />)); await settle();
    assert(!busy && stub.calls.at(-1)?.state === "aborted", "unmount aborts and releases its own busy");
    stub.release(); await settle(); assert(!busy, "late response cannot relock the panel");
  } catch (error) { failure = error instanceof Error ? error.message : String(error); }
  finally { root.unmount(); stub.release(); await settle(); }
  results.push({ name: "문서 목적 지연 조회·정의 충돌·재조회·이력 쪽·busy 해제", passed: failure === undefined, reason: failure, checks: checks - startChecks,
    counts: Object.fromEntries([...new Set(stub.calls.map(call => call.op))].map(op => [op, stub.count(op)])), responseBytes: stub.calls.reduce((n, c) => n + c.responseBytes, 0), requestBytes: stub.calls.reduce((n, c) => n + c.requestBytes, 0), maxPending: stub.maxPending, calls: stub.calls });
  publish();
}

async function purposeDefinitionCurrency() {
  const stub = new Stub(0), root = createRoot(host), startChecks = checks;
  const old = { id: "fixture-purpose", scope: "personal" as const, kind: "subject" as const, label: "기존 이름", revision: "1",
    definition: { purpose: "기존 목적", include: "포함", exclude: "제외" } };
  let dirty = false, changed = 0, failure: string | undefined;
  const render = () => flushSync(() => root.render(<div id="fixture-surface"><SubjectPurpose node={{ ...old }} scope="personal" csrf="synthetic-only" request={stub.request}
    visible disabled={false} onBusy={() => {}} onDirtyChange={value => { dirty = value; }} onChange={() => { changed++; }} /></div>));
  stub.reply = (op, body) => {
    assert(op === "subject-define" && body.revision === 1, "definition saves the current CAS exactly once");
    return { id: old.id, name: body.name, revision: 2, definition: body.definition };
  };
  try {
    render(); await settle(); counts(stub, {}, "definition reads only graph metadata");
    await button("목적 정의 편집"); await type("input", "저장한 이름"); await type("fieldset > label:nth-of-type(2) textarea", "저장한 목적");
    await button("정의 저장"); render(); await settle();
    assert(changed === 1 && host.querySelector("dd")?.textContent === "저장한 목적", "older delayed graph cannot replace the successful save response");
    await button("목적 정의 편집"); await type("input", "취소할 이름"); await type("fieldset > label:nth-of-type(2) textarea", "취소할 목적"); assert(dirty, "new draft reports dirty");
    await button("취소"); assert(!dirty && host.querySelector("dd")?.textContent === "저장한 목적", "cancel reads the confirmed current criteria");
    await button("목적 정의 편집");
    assert(element<HTMLInputElement>("input").value === "저장한 이름" && element<HTMLTextAreaElement>("fieldset > label:nth-of-type(2) textarea").value === "저장한 목적", "reopen restores current name and definition, not canceled input");
    counts(stub, { "subject-define": 1 }, "currency protection adds no eager reads or duplicate writes");
  } catch (error) { failure = error instanceof Error ? error.message : String(error); }
  finally { root.unmount(); await settle(); }
  results.push({ name: "목적 정의 저장 응답·지연 graph·취소 기준 보존", passed: failure === undefined, reason: failure, checks: checks - startChecks,
    counts: { "subject-define": stub.count("subject-define") }, responseBytes: stub.calls.reduce((n, c) => n + c.responseBytes, 0), requestBytes: stub.calls.reduce((n, c) => n + c.requestBytes, 0), maxPending: stub.maxPending, calls: stub.calls });
  publish();
}

async function main() {
  await mediaPreviewControls();
  await originalEditing();
  await purposeManagement();
  await purposeDefinitionCurrency();
  await scenario("회사 curated 기록 무변경 저장·묶음만 변경·해제 보존", 1, "memory", async stub => {
    const original = JSON.stringify([stub.item.body, stub.item.curation, stub.item.evidence]);
    stub.reply = (op, body) => {
      assert(body.scope === "meenseek", "회사 범위만 요청한다");
      assert(op !== "correct", "동일 내용의 회사 기록을 correct로 재구성하지 않는다");
      if (op !== "grouping-set") return undefined;
      assert(body.mode === "manual" || body.mode === "off", "회사 기록은 자동 분류를 요청하지 않는다");
      assert(body.revision === stub.item.revision && body.memory === undefined, "metadata CAS만 전송한다");
      if (body.subject_id !== stub.item.subject_id) stub.item = { ...stub.item, subject_id: body.subject_id as string | null,
        subject_name: body.subject_id ? "합성 묶음 0" : null, revision: stub.item.revision + 1 };
      return stub.item;
    };
    await click("#fixture-managing"); await button("기록 정정"); await button("정정 저장");
    assert(stub.item.revision === 1, "같은 회사 소속의 무변경 저장은 no-op");
    await button("기록 정정"); await toggle("추가 설정");
    const select = element<HTMLSelectElement>('select[aria-label="기록 묶음"]'); select.focus(); await settle();
    flushSync(() => { select.value = "fixture-subject-0"; select.dispatchEvent(new Event("change", { bubbles: true })); }); await settle();
    await button("정정 저장"); assert(stub.item.subject_id === "fixture-subject-0", "내용 정정 없이 목적을 배정한다");
    await toggle("기록 묶음 · 합성 묶음 0");
    assert(!host.textContent?.includes("Codex가 다시 분류"), "회사 읽기에는 자동 분류 버튼이 없다");
    await button("묶음 없음"); assert(stub.item.subject_id === null, "회사 읽기에서 소속 해제 가능");
    assert(JSON.stringify([stub.item.body, stub.item.curation, stub.item.evidence]) === original, "본문·검토·stale 근거를 그대로 보존한다");
    counts(stub, { read: 1, subjects: 1, "grouping-set": 3 }, "회사 metadata 저장은 정정 호출 없이 한 번씩");
  }, { scope: "meenseek", configure: stub => {
    stub.item.curation = { review_id: "synthetic-review", source_id: "synthetic-source", applicability: "합성 회사 범위", reason: "보존할 검토 이유" };
    stub.item.evidence[0].current = false;
  } });
  await scenario("새 목적 묶음 원자 생성·응답 유실 뒤 변경된 정의 보호", 0, "memory", async stub => {
    let current: { id: string; name: string; revision: number; definition: { purpose: string; include: string; exclude: string } } | null = null;
    let key = "";
    stub.reply = (op, body) => {
      if (op !== "subject-create") return undefined;
      assert(body.scope === "personal", "new purpose scope");
      if (!current) {
        assert(!!body.definition, "birth includes all criteria in one request");
        key = String(body.idempotency_key);
        current = { id: "fixture-created-purpose", name: String(body.name), revision: 0, definition: body.definition as NonNullable<typeof current>["definition"] };
        // The server committed, but its response was lost. Another writer then changed the purpose.
        current = { ...current, revision: 1, definition: { ...current.definition, purpose: "다른 작성자가 확인한 현재 목적" } };
        throw new Error("합성 응답 유실");
      }
      assert(body.idempotency_key === key, "retry retains its immutable birth intent");
      return current;
    };
    await toggle("추가 설정"); await toggle("새 묶음 만들기");
    await type("details:has(> summary) input[maxlength='80']", "합성 새 목적");
    const fields = "details:has(> summary) .topic-label textarea";
    const values = ["합성 생성 목적", "합성 포함 기준", "합성 제외 기준"];
    const controls = [...host.querySelectorAll<HTMLTextAreaElement>(fields)].filter(isShown);
    assert(controls.length === 3, "one complete criteria form");
    for (let index = 0; index < controls.length; index++) {
      flushSync(() => { Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(controls[index], values[index]); controls[index].dispatchEvent(new Event("input", { bubbles: true })); }); await settle();
    }
    await button("묶음 만들기"); assert(host.textContent?.includes("합성 응답 유실"), "failed response keeps the form retryable");
    await button("묶음 만들기");
    assert(host.textContent?.includes("만든 묶음의 목적이 이후 변경되었습니다"), "changed current criteria require review");
    assert(element<HTMLSelectElement>('select[aria-label="기록 묶음"]').value === "", "retry cannot silently select changed criteria");
    counts(stub, { "subject-create": 2 }, "no secondary definition write on retry");
  }, { creating: true });
  await scenario("Memory 읽기 화면에서 묶음 변경", 1, "memory", async stub => {
    await waitFor(() => !!host.querySelector(".memory-body"), "기록 읽기");
    counts(stub, { read: 1 }, "초기 읽기에서 묶음 목록을 요청하지 않음");
    await toggle("기록 묶음 · 없음");
    await waitFor(() => stub.calls.some(call => call.op === "subjects" && call.state === "resolved"), "읽기 화면의 묶음 목록");
    counts(stub, { read: 1, subjects: 1 }, "묶음 선택을 열 때만 조회");
    await button("합성 묶음 0에 묶기");
    assert(stub.changed.at(-1)?.subject_id === "fixture-subject-0", "관리 없이 묶음 변경");
    counts(stub, { read: 1, subjects: 1, "grouping-set": 1 }, "묶음 변경은 한 번의 쓰기");
    await button("묶음 없음");
    assert(stub.changed.at(-1)?.subject_id === null && stub.changed.at(-1)?.grouping?.mode === "off", "기록을 보존하고 묶음만 해제");
    counts(stub, { read: 1, subjects: 1, "grouping-set": 2 }, "묶음 해제는 한 번의 쓰기");
  });
  for (const size of [0, 1, 20]) {
    await scenario(`Memory 정상·draft·subjects·쓰기·history (${size})`, size, "memory", async stub => {
      await waitFor(() => !!host.querySelector(".memory-body"), "기록 읽기");
      counts(stub, { read: 1, subjects: 0 }, "초기 읽기");
      await click("#fixture-managing"); counts(stub, { read: 1 }, "관리만 열기");
      await button("기록 정정"); await toggle("추가 설정");
      await type('.memory-editor form input[maxlength="160"]', "보존할 draft");
      const title = element<HTMLInputElement>('.memory-editor form input[maxlength="160"]');
      await click("#fixture-managing"); await click("#fixture-managing");
      assert(element('.memory-editor form input[maxlength="160"]') === title && title.value === "보존할 draft", "관리 접기에도 같은 draft DOM 보존");
      counts(stub, { read: 1, subjects: 0 }, "제목 편집은 묶음을 조회하지 않음");
      element<HTMLSelectElement>('select[aria-label="기록 묶음"]').focus(); await settle();
      await waitFor(() => stub.calls.some(call => call.op === "subjects" && call.state === "resolved"), "묶음 첫 페이지");
      counts(stub, { read: 1, subjects: 1 }, "추가 설정 첫 열기");
      assert(element<HTMLSelectElement>('select[aria-label="기록 묶음"]').options.length === size + 1, "묶음 결과 크기 반영");
      await toggle("추가 설정"); await toggle("추가 설정");
      await click("#fixture-visible"); await click("#fixture-visible");
      await click("#fixture-managing"); await click("#fixture-managing");
      counts(stub, { read: 1, subjects: 1 }, "닫기·재열기·복귀 캐시");
      await button("정정 저장");
      assert(stub.changed.length === 1 && stub.changed[0]?.title === "보존할 draft", "쓰기 응답 Item 전달");
      counts(stub, { read: 1, subjects: 1, correct: 1 }, "정정 응답 재사용");
      await toggle("기록 상세·변경 이력"); await button("변경 이력 읽기");
      counts(stub, { read: 1, subjects: 1, correct: 1, history: 1 }, "이력 첫 페이지");
      assert(host.querySelectorAll("article.history").length === Math.min(size, 10), "이력 첫 페이지 크기");
      await toggle("기록 상세·변경 이력"); await toggle("기록 상세·변경 이력");
      await click("#fixture-managing"); await click("#fixture-managing");
      counts(stub, { read: 1, subjects: 1, correct: 1, history: 1 }, "이력 재열기 캐시");
      if (size > 10) { await button("이력 더 보기"); assert(host.querySelectorAll("article.history").length === size, "이력 다음 페이지 누적"); }
      counts(stub, { read: 1, subjects: 1, correct: 1, history: size > 10 ? 2 : 1 }, "페이지별 호출 상한");
      assert(stub.maxPending === 1, "사용자 순차 동작에 중복 동시 요청 없음");
    });
    await scenario(`Documents 읽기·관리·draft·local reload (${size})`, size, "documents", async stub => {
      await waitFor(() => !!host.querySelector(".document-preview"), "문서 읽기");
      counts(stub, { "document-read": 1 }, "초기 문서 읽기");
      await click("#fixture-managing"); await toggle("분류 수정");
      await type("#topics", "보존할 문서 태그"); const topics = element<HTMLTextAreaElement>("#topics");
      await click("#fixture-managing"); await click("#fixture-managing");
      assert(element("#topics") === topics && topics.value === "보존할 문서 태그", "관리 접기에도 문서 draft 보존");
      await click("#fixture-visible"); await click("#fixture-visible");
      counts(stub, { "document-read": 1 }, "문서 관리·탭 복귀 추가 조회 없음");
      if (size > 0) await toggle("분류·연결 변경 이력");
      counts(stub, { "document-read": 1 }, "이미 받은 문서 이력 사용");
      await button("다시 불러오기");
      await waitFor(() => stub.count("document-read") === 2 && !!host.querySelector(".document-preview"), "문서 로컬 reload");
      counts(stub, { "document-read": 2 }, "로컬 reload 1회");
      assert(stub.documentChanges === 0, "로컬 읽기 재시도는 상위 onChange를 호출하지 않음");
      assert(stub.maxPending === 1, "문서 요청 중복 없음");
    });
  }
  for (const variant of ["same", "wrong-id", "wrong-scope"] as const) {
    const seed = { ...memory(1), ...(variant === "wrong-id" ? { id: "foreign-id" } : variant === "wrong-scope" ? { scope: "meenseek" as const } : {}) };
    await scenario(`Memory seed ${variant}`, 1, "memory", async stub => {
      await waitFor(() => !!host.querySelector(".memory-body"), "seed 또는 read 반영");
      counts(stub, { read: variant === "same" ? 0 : 1 }, "scope/id seed 검증");
      await click("#fixture-managing"); await button("기록 정정"); await toggle("추가 설정");
      await type('.memory-editor form input[maxlength="160"]', "seed보다 우선인 draft");
      await click("#fixture-seed");
      assert(element<HTMLInputElement>('.memory-editor form input[maxlength="160"]').value === "seed보다 우선인 draft", "후속 seed prop이 기존 draft를 덮어쓰지 않음");
      counts(stub, { read: variant === "same" ? 0 : 1 }, "seed prop 변경 추가 조회 없음");
    }, { seed });
  }
  await scenario("이미 확인한 근거 변경은 이전의 최신 상태 표시를 무효화", 1, "memory", async stub => {
    assert(host.textContent?.includes("현재 출처와 일치"), "처음 읽은 근거는 당시 현재 상태");
    await click("#fixture-source-changed");
    assert(host.textContent?.includes("열어 둔 기록이나 근거의 변경을 확인했습니다."), "새 graph 근거 상태를 즉시 알림");
    assert(!host.textContent?.includes("현재 출처와 일치"), "이전 상태를 현재 근거라고 표시하지 않음");
    counts(stub, {}, "변경 알림에 추가 조회 없음");
  }, { seed: memory(1) });
  await scenario("근거 원문은 시점별 조회·재열기 캐시", 1, "memory", async stub => {
    counts(stub, {}, "seed 읽기와 근거 미열람은 요청 없음");
    await button("기록 당시 원문");
    assert(host.querySelector(".evidence-preview")?.textContent?.includes("당시 원문 2"), "현재 기록 시점의 근거 미리보기");
    await button("기록 당시 원문"); await button("기록 당시 원문");
    counts(stub, { "evidence-read": 1 }, "같은 시점 원문 재열기는 캐시");
    await click("#fixture-managing"); await toggle("기록 상세·변경 이력"); await button("변경 이력 읽기");
    await click("article.history button", "기록 당시 원문");
    assert(host.querySelector("article.history .evidence-preview")?.textContent?.includes("당시 원문 1"), "다른 시점은 자기 원문으로 구분");
    counts(stub, { "evidence-read": 2, history: 1 }, "원문 시점당 한 번만 조회");
  }, { seed: { ...memory(1), revision: 2 } });
  const nativeEvidence: Item["evidence"][number] = {
    ...memory(1).evidence[0], entity_id: "fixture-native-source", kind: "record", path: "관측: A/B 실험 원값",
    semantics: { kind: "fact", title_from_body: true, effective_from: null, effective_until: null },
  };
  const curated: Item = {
    ...memory(1), revision: 2, origin: "assistant", evidence: [nativeEvidence],
    curation: { review_id: "fixture-review-current", source_id: nativeEvidence.entity_id, applicability: "현재: 신규 방문자의 완료 행동", reason: "완료 관측을 확인해 적용 대상을 좁혔다." },
  };
  await scenario("정리 이력과 native 원문의 조건·종류·A/B 제목 보존", 1, "memory", async stub => {
    const earlier: Item = {
      ...curated, revision: 1,
      evidence: [{ ...nativeEvidence, path: "도입 A/B 결정", semantics: { ...nativeEvidence.semantics!, kind: "decision", title_from_body: false } }],
      curation: { review_id: "fixture-review-past", source_id: nativeEvidence.entity_id, applicability: "과거: 내부 참여자의 시범 운영", reason: "외부 관측 전이라 내부 운영에만 적용했다." },
    };
    stub.reply = (op, body) => {
      if (op === "history") return { items: [{ revision: earlier.revision, status: earlier.status, subject_id: null, document: earlier, changed_at: earlier.updated_at }], next_before_revision: null };
      if (op === "evidence-read") {
        assert(body.id === memoryId && body.entity_id === nativeEvidence.entity_id, "선택한 기록의 native 근거만 요청");
        assert(body.revision === 1 || body.revision === 2, "실제 선택한 과거 또는 현재 시점 요청");
        return { available: true, evidence: (body.revision === 2 ? curated : earlier).evidence[0], content: body.revision === 2 ? "---\n관측: A/B 실험 원값\n---\n\n**현재 관측**을 보존한다." : "**이전 기준**은 내부 시범 운영이었다." };
      }
    };
    counts(stub, {}, "정리 내용과 근거 이름은 받은 Item으로 표시");
    assert(element(".memory-body + .section > p").textContent === curated.curation!.applicability, "현재 적용 범위 표시");
    await click(".memory-body + .section summary", "정리 이유");
    assert(element(".memory-body + .section details > p").textContent === curated.curation!.reason, "현재 정리 이유 표시");
    assert(element(".evidence-source > h4").textContent === nativeEvidence.path, "native A/B 제목을 파일 경로처럼 자르지 않음");
    await click(".evidence-source summary", "출처 위치");
    assert(element(".evidence-source dt", "기록 종류").nextElementSibling?.textContent === "사실", "현재 근거의 원래 기록 종류 표시");
    await button("기록 당시 원문");
    const currentPreview = element(".evidence-source .document-preview");
    assert(!currentPreview.querySelector("h1"), "native 자동 제목을 원문에 다시 합성하지 않음");
    assert(currentPreview.textContent?.includes("관측: A/B 실험 원값") && currentPreview.querySelector("strong")?.textContent === "현재 관측", "native 원문은 YAML처럼 보이는 내용도 보존하며 Markdown으로 읽음");
    await click("#fixture-managing"); await toggle("기록 상세·변경 이력"); await button("변경 이력 읽기");
    assert(element("article.history > .section > p").textContent === earlier.curation!.applicability, "과거의 별도 적용 범위 보존");
    await click("article.history summary", "정리 이유");
    assert(element("article.history > .section details > p").textContent === earlier.curation!.reason, "과거의 별도 정리 이유 보존");
    assert(!element("article.history").textContent?.includes(curated.curation!.applicability), "현재 적용 범위로 과거 조건을 덮어쓰지 않음");
    assert(element("article.history > div > p", earlier.evidence[0].path).textContent === "도입 A/B 결정", "과거 native A/B 제목 보존");
    await click("article.history summary", "출처 위치");
    assert(element("article.history dt", "기록 종류").nextElementSibling?.textContent === "결정", "과거 근거의 기록 종류를 현재 사실로 바꾸지 않음");
    await click("article.history button", "기록 당시 원문");
    const pastPreview = element("article.history .evidence-preview .document-preview");
    assert(pastPreview.querySelector("h1")?.textContent === "도입 A/B 결정", "과거 native 수동 제목은 원래 전체 제목으로 표시");
    assert(pastPreview.querySelector("strong")?.textContent === "이전 기준", "과거 native 원문도 자기 시점의 Markdown으로 표시");
    counts(stub, { history: 1, "evidence-read": 2 }, "이력과 선택한 시점별 원문만 조회");
  }, { seed: curated });
  await scenario("Memory 새 기록 초기 0회·응답 재사용", 0, "memory", async stub => {
    counts(stub, {}, "새 기록 작성 초기");
    await type(".memory-editor form textarea[required]", "합성 새 본문");
    await button("저장");
    assert(stub.changed[0]?.body === "합성 새 본문", "새 기록의 반환 Item 전달");
    counts(stub, { remember: 1 }, "생성 응답 재사용, 추가 read 없음");
  }, { creating: true });
  await scenario("Memory subjects 실패·명시 재시도", 20, "memory", async stub => {
    await click("#fixture-managing"); await button("기록 정정"); await toggle("추가 설정");
    element<HTMLSelectElement>('select[aria-label="기록 묶음"]').focus(); await settle();
    await waitFor(() => stub.calls[0]?.state === "failed", "묶음 실패");
    counts(stub, { subjects: 1 }, "최초 실패");
    await click("#fixture-visible"); await click("#fixture-visible");
    await toggle("추가 설정"); await toggle("추가 설정");
    counts(stub, { subjects: 1 }, "실패 뒤 자동 retry 없음");
    await button("묶음 다시 불러오기");
    await waitFor(() => stub.calls[1]?.state === "resolved", "명시 묶음 재시도");
    counts(stub, { subjects: 2 }, "명시 retry 1회");
    await click("#fixture-visible"); await click("#fixture-visible");
    counts(stub, { subjects: 2 }, "retry 성공 캐시");
  }, { seed: memory(20), modes: ["subjects", ["fail"]] });
  await scenario("Memory subjects abort·복귀", 1, "memory", async stub => {
    await click("#fixture-managing"); await button("기록 정정"); await toggle("추가 설정");
    element<HTMLSelectElement>('select[aria-label="기록 묶음"]').focus(); await settle();
    counts(stub, { subjects: 1 }, "진행 중 묶음 요청");
    assert(stub.calls.at(0)?.state === "pending", "응답 전 상태 유지");
    await click("#fixture-visible");
    assert(stub.calls.at(0)?.state === "aborted", "숨김이 실제 AbortSignal 취소");
    await click("#fixture-visible");
    await waitFor(() => stub.calls[1]?.state === "resolved", "취소 후 재조회");
    stub.release(); await settle();
    counts(stub, { subjects: 2 }, "abort 후 복귀 1회");
    assert(stub.calls[0].responseBytes === 0 && stub.calls[1].responseBytes > 0, "취소된 응답 미전달");
  }, { seed: memory(1), modes: ["subjects", ["hold"]] });
  for (const kind of ["memory", "documents"] as const) {
    const op = kind === "memory" ? "read" : "document-read";
    await scenario(`${kind} 초기 실패·로컬 재시도`, 1, kind, async stub => {
      await waitFor(() => stub.calls[0]?.state === "failed", "초기 읽기 실패");
      await click("#fixture-visible"); await click("#fixture-visible");
      await click("#fixture-managing"); await click("#fixture-managing");
      counts(stub, { [op]: 1 }, "실패 뒤 visibility·관리 전환은 자동 재시도 없음");
      await button(kind === "memory" ? "기록 다시 불러오기" : "다시 불러오기");
      await waitFor(() => stub.calls[1]?.state === "resolved", "초기 읽기 명시 재시도");
      counts(stub, { [op]: 2 }, "초기 실패 후 retry 1회");
      assert(stub.documentChanges === 0, "읽기 실패 복구가 상위 갱신을 유발하지 않음");
    }, { modes: [op, ["fail"]] });
    await scenario(`${kind} 초기 read abort·복귀`, 1, kind, async stub => {
      counts(stub, { [op]: 1 }, "진행 중 초기 read");
      await click("#fixture-visible"); assert(stub.calls[0].state === "aborted", "초기 read AbortSignal 취소");
      await click("#fixture-visible");
      await waitFor(() => stub.calls[1]?.state === "resolved", "read 복귀 재시도");
      stub.release(); await settle(); counts(stub, { [op]: 2 }, "초기 read abort 후 재시도 1회");
    }, { modes: [op, ["hold"]] });
  }
  publish(true);
}
void main().catch(error => { output.dataset.status = "fail"; output.textContent = `FAIL · fixture 실행 오류: ${String(error)}\n${JSON.stringify(results, null, 2)}`; });
