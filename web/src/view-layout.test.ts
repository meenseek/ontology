/// <reference types="node" />
import assert from "node:assert/strict";
import { test } from "node:test";
import { PerspectiveCamera, Vector3 } from "three";
import { constellationView, reconcile } from "./graph.ts";
import type { GraphLink, GraphNode, PositionedNode, Snapshot } from "./graph.ts";
import { nucleusLevel, nucleusView } from "./nuclei.ts";
import { Positions } from "./positions.ts";
import type { LayoutView, Point } from "./positions.ts";
import { coreCameraDistance, LAYOUT_WORLD_SPACING, nodeScreenSize, nodeVisualRadius } from "./presentation.ts";
import { planCoreView, screenPlane, overviewFrame, coreReadingFrame, coreReadingInsets } from "./view-layout.ts";

test("opening an already readable page retains its zoom while large content still fits", () => {
  assert.equal(coreCameraDistance(300, 700, 1.7, 500), 500);
  assert.equal(coreCameraDistance(900, 700, 1.7, 500), 900);
});

test("initial framing clears fixed-size summaries and actual map controls", () => {
  for (const size of [{ width: 1440, height: 680 }, { width: 390, height: 560 }]) {
    const camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000), target = { x: 0, y: 0, z: 0 };
    camera.position.set(0, 0, 1200); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
    const nodes = [{ x: -85, y: 40, z: 20 }, { x: 85, y: -40, z: -20 }];
    const obstacles = [{ left: 12, right: size.width - 12, top: 10, bottom: 64 }, { left: size.width - 180, right: size.width - 14, top: size.height - 160, bottom: size.height - 14 }];
    const frame = overviewFrame(camera, target, nodes, size, { radius: () => 68, obstacles })!;
    assert.ok(frame, "there is room to enlarge this sparse graph safely");
    camera.position.set(frame.position.x, frame.position.y, frame.position.z); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
    for (const node of nodes) {
      const p = new Vector3(node.x, node.y, node.z).project(camera), x = (p.x + 1) * size.width / 2, y = (1 - p.y) * size.height / 2;
      assert.ok(x >= 92 && x <= size.width - 92 && y >= 92 && y <= size.height - 92);
      for (const box of obstacles) assert.ok(Math.hypot(x - Math.max(box.left, Math.min(x, box.right)), y - Math.max(box.top, Math.min(y, box.bottom))) >= 76);
    }
  }
});

test("sparse overview framing enlarges a small graph without changing coordinates or camera direction", () => {
  for (const size of [{ width: 1440, height: 680 }, { width: 390, height: 560 }]) {
    const camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000);
    const target = { x: 10, y: -20, z: 30 };
    camera.position.set(310, 180, 1030); camera.lookAt(target.x, target.y, target.z); camera.updateMatrixWorld();
    const nodes = Array.from({ length: 27 }, (_, i) => ({ x: target.x + Math.cos(i) * 90, y: target.y + Math.sin(i) * 70, z: target.z + i - 13 }));
    const before = structuredClone(nodes), originalCamera = camera.position.clone();
    const frame = overviewFrame(camera, target, nodes, size)!;
    assert.ok(frame);
    const original = originalCamera.clone().sub(new Vector3(target.x, target.y, target.z));
    const next = new Vector3(frame.position.x, frame.position.y, frame.position.z).sub(new Vector3(target.x, target.y, target.z));
    assert.ok(next.length() < original.length() && next.length() >= original.length() / 2.5 - 1e-7);
    assert.ok(next.clone().normalize().distanceTo(original.normalize()) < 1e-7);
    assert.deepEqual(nodes, before); assert.ok(camera.position.equals(originalCamera));
    assert.deepEqual(frame.target, target);
    assert.equal(overviewFrame(camera, target, [nodes[0]], size), null);
    assert.equal(overviewFrame(camera, target, Array.from({ length: 61 }, () => nodes[0]), size), null);
    const wide = nodes.map(node => ({ ...node, x: node.x * 100 }));
    assert.equal(overviewFrame(camera, target, wide, size), null, "a graph that already fills the screen is never zoomed farther in");
  }
});

