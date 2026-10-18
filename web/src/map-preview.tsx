import { createRoot } from "react-dom/client";
import { Group, Scene, Vector3 } from "three";
import App from "./App";
import type { GraphNode, Scope, Snapshot } from "./graph";
import { Positions } from "./positions";
import "./style.css";

// Development-only visual fixture. No request reaches a server or a user's store.
if (!import.meta.env.DEV) throw new Error("지도 미리보기는 개발 서버에서만 사용할 수 있습니다.");
// Read actual rendered coordinates so native expansion QA can distinguish layout from camera motion.
const originalRender = Scene.prototype.onBeforeRender, world = new Vector3();
const originalInstall = Positions.prototype.install;
let positions: Positions | null = null;
Positions.prototype.install = function (...args) { positions = this; return originalInstall.apply(this, args); };
const motionSamples: { time: number; camera: { x: number; y: number; z: number }; layout: boolean; dragging: boolean; settling: boolean; groups: { id: string; count: number; radius: number; retiring: number; minOpacity: number }[] }[] = [];
let lastSample = 0;
Scene.prototype.onBeforeRender = function (renderer, scene, camera, geometry, material, group) {
  originalRender.call(this, renderer, scene, camera, geometry, material, group);
  if (performance.now() - lastSample < 80) return;
  lastSample = performance.now();
  const rect = renderer.domElement.getBoundingClientRect();
  const stars: { id: string; x: number; y: number; z: number; px: number; py: number; opacity: number; retiring: boolean }[] = [];
  this.traverse(object => {
    if (!(object instanceof Group) || !object.userData.nodeId || !object.visible) return;
    object.getWorldPosition(world);
    const { x, y, z } = world;
    world.project(camera);
    stars.push({ id: object.userData.nodeId, x, y, z, px: (world.x + 1) * rect.width / 2, py: (1 - world.y) * rect.height / 2,
      opacity: object.userData.opacity?.() ?? 1, retiring: object.userData.retiring?.() ?? false });
  });
  const notice = document.querySelector<HTMLElement>(".fixture-notice")!;
  notice.dataset.frame = JSON.stringify({ stars, camera: camera.position, width: rect.width, height: rect.height });
  const groups = stars.filter(star => star.id.startsWith("preview-purpose-")).map(hub => {
    const members = stars.filter(star => star.id.startsWith(`preview-${hub.id.slice("preview-purpose-".length)}-`));
    return { id: hub.id, count: members.length, radius: Math.max(0, ...members.map(star => Math.hypot(star.px - hub.px, star.py - hub.py))),
      retiring: members.filter(star => star.retiring).length, minOpacity: Math.min(1, ...members.map(star => star.opacity)) };
  });
  const sample = { time: performance.now(), camera: { x: camera.position.x, y: camera.position.y, z: camera.position.z }, layout: positions?.layoutMoving ?? false, dragging: positions?.dragging ?? false, settling: positions?.settling ?? false, groups };
  const moving = sample.layout || sample.dragging || sample.settling, previous = motionSamples.at(-1);
  const wasMoving = previous && (previous.layout || previous.dragging || previous.settling);
  if (moving || wasMoving || !previous) {
    if (moving && !wasMoving) motionSamples.length = 0;
    motionSamples.push(sample);
    if (motionSamples.length > 60) motionSamples.shift();
    notice.dataset.motion = JSON.stringify(motionSamples);
  }
};
// Readable DOM evidence for native pointer QA; this fixture is excluded from production.
for (const eventName of ["pointerdown", "pointerup"] as const) window.addEventListener(eventName, event => {
  const target = event.target as HTMLElement;
  document.querySelector<HTMLElement>(".fixture-notice")!.dataset[eventName] = JSON.stringify({ x: event.clientX, y: event.clientY, target: target.tagName, class: target.className });
}, { capture: true });
const sparse = new URLSearchParams(window.location.search).get("sample") === "sparse";
const names = ["제품 전략과 의사결정", "보안 플랫폼 개발·검증과 사용자 경험을 함께 개선하기", "개발과 품질", "프로젝트 실행", "글쓰기와 지식 정리", "새로운 실험"];
const counts = sparse ? [6, 5, 4, 3, 2, 2] : [225, 39, 22, 21, 20, 12];
const titles = ["사용자의 맥락을 이해하는 방법", "작은 결정을 오래 기억하기", "제품의 방향을 정하는 질문", "읽기와 탐색의 균형", "이번 주 실험에서 배운 것", "반복 작업의 부담을 줄이기", "팀이 같은 그림을 보는 순간", "아이디어에서 실제 결과까지", "오래 쓰는 도구의 조건", "자연스럽게 이어지는 인터랙션", "기록을 다시 찾기 위한 기준", "변화하는 지식을 연결하는 방식"];
// Members after the document chain mix in memories so kind colors, evidence and inactive states are visible.
const isMemory = (index: number) => index >= 13 && index % 3 === 2;
const inactive: Record<string, Partial<GraphNode>> = { "preview-1-14": { status: "proposed" }, "preview-1-17": { status: "withdrawn" }, "preview-1-20": { temporal: "expired" } };
const memory = (id: string, title: string, extra: Partial<GraphNode> = {}): GraphNode => ({ id, scope: "personal", kind: "memory", title, label: title, memory_kind: "record", status: "accepted", temporal: "current", supported: true, ...extra, ...inactive[id] });
const nodes: GraphNode[] = names.flatMap((name, group) => {
  const subject = `preview-purpose-${group}`;
  return [{ id: subject, scope: "personal", kind: "subject", label: name } as GraphNode,
    ...Array.from({ length: counts[group] }, (_, index): GraphNode => isMemory(index)
      ? memory(`preview-${group}-${index}`, titles[index % titles.length], { subject_id: subject, subject_name: name })
      : { id: `preview-${group}-${index}`, scope: "personal", kind: "document", title: titles[index % titles.length], label: `synthetic/${name}/${index}.md`,
        subject_id: subject, subject_name: name, source_kind: "original", context_scope: "personal", context_path: `synthetic/${group}/${index}.md`, status: "ok", present: true, current: true })];
});
for (let index = 0; index < (sparse ? 6 : 23); index++) nodes.push(index % 4 === 3 ? memory(`preview-single-${index}`, titles[index % titles.length]) : { id: `preview-single-${index}`, scope: "personal", kind: "document", title: titles[index % titles.length], label: `synthetic/ungrouped/${index}.md`,
  source_kind: "original", context_scope: "personal", context_path: `synthetic/ungrouped/${index}.md`, status: "ok", present: true, current: true });
