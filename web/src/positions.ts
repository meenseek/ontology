import { visualSatellites } from "./graph";
import type { Model, PositionedNode } from "./graph";

export type Point = { x: number; y: number; z: number };
const point = ({ x, y, z }: Point): Point => ({ x, y, z });
const add = (a: Point, b: Point, scale = 1): Point => ({ x: a.x + b.x * scale, y: a.y + b.y * scale, z: a.z + b.z * scale });
const zero = (): Point => ({ x: 0, y: 0, z: 0 });
export function fixPosition(node: PositionedNode, value: Point) {
  node.x = node.fx = value.x; node.y = node.fy = value.y; node.z = node.fz = value.z;
}
type Follower = { offset: Point; velocity: Point };
type Pull = { id: string; start: Point; threshold: number; followers: Map<string, Follower>; moved: boolean };
type Slot = { x: number; y: number };
function compactSlots(root: string, members: string[], adjacency: Map<string, Set<string>>) {
  const slots: Slot[] = [];
  // Exact triangular lattice: every nearest-neighbor spacing is one unit.
  for (let ring = 1; slots.length < members.length; ring++) {
    let q = ring, r = 0;
    for (const [dq, dr] of [[0, -1], [-1, 0], [-1, 1], [0, 1], [1, 0], [1, -1]]) {
      for (let step = 0; step < ring; step++) {
        slots.push({ x: q + r / 2, y: r * Math.sqrt(3) / 2 }); q += dq; r += dr;
      }
    }
  }
  const allowed = new Set([root, ...members]), visited = new Set([root]), order = [root];
  // BFS keeps direct neighbors near the drop center before more distant ones.
  for (let index = 0; order.length < allowed.size || index < order.length; index++) {
    if (index === order.length) {
      const next = members.find(id => !visited.has(id))!; visited.add(next); order.push(next);
    }
    for (const id of [...(adjacency.get(order[index]) ?? [])].filter(id => allowed.has(id)).sort()) {
      if (!visited.has(id)) { visited.add(id); order.push(id); }
    }
  }
  const placed = new Map<string, Slot>([[root, { x: 0, y: 0 }]]);
  for (const id of order.slice(1)) {
    const neighbors = [...(adjacency.get(id) ?? [])].flatMap(other => placed.has(other) ? [placed.get(other)!] : []);
    const center = neighbors.length ? neighbors.reduce((sum, slot) => ({ x: sum.x + slot.x / neighbors.length, y: sum.y + slot.y / neighbors.length }), { x: 0, y: 0 }) : { x: 0, y: 0 };
    // Distance to the centroid minimizes summed squared lengths to placed neighbors.
    // A tiny center preference breaks ties into a compact, deterministic constellation.
    const balance = [...placed.values()].reduce((sum, slot) => ({ x: sum.x + slot.x / placed.size, y: sum.y + slot.y / placed.size }), { x: 0, y: 0 });
    let best = 0, score = Infinity;
    for (const [index, slot] of slots.entries()) {
      const candidate = (slot.x - center.x) ** 2 + (slot.y - center.y) ** 2 + .001 * (slot.x ** 2 + slot.y ** 2) + .000001 * (slot.x * balance.x + slot.y * balance.y);
      if (candidate < score - 1e-9) { score = candidate; best = index; }
    }
    placed.set(id, slots.splice(best, 1)[0]);
  }
  placed.delete(root); return placed;
}
type Spring = { anchor: Point; followers: Map<string, Follower>; last: number; deadline: number; settled: boolean };
// Session-only coordinates. Membership and the automatic layout remain owned by reconcile().
export class Positions {
  revision = 0;
  private nodes = new Map<string, PositionedNode>();
  private satellites = new Map<string, string>();
  private adjacency = new Map<string, Set<string>>();
  private baseline = new Map<string, Point>();
  private gesture: Pull | null = null;
  private spring: Spring | null = null;
  get dragging() { return this.gesture !== null; }
  install(model: Model) {
    this.cancel();
    const retained = this.nodes, priorSatellites = this.satellites;
    this.satellites = visualSatellites(model);
    this.baseline = new Map(model.nodes.map(n => [n.id, point(n)]));
    this.nodes = new Map(model.nodes.map(n => [n.id, n]));
    this.adjacency = new Map(model.nodes.map(n => [n.id, new Set<string>()]));
    for (const link of model.links) {
      this.adjacency.get(link.source)?.add(link.target); this.adjacency.get(link.target)?.add(link.source);
    }
    for (const node of model.nodes) {
      const prior = retained.get(node.id);
      const host = this.satellites.get(node.id);
      if (prior && host === priorSatellites.get(node.id)) fixPosition(node, prior);
      else if (host && retained.has(host)) {
        // New/reanchored satellites follow a host's temporary session translation.
        const original = this.baseline.get(host)!, current = retained.get(host)!;
        fixPosition(node, add(node, add(current, original, -1)));
      }
    }
    this.revision++;
  }
  reset() {
    this.cancel();
    for (const [id, value] of this.baseline) fixPosition(this.nodes.get(id)!, value);
    this.revision++;
  }
  begin(id: string, unitsPerPixel: number, plane = { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24 }) {
    this.cancel();
    const node = this.nodes.get(id);
    if (!node || !Number.isFinite(unitsPerPixel) || unitsPerPixel <= 0) return;
    const visualCluster = (n: PositionedNode) => this.nodes.get(this.satellites.get(n.id) ?? n.id)!.cluster;
    const hostCluster = visualCluster(node);
    const members = [...this.nodes.values()].filter(n => visualCluster(n) === hostCluster && n.id !== id).sort((a, b) => a.id < b.id ? -1 : 1);
    const followers = new Map<string, Follower>();
    const spacing = Math.max(24, plane.spacingPixels) * unitsPerPixel;
    for (const [member, slot] of compactSlots(id, members.map(n => n.id), this.adjacency)) {
      followers.set(member, { offset: add(add(zero(), plane.right, slot.x * spacing), plane.up, slot.y * spacing), velocity: zero() });
    }
    this.gesture = { id, start: point(node), threshold: unitsPerPixel * 6, followers, moved: false };
  }
  move(id: string, value: Point) {
    const drag = this.gesture;
    if (!drag || drag.id !== id || ![value.x, value.y, value.z].every(Number.isFinite)) return;
    fixPosition(this.nodes.get(id)!, value);
    const delta = add(value, drag.start, -1);
    if (Math.hypot(delta.x, delta.y, delta.z) >= drag.threshold) drag.moved = true;
    this.revision++;
  }
  release(now: number, reduced: boolean) {
    const drag = this.gesture;
    this.gesture = null;
    if (!drag?.moved || !Number.isFinite(now)) return;
    // The drop point stays fixed. Only followers spring into the frozen layout.
    this.spring = { anchor: point(this.nodes.get(drag.id)!), followers: drag.followers, last: now, deadline: now + 1100, settled: false };
    if (reduced) this.advance(now, true);
  }
  advance(now: number, reduced: boolean) {
    const spring = this.spring;
    if (!spring || spring.settled || !Number.isFinite(now)) return;
    const dt = Math.max(0, (now - spring.last) / 1000);
    if (!dt && !reduced) return;
    const finish = reduced || now >= spring.deadline;
    // Exact underdamped solution (20 rad/s, damping ratio .43): frame-rate
    // independent, visible overshoot, then a bounded 1.1-second settling tail.
    const decay = 8.6, frequency = Math.sqrt(400 - decay * decay);
    const attenuation = Math.exp(-decay * dt), cosine = Math.cos(frequency * dt), sine = Math.sin(frequency * dt);
    for (const [id, follower] of spring.followers) {
      const node = this.nodes.get(id)!, target = add(spring.anchor, follower.offset), next = point(node);
      for (const axis of ["x", "y", "z"] as const) {
        const displacement = node[axis] - target[axis], velocity = follower.velocity[axis];
        next[axis] = finish ? target[axis] : target[axis] + attenuation * (displacement * cosine + (velocity + decay * displacement) / frequency * sine);
        follower.velocity[axis] = finish ? 0 : attenuation * (velocity * cosine - (decay * velocity + 400 * displacement) / frequency * sine);
      }
      fixPosition(node, next);
    }
    spring.last = Math.max(spring.last, now); spring.settled = finish;
    this.revision++;
  }
  cancel() {
    // Freeze current coordinates on cancellation; do not teleport to a target.
    this.gesture = null; this.spring = null;
  }
}