const doc = (id: string): GraphNode => ({ id, scope: "personal", kind: "document", label: id, status: "ok", present: true, current: true });
const edge = (source: string, target: string): GraphLink => ({ source, target, kind: "related", current: true });
const point = ({ x, y, z }: Point) => ({ x, y, z });
function fixture(sizes: number[], outside: number, size: { width: number; height: number }, distanceScale = 1) {
  const nodes = sizes.flatMap(count => Array.from({ length: count }, (_, i) => doc(`g${count}-${i}`)))
    .concat(Array.from({ length: outside }, (_, i) => doc(`outside-${String(i).padStart(3, "0")}`)));
  const links = sizes.flatMap(count => Array.from({ length: count - 1 }, (_, i) => edge(`g${count}-0`, `g${count}-${i + 1}`)));
  const source: Snapshot = { scope: "personal", query: "", focus: { id: null, found: false }, nodes, links,
    matched: nodes.length, totals: { documents: nodes.length, memories: 0, markers: 0, links: links.length },
    returned: { knowledge: nodes.length, markers: 0, links: links.length }, omitted: { nodes: 0, links: 0 },
    eligible: { nodes: nodes.length, links: links.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
  const model = reconcile(source), positions = new Positions(); positions.install(model, true);
  const overview = constellationView(model.nodes, model.links, null, null);
  const axes = ["x", "y", "z"] as const;
  const center = Object.fromEntries(axes.map(axis => [axis,
    (Math.min(...overview.nodes.map(node => node[axis])) + Math.max(...overview.nodes.map(node => node[axis]))) / 2])) as Point;
  const radius = Math.max(18, ...overview.nodes.map(node => new Vector3(node.x, node.y, node.z).distanceTo(new Vector3(center.x, center.y, center.z)) + 6));
  const camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000);
  const vertical = camera.fov * Math.PI / 180, horizontal = 2 * Math.atan(Math.tan(vertical / 2) * size.width / size.height);
  const distance = Math.max(radius * 1.15 / Math.sin(Math.min(vertical, horizontal) / 2), size.height * camera.projectionMatrix.elements[5] / 2) * distanceScale;
  camera.position.set(center.x, center.y, center.z + distance); camera.lookAt(center.x, center.y, center.z); camera.updateMatrixWorld();
  const lod = nucleusLevel(LAYOUT_WORLD_SPACING * size.height * camera.projectionMatrix.elements[5] / (2 * distance), 0);
  const spatial = nucleusView(overview.nodes, model.links, lod, null), counts = new Map([...spatial.counts, ...overview.counts]);
  const plane = screenPlane(camera, () => size, null, counts);
  const cohorts = new Map([...overview.cores.map(core => [core.hub, [...core.members]] as const),
    ...[...spatial.groups].map(([id, group]) => [id, group.members] as const)]);
  positions.packOverview(spatial.nodes.map(node => ({ id: node.id, ...plane.project(node), radius: plane.radius(node, plane.project(node).depth) })), overview.links,
    cohorts, plane, 0, true);
  const locks = new Map<string, { representative?: string; members: readonly string[]; level: number }>([...spatial.groups].map(([id, group]) => [id, { members: group.members, level: lod }]));
  return { source, model, positions, camera, center, size, lod, locks };
}
function samePoints(nodes: PositionedNode[], before: ReadonlyMap<string, { x: number; y: number; z: number }>, message: string) {
  for (const node of nodes) {
    const at = before.get(node.id)!;
    assert.ok(Math.hypot(node.x - at.x, node.y - at.y, node.z - at.z) < 1e-7, `${message}: ${node.id}`);
  }
}
function open(f: ReturnType<typeof fixture>, hub: string, page: number | undefined, options: {
  selected?: string | null; camera?: PerspectiveCamera; target?: Point; automatic?: boolean; key?: string; now?: number; reduced?: boolean; spatial?: { level: number; members: ReadonlySet<string>; camera: Point; target: Point };
} = {}) {
  const selected = options.selected ?? null;
  const semantic = constellationView(f.model.nodes, f.model.links, selected, selected ? null : hub, page);
  const group = semantic.cores.find(core => core.hub === hub)!;
  let plan: ReturnType<typeof planCoreView> | undefined;
  const render: LayoutView = { key: options.key ?? `${page}:${selected}`, resolve: (desired, moving, anchor) => {
    plan = planCoreView(desired, moving, anchor, { camera: options.camera ?? f.camera, cameraTarget: options.target ?? f.center,
      size: f.size, automatic: options.automatic ?? true, spatial: options.spatial, nodes: semantic.nodes, links: f.model.links, counts: semantic.counts,
      members: semantic.disclosure?.visible ?? group.members, selected, lod: f.lod, locks: f.locks,
      cohorts: new Map(semantic.cores.filter(core => semantic.counts.has(core.hub)).map(core => [core.hub, [...core.members]])) },
    (desired, moving, held, plane) => f.positions.clearLayout(desired, moving, held, plane));
    for (const [id, members] of plan.memberships) f.locks.set(id, { members, level: plan.level });
    return plan.positions;
  } };
  f.positions.showCore(hub, options.now ?? 0, options.reduced ?? true, semantic.disclosure?.index, semantic.disclosure?.visible, undefined, render);
  return { plan, semantic, group, spatial: options.spatial };
}
function assertClear(f: ReturnType<typeof fixture>, opened: ReturnType<typeof open>, selected: string | null = null) {
  const plan = opened.plan!;
  const nodes = opened.semantic.nodes.map(node => ({ ...node, ...plan.positions.get(node.id)! }));
  const actual = nucleusView(nodes, f.model.links, plan.level, selected, opened.spatial?.members,
    new Map([...f.locks].filter(([, group]) => group.level === plan.level).map(([id, group]) => [id, group.members])));
  assert.deepEqual(actual.nodes.map(node => node.id), plan.visible, "the predicted view matches the committed LOD and coordinates");
  const counts = new Map([...actual.counts, ...opened.semantic.counts]), plane = screenPlane(plan.camera, () => f.size, selected, counts);
  for (const a of actual.nodes.filter(node => opened.group.members.has(node.id) || node.id === selected)) {
    const at = plane.project(a), ra = plane.radius(a, at.depth);
    if (at.depth <= 0) continue;
    for (const b of actual.nodes) {
      if (a.id === b.id) continue;
      const bt = plane.project(b), rb = plane.radius(b, bt.depth);
      if (bt.depth > 0) assert.ok(Math.hypot(at.x - bt.x, at.y - bt.y) >= ra + rb + 1 - 1e-6,
        `${a.id}/${b.id}: rendered bodies and rings have clearance in the final camera`);
    }
  }
  return plane;
}

function assertWholeShape(plane: ReturnType<typeof screenPlane>, before: ReadonlyMap<string, { x: number; y: number; z: number }>, after: ReadonlyMap<string, { x: number; y: number; z: number }>, ids: readonly string[], anchor: string) {
  const origin = plane.project(before.get(anchor)!), final = plane.project(after.get(anchor)!);
  const offsets = ids.filter(id => id !== anchor).map(id => {
    const a = plane.project(before.get(id)!), b = plane.project(after.get(id)!);
    assert.ok(Math.abs(a.depth - b.depth) < 1e-6, "clearance preserves each star's camera depth");
    return { ax: a.x - origin.x, ay: a.y - origin.y, bx: b.x - final.x, by: b.y - final.y };
  });
  const first = offsets.find(at => Math.hypot(at.ax, at.ay) > 1e-6)!;
  const squared = first.ax ** 2 + first.ay ** 2;
  const cosine = (first.ax * first.bx + first.ay * first.by) / squared;
  const sine = (first.ax * first.by - first.ay * first.bx) / squared;
  for (const at of offsets) assert.ok(Math.hypot(at.bx - (at.ax * cosine - at.ay * sine), at.by - (at.ax * sine + at.ay * cosine)) < 1e-5,
    "every star follows one whole-shape scale and rotation instead of acquiring a new direction");
}

test("a render invalidation during disclosure preserves its intended destination", () => {
  for (const elapsed of [16, 160, 400]) {
    const f = fixture([40], 3, { width: 1440, height: 700 });
    const view = constellationView(f.model.nodes, f.model.links, null, "g40-0", 0);
    let destination: ReadonlyMap<string, Point> | undefined;
    f.positions.showCore("g40-0", 0, false, 0, view.disclosure!.visible, undefined, { key: "initial", resolve: desired => { destination = new Map(desired); return new Map(desired); } });
    f.positions.advance(elapsed, false);
    f.positions.showCore("g40-0", elapsed, false, 0, view.disclosure!.visible, undefined, { key: "resized", resolve: desired => {
      for (const [id, expected] of destination!) {
        const at = desired.get(id)!;
        assert.ok(Math.hypot(at.x - expected.x, at.y - expected.y, at.z - expected.z) < 1e-7, `${elapsed}ms: ${id} resumes toward the same final point`);
      }
      return new Map(desired);
    } });
    f.positions.advance(2000, true);
    samePoints(f.model.nodes, destination!, "render invalidation cannot freeze a partly opened page");
  }
});

test("short mobile reading frames use real space beside the expanded minimap", () => {
  for (const height of [364.7578125, 480, 560]) {
    const size = { width: 390, height }, actions = { left: 14, top: 14, right: 376, bottom: 117 };
    const minimap = { left: 220, top: height - 228, right: 376, bottom: height - 52 };
    const insets = coreReadingInsets(size, actions, minimap);
    const camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000);
    camera.position.set(0, 0, 500); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
    const nodes = [{ x: 0, y: 0, z: 0 }, ...Array.from({ length: 12 }, (_, i) => ({ x: 90 * Math.cos(i * Math.PI / 6), y: 100 * Math.sin(i * Math.PI / 6), z: 0 }))];
    const frame = coreReadingFrame(camera, { x: 0, y: 0, z: 0 }, nodes, size, insets);
    camera.position.copy(frame.position); camera.lookAt(frame.target); camera.updateMatrixWorld();
    for (const node of nodes) {
      const at = new Vector3(node.x, node.y, node.z).project(camera), x = (at.x + 1) * size.width / 2, y = (1 - at.y) * size.height / 2;
      assert.ok(x >= 8 && x <= size.width - 8 && y >= 8 && y <= size.height - 8);
      assert.ok([actions, minimap].every(box => x < box.left - 6 || x > box.right + 6 || y < box.top - 6 || y > box.bottom + 6));
    }
    assert.equal(insets.right !== undefined, height - 141 - 252 < 100);
  }
});

