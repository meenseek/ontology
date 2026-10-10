import { Vector3 } from "three";
import type { Camera, PerspectiveCamera } from "three";
import { COLLISION_GAP, nearestFreePoint } from "./clearance";
import { expandedCoreCameraFrame, knowledge } from "./graph";
import type { GraphLink, PositionedNode } from "./graph";
import { nucleusLevel, nucleusView } from "./nuclei";
import { DEFAULT_LINK_PIXELS, LAYOUT_WORLD_SPACING, coreCameraDistance, focusedCameraDistance, nodeScreenSize, nodeVisualRadius } from "./presentation";
import { starCohort } from "./positions";
import type { Point, Positions } from "./positions";

/** Shared projection and footprints; a drag uses its live camera, a layout freezes a clone. */
export function screenPlane(camera: Camera, viewport: () => { width: number; height: number }, selected: string | null,
  counts: ReadonlyMap<string, number>) {
  camera.updateMatrixWorld();
  const projected = new Vector3();
  return {
    right: new Vector3().setFromMatrixColumn(camera.matrixWorld, 0),
    up: new Vector3().setFromMatrixColumn(camera.matrixWorld, 1), spacingPixels: DEFAULT_LINK_PIXELS,
    basis: () => { camera.updateMatrixWorld(); return { right: new Vector3().setFromMatrixColumn(camera.matrixWorld, 0), up: new Vector3().setFromMatrixColumn(camera.matrixWorld, 1) }; },
    viewKey: () => { camera.updateMatrixWorld(); const { width, height } = viewport(); return `${width}|${height}|${camera.matrixWorld.elements.join(",")}|${camera.projectionMatrix.elements.join(",")}`; },
    worldPerPixel: (depth: number) => 2 * depth / (viewport().height * camera.projectionMatrix.elements[5]),
    isVisible: (at: { x: number; y: number }, radius: number) => { const { width, height } = viewport(); return Math.abs(at.x) <= width / 2 + radius && Math.abs(at.y) <= height / 2 + radius; },
    radius: (node: PositionedNode, depth: number) => nodeVisualRadius(
      nodeScreenSize(node.kind, depth, viewport().height, camera.projectionMatrix.elements[5]), node.id === selected, node.changed, counts.get(node.id)),
    project: (value: Point) => {
      camera.updateMatrixWorld();
      projected.set(value.x, value.y, value.z).applyMatrix4(camera.matrixWorldInverse);
      const depth = camera.projectionMatrix.elements[11] === -1 ? -projected.z : 1;
      projected.applyMatrix4(camera.projectionMatrix);
      const { width, height } = viewport();
      return { x: projected.x * width / 2, y: projected.y * height / 2, depth };
    },
  };
}

/** The same spatial reveal frame is used by the gesture and immutable layout plan. */
export function spatialCameraFrame(camera: PerspectiveCamera, cameraTarget: Point, group: { center: Point; radius: number }, size: { width: number; height: number }) {
  const vertical = camera.fov * Math.PI / 180;
  const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * size.width / size.height);
  const fitDistance = group.radius * 1.15 / Math.sin(Math.min(vertical, horizontal) / 2);
  const readableDistance = LAYOUT_WORLD_SPACING * size.height * camera.projectionMatrix.elements[5] / (2 * 28);
  const offset = camera.position.clone().sub(new Vector3(cameraTarget.x, cameraTarget.y, cameraTarget.z));
  if (offset.lengthSq() < 1e-9) offset.set(0, 0, 1);
  offset.normalize().multiplyScalar(Math.max(50, Math.min(fitDistance, readableDistance)));
  return { position: { x: group.center.x + offset.x, y: group.center.y + offset.y, z: group.center.z + offset.z }, target: group.center };
}

type CoreView = {
  camera: PerspectiveCamera; cameraTarget: Point; size: { width: number; height: number }; automatic: boolean;
  nodes: readonly PositionedNode[]; links: readonly GraphLink[]; counts: ReadonlyMap<string, number>;
  members: ReadonlySet<string>; selected: string | null; lod: number;
  cohorts?: ReadonlyMap<string, readonly string[]>;
  spatial?: { level: number; members: ReadonlySet<string>; camera: Point; target: Point } | null;
  locks?: ReadonlyMap<string, { members: readonly string[]; level: number }>;
};

