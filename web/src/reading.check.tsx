import { useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import Memory from "./Memory";
import type { Item } from "./Memory";
import Documents from "./Documents";
import type { Request } from "./graph";
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
    const op = url === "/api/brain" ? String(body.op) : url === `/api/records/${documentId}?scope=personal` && !options?.method ? "document-read" : `unexpected:${url}`;
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

function Scene({ stub, kind, seed, creating = false }: { stub: Stub; kind: "memory" | "documents"; seed?: Item | null; creating?: boolean }) {
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
      {kind === "memory" ? <Memory latestNode={sourceChanged ? { id: memoryId, scope: "personal", kind: "memory", label: "합성 기록", revision: String(initialItem?.revision ?? 1), supported: false } : null} visible={visible} managing={managing} initialItem={initialItem} scope="personal" csrf="synthetic-only" request={stub.request} selectedId={creating ? null : memoryId} onBusy={quiet} onChange={value => stub.changed.push(value)} onNavigate={quiet} onMetadataChange={quiet} /> : <Documents visible={visible} managing={managing} scope="personal" id={documentId} csrf="synthetic-only" allAreas={[]} request={stub.request} onBusy={quiet} onChange={() => { stub.documentChanges++; }} onNavigate={quiet} />}
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
async function scenario(name: string, size: number, kind: "memory" | "documents", run: (stub: Stub) => Promise<void>, setup: { seed?: Item | null; creating?: boolean; modes?: [string, Mode[]] } = {}) {
  const stub = new Stub(size), startChecks = checks;
  if (setup.modes) stub.next(setup.modes[0], ...setup.modes[1]);
  const root = createRoot(host);
  let reason: string | undefined;
  try {
    flushSync(() => root.render(<Scene stub={stub} kind={kind} seed={setup.seed} creating={setup.creating} />));
    await settle(); await run(stub); await settle();
    assert(stub.calls.every(call => call.signal), "모든 요청에 AbortSignal 필요");
    assert(stub.calls.every(call => call.state !== "pending"), "완료 시 미결 요청 없음");
    assert(networkAttempts === 0, "실제 fetch 호출 금지");
  } catch (error) { reason = error instanceof Error ? error.message : String(error); }
  finally { flushSync(() => root.unmount()); stub.release(); await settle(); }
  results.push({ name, passed: reason === undefined, checks: checks - startChecks, reason, counts: Object.fromEntries([...new Set(stub.calls.map(call => call.op))].map(op => [op, stub.count(op)])), responseBytes: stub.calls.reduce((sum, call) => sum + call.responseBytes, 0), requestBytes: stub.calls.reduce((sum, call) => sum + call.requestBytes, 0), maxPending: stub.maxPending, calls: stub.calls });
  publish();
}

async function main() {
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