test("purpose reading frames have the same scale on repeat entry and fit depth plus control insets", () => {
  for (const size of [{ width: 1440, height: 680 }, { width: 390, height: 490 }]) {
    const nodes = Array.from({ length: 13 }, (_, i) => ({ x: Math.cos(i) * 90, y: Math.sin(i) * 100, z: i * 3 }));
    const before = structuredClone(nodes), camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000);
    const insets = { top: 130, bottom: 110 };
    const frames = [500, 2000].map(distance => {
      camera.position.set(0, 0, distance); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
      return coreReadingFrame(camera, { x: 0, y: 0, z: 0 }, nodes, size, insets);
    });
    assert.ok(frames[0].position.distanceTo(frames[1].position) < 1e-7);
    assert.ok(frames[0].target.distanceTo(frames[1].target) < 1e-7);
    camera.position.copy(frames[0].position); camera.lookAt(frames[0].target); camera.updateMatrixWorld();
    const projected = nodes.map(at => new Vector3(at.x, at.y, at.z).project(camera));
    for (const at of projected) {
      const x = (at.x + 1) * size.width / 2, y = (1 - at.y) * size.height / 2;
      assert.ok(x >= 70 && x <= size.width - 70 && y >= insets.top - 1 && y <= size.height - insets.bottom + 1, "every star stays inside the reading rectangle");
    }
    assert.deepEqual(nodes, before);
  }
});

test("static pages use actual screen footprints and restore their overview across desktop and mobile", () => {
  for (const size of [{ width: 1280, height: 900 }, { width: 390, height: 800 }]) {
    const f = fixture([22, 40, 75, 120], 500, size);
    const before = new Map(f.model.nodes.map(node => [node.id, point(node)]));
    for (const count of [22, 40, 75, 120]) {
      const hub = `g${count}-0`;
      for (const page of [0, 1, 0]) {
        const opened = open(f, hub, page), plane = assertClear(f, opened);
        for (const link of opened.semantic.links) {
          const a = plane.project(opened.plan!.positions.get(link.source)!), b = plane.project(opened.plan!.positions.get(link.target)!);
          assert.ok(Math.hypot(a.x - b.x, a.y - b.y) < 180, `${count}, page ${page}: a local page cannot grow a runaway edge (${Math.hypot(a.x - b.x, a.y - b.y).toFixed(2)}px)`);
        }
        samePoints(f.model.nodes.filter(node => !opened.group.members.has(node.id) || node.id === hub), before, "opening and paging preserve the background");
      }
      f.positions.showCore(null, 0, true);
      samePoints(f.model.nodes, before, "closing restores the overview");
      const refreshed = reconcile(f.source, f.model); f.positions.install(refreshed, true); f.model = refreshed;
      samePoints(f.model.nodes, before, "an unchanged refresh retains the overview");
      const full = open(f, hub, undefined); assertClear(f, full);
      f.positions.reset(); f.positions.showCore(null, 0, true);
      open(f, hub, 0, { now: 10, reduced: false }); f.positions.advance(110, false);
      f.positions.showCore(null, 110, false); f.positions.advance(800, false);
      assert.equal(f.positions.layoutMoving, false);
      samePoints(f.model.nodes, before, "reset and an interrupted expansion restore the overview");
    }
  }
});

test("automatic expansion includes background revealed by its future LOD", () => {
  const f = fixture([22], 750, { width: 1280, height: 900 }, 1.6);
  assert.equal(f.lod, 2, "the valid overview starts in the more aggregated band");
  const overview = constellationView(f.model.nodes, f.model.links, null, null);
  const oldIds = new Set(nucleusView(overview.nodes, f.model.links, f.lod, null).nodes.map(node => node.id));
  const opened = open(f, "g22-0", 0); assert.equal(opened.plan!.level, 1);
  assert.ok(opened.plan!.visible.some(id => id.startsWith("outside-") && !oldIds.has(id)), "automatic framing exposes previously hidden obstacles");
  assertClear(f, opened);
});

test("selection clears the moving focus before freezing rotated camera clearance", () => {
  for (const direction of [new Vector3(0, 0, 1), new Vector3(.7, .4, 1).normalize()]) {
    const f = fixture([40], 300, { width: 1280, height: 900 });
    const camera = f.camera.clone(), target = new Vector3(f.center.x, f.center.y, f.center.z);
    const distance = camera.position.distanceTo(target);
    camera.position.copy(target).addScaledVector(direction, distance); camera.lookAt(target); camera.updateMatrixWorld();
    const before = new Map(f.model.nodes.map(node => [node.id, point(node)]));
    const selected = "g40-1", opened = open(f, "g40-0", 0, { selected, camera });
    const plane = assertClear(f, opened, selected), at = plane.project(opened.plan!.positions.get(selected)!);
    assert.ok(Math.hypot(at.x, at.y) < 1e-7, `the resolved selection remains at the exact camera target: ${JSON.stringify({ at, target: opened.plan!.target, selected: opened.plan!.positions.get(selected) })}`);
    samePoints(f.model.nodes.filter(node => !opened.group.members.has(node.id)), before, "focus resolution preserves the unrelated background");
    f.positions.showCore(null, 0, true); samePoints(f.model.nodes, before, "closing selection restores canonical coordinates");
  }
});