nodes.push({ id: "preview-topic-shared", scope: "personal", kind: "topic", label: "공통 관점" });
const links: Snapshot["links"] = nodes.filter(node => node.subject_id).map(node => ({ source: node.id, target: node.subject_id!, kind: "subject", current: true }));
for (let group = 0; group < 6; group++) for (let index = 0; index < Math.min(12, counts[group] - 1); index++) links.push({ source: `preview-${group}-${index}`, target: `preview-${group}-${index + 1}`, kind: index % 2 ? "related" : "reference", current: true });
links.push({ source: "preview-0-0", target: "preview-1-0", kind: "reference", current: true });
// Each memory cites the document before it; a few citations in the large group refer to past revisions.
for (let group = 0; group < 6; group++) for (let index = 13; index < counts[group]; index++) {
  if (!isMemory(index) || (group === 0 && index % 9 !== 2)) continue;
  links.push({ source: `preview-${group}-${index}`, target: `preview-${group}-${index - 1}`, kind: "evidence", current: !(group === 0 && index % 27 === 2) });
}
for (const member of ["preview-2-0", "preview-2-1", "preview-2-2", "preview-3-0", "preview-3-1", "preview-3-2"].filter(id => nodes.some(node => node.id === id))) links.push({ source: member, target: "preview-topic-shared", kind: "topic", current: true });
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
window.fetch = async (input, options) => {
  const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
  if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
  if (url.pathname === "/api/graph") {
    const scope: Scope = url.searchParams.get("scope") === "meenseek" ? "meenseek" : "personal", query = url.searchParams.get("q") ?? "", focus = url.searchParams.get("focus");
    const included = nodes.filter(node => !query || `${node.label} ${node.title}`.includes(query) || node.id === focus).map(node => ({ ...node, scope }));
    const ids = new Set(included.map(node => node.id)), includedLinks = links.filter(link => ids.has(link.source) && ids.has(link.target));
    return json({ scope, query, focus: { id: focus, found: !!focus && ids.has(focus) }, nodes: included, links: includedLinks, matched: included.length,
      totals: { documents: included.filter(node => node.kind === "document").length, memories: included.filter(node => node.kind === "memory").length, markers: included.filter(node => node.kind === "subject" || node.kind === "topic").length, links: includedLinks.length },
      returned: { knowledge: included.filter(node => node.kind === "document" || node.kind === "memory").length, markers: included.filter(node => node.kind === "subject" || node.kind === "topic").length, links: includedLinks.length },
      omitted: { nodes: 0, links: 0 }, eligible: { nodes: included.length, links: includedLinks.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false } satisfies Snapshot);
  }
  if (url.pathname === "/api/context/read") {
    const node = nodes.find(node => node.context_path === url.searchParams.get("path"));
    return json({ metadata: { scope: "personal", path: node?.context_path, revision: 1, content_digest: "synthetic", byte_len: 200 }, title: node?.title,
      content: `# ${node?.title ?? "합성 원문"}\n\n이 화면은 지도와 모션을 확인하기 위한 합성 자료입니다.\n\n## 탐색의 흐름\n\n자료를 읽고 연결을 따라간 뒤 같은 지도로 돌아옵니다.\n\n- 자료를 선택하면 별을 가까이에서 확인합니다.\n- 묶음을 펼치고 접거나 키보드로 탐색합니다.\n- 직접 배치한 위치는 세션 동안 유지합니다.` });
  }
  if (url.pathname === "/api/context/history") return json({ items: [], next_before: null });
  if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
  if (url.pathname === "/api/brain" && typeof options?.body === "string" && JSON.parse(options.body).op === "subjects") return json({ items: [], next_after: null });
  return json({ error: "합성 미리보기에서는 자료를 저장하지 않습니다." }, 403);
};
createRoot(document.getElementById("root")!).render(<App />);

// Exercise the reported dense scene through the actual App and renderer. Pure
// placement tests cannot detect an animation interrupted by a changing status row.
if (new URLSearchParams(window.location.search).has("check")) void (async () => {
  const notice = document.querySelector<HTMLElement>(".fixture-notice")!;
  type Frame = { stars: { id: string; px: number; py: number; retiring: boolean; opacity: number }[]; camera: { x: number; y: number; z: number }; width: number; height: number };
  const frame = () => JSON.parse(notice.dataset.frame!) as Frame;
  const assert = (value: unknown, message: string) => { if (!value) throw new Error(message); };
  const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
  const settled = async () => {
    let stable = 0, previous = "";
    for (let i = 0; i < 100; i++) {
      await wait(80);
      if (!notice.dataset.frame) continue;
      const current = JSON.stringify(frame());
      stable = !positions?.layoutMoving && current === previous ? stable + 1 : 0;
      if (stable >= 3) return;
      previous = current;
    }
    throw new Error("지도 전환이 정착하지 않았습니다.");
  };
  const button = (text: string) => {
    const element = [...document.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent?.trim() === text || node.getAttribute("aria-label") === text);
    assert(element && !element.disabled, `조작 버튼: ${text}`); return element!;
  };
  const labels = () => [...document.querySelectorAll<HTMLElement>(".node-label:not([hidden])")];
  const names = () => labels().filter(node => node.dataset.nodeId!.startsWith("preview-purpose-")).map(node => [node.dataset.nodeId, node.style.left, node.style.top]);
  const members = (group: number) => frame().stars.filter(star => star.id.startsWith(`preview-${group}-`) && !star.retiring);
  const inspect = (group: number) => {
    const f = frame(), stars = members(group), origin = document.querySelector(".graph-canvas")!.getBoundingClientRect();
    const controls = [...document.querySelectorAll<HTMLElement>(".graph-core-actions, .graph-minimap")].map(element => element.getBoundingClientRect());
    assert(stars.length === 12, "쪽 보기에는 구성원 12개가 남아야 합니다.");
    for (const star of stars) {
      assert(star.px >= 8 && star.px <= f.width - 8 && star.py >= 8 && star.py <= f.height - 8, `${star.id}: 화면 밖`);
      assert(controls.every(box => star.px + origin.left < box.left - 6 || star.px + origin.left > box.right + 6 || star.py + origin.top < box.top - 6 || star.py + origin.top > box.bottom + 6), `${star.id}: 조작 영역 뒤`);
    }
    for (const label of labels()) {
      const box = label.getBoundingClientRect();
      assert(box.left >= origin.left - 1 && box.right <= origin.right + 1 && box.top >= origin.top - 1 && box.bottom <= origin.bottom + 1, "제목이 화면 경계에서 잘렸습니다.");
    }
    return { members: stars.length, titles: labels().length,
      width: Math.round(Math.max(...stars.map(star => star.px)) - Math.min(...stars.map(star => star.px))),
      height: Math.round(Math.max(...stars.map(star => star.py)) - Math.min(...stars.map(star => star.py))) };
  };
  const results: unknown[] = [];
  try {
    await settled();
    const baseline = JSON.stringify(names());
    const canvas = document.querySelector<HTMLCanvasElement>(".graph-canvas canvas")!, rect = canvas.getBoundingClientRect();
    for (const id of ["preview-single-3", "preview-single-11"]) {
      const star = frame().stars.find(star => star.id === id)!;
      canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse", clientX: rect.left + star.px, clientY: rect.top + star.py }));
      await wait(240);
      assert(document.querySelector(".graph-tooltip"), "실제 렌더러의 가리키기 반응이 없습니다.");
      assert(JSON.stringify(names()) === baseline, "가리키기로 묶음 이름이 이동했습니다.");
      canvas.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, relatedTarget: document.body }));
      await wait(240);
      assert(JSON.stringify(names()) === baseline, "가리키기 종료 뒤 이름이 복귀하지 않았습니다.");
    }
    results.push({ hoverRestoresNames: true });
    for (const group of [0, 1]) {
      document.querySelector<HTMLButtonElement>(`.graph-group-target[data-node-id="preview-purpose-${group}"]`)!.click();
      await settled(); results.push({ group, page: 1, ...inspect(group) });
      const hovered = document.querySelector<HTMLElement>(".graph-tooltip")?.dataset.nodeId;
      assert(frame().stars.filter(star => !star.id.startsWith(`preview-${group}-`) && star.id !== `preview-purpose-${group}` && !star.retiring && star.id !== hovered).every(star => star.opacity <= .13), "주변 별이 펼친 구성원과 구분되지 않습니다.");
      button("다음").click(); await settled(); results.push({ group, page: 2, ...inspect(group) });
      if (group === 0) {
        button("전체 보기").click(); await settled();
        assert(members(0).length === 225, "전체 보기에서 구성원이 누락됐습니다.");
        const lengths = labels().map(label => +label.style.getPropertyValue("--leader-length").replace("px", ""));
        assert(lengths.every(length => length <= 38), "전체 보기의 제목 지시선이 너무 깁니다.");
        results.push({ fullMembers: 225, titles: labels().length, longestGuide: Math.round(Math.max(0, ...lengths)) });
      }
      button("묶음 접기").click(); await settled();
    }
    // Filter entry is the same non-hover state on each pass, including on a
    // desktop browser resized to mobile. Touch clears the retained mouse hit.
    const surface = document.querySelector<HTMLElement>(".graph-canvas")!;
    surface.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "touch", pointerId: 99, button: 0 }));
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "touch", pointerId: 99, button: 0 }));
    let reference = "", referenceLabels = "";
    for (let repeat = 0; repeat < 4; repeat++) {
      button("필터·묶음").click(); await wait(40);
      [...document.querySelectorAll<HTMLButtonElement>(".cluster-list button")].find(node => node.querySelector("strong")?.textContent === "개발과 품질")!.click();
      await settled();
      const coordinates = members(2).map(star => [star.id, Math.round(star.px * 10), Math.round(star.py * 10)]);
      const current = JSON.stringify(coordinates);
      assert(!reference || current === reference, "같은 필터 재진입의 구도가 달라졌습니다."); reference = current;
      const titlePositions = JSON.stringify(labels().map(label => [label.dataset.nodeId,
        ...[label.style.left, label.style.top, ...["x", "y", "length", "angle"].map(key => label.style.getPropertyValue(`--leader-${key}`))].map(value => Math.round(Number.parseFloat(value) * 10)),
        label.classList.contains("routed-label") ? label.querySelector("polyline")!.getAttribute("points")!.split(/[ ,]/).map(value => Math.round(Number(value) * 10)) : []]).sort());
      assert(!referenceLabels || titlePositions === referenceLabels, "같은 필터 재진입의 제목·지시선이 달라졌습니다."); referenceLabels = titlePositions;
      button("묶음 해제").click(); await settled();
    }
    results.push({ repeatedFilterEntries: 4, sameProjection: true, sameLabelsAndGuides: true });
    const input = document.querySelector<HTMLInputElement>("#graph-search")!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "검색결과없음");
    input.dispatchEvent(new Event("input", { bubbles: true })); await wait(40);
    document.querySelector<HTMLButtonElement>(".map-search button")!.click(); await wait(160);
    assert(document.querySelector(".map-empty") && !document.querySelector(".active-filters"), "빈 화면에 중복 초기화 동작이 남았습니다.");
    assert(!document.querySelector(".map-status")!.textContent!.includes("일치 구절"), "검색 0건에 일치 구절 안내가 남았습니다.");
    button("검색·조건 초기화").click(); await settled();
    results.push({ emptySearch: true });
    notice.textContent = `PASS · ${innerWidth}px · ${JSON.stringify(results)}`;
    notice.dataset.result = JSON.stringify({ passed: true, results });
  } catch (error) {
    notice.textContent = `FAIL · ${error instanceof Error ? error.message : String(error)}`;
    notice.dataset.result = JSON.stringify({ passed: false, results, error: notice.textContent });
  }
})();
