/// <reference types="node" />
import assert from "node:assert/strict";
import { test } from "node:test";
import { PerspectiveCamera, Vector3 } from "three";
import { constellationView, reconcile } from "./graph.ts";
import type { GraphLink, GraphNode, PositionedNode, Snapshot } from "./graph.ts";
import { nucleusLevel, nucleusView } from "./nuclei.ts";
import { Positions } from "./positions.ts";
import type { LayoutView, Point } from "./positions.ts";
import { LAYOUT_WORLD_SPACING } from "./presentation.ts";
import { planCoreView, screenPlane } from "./view-layout.ts";

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
  const locks = new Map([...spatial.groups].map(([id, group]) => [id, { members: group.members, level: lod }]));
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