test("content footprint changes use the current frame and shrinking footprints preserve positions", () => {
  const f = fixture([22], 300, { width: 1280, height: 900 }), opened = open(f, "g22-0", 0);
  const camera = opened.plan!.camera, target = opened.plan!.target;
  for (const node of f.model.nodes.filter(node => node.id.startsWith("g22-"))) node.changed = true;
  const grown = open(f, "g22-0", 0, { key: "change-rings", automatic: false, camera, target });
  assertClear(f, grown);
  assert.deepEqual(grown.plan!.camera.position, camera.position, "metadata growth cannot change the camera");
  const before = new Map(f.model.nodes.map(node => [node.id, point(node)]));
  for (const node of f.model.nodes) node.changed = false;
  const shrunk = open(f, "g22-0", 0, { key: "plain-rings", automatic: false, camera, target });
  assertClear(f, shrunk); samePoints(f.model.nodes, before, "sufficient clearance survives shrinking rings without repacking");
  const repeated = open(f, "g22-0", 0, { key: "plain-rings", automatic: false, camera, target });
  assert.equal(repeated.plan, undefined, "an unchanged render key does not run another layout");
});


test("a growing hub footprint clears background cohorts while its camera and focus stay fixed", () => {
  for (const sizes of [[22], [22, 40]]) for (const selected of [null, "g22-1"]) {
    const f = fixture(sizes, 100, { width: 1280, height: 900 }, selected ? 1.6 : 1);
    const opened = open(f, "g22-0", 0, { selected }), camera = opened.plan!.camera, target = opened.plan!.target;
    const focus = selected ?? "g22-0", before = new Map(f.model.nodes.map(node => [node.id, point(node)]));
    const semantic = constellationView(f.model.nodes, f.model.links, selected, selected ? null : "g22-0", 0);
    const spatial = nucleusView(semantic.nodes, f.model.links, opened.plan!.level, selected);
    const cohorts = new Map([...semantic.cores.filter(core => semantic.counts.has(core.hub)).map(core => [core.hub, [...core.members]] as const),
      ...[...spatial.groups].map(([id, group]) => [id, group.members] as const)]);
    f.model.nodes.find(node => node.id === focus)!.changed = true;
    const grown = open(f, "g22-0", 0, { selected, key: "growing-focus", camera, target, automatic: false });
    assertClear(f, grown, selected);
    assert.deepEqual(grown.plan!.camera.position, camera.position);
    assert.deepEqual(grown.plan!.target, target);
    assert.deepEqual(point(f.model.nodes.find(node => node.id === focus)!), before.get(focus), "content changes preserve the focused star");
    for (const [id, members] of cohorts) {
      const from = before.get(id)!, to = grown.plan!.positions.get(id)!;
      for (const member of members) {
        const original = before.get(member)!, at = grown.plan!.positions.get(member)!;
        assert.ok(Math.hypot(at.x - original.x - to.x + from.x, at.y - original.y - to.y + from.y, at.z - original.z - to.z + from.z) < 1e-7,
          "each moved summary retains the exact relative positions of its hidden members");
      }
    }
  }
});


test("a spatial reveal while a core is open clears the frame the user chose and restores it on close", () => {
  const f = fixture([22], 100, { width: 1280, height: 900 }, 1.6), core = open(f, "g22-0", 0);
  const spatial = nucleusView(core.semantic.nodes, f.model.links, core.plan!.level, null, undefined,
    new Map([...f.locks].filter(([, group]) => group.level === core.plan!.level).map(([id, group]) => [id, group.members])));
  const group = [...spatial.groups.values()].find(group => group.members.length >= 6)!;
  assert.ok(group, "a real proximity cohort is available while the relation core is open");
  const reveal = { members: new Set(group.members), level: core.plan!.level, camera: point(core.plan!.camera.position), target: core.plan!.target };
  const opened = open(f, "g22-0", 0, { key: "spatial-open", camera: core.plan!.camera, target: core.plan!.target, spatial: reveal });
  assertClear(f, opened);
  assert.deepEqual(opened.plan!.target, group.center, "the spatial group takes camera priority over the open relation core");
  assert.notDeepEqual(opened.plan!.target, opened.plan!.positions.get("g22-0"));
  const closed = open(f, "g22-0", 0, { key: "spatial-close", camera: opened.plan!.camera, target: opened.plan!.target,
    spatial: { ...reveal, members: new Set() } });
  assertClear(f, closed);
  assert.deepEqual(point(closed.plan!.camera.position), reveal.camera);
  assert.deepEqual(closed.plan!.target, reveal.target, "closing the spatial group restores its exact prior frame");
  assert.deepEqual(closed.plan!.memberships.get(group.representative), group.members, "explicitly revealed bodies collapse back into their prior group");
});


test("a plan retains the chosen representative when another detail band holds its singleton", () => {
  const f = fixture([22], 100, { width: 1280, height: 900 }, 1.6);
  const [originalId, original] = [...f.locks].find(([, group]) => group.members.length >= 6)!;
  const representative = original.members.find(id => id !== originalId)!;
  f.locks.clear();
  f.locks.set(JSON.stringify([f.lod, representative]), { representative, members: original.members, level: f.lod });
  f.locks.set(JSON.stringify([0, representative]), { representative, members: [representative], level: 0 });
  const opened = open(f, "g22-0", 0, { automatic: false, key: "multiple-detail-band-locks" });
  assert.equal(opened.plan!.level, f.lod);
  assert.deepEqual(opened.plan!.memberships.get(representative), original.members, "a different band's singleton cannot replace the current cohort or its representative");
});

