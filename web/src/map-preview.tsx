import { createRoot } from "react-dom/client";
import { Group, Line, Scene, Vector3 } from "three";
import App from "./App";
import type { GraphNode, Scope, Snapshot } from "./graph";
import { Positions } from "./positions";
import { COLLISION_GAP } from "./clearance";
import { leaderHitsBox, nodeScreenSize, nodeVisualRadius, type LabelLeader } from "./presentation";
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
  const stars: { id: string; x: number; y: number; z: number; px: number; py: number; radius: number; opacity: number; retiring: boolean }[] = [];
  const relations: (LabelLeader & { opacity: number })[] = [];
  this.traverse(object => {
    if (object instanceof Line && object.visible && !Array.isArray(object.material) && object.material.opacity > 0) {
      const points = object.geometry.getAttribute("position");
      if (points?.count === 2) {
        const a = new Vector3().fromBufferAttribute(points, 0).applyMatrix4(object.matrixWorld).project(camera);
        const b = new Vector3().fromBufferAttribute(points, 1).applyMatrix4(object.matrixWorld).project(camera);
        if (Math.abs(a.z) <= 1 && Math.abs(b.z) <= 1) {
          const x1 = (a.x + 1) * rect.width / 2, y1 = (1 - a.y) * rect.height / 2;
          const x2 = (b.x + 1) * rect.width / 2, y2 = (1 - b.y) * rect.height / 2;
          relations.push({ x1, y1, x2, y2, length: Math.hypot(x2 - x1, y2 - y1), opacity: +object.material.opacity.toFixed(4) });
        }
      }
    }
    if (!(object instanceof Group) || !object.userData.nodeId || !object.visible) return;
    object.getWorldPosition(world);
    const { x, y, z } = world;
    const node = nodes.find(node => node.id === object.userData.nodeId);
    const depth = -world.clone().applyMatrix4(camera.matrixWorldInverse).z;
    // Read the count-sized sprite that was actually drawn, rather than treating a summary as a 10px marker.
    const glyph = object.getObjectByName("summary-glyph");
    const radius = glyph ? glyph.scale.x * camera.projectionMatrix.elements[5] * rect.height / 4 :
      nodeVisualRadius(nodeScreenSize(node?.kind ?? "folder", depth, rect.height, camera.projectionMatrix.elements[5]), object.userData.nodeId === new URLSearchParams(location.search).get("focus"), false);
    world.project(camera);
    stars.push({ id: object.userData.nodeId, x, y, z, px: (world.x + 1) * rect.width / 2, py: (1 - world.y) * rect.height / 2, radius,
      opacity: object.userData.opacity?.() ?? 1, retiring: object.userData.retiring?.() ?? false });
  });
  const notice = document.querySelector<HTMLElement>(".fixture-notice")!;
  notice.dataset.frameSequence = String(Number(notice.dataset.frameSequence ?? 0) + 1);
  notice.dataset.frame = JSON.stringify({ stars, relations, camera: camera.position, width: rect.width, height: rect.height });
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
    const included = nodes.filter(node => !query || `${node.label} ${node.title}`.includes(query) || node.id === focus).map(node => ({ ...node, scope, excerpt: query ? `합성 자료에서 ${query} 검색어와 일치하는 내용입니다.` : undefined }));
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
const check = new URLSearchParams(window.location.search).get("check");
createRoot(document.getElementById("root")!).render(<App />);