/** Plan coordinates and the exact camera used to clear them, without touching live render state. */
export function planCoreView(desired: ReadonlyMap<string, Point>, movable: ReadonlySet<string>, hub: string,
  view: CoreView, clear: Positions["clearLayout"]) {
  const camera = view.camera.clone(), target = new Vector3(view.cameraTarget.x, view.cameraTarget.y, view.cameraTarget.z);
  const nodes = view.nodes.map(node => ({ ...node, ...(desired.get(node.id) ?? node) }));
  const focus = desired.get(view.selected ?? hub)!;
  if (view.automatic) {
    if (!view.selected && view.spatial) {
      const members = nodes.filter(node => view.spatial!.members.has(node.id));
      if (members.length) {
        const center = members.reduce((sum, node) => ({ x: sum.x + node.x / members.length, y: sum.y + node.y / members.length, z: sum.z + node.z / members.length }), { x: 0, y: 0, z: 0 });
        const radius = Math.max(...members.map(node => Math.hypot(node.x - center.x, node.y - center.y, node.z - center.z))) + 6;
        const frame = spatialCameraFrame(camera, target, { center, radius }, view.size);
        target.set(frame.target.x, frame.target.y, frame.target.z); camera.position.set(frame.position.x, frame.position.y, frame.position.z);
      } else {
        // Closing restores the captured frame, rather than re-framing the core.
        target.set(view.spatial.target.x, view.spatial.target.y, view.spatial.target.z);
        camera.position.set(view.spatial.camera.x, view.spatial.camera.y, view.spatial.camera.z);
      }
    } else {
      const offset = camera.position.clone().sub(target);
      const vertical = camera.fov * Math.PI / 180;
      const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * view.size.width / view.size.height);
      const radius = expandedCoreCameraFrame(nodes, view.links, hub, view.members)?.radius ?? 18;
      const distance = view.selected ? focusedCameraDistance(offset.length(), view.size.height, camera.projectionMatrix.elements[5]) :
        coreCameraDistance(radius * 1.15 / Math.sin(Math.min(vertical, horizontal) / 2), view.size.height, camera.projectionMatrix.elements[5]);
      if (offset.lengthSq() < 1e-9) offset.set(0, 0, 1);
      target.copy(focus); camera.position.copy(target).add(offset.normalize().multiplyScalar(distance));
    }
    camera.lookAt(target);
  }
  camera.updateMatrixWorld();
  const scale = view.size.height * camera.projectionMatrix.elements[5] / (2 * camera.position.distanceTo(target));
  const level = Math.max(nucleusLevel(LAYOUT_WORLD_SPACING * scale, view.lod), view.spatial?.members.size ? view.spatial.level : 0);
  const locks = new Map([...view.locks ?? []].filter(([, group]) => group.level === level).map(([id, group]) => [id, group.members]));
  const spatial = nucleusView(nodes, view.links, level, view.selected, view.spatial?.members, locks);
  const counts = new Map(spatial.counts);
  for (const [id, count] of view.counts) counts.set(id, count - Number(nodes.find(node => node.id === id)?.kind === "folder"));
  const plane = { ...screenPlane(camera, () => view.size, view.selected, counts),
    visible: spatial.nodes.map(node => node.id), isVisible: () => true, compactReveal: !view.selected };
  const next = new Map(desired);
  let moving = new Set(movable);
  let held = hub;
  const adjacency = new Map(plane.visible.map(id => [id, new Set<string>()]));
  for (const link of view.links) {
    if (!adjacency.has(link.source) || !adjacency.has(link.target)) continue;
    adjacency.get(link.source)!.add(link.target); adjacency.get(link.target)!.add(link.source);
  }
  const fan = view.selected ? starCohort(view.selected, new Set(plane.visible), adjacency) : null;
  const focusedFan = fan?.hub === hub && fan.leaves.every(id => id === view.selected || moving.has(id)) ? fan : null;
  if (view.selected && view.automatic) {
    // Resolve the moving focus first. Translating camera and target in their
    // plane preserves every depth; each fixed star defines a forbidden focus circle.
    if (focusedFan) moving.add(hub);
    const selected = nodes.find(node => node.id === view.selected)!;
    const selectedRadius = plane.radius(selected, plane.project(focus).depth);
    const circles = spatial.nodes.filter(node => node.id !== selected.id && !moving.has(node.id)).flatMap(node => {
      const at = plane.project(node), radius = plane.radius(node, at.depth), world = plane.worldPerPixel(at.depth);
      return at.depth > 0 && radius > 0 ? [{ id: node.id, x: at.x * world, y: at.y * world,
        radius: (selectedRadius + radius + COLLISION_GAP) * world }] : [];
    });
    const at = nearestFreePoint({ id: selected.id, x: 0, y: 0 }, circles);
    const delta = plane.right.clone().multiplyScalar(at.x).addScaledVector(plane.up, at.y);
    const translated = focusedFan ? [focusedFan.hub, ...focusedFan.leaves] : [selected.id];
    for (const id of translated) {
      const at = next.get(id)!; next.set(id, { x: at.x + delta.x, y: at.y + delta.y, z: at.z + delta.z });
    }
    target.add(delta); camera.position.add(delta); camera.lookAt(target); camera.updateMatrixWorld();
    moving.delete(selected.id); held = selected.id;
  } else if (!view.automatic) {
    // Content footprints grow in the existing frame. Keep its focus still and
    // let the other visible bodies yield, including collapsed background groups.
    held = view.selected ?? hub;
    moving = new Set(plane.visible.filter(id => id !== held));
  }
  const resolved = clear(next, moving, held, plane);
  const visible = new Set(plane.visible);
  const cohorts = new Map([...view.cohorts ?? [], ...[...spatial.groups].map(([id, group]) => [id, group.members] as const)]);
  for (const [id, members] of cohorts) {
    const from = next.get(id), to = resolved.get(id);
    if (!from || !to || from.x === to.x && from.y === to.y && from.z === to.z) continue;
    for (const member of members) {
      const at = next.get(member);
      if (at && !visible.has(member)) resolved.set(member, { x: at.x + to.x - from.x, y: at.y + to.y - from.y, z: at.z + to.z - from.z });
    }
  }
  const memberships = new Map([...spatial.groups].map(([id, group]) => [id, group.members]));
  const linked = new Set(view.links.flatMap(link => [link.source, link.target]));
  for (const node of spatial.nodes) {
    if (knowledge(node) && !linked.has(node.id) && !memberships.has(node.id) && node.id !== view.selected && !view.spatial?.members.has(node.id)) {
      memberships.set(node.id, [node.id]);
    }
  }
  return { positions: resolved, camera, target: { x: target.x, y: target.y, z: target.z }, level, counts, visible: plane.visible, memberships };
}