test("content clearance commits spatial memberships formed after manual LOD changes", () => {
  for (const size of [{ width: 390, height: 800 }, { width: 1280, height: 900 }]) {
    const f = fixture([22, 40], 100, size), target = new Vector3(f.center.x, f.center.y, f.center.z);
    // Start with the actual overview, then change only its camera/LOD. No overview
    // packing runs again, so future memberships must be owned by the new plan.
    const offset = f.camera.position.clone().sub(target).multiplyScalar(size.width === 390 ? 2 : 1.8);
    f.camera.position.copy(target).add(offset); f.camera.lookAt(target); f.camera.updateMatrixWorld();
    f.lod = nucleusLevel(LAYOUT_WORLD_SPACING * size.height * f.camera.projectionMatrix.elements[5] / (2 * f.camera.position.distanceTo(target)), f.lod);
    const opened = open(f, "g22-0", 0);
    for (const node of f.model.nodes) node.changed = true;
    const grown = open(f, "g22-0", 0, { key: "all-rings-grow", automatic: false, camera: opened.plan!.camera, target: opened.plan!.target });
    const plane = assertClear(f, grown), plan = grown.plan!;
    const actual = nucleusView(grown.semantic.nodes, f.model.links, plan.level, null, undefined,
      new Map([...f.locks].filter(([, group]) => group.level === plan.level).map(([id, group]) => [id, group.members])));
    for (const [index, a] of actual.nodes.entries()) for (const b of actual.nodes.slice(index + 1)) {
      const at = plane.project(a), bt = plane.project(b);
      if (at.depth > 0 && bt.depth > 0) assert.ok(Math.hypot(at.x - bt.x, at.y - bt.y) >= plane.radius(a, at.depth) + plane.radius(b, bt.depth) + 1 - 1e-6,
        `${a.id}/${b.id}: every committed content footprint has clearance`);
    }
  }
});


test("a committed view preserves singleton bodies as ordinary stars", () => {
  const f = fixture([22], 500, { width: 390, height: 800 }, 1.6);
  const opened = open(f, "g22-0", 0);
  for (const node of f.model.nodes) node.changed = true;
  const grown = open(f, "g22-0", 0, { key: "all-growth", automatic: false, camera: opened.plan!.camera, target: opened.plan!.target });
  assertClear(f, grown);
  const singles = [...grown.plan!.memberships].filter(([, members]) => members.length === 1);
  assert.ok(singles.length > 0);
  for (const [id] of singles) assert.equal(grown.plan!.counts.has(id), false, "a reserved singleton has no summary glyph or count");
});


test("content growth clears an asymmetric constellation while movable background yields", () => {
  for (const size of [{ width: 1280, height: 900 }, { width: 390, height: 800 }]) {
    const f = fixture([22], 100, size, 1.6), opened = open(f, "g22-0", 0);
    for (const node of f.model.nodes) node.changed = true;
    const grown = open(f, "g22-0", 0, { key: "uniform-growth", automatic: false, camera: opened.plan!.camera, target: opened.plan!.target });
    const plane = assertClear(f, grown), center = plane.project(grown.plan!.positions.get(grown.group.hub)!);
    const radii = [...grown.group.members].filter(id => id !== grown.group.hub && grown.plan!.visible.includes(id)).map(id => {
      const at = plane.project(grown.plan!.positions.get(id)!); return Math.round(Math.hypot(at.x - center.x, at.y - center.y) * 1000);
    });
    assert.ok(new Set(radii).size > radii.length / 2, "compact expansion avoids concentric uniform rings");
    const settled = new Map(f.model.nodes.map(node => [node.id, point(node)]));
    for (const node of f.model.nodes) node.changed = false;
    open(f, "g22-0", 0, { key: "asymmetric-shrink", automatic: false, camera: grown.plan!.camera, target: grown.plan!.target });
    samePoints(f.model.nodes, settled, "shrinking bodies preserve the settled silhouette");
    assert.deepEqual(grown.plan!.camera.position, opened.plan!.camera.position);
    assert.deepEqual(grown.plan!.target, opened.plan!.target);
  }
});


test("a selected leaf stays anchored while its whole constellation clears", () => {
  for (const size of [{ width: 1280, height: 900 }, { width: 390, height: 800 }]) {
    for (const direction of [new Vector3(0, 0, 1), new Vector3(.7, .4, 1).normalize()]) {
      const f = fixture([22], 100, size), camera = f.camera.clone(), target = new Vector3(f.center.x, f.center.y, f.center.z);
      camera.position.copy(target).addScaledVector(direction, f.camera.position.distanceTo(target)); camera.lookAt(target); camera.updateMatrixWorld();
      const before = new Map(f.model.nodes.map(node => [node.id, point(node)])), selected = "g22-1";
      const opened = open(f, "g22-0", 0, { selected, camera });
      assertClear(f, opened, selected);
      assert.deepEqual(point(f.model.nodes.find(node => node.id === selected)!), opened.plan!.target, "the selected star remains the exact camera target");
      samePoints(f.model.nodes.filter(node => !opened.group.members.has(node.id)), before, "selecting a fan preserves unrelated background");
      const anchor = point(f.model.nodes.find(node => node.id === selected)!);
      const silhouette = new Map(f.model.nodes.map(node => [node.id, point(node)]));
      for (const node of f.model.nodes) node.changed = true;
      const grown = open(f, "g22-0", 0, { selected, key: "selected-growth", automatic: false, camera: opened.plan!.camera, target: opened.plan!.target });
      const plane = assertClear(f, grown, selected);
      assertWholeShape(plane, silhouette, grown.plan!.positions, [...grown.group.members].filter(id => grown.plan!.visible.includes(id)), selected);
      assert.deepEqual(point(f.model.nodes.find(node => node.id === selected)!), anchor);
      assert.deepEqual(grown.plan!.camera.position, opened.plan!.camera.position);
      assert.deepEqual(grown.plan!.target, opened.plan!.target);
      const stable = new Map(f.model.nodes.map(node => [node.id, point(node)]));
      for (const node of f.model.nodes) node.changed = false;
      open(f, "g22-0", 0, { selected, key: "selected-shrink", automatic: false, camera: grown.plan!.camera, target: grown.plan!.target });
      samePoints(f.model.nodes, stable, "shrinking selected footprints preserves sufficient clearance");
      f.positions.showCore(null, 0, true); samePoints(f.model.nodes, before, "closing selection restores every canonical coordinate");
    }
  }
});


test("a full selected fan crossing the camera plane still clears its front-facing bodies", () => {
  const f = fixture([120], 0, { width: 1280, height: 900 });
  const before = new Map(f.model.nodes.map(node => [node.id, point(node)]));
  const camera = f.camera.clone(), target = new Vector3(f.center.x, f.center.y, f.center.z);
  camera.position.copy(target).addScaledVector(new Vector3(.7, .4, 1).normalize(), f.camera.position.distanceTo(target));
  camera.lookAt(target); camera.updateMatrixWorld();
  const selected = "g120-1", opened = open(f, "g120-0", undefined, { selected, camera });
  const plane = assertClear(f, opened, selected);
  assert.ok(opened.plan!.visible.some(id => plane.project(opened.plan!.positions.get(id)!).depth <= 0), "this scene exercises the camera-plane fallback");
  assert.deepEqual(point(f.model.nodes.find(node => node.id === selected)!), opened.plan!.target);
  f.positions.showCore(null, 0, true); samePoints(f.model.nodes, before, "closing the full view restores canonical coordinates");
});


