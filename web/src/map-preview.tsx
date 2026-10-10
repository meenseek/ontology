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
const names = ["제품 전략과 의사결정", "디자인과 사용성", "개발과 품질", "프로젝트 실행", "글쓰기와 지식 정리", "새로운 실험"];
const counts = [225, 39, 22, 21, 20, 12];
const titles = ["사용자의 맥락을 이해하는 방법", "작은 결정을 오래 기억하기", "제품의 방향을 정하는 질문", "읽기와 탐색의 균형", "이번 주 실험에서 배운 것", "반복 작업의 부담을 줄이기", "팀이 같은 그림을 보는 순간", "아이디어에서 실제 결과까지", "오래 쓰는 도구의 조건", "자연스럽게 이어지는 인터랙션", "기록을 다시 찾기 위한 기준", "변화하는 지식을 연결하는 방식"];
const nodes: GraphNode[] = names.flatMap((name, group) => {
  const subject = `preview-purpose-${group}`;
  return [{ id: subject, scope: "personal", kind: "subject", label: name } as GraphNode,
    ...Array.from({ length: counts[group] }, (_, index): GraphNode => ({ id: `preview-${group}-${index}`, scope: "personal", kind: "document", title: titles[index % titles.length], label: `synthetic/${name}/${index}.md`,
      subject_id: subject, subject_name: name, source_kind: "original", context_scope: "personal", context_path: `synthetic/${group}/${index}.md`, status: "ok", present: true, current: true }))];
});
for (let index = 0; index < 23; index++) nodes.push({ id: `preview-single-${index}`, scope: "personal", kind: "document", title: titles[index % titles.length], label: `synthetic/ungrouped/${index}.md`,
  source_kind: "original", context_scope: "personal", context_path: `synthetic/ungrouped/${index}.md`, status: "ok", present: true, current: true });
const links: Snapshot["links"] = nodes.filter(node => node.subject_id).map(node => ({ source: node.id, target: node.subject_id!, kind: "subject", current: true }));
for (let group = 0; group < 6; group++) for (let index = 0; index < 12; index++) links.push({ source: `preview-${group}-${index}`, target: `preview-${group}-${index + 1}`, kind: index % 2 ? "related" : "reference", current: true });
links.push({ source: "preview-0-0", target: "preview-1-0", kind: "reference", current: true });
const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
window.fetch = async (input, options) => {
  const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
  if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
  if (url.pathname === "/api/graph") {
    const scope: Scope = url.searchParams.get("scope") === "meenseek" ? "meenseek" : "personal", query = url.searchParams.get("q") ?? "", focus = url.searchParams.get("focus");
    const included = nodes.filter(node => !query || `${node.label} ${node.title}`.includes(query) || node.id === focus).map(node => ({ ...node, scope }));
    const ids = new Set(included.map(node => node.id)), includedLinks = links.filter(link => ids.has(link.source) && ids.has(link.target));
    return json({ scope, query, focus: { id: focus, found: !!focus && ids.has(focus) }, nodes: included, links: includedLinks, matched: included.length,
      totals: { documents: included.filter(node => node.kind === "document").length, memories: 0, markers: included.filter(node => node.kind === "subject").length, links: includedLinks.length },
      returned: { knowledge: included.filter(node => node.kind === "document").length, markers: included.filter(node => node.kind === "subject").length, links: includedLinks.length },
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