// Exercise the reported dense scene through the actual App and renderer. Pure
// placement tests cannot detect an animation interrupted by a changing status row.
if (check !== null) void (async () => {
  const notice = document.querySelector<HTMLElement>(".fixture-notice")!;
  type Frame = { stars: { id: string; px: number; py: number; radius: number; retiring: boolean; opacity: number }[]; relations: (LabelLeader & { opacity: number })[]; camera: { x: number; y: number; z: number }; width: number; height: number };
  const frame = () => JSON.parse(notice.dataset.frame!) as Frame;
  const assert = (value: unknown, message: string) => { if (!value) throw new Error(message); };
  const wait = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
  const settled = async () => {
    let stable = 0, previous = "", sequence = "";
    // Include cold loading of the lazy renderer, then observe distinct settled
    // frames and the end of retiring bodies rather than a paused frame snapshot.
    for (let i = 0; i < 375; i++) {
      await wait(80);
      if (!notice.dataset.frame || notice.dataset.frameSequence === sequence) continue;
      sequence = notice.dataset.frameSequence!;
      const current = JSON.stringify(frame());
      stable = !positions?.layoutMoving && !frame().stars.some(star => star.retiring) && current === previous ? stable + 1 : 0;
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
  const backdrop = (protectedNames: boolean) => {
    assert(document.querySelector<HTMLElement>(".graph-canvas")!.dataset.labelBackdrop === (protectedNames ? "protected" : "clear"), "탐색 상태의 제목 배경이 다릅니다.");
    for (const label of labels().filter(label => label.classList.contains("compact") && !label.matches(".hovered, .focused, :hover, :focus-visible"))) {
      const style = getComputedStyle(label), painted = style.backgroundColor !== "rgba(0, 0, 0, 0)" || style.backgroundImage !== "none";
      assert(painted === protectedNames, "평상시 전체보기와 읽기 화면의 제목 배경이 구분되지 않습니다.");
    }
  };
  const names = () => labels().filter(node => node.dataset.nodeId!.startsWith("preview-purpose-")).map(node => [node.dataset.nodeId, node.style.left, node.style.top]);
  const members = (group: number) => frame().stars.filter(star => star.id.startsWith(`preview-${group}-`) && !star.retiring);
  const separated = (stars: Frame["stars"]) => {
    for (const [index, star] of stars.entries()) for (const other of stars.slice(index + 1))
      assert(Math.hypot(star.px - other.px, star.py - other.py) >= star.radius + other.radius + COLLISION_GAP - .01, `${star.id}/${other.id}: 최종 화면에서 별 간격 부족`);
  };
  const inspect = (group: number, expected = 12) => {
    const f = frame(), stars = members(group), origin = document.querySelector(".graph-canvas")!.getBoundingClientRect();
    const controls = [...document.querySelectorAll<HTMLElement>(".graph-core-actions, .graph-minimap")].map(element => element.getBoundingClientRect());
    assert(stars.length === expected, `보기에는 구성원 ${expected}개가 남아야 합니다.`);
    separated(stars);
    for (const star of stars) {
      assert(star.px >= 8 && star.px <= f.width - 8 && star.py >= 8 && star.py <= f.height - 8, `${star.id}: 화면 밖`);
      assert(controls.every(box => star.px + origin.left < box.left - 6 || star.px + origin.left > box.right + 6 || star.py + origin.top < box.top - 6 || star.py + origin.top > box.bottom + 6), `${star.id}: 조작 영역 뒤`);
    }
    for (const label of labels()) {
      const box = label.getBoundingClientRect();
      const gap = innerWidth <= 700 ? 8 : 0;
      assert(box.left >= origin.left + gap - .1 && box.right <= origin.right - gap + .1 && box.top >= origin.top + gap - .1 && box.bottom <= origin.bottom - gap + .1,
        `${label.dataset.nodeId}: 제목의 화면 끝 여백 ${Math.min(box.left - origin.left, origin.right - box.right, box.top - origin.top, origin.bottom - box.bottom).toFixed(2)}px`);
    }
    return { members: stars.length, titles: labels().length,
      width: Math.round(Math.max(...stars.map(star => star.px)) - Math.min(...stars.map(star => star.px))),
      height: Math.round(Math.max(...stars.map(star => star.py)) - Math.min(...stars.map(star => star.py))) };
  };
  const guidePaths = (label: HTMLElement): LabelLeader[] => {
    const origin = document.querySelector(".graph-canvas")!.getBoundingClientRect(), box = label.getBoundingClientRect();
    const x = box.left - origin.left, y = box.top - origin.top;
    const segment = (x1: number, y1: number, x2: number, y2: number) => ({ x1, y1, x2, y2, length: Math.hypot(x2 - x1, y2 - y1) });
    if (label.classList.contains("routed-label")) {
      const values = label.querySelector("polyline")!.getAttribute("points")!.split(/[ ,]/).map(Number);
      return Array.from({ length: values.length / 2 - 1 }, (_, i) => segment(x + values[i * 2], y + values[i * 2 + 1], x + values[i * 2 + 2], y + values[i * 2 + 3]));
    }
    if (!label.classList.contains("offset-label")) return [];
    const value = (key: string) => Number.parseFloat(label.style.getPropertyValue(`--leader-${key}`));
    const x1 = x + value("x"), y1 = y + value("y"), length = value("length"), angle = value("angle");
    return [segment(x1, y1, x1 + length * Math.cos(angle), y1 + length * Math.sin(angle))];
  };
  const starClearance = () => {
    const f = frame(), origin = document.querySelector(".graph-canvas")!.getBoundingClientRect();
    for (const label of labels().filter(label => !guidePaths(label).length)) {
      const box = label.getBoundingClientRect(), stars = f.stars.filter(star => !star.retiring && star.opacity >= .2);
      const own = stars.find(star => star.id === label.dataset.nodeId);
      assert(own, `무지시선 제목의 별이 없습니다: ${label.dataset.nodeId}`);
      const gap = (star: Frame["stars"][number]) => Math.hypot(star.px - Math.max(box.left - origin.left, Math.min(box.right - origin.left, star.px)),
        star.py - Math.max(box.top - origin.top, Math.min(box.bottom - origin.top, star.py))) - star.radius;
      assert(stars.every(star => star.id === own!.id || gap(star) + .1 >= gap(own!)), `지시선 없는 제목이 다른 별에 더 가깝습니다: ${label.dataset.nodeId}`);
    }
    const crossings = labels().flatMap(label => guidePaths(label).flatMap(line => frame().stars.filter(star => {
      if (star.id === label.dataset.nodeId || star.retiring || star.opacity < .2) return false;
      const dx = line.x2 - line.x1, dy = line.y2 - line.y1;
      const t = Math.max(0, Math.min(1, ((star.px - line.x1) * dx + (star.py - line.y1) * dy) / Math.max(.001, dx * dx + dy * dy)));
      return Math.hypot(star.px - line.x1 - t * dx, star.py - line.y1 - t * dy) < star.radius - .1;
    }).map(star => ({ title: label.dataset.nodeId, star: star.id }))));
    assert(!crossings.length, `제목 지시선이 다른 별을 관통합니다: ${JSON.stringify(crossings)}`);
    return crossings.length;
  };
  const results: unknown[] = [];
  const checkTransitions = async () => {
    // Folder/view changes and a sidebar choice can arrive before the previous
    // camera frame runs. The choice must still disclose and frame its members.
    for (const delay of check === "transitions-settled" ? [-1] : [0, 24, 96]) {
      const pause = () => delay < 0 ? settled() : wait(delay);
      if (delay === 96) {
        document.querySelector<HTMLButtonElement>('.graph-group-target[data-node-id="preview-purpose-0"]')!.click();
        await settled();
        assert(document.querySelector(".graph-core-actions"), "묶음 기준 전환 전에 펼침을 준비하지 못했습니다.");
      }
      button("필터·묶음").click(); await wait(0);
      const folderToggle = [...document.querySelectorAll<HTMLInputElement>(".map-sidebar input")].find(input => input.closest("label")?.textContent?.includes("폴더 연결 표시"))!;
      if (!folderToggle.checked) folderToggle.click();
      if (delay < 0) await settled();
      button("닫기").click(); await wait(0);
      const settings = document.querySelector<HTMLDetailsElement>(".view-options")!;
      settings.querySelector("summary")!.click();
      const view = settings.querySelector<HTMLSelectElement>('select[aria-label="묶음 기준"]')!;
      view.value = "relationships"; view.dispatchEvent(new Event("change", { bubbles: true })); await pause();
      view.value = "purpose"; view.dispatchEvent(new Event("change", { bubbles: true })); await pause();
      const connections = [...settings.querySelectorAll<HTMLInputElement>("input")].find(input => input.closest("label")?.textContent?.includes("연결선 모두 보기"))!;
      if (!connections.checked) connections.click();
      if (delay < 0) await settled();
      settings.querySelector("summary")!.click();
      button("필터·묶음").click(); await wait(0);
      [...document.querySelectorAll<HTMLInputElement>(".map-sidebar input")].find(input => input.closest("label")?.textContent?.includes("폴더 연결 표시"))!.click(); await pause();
      [...document.querySelectorAll<HTMLButtonElement>(".cluster-list button")].find(node => node.querySelector("strong")?.textContent === nodes.find(node => node.id === "preview-purpose-1")!.label)!.click();
      await settled();
      assert(document.querySelector(".graph-core-actions"), "표시 전환 뒤 필터의 자동 펼침이 사라졌습니다.");
      results.push({ filteredTransitionDelay: delay, page: 1, ...inspect(1) });
      button("다음").click(); await settled();
      results.push({ filteredTransitionDelay: delay, page: 2, ...inspect(1) });
      if (delay === 0) {
        button("묶음 접기").click(); await settled();
        assert(!document.querySelector(".graph-core-actions"), "필터 묶음 접기가 유지되지 않습니다.");
        button("필터·묶음").click(); await wait(0);
        [...document.querySelectorAll<HTMLButtonElement>(".cluster-list button")].find(node => node.querySelector("strong")?.textContent === nodes.find(node => node.id === "preview-purpose-1")!.label)!.click();
        await settled(); results.push({ filteredCloseAndReopen: true, ...inspect(1) });
      }
      button("묶음 해제").click(); await settled();
      settings.querySelector("summary")!.click(); connections.click(); settings.querySelector("summary")!.click(); await settled();
    }
    const minimapToggle = document.querySelector<HTMLButtonElement>('button[aria-label="미니맵 펼치기"]');
    minimapToggle?.click(); if (minimapToggle) await settled();
    const fit = button("전체 맞춤"); fit.focus(); fit.click(); await settled();
    assert(document.activeElement === fit && !document.querySelector(".graph-core-actions"), "전체 맞춤의 초점이나 접힌 전체 보기 복귀가 유지되지 않습니다.");
    if (minimapToggle) { button("미니맵 접기").click(); await settled(); }
    results.push({ fitOverviewRestored: true, fitFocusPreserved: true });
  };
  try {
    await settled();
    if (check === "readable-bounds") {
      const coverage = () => {
        const shown = new Set(labels().map(label => label.dataset.nodeId));
        for (let group = 0; group < 5; group++) {
          const id = `preview-purpose-${group}`;
          const pinned = document.querySelector<HTMLElement>(`.graph-group-target.core-name[data-node-id="${id}"]`);
          assert(shown.has(id) || !!pinned, `필수 묶음 이름 누락: ${id}`);
        }
      };
      const bodiesClear = () => {
        const origin = document.querySelector(".graph-canvas")!.getBoundingClientRect();
        const obstacles = [...document.querySelectorAll<HTMLElement>(".graph-minimap,.graph-quick-help,.graph-group-navigation.with-keys,.graph-core-actions")]
          .filter(element => element.offsetWidth && element.offsetHeight).map(element => element.getBoundingClientRect());
        for (const star of frame().stars.filter(star => !star.retiring && star.opacity > .99)) {
          const x = star.px + origin.left, y = star.py + origin.top;
          assert(x-star.radius >= origin.left && x+star.radius <= origin.right && y-star.radius >= origin.top && y+star.radius <= origin.bottom, `${star.id}: 별 표시 영역이 화면 밖`);
          assert(obstacles.every(box => Math.hypot(x-Math.max(box.left,Math.min(box.right,x)), y-Math.max(box.top,Math.min(box.bottom,y))) >= star.radius-.1), `${star.id}: 실제 별 반지름이 조작 영역에 가림`);
        }
      };
      coverage(); bodiesClear(); starClearance();
      results.push({ phase: "overview", requiredGroupIds: [0,1,2,3,4], actualRadiusClearance: true });
      for (const group of [0,1]) {
        button("목록 보기").click(); await wait(0);
        [...document.querySelectorAll<HTMLButtonElement>(".graph-list>button")].find(row => row.querySelector("strong")?.textContent === nodes.find(node => node.id === `preview-purpose-${group}`)!.label)!.click();
        button("3D 보기").click(); await settled();
        if (innerWidth <= 900) { button("관리 패널 닫기").click(); await settled(); document.querySelector<HTMLButtonElement>(`.graph-group-target[data-node-id="preview-purpose-${group}"]`)!.click(); await settled(); }
        const readable = () => {
          inspect(group); bodiesClear(); starClearance();
          const titles = labels().filter(label => label.dataset.nodeId?.startsWith(`preview-${group}-`));
          assert(titles.length >= 2, "읽기 프레임의 제목이 모두 보류되거나 한 개만 남음");
          const separator = document.querySelector<HTMLElement>(".graph-core-separator")!;
          const chrome = document.querySelector(".graph-chrome")!.getBoundingClientRect();
          if (chrome.width <= 520) assert(getComputedStyle(separator).display === "none", "좁은 실제 너비에 가운뎃점이 고립됨");
          return titles.length;
        };
        let titleCount = readable();
        button("다음").click(); await settled(); titleCount = Math.min(titleCount, readable());
        button("필터·묶음").click(); await wait(100); await settled();
        const filter = document.querySelector(".map-sidebar")!;
        if (filter.getAttribute("aria-modal") === "true") {
          assert(document.querySelector(".galaxy")!.hasAttribute("inert"), "필터 모달 뒤 지도 입력이 열림");
          assert(filter.contains(document.activeElement), "필터 모달로 초점이 옮겨지지 않음");
          if (innerWidth > 900) assert(document.querySelector(".management-panel")!.hasAttribute("inert"), "필터 모달 뒤 상세 입력이 열림");
        } else titleCount = Math.min(titleCount, readable());
        button("탐색 조건 닫기").click(); await settled(); titleCount = Math.min(titleCount, readable());
        button("묶음 접기").click(); await settled(); coverage();
        results.push({ group, longTitle: group === 1, pages: [1,2], minimumVisibleTitles: titleCount, foldedNameCoverage: true });
      }
      notice.textContent = `PASS · ${innerWidth}px · 이름·긴 제목·실제 별 가림 검사`;
      notice.dataset.result = JSON.stringify({ passed: true, results }); return;
    }
    if (check === "activation" || check === "activation-extra" || check === "interface") {
      const canvas = document.querySelector<HTMLCanvasElement>(".graph-canvas canvas")!;
      const capture = canvas.setPointerCapture, releaseCapture = canvas.releasePointerCapture;
      // Synthetic pointers are not registered with the browser capture service.
      // Only this test ID bypasses that service; production handlers and ray picking run unchanged.
      if (check?.startsWith("activation")) {
      canvas.setPointerCapture = function (id) { if (id < 9100) capture.call(this, id); };
      canvas.releasePointerCapture = function (id) { if (id < 9100) releaseCapture.call(this, id); };
      }
      const point = () => {
        const star = frame().stars.find(star => star.id === "preview-purpose-0")!;
        const box = canvas.getBoundingClientRect();
        return { x: box.left + star.px, y: box.top + star.py };
      };
      const pointer = (type: string, x: number, y: number, id = 9101, pointerType = "mouse") => canvas.dispatchEvent(new PointerEvent(type,
        { bubbles: true, pointerId: id, pointerType, button: 0, buttons: type === "pointerup" || type === "pointercancel" ? 0 : 1, clientX: x, clientY: y }));
      const unchanged = () => assert(!document.querySelector(".graph-core-actions, .management-panel"), "취소하거나 끈 입력이 자료를 활성화했습니다.");
      if (check?.startsWith("activation")) try {
        for (const hoverMs of check === "activation-extra" ? [] : [0, 500]) {
          const p = point(); pointer("pointermove", p.x, p.y); await wait(hoverMs);
          pointer("pointerdown", p.x, p.y); pointer("pointerup", p.x, p.y);
          await settled(); await wait(450);
          assert(document.querySelector(".graph-core-actions") && !document.querySelector(".management-panel"), `hover ${hoverMs}ms 뒤 묶음 클릭이 한 번의 펼침으로 끝나지 않았습니다.`);
          inspect(0);
          button("묶음 접기").click(); await settled();
          results.push({ canvasClickAfterHoverMs: hoverMs, expandsOnce: true });
        }
        for (const ending of check === "activation-extra" ? ["multitouch-jitter", "multitouch-drag"] : ["pointercancel", "blur", "roundtrip", "multitouch"]) {
          const p = point(); pointer("pointermove", p.x, p.y); await wait(160);
          pointer("pointerdown", p.x, p.y);
          if (ending.startsWith("multitouch-")) pointer("pointermove", p.x + (ending === "multitouch-jitter" ? 3 : 9), p.y);
          if (ending === "roundtrip") { pointer("pointermove", p.x + 9, p.y); pointer("pointermove", p.x, p.y); }
          else if (ending.startsWith("multitouch")) { pointer("pointerdown", p.x + 20, p.y, 9102, "touch"); pointer("pointerup", p.x + 20, p.y, 9102, "touch"); }
          else if (ending === "blur") window.dispatchEvent(new Event("blur"));
          else pointer("pointercancel", p.x, p.y);
          pointer("pointerup", p.x, p.y); await settled(); await wait(450); unchanged();
          assert(!(positions as Positions | null)?.dragging, "다중 포인터 해제 뒤 끌기 상태가 남았습니다.");
          results.push({ ending, accidentalActivation: false });
        }
        if (check === "activation-extra") {
          const p = point(); pointer("pointerdown", p.x, p.y); pointer("pointermove", p.x + 2, p.y); pointer("pointerup", p.x + 2, p.y);
          await settled();
          assert(document.querySelector(".graph-core-actions") && !document.querySelector(".management-panel"), "다중 포인터 종료 뒤 작은 흔들림을 동반한 클릭이 작동하지 않습니다.");
          button("묶음 접기").click(); await settled();
          results.push({ clickAfterMultitouch: true, smallJitterTap: true });
        }
      } finally { canvas.setPointerCapture = capture; canvas.releasePointerCapture = releaseCapture; }
      const before = canvas.getBoundingClientRect();
      button("필터·묶음").click(); await wait(40);
      const sidebar = document.querySelector(".map-sidebar")!.getBoundingClientRect(), after = canvas.getBoundingClientRect();
      assert(before.width === after.width && before.height === after.height, "필터 열기로 캔버스 크기가 바뀌었습니다.");
      if (innerWidth > 900) for (const selector of [".map-toolbar", ".map-status", ".map-legend"]) {
        const box = document.querySelector(selector)!.getBoundingClientRect();
        assert(box.height === 0 || sidebar.bottom <= box.top + .1 || sidebar.top >= box.bottom - .1, `${selector}: 필터가 결과 문맥을 덮었습니다.`);
      }
      button("탐색 조건 닫기").click(); await wait(40);
      button("목록 보기").click(); await wait(40);
      const rows = [...document.querySelectorAll<HTMLButtonElement>(".graph-list > button")];
      const sampleRow = rows.find(row => row.querySelector(".node-symbol i"))!;
      assert(sampleRow && sampleRow.getBoundingClientRect().height < 95 && sampleRow.getBoundingClientRect().height >= 44, "목록 행 밀도나 조작 높이가 맞지 않습니다.");
      const dot = sampleRow.querySelector(".node-symbol i")!.getBoundingClientRect();
      assert(dot.width >= 8 && dot.height >= 8, "목적 색 점이 작습니다.");
      rows.find(row => row.querySelector("strong")?.textContent === "제품 전략과 의사결정")!.click(); await wait(80);
      const connectionSection = [...document.querySelectorAll<HTMLElement>(".marker-connections section")].find(section => section.querySelector("h3")?.textContent?.startsWith("구성원"));
      assert(connectionSection?.querySelectorAll("li").length === 225, "목적의 전체 구성원을 읽을 수 없습니다.");
      assert(connectionSection && !connectionSection.querySelector(".compact-list") && getComputedStyle(connectionSection.querySelector("ul")!).overflowY === "visible", "구성원 안에 별도 스크롤 목록이 남습니다.");
      assert([...connectionSection!.querySelectorAll("button")].every(row => !row.querySelector("span")), "구성원 행마다 같은 관계 라벨이 반복됩니다.");
      results.push({ filterContextUncovered: true, canvasSizePreserved: true, memberRows: 225, nestedMemberScroll: false, listRowHeight: sampleRow.getBoundingClientRect().height });
      notice.textContent = `PASS · ${innerWidth}px · 입력과 화면 구조 검사`;
      notice.dataset.result = JSON.stringify({ passed: true, results }); return;
    }
    if (check?.startsWith("transitions")) {
      await checkTransitions();
      notice.textContent = `PASS · ${innerWidth}px · 합성 자료 검사`;
      notice.dataset.result = JSON.stringify({ passed: true, results }); return;
    }
    backdrop(false);
    const hoverRules: string[] = [];
    const inspectRules = (rules: CSSRuleList, conditions: string[] = []) => {
      for (const rule of rules) {
        if (rule instanceof CSSMediaRule) inspectRules(rule.cssRules, [...conditions, rule.conditionText]);
        else if (rule instanceof CSSStyleRule && rule.selectorText.includes(":hover")) {
          assert(conditions.some(condition => condition.includes("hover: hover") && condition.includes("pointer: fine")), `터치에도 hover 장식이 적용됩니다: ${rule.selectorText}`);
          hoverRules.push(rule.selectorText);
        }
      }
    };
    for (const sheet of document.styleSheets) inspectRules(sheet.cssRules);
    assert(hoverRules.length > 0, "hover 조건 검사가 실제 스타일을 읽지 못했습니다.");
    results.push({ pointerConditionalHoverRules: hoverRules.length, physicalTouch: "unverified" });
    const origin = document.querySelector(".graph-canvas")!.getBoundingClientRect();
    const overviewCrossings = labels().flatMap(label => {
      const box = label.getBoundingClientRect();
      return frame().relations.filter(line => leaderHitsBox(line, { left: box.left - origin.left, top: box.top - origin.top, right: box.right - origin.left, bottom: box.bottom - origin.top }, 0));
    }).length;
    assert(overviewCrossings === 0, "첫 화면의 관계선이 투명한 제목을 지나갑니다.");
    const overviewLeaderStarCrossings = starClearance();
    for (let group = 0; group < 5; group++) {
      const id = `preview-purpose-${group}`;
      assert(names().some(([named]) => named === id) || !!document.querySelector(`.graph-group-target.core-name[data-node-id="${id}"]`), `주요 묶음 이름 누락: ${id}`);
    }
    results.push({ overviewRelationTitleCrossings: overviewCrossings, overviewLeaderStarCrossings, overviewNames: names().length });
    if (new URLSearchParams(location.search).get("check") === "overview") {
      notice.textContent = `PASS · ${innerWidth}px · 합성 자료 검사`;
      notice.dataset.result = JSON.stringify({ passed: true, results }); return;
    }
    await checkTransitions();
    const options = document.querySelector<HTMLDetailsElement>(".view-options")!;
    options.querySelector("summary")!.click();
    const connections = [...options.querySelectorAll<HTMLInputElement>("input[type=checkbox]")].find(input => input.closest("label")!.textContent!.includes("연결선 모두 보기"))!;
    connections.click(); await settled(); backdrop(true);
    connections.click(); options.querySelector("summary")!.click(); await settled(); backdrop(false);
    const keyboardTarget = document.querySelector<HTMLButtonElement>(".graph-group-target[data-node-id=preview-purpose-0]")!;
    keyboardTarget.focus(); await wait(240);
    const focusedName = document.querySelector<HTMLElement>(".node-label.focused:not([hidden])") ?? (keyboardTarget.classList.contains("core-name") ? keyboardTarget : null);
    assert(focusedName && (getComputedStyle(focusedName).backgroundColor !== "rgba(0, 0, 0, 0)" || getComputedStyle(focusedName).backgroundImage !== "none"), "키보드 초점 제목의 보호 배경이 없습니다.");
    keyboardTarget.blur(); await settled(); backdrop(false);
    results.push({ overviewBackdrop: "clear", allConnectionsBackdrop: "protected", keyboardFocusBackdrop: "protected" });
    const baseline = JSON.stringify(names());
    const canvas = document.querySelector<HTMLCanvasElement>(".graph-canvas canvas")!, rect = canvas.getBoundingClientRect();
    const hoverTargets = frame().stars.filter(star => star.id.startsWith("preview-single-") && !star.retiring &&
      star.px > 8 && star.px < rect.width - 8 && star.py > 80 && star.py < rect.height - 40).slice(0, 2);
    assert(hoverTargets.length === 2, "가리키기를 검사할 실제 별 두 개가 필요합니다.");
    for (const star of hoverTargets) {
      canvas.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerType: "mouse", clientX: rect.left + star.px, clientY: rect.top + star.py }));
      await wait(240);
      assert(document.querySelector(".graph-tooltip"), "실제 렌더러의 가리키기 반응이 없습니다.");
      const hoveredLabel = document.querySelector<HTMLElement>(".node-label.hovered");
      if (hoveredLabel) assert(getComputedStyle(hoveredLabel).backgroundImage !== "none" || getComputedStyle(hoveredLabel).backgroundColor !== "rgba(0, 0, 0, 0)", "가리킨 제목의 보호 배경이 없습니다.");
      assert(JSON.stringify(names()) === baseline, "가리키기로 묶음 이름이 이동했습니다.");
      canvas.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, relatedTarget: document.body }));
      await wait(240);
      assert(!document.querySelector(".graph-tooltip"), "지도 밖에 툴팁이 남았습니다.");
      assert(JSON.stringify(names()) === baseline, "가리키기 종료 뒤 이름이 복귀하지 않았습니다.");
    }
    results.push({ hoverRestoresNames: true });
    for (const group of [0, 1]) {
      document.querySelector<HTMLButtonElement>(`.graph-group-target[data-node-id="preview-purpose-${group}"]`)!.click();
      await settled(); backdrop(true); starClearance(); results.push({ group, page: 1, ...inspect(group) });
      const title = document.querySelector<HTMLElement>(".graph-core-title")!, count = title.querySelector<HTMLElement>(".graph-core-count")!;
      assert(count.getClientRects().length === 1 && !count.textContent!.includes("·"), "묶음 개수가 나뉘거나 구분점이 고립됩니다.");
      if (innerWidth <= 700) assert(getComputedStyle(title.querySelector(".graph-core-separator")!).display === "none", "모바일 개수 줄에 구분점이 남았습니다.");
      const background = frame().stars.filter(star => star.opacity <= .13 && !star.retiring);
      const endpoint = (x: number, y: number) => background.some(star => Math.hypot(star.px - x, star.py - y) < 1);
      const backgroundEdges = frame().relations.filter(line => endpoint(line.x1, line.y1) && endpoint(line.x2, line.y2));
      assert(backgroundEdges.length > 0 && backgroundEdges.every(line => line.opacity <= .061), "흐린 배경 별 사이의 선이 원래 밝기로 남았습니다.");
      results.push({ group, backgroundEdges: backgroundEdges.length, maxBackgroundEdgeOpacity: Math.max(...backgroundEdges.map(line => line.opacity)) });
      const hovered = document.querySelector<HTMLElement>(".graph-tooltip")?.dataset.nodeId;
      assert(frame().stars.filter(star => !star.id.startsWith(`preview-${group}-`) && star.id !== `preview-purpose-${group}` && !star.retiring && star.id !== hovered).every(star => star.opacity <= .13), "주변 별이 펼친 구성원과 구분되지 않습니다.");
      button("다음").click(); await settled(); results.push({ group, page: 2, ...inspect(group) });
      if (group === 1 && innerWidth <= 700) {
        button("미니맵 펼치기").click(); await settled();
        results.push({ group, page: 2, expandedMinimap: true, ...inspect(group) });
        button("이전").click(); await settled();
        results.push({ group, page: 1, expandedMinimap: true, ...inspect(group) });
        button("다음").click(); await settled();
        results.push({ group, page: 2, expandedMinimapBeforePaging: true, ...inspect(group) });
        button("미니맵 접기").click(); await settled();
      }
      if (group === 0) {
        const mode = button("전체 보기"); mode.focus(); mode.click(); await settled();
        assert(document.activeElement === mode && mode === button("나눠 보기"), "전체 보기 전환에서 키보드 초점이 사라졌습니다.");
        inspect(0, 225);
        const lengths = labels().map(label => +label.style.getPropertyValue("--leader-length").replace("px", ""));
        assert(lengths.every(length => length <= 38), "전체 보기의 제목 지시선이 너무 깁니다.");
        results.push({ fullMembers: 225, titles: labels().length, longestGuide: Math.round(Math.max(0, ...lengths)) });
        mode.click(); await settled();
        assert(document.activeElement === mode && mode === button("전체 보기"), "나눠 보기 전환에서 키보드 초점이 사라졌습니다.");
        results.push({ modeToggleFocusPreserved: true });
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
      const fields = [...document.querySelectorAll<HTMLElement>(".filter-row select")].map(field => [field.getBoundingClientRect().height, getComputedStyle(field).fontSize, getComputedStyle(field.parentElement!).fontSize, getComputedStyle(field.parentElement!).color]);
      assert(fields.length === 3 && fields.every(field => JSON.stringify(field) === JSON.stringify(fields[0])) && Number(fields[0][0]) >= 44, "지도 선택 필드의 높이·글자·라벨 규격이 다릅니다.");
      [...document.querySelectorAll<HTMLButtonElement>(".cluster-list button")].find(node => node.querySelector("strong")?.textContent === "개발과 품질")!.click();
      await settled();
      backdrop(true);
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
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "사용자의 맥락");
    input.dispatchEvent(new Event("input", { bubbles: true })); await wait(40);
    document.querySelector<HTMLButtonElement>(".map-search button")!.click(); await settled(); backdrop(true);
    assert(!document.querySelector(".map-status")!.textContent!.includes("일치 구절"), "발췌가 없는 3D 보기에 발췌 안내가 남았습니다.");
    const listAction = button("목록 보기");
    assert(!listAction.hasAttribute("aria-pressed"), "목록으로 바꾸는 동작이 현재 선택으로 표시됩니다.");
    listAction.click(); await wait(80);
    assert(document.querySelector(".search-excerpt") && document.querySelector(".map-status")!.textContent!.includes("일치 구절"), "목록의 실제 발췌와 안내가 일치하지 않습니다.");
    assert(!button("3D 보기").hasAttribute("aria-pressed"), "3D로 바꾸는 동작이 현재 선택으로 표시됩니다.");
    button("3D 보기").click(); await settled();
    assert(!document.querySelector(".map-status")!.textContent!.includes("일치 구절"), "3D로 돌아온 뒤 발췌 안내가 남았습니다.");
    button("검색 해제").click(); await settled(); backdrop(false);
    results.push({ searchBackdrop: "protected", clearedSearchBackdrop: "clear", viewActionSemantics: true, excerptHintMatchesView: true });
    const emptyInput = document.querySelector<HTMLInputElement>("#graph-search")!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(emptyInput, "검색결과없음");
    emptyInput.dispatchEvent(new Event("input", { bubbles: true })); await wait(40);
    document.querySelector<HTMLButtonElement>(".map-search button")!.click(); await wait(160);
    assert(document.querySelector(".map-empty") && !document.querySelector(".active-filters"), "빈 화면에 중복 초기화 동작이 남았습니다.");
    assert(!document.querySelector(".map-status")!.textContent!.includes("일치 구절"), "검색 0건에 일치 구절 안내가 남았습니다.");
    assert(!document.querySelector(".map-legend > span"), "빈 검색 화면에 지도 도형 범례가 남았습니다.");
    assert(document.querySelector(".map-legend details"), "빈 검색에서도 관계·상태 설명은 읽을 수 있어야 합니다.");
    button("검색·조건 초기화").click(); await settled();
    results.push({ emptySearch: true, emptyVisualLegend: false });
    // Source-only wrapping tests cannot observe the actual remaining width beside a detail panel.
    button("목록 보기").click(); await wait(40);
    [...document.querySelectorAll<HTMLButtonElement>(".graph-list button")].find(node => node.querySelector("strong")?.textContent === "개발과 품질")!.click(); await wait(160);
    const units = [...document.querySelectorAll<HTMLElement>(".map-title .count-breakdown > span")];
    assert(units.length >= 4 && units.every(unit => {
      const rects = [...unit.getClientRects()];
      return rects.length > 0 && rects.every(rect => Math.abs(rect.top - rects[0].top) < 1);
    }), "종류 이름과 숫자가 다른 줄로 나뉩니다.");
    const actions = [...document.querySelectorAll<HTMLElement>(".map-action-buttons > button, .map-action-buttons > details")].map(node => node.getBoundingClientRect());
    assert(actions.length === 3 && actions.every(box => Math.abs(box.top - actions[0].top) < 1), "지도 동작 버튼 하나가 별도 줄로 밀립니다.");
    const toolbar = document.querySelector(".map-toolbar")!.getBoundingClientRect();
    assert(actions.every(box => box.left >= toolbar.left && box.right <= toolbar.right), "지도 동작이 화면 밖으로 나갑니다.");
    results.push({ listAndDetail: true, atomicCounts: true, actionRow: true });
    notice.textContent = `PASS · ${innerWidth}px · 합성 자료 검사`;
    notice.dataset.result = JSON.stringify({ passed: true, results });
  } catch (error) {
    notice.textContent = `FAIL · ${error instanceof Error ? error.message : String(error)}`;
    notice.dataset.result = JSON.stringify({ passed: false, results, error: notice.textContent });
  }
})();