test("purpose pages keep their real chain and preserve the held marker and background", () => {
  const f = fixture([40], 3, { width: 1280, height: 720 });
  const hub = "purpose";
  f.source.nodes = f.source.nodes.map(node => node.id.startsWith("g40-") ? { ...node, subject_id: hub, subject_name: "목적" } : node);
  f.source.nodes.push({ id: hub, scope: "personal", kind: "subject", label: "목적" });
  f.source.links = f.source.nodes.filter(node => node.subject_id).map(node => ({ source: node.id, target: hub, kind: "subject", current: true }));
  for (let i = 0; i < 39; i++) f.source.links.push(edge(`g40-${i}`, `g40-${i + 1}`));
  const model = reconcile(f.source, undefined, false, "purpose"), positions = new Positions(); positions.install(model, true);
  const background = model.nodes.filter(node => node.id.startsWith("outside-")), before = new Map(model.nodes.map(node => [node.id, point(node)]));
  for (const page of [0, 1]) {
    const view = constellationView(model.nodes, model.links, null, hub, page, "purpose");
    assert.ok(view.disclosure);
    positions.showCore(hub, page * 1000, true, page, view.disclosure.visible);
    samePoints(background, before, "page expansion leaves the background in place");
    samePoints(model.nodes.filter(node => node.id === hub), before, "the classification anchor remains held");
    const visible = view.disclosure.visible, real = view.links.filter(link => link.kind === "related" && visible.has(link.source) && visible.has(link.target));
    assert.ok(real.length > 0, "each page has actual relations");
    const distance = (a: string, b: string) => { const p = positions.layoutTarget(a)!, q = positions.layoutTarget(b)!; return Math.hypot(p.x - q.x, p.y - q.y); };
    assert.ok(Math.max(...real.map(link => distance(link.source, link.target))) < 100, "actual page links remain local instead of receiving unrelated page slots");
  }
  positions.showCore(null, 3000, true);
  samePoints(model.nodes, before, "closing restores the original 3D overview");
});

test("a short purpose view preserves final body clearance and actual control space after paging", () => {
  for (const size of [{ width: 390, height: 364.7578125 }, { width: 360, height: 501 }]) {
    const f = fixture([39], 23, size), hub = "purpose";
    f.source.nodes = f.source.nodes.map(node => node.id.startsWith("g39-") ? { ...node, subject_id: hub } : node);
    f.source.nodes.push({ id: hub, scope: "personal", kind: "subject", label: "긴 이름의 목적 묶음" });
    f.source.links = f.source.nodes.filter(node => node.subject_id).map(node => ({ source: node.id, target: hub, kind: "subject", current: true }));
    for (let i = 0; i < 12; i++) f.source.links.push(edge(`g39-${i}`, `g39-${i + 1}`));
    const model = reconcile(f.source, undefined, false, "purpose"), positions = new Positions(); positions.install(model, true);
    const actions = { left: 14, top: 14, right: size.width - 14, bottom: size.width === 360 ? 128.8 : 117 }, minimap = { left: size.width - 170, top: f.size.height - 228, right: size.width - 14, bottom: f.size.height - 52 };
    const insets = coreReadingInsets(f.size, actions, minimap);
    for (const page of [0, 1, 0, 1]) {
      const semantic = constellationView(model.nodes, model.links, null, hub, page, "purpose");
      let plan: ReturnType<typeof planCoreView> | undefined;
      positions.showCore(hub, page * 1000, true, page, semantic.disclosure!.visible, undefined, { key: String(page), resolve: (desired, moving, anchor) => {
        plan = planCoreView(desired, moving, anchor, { camera: f.camera, cameraTarget: f.center, size: f.size, automatic: true,
          nodes: semantic.nodes, links: model.links, counts: semantic.counts, members: semantic.disclosure!.visible,
          selected: null, lod: f.lod, reading: true, insets }, (desired, moving, held, plane) => positions.clearLayout(desired, moving, held, plane));
        return plan.positions;
      } });
      const plane = screenPlane(plan!.camera, () => f.size, null, plan!.counts);
      const members = semantic.nodes.filter(node => semantic.disclosure!.visible.has(node.id)).map(node => {
        const at = plane.project(plan!.positions.get(node.id)!);
        return { ...at, radius: plane.radius(node, at.depth) };
      });
      for (const [index, at] of members.entries()) {
        const x = at.x + f.size.width / 2, y = f.size.height / 2 - at.y;
        assert.ok(x >= 8 && x <= f.size.width - 8 && y >= 8 && y <= f.size.height - 8);
        assert.ok([actions, minimap].every(box => x < box.left - 6 || x > box.right + 6 || y < box.top - 6 || y > box.bottom + 6));
        for (const other of members.slice(index + 1)) assert.ok(Math.hypot(at.x - other.x, at.y - other.y) >= at.radius + other.radius + 1 - 1e-6,
          "the final camera must preserve body clearance instead of shrinking a cleared layout");
      }
    }
  }
});


test("a selected purpose marker frames its disclosed page rather than an individual closeup", () => {
  for (const size of [{ width: 680, height: 700 }, { width: 780, height: 700 }, { width: 390, height: 540 }]) {
    const camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000);
    camera.position.set(0, 0, 500); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
    const hub: PositionedNode = { id: "purpose", kind: "subject", scope: "personal", label: "목적", x: 0, y: 0, z: 0 } as PositionedNode;
    const nodes = [hub, ...Array.from({ length: 12 }, (_, i) => ({ ...hub, id: `member-${i}`, kind: "document" as const,
      x: (i % 4 - 1.5) * 64, y: (Math.floor(i / 4) - 1) * 74, z: i % 3 * 10 }))];
    const desired = new Map(nodes.map(node => [node.id, { x: node.x, y: node.y, z: node.z }]));
    const insets = { top: 110, bottom: 150, left: 40, right: 40 };
    const plan = planCoreView(desired, new Set(nodes.slice(1).map(node => node.id)), hub.id, {
      camera, cameraTarget: { x: 0, y: 0, z: 0 }, size, automatic: true, nodes, links: [], counts: new Map(),
      members: new Set(nodes.map(node => node.id)), selected: hub.id, lod: 0, reading: true, insets,
    }, values => new Map(values));
    for (const at of plan.positions.values()) {
      const p = new Vector3(at.x, at.y, at.z).project(plan.camera), x = (p.x + 1) * size.width / 2, y = (1 - p.y) * size.height / 2;
      assert.ok(x >= insets.left - 1 && x <= size.width - insets.right + 1 && y >= insets.top - 1 && y <= size.height - insets.bottom + 1,
        `selected collection member stays in its reading rectangle at ${size.width}px`);
    }
  }
});

// Settled 360×740 synthetic input; the old minimum-distance early exit kept a covered body.
test("initial framing recovers an unsafe narrow pose even when approaching is impossible", () => {
  const size = {"height": 449, "width": 360}, target = {"x": 1.6887970896347184, "y": -2.9177088023344595, "z": 4.115896008043606};
  const camera = new PerspectiveCamera(50, size.width / size.height, .1, 20000);
  camera.position.set(1.6887970896347184,-2.9177088023344244,579.8694213466977); camera.lookAt(target.x, target.y, target.z); camera.updateMatrixWorld();
  const bodies = [
    {"id":"preview-5-0","kind":"document","x":73.02620324142721,"y":79.93619628877536,"z":-49.57289030812409,"count":0},
    {"id":"preview-5-1","kind":"document","x":94.8809696730771,"y":54.75006494397529,"z":-37.13133348825912,"count":0},
    {"id":"preview-5-10","kind":"document","x":-94.27868831440809,"y":-8.782435722807094,"z":-39.01488606951422,"count":0},
    {"id":"preview-5-11","kind":"document","x":-107.63742282410033,"y":21.628817883202288,"z":-56.00892854098871,"count":0},
    {"id":"preview-5-2","kind":"document","x":81.80274532804388,"y":22.419213082463877,"z":-6.1993694492517575,"count":0},
    {"id":"preview-5-3","kind":"document","x":80.12807091187304,"y":-7.619027736279957,"z":48.150776185997294,"count":0},
    {"id":"preview-5-4","kind":"document","x":62.86523678069581,"y":-29.464953615750396,"z":29.63428042540696,"count":0},
    {"id":"preview-5-5","kind":"document","x":38.127338514692205,"y":-41.224243119306195,"z":-6.205793698187592,"count":0},
    {"id":"preview-5-6","kind":"document","x":13.138962903101204,"y":-58.27939493934115,"z":-23.924742114028476,"count":0},
    {"id":"preview-5-7","kind":"document","x":-16.8749228794234,"y":-55.89633358206402,"z":-10.068080931826948,"count":0},
    {"id":"preview-5-8","kind":"document","x":-43.20340224424955,"y":-48.65525316403416,"z":40.12487974284586,"count":0},
    {"id":"preview-5-9","kind":"document","x":-65.81664599036804,"y":-29.451920476386853,"z":19.096931009315945,"count":0},
    {"id":"preview-purpose-0","kind":"subject","x":7.067506389382384,"y":-150.41114897085427,"z":7.412551706524292,"count":225},
    {"id":"preview-purpose-1","kind":"subject","x":4.180440381926884,"y":25.473281377985376,"z":10.81861469587091,"count":39},
    {"id":"preview-purpose-2","kind":"subject","x":-55.3615689304372,"y":65.44769657542156,"z":17.65506854653333,"count":22},
    {"id":"preview-purpose-3","kind":"subject","x":100.38221751039173,"y":-54.788204579094085,"z":-4.748861123386145,"count":21},
    {"id":"preview-purpose-4","kind":"subject","x":2.3170071735403397,"y":125.85779694918824,"z":-26.177983043038317,"count":20},
    {"id":"preview-purpose-5","kind":"subject","x":-89.34236205777967,"y":67.5808334585056,"z":71.02486718487293,"count":0},
    {"id":"preview-single-0","kind":"document","x":46.37323800198223,"y":117.6108981404485,"z":32.66622749028515,"count":0},
    {"id":"preview-single-1","kind":"document","x":-63.83683163475274,"y":101.20906951145513,"z":37.32792741119864,"count":0},
    {"id":"preview-single-10","kind":"document","x":68.48749590272323,"y":100.96961562661328,"z":-46.51302988249113,"count":0},
    {"id":"preview-single-11","kind":"memory","x":100.6409211386668,"y":113.33530673463596,"z":-17.928337423206187,"count":0},
    {"id":"preview-single-12","kind":"document","x":-111.061435049007,"y":-60.779266386314106,"z":34.15462777417312,"count":0},
    {"id":"preview-single-13","kind":"document","x":97.17609781940668,"y":69.55374807578026,"z":37.85038773760772,"count":0},
    {"id":"preview-single-14","kind":"document","x":159.65022264117755,"y":-10.994382686902185,"z":-22.351821688424224,"count":0},
    {"id":"preview-single-15","kind":"memory","x":-73.87144473050627,"y":-84.59611616780388,"z":48.791546093321145,"count":0},
    {"id":"preview-single-16","kind":"document","x":-120.10161585259648,"y":54.8010407262671,"z":18.756159405909813,"count":0},
    {"id":"preview-single-17","kind":"document","x":-162.36672312365488,"y":34.83108304207886,"z":-38.98298845622202,"count":0},
    {"id":"preview-single-18","kind":"document","x":-96.73947824068448,"y":-28.86308629694804,"z":55.103457112987435,"count":0},
    {"id":"preview-single-19","kind":"memory","x":151.30114471525698,"y":81.84666637928284,"z":-46.97683186449831,"count":0},
    {"id":"preview-single-2","kind":"document","x":110.59342839087606,"y":6.10481728911703,"z":50.67937780264811,"count":0},
    {"id":"preview-single-20","kind":"document","x":-145.1877241963875,"y":-21.22264678139166,"z":-5.369961792147389,"count":0},
    {"id":"preview-single-21","kind":"document","x":-170.49118999241492,"y":-63.90032698356913,"z":-64.85462751794923,"count":0},
    {"id":"preview-single-22","kind":"document","x":-107.8448415451862,"y":108.23078799247185,"z":-19.764704906846937,"count":0},
    {"id":"preview-single-3","kind":"memory","x":-115.2617045235391,"y":7.9212374007721,"z":41.86364970342013,"count":0},
    {"id":"preview-single-4","kind":"document","x":79.33860238277215,"y":-88.51528594053718,"z":43.376656095281945,"count":0},
    {"id":"preview-single-5","kind":"document","x":167.80958994450435,"y":-73.39462793854305,"z":-62.096691605117066,"count":0},
    {"id":"preview-single-6","kind":"document","x":-67.4118603483995,"y":-50.99614642641765,"z":73.08641953403645,"count":0},
    {"id":"preview-single-7","kind":"memory","x":-123.61788607238476,"y":-106.45233049377538,"z":-33.47091632975556,"count":0},
    {"id":"preview-single-8","kind":"document","x":-71.93783527897621,"y":158.5020880260288,"z":-56.37184738998015,"count":0},
    {"id":"preview-single-9","kind":"document","x":134.5845034474588,"y":38.05545783715301,"z":7.074054514306549,"count":0},
    {"id":"preview-topic-shared","kind":"topic","x":-30.925651661247016,"y":87.75876719118246,"z":65.6446455609903,"count":0},
  ];
  const obstacles = [{"bottom":70,"left":12,"right":348,"top":10},{"bottom":60,"left":12,"right":184,"top":10},{"bottom":60,"left":192,"right":364,"top":10},{"bottom":60,"left":372,"right":544,"top":10},{"bottom":60,"left":552,"right":724,"top":10},{"bottom":60,"left":732,"right":904,"top":10},{"bottom":386.1171875,"left":190,"right":346,"top":332.1171875},{"bottom":435.7109375,"left":14,"right":346,"top":396.1171875}];
  const radius = (body: typeof bodies[number], depth: number) => nodeVisualRadius(nodeScreenSize(body.kind, depth, size.height, camera.projectionMatrix.elements[5]), false, false, body.count);
  const projected = () => bodies.map(body => { const world=new Vector3(body.x,body.y,body.z), depth=-world.clone().applyMatrix4(camera.matrixWorldInverse).z, p=world.project(camera); return { x:(p.x+1)*size.width/2, y:(1-p.y)*size.height/2, radius:radius(body,depth) }; });
  const blocked = () => projected().filter(body => obstacles.some(box => Math.hypot(body.x-Math.max(box.left,Math.min(box.right,body.x)),body.y-Math.max(box.top,Math.min(box.bottom,body.y))) < body.radius));
  assert.ok(blocked().length > 0, "the regression input actually covers a rendered body");
  const frame = overviewFrame(camera, target, bodies, size, { radius, obstacles });
  assert.ok(frame, "an unsafe original pose requires a safe replacement, not a no-op");
  camera.position.set(frame.position.x, frame.position.y, frame.position.z); camera.lookAt(frame.target.x, frame.target.y, frame.target.z); camera.updateMatrixWorld();
  assert.equal(blocked().length, 0);
  for (const body of projected()) assert.ok(body.x-body.radius>=16 && body.x+body.radius<=size.width-16 && body.y-body.radius>=16 && body.y+body.radius<=size.height-16);
});

test("initial framing translates when every distance at the original target is obstructed", () => {
  const size = { width: 360, height: 449 }, target = { x: 0, y: 0, z: 0 };
  const camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000);
  camera.position.set(0, 0, 1200); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
  const bodies = [{ x: -60, y: 0, z: 0 }, { x: 60, y: 0, z: 0 }];
  // One body's center stays inside the right-half obstacle at every old distance.
  const obstacles = [{ left: 180, top: 0, right: 360, bottom: 449 }];
  const frame = overviewFrame(camera, target, bodies, size, { radius: () => 21, obstacles });
  assert.ok(frame, "a safe left-hand pocket is reachable by camera translation");
  camera.position.set(frame.position.x, frame.position.y, frame.position.z); camera.lookAt(frame.target.x, frame.target.y, frame.target.z); camera.updateMatrixWorld();
  for (const body of bodies) {
    const p = new Vector3(body.x, body.y, body.z).project(camera), x = (p.x + 1) * size.width / 2;
    assert.ok(x - 21 >= 16 && x + 21 <= obstacles[0].left);
  }
});

test("impossible framing exhausts bounded candidates without mutating the live camera or stars", () => {
  const size = { width: 360, height: 449 }, target = { x: 0, y: 0, z: 0 };
  const camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000);
  camera.position.set(0, 0, 500); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
  const bodies = [{ x: -60, y: 0, z: 0 }, { x: 60, y: 0, z: 0 }], before = JSON.stringify(bodies);
  const pose = camera.position.clone(), rotation = camera.quaternion.clone();
  let calls = 0;
  assert.equal(overviewFrame(camera, target, bodies, size, { radius: () => { calls++; return 21; }, obstacles: [{ left: 0, top: 0, right: 360, bottom: 449 }] }), null);
  assert.ok(calls > 2 && calls < 1000, "the exhaustive failure search remains bounded");
  assert.ok(camera.position.equals(pose) && camera.quaternion.equals(rotation));
  assert.equal(JSON.stringify(bodies), before);
});


test("dense initial views clear minimap footprints without enabling automatic approach", () => {
  const size = { width: 360, height: 449 }, target = { x: 0, y: 0, z: 0 };
  const camera = new PerspectiveCamera(60, size.width / size.height, .1, 20000);
  camera.position.set(0, 0, 500); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
  const bodies = [...Array.from({ length: 61 }, (_, i) => ({ x: -80 + (i % 10) * 16, y: 10 + Math.floor(i / 10) * 12, z: 0, radius: 4 })), { x: 70, y: -80, z: 0, radius: 68 }];
  const obstacle = { left: 190, right: 346, top: 332, bottom: 386 };
  const covered = () => bodies.some(body => {
    const p = new Vector3(body.x, body.y, body.z).project(camera), x = (p.x + 1) * size.width / 2, y = (1 - p.y) * size.height / 2;
    return Math.hypot(x - Math.max(obstacle.left, Math.min(obstacle.right, x)), y - Math.max(obstacle.top, Math.min(obstacle.bottom, y))) < body.radius;
  });
  assert.ok(covered());
  const frame = overviewFrame(camera, target, bodies, size, { radius: body => body.radius, obstacles: [obstacle] });
  assert.ok(frame, "more than 60 bodies must not bypass initial clearance");
  assert.ok(new Vector3(frame.position.x, frame.position.y, frame.position.z).distanceTo(new Vector3(frame.target.x, frame.target.y, frame.target.z)) >= 500);
  camera.position.set(frame.position.x, frame.position.y, frame.position.z); camera.lookAt(frame.target.x, frame.target.y, frame.target.z); camera.updateMatrixWorld();
  assert.equal(covered(), false);
  assert.equal(overviewFrame(camera, frame.target, bodies, size, { radius: body => body.radius, obstacles: [obstacle] }), null, "an already safe dense view keeps its pose");
});
