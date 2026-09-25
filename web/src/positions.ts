import { visualSatellites } from "./graph";
import type { Model, PositionedNode } from "./graph";
import { separateDiscs } from "./clearance";

export type Point = { x: number; y: number; z: number };
const point = ({ x, y, z }: Point): Point => ({ x, y, z });
const add = (a: Point, b: Point, scale = 1): Point => ({ x: a.x + b.x * scale, y: a.y + b.y * scale, z: a.z + b.z * scale });
const zero = (): Point => ({ x: 0, y: 0, z: 0 });
export function fixPosition(node: PositionedNode, value: Point) {
  node.x = node.fx = value.x; node.y = node.fy = value.y; node.z = node.fz = value.z;
}
type Follower = { offset: Point };
type TensionEdge = { from: string; to: string; limit: number };
type Slot = { x: number; y: number };
type ScreenPoint = Slot & { depth: number };
type DragPlane = { right: Point; up: Point; spacingPixels: number; project?: (value: Point) => ScreenPoint;
  visible?: readonly string[]; radius?: (node: PositionedNode, depth: number) => number; worldPerPixel?: (depth: number) => number;
  isVisible?: (at: ScreenPoint, radius: number) => boolean };
type Clearance = { held: string; visible: string[]; right: Point; up: Point; project: (value: Point) => ScreenPoint;
  radius: (node: PositionedNode, depth: number) => number; worldPerPixel: (depth: number) => number;
  isVisible: (at: ScreenPoint, radius: number) => boolean };
type Pull = { id: string; start: Point; threshold: number; tolerance: number; followers: Map<string, Follower>; edges: TensionEdge[]; project: (value: Point) => ScreenPoint; clearance: Clearance | null; moved: boolean; last: number | null };
type Axial = { q: number; r: number };
const directions: Axial[] = [{ q: 1, r: 0 }, { q: 0, r: -1 }, { q: -1, r: 1 }, { q: -1, r: 0 }, { q: 0, r: 1 }, { q: 1, r: -1 }];
const axialKey = ({ q, r }: Axial) => `${q},${r}`;
const axialDistance = (a: Axial, b: Axial) => (a.q - b.q) ** 2 + (a.q - b.q) * (a.r - b.r) + (a.r - b.r) ** 2;
const slot = ({ q, r }: Axial): Slot => ({ x: q + r / 2, y: r * Math.sqrt(3) / 2 });
function cycleGeometry(count: number): Axial[] {
  type CycleSlot = Axial & { next: CycleSlot };
  const start = { q: 0, r: 0 } as CycleSlot, first = { q: 1, r: 0 } as CycleSlot, third = { q: 0, r: 1 } as CycleSlot;
  start.next = first; first.next = third; third.next = start;
  const occupied = new Set([axialKey(start), axialKey(first), axialKey(third)]);
  type Candidate = { from: CycleSlot; to: CycleSlot; point: Axial; distance: number };
  let frontier: Candidate[] = [];
  const offer = (from: CycleSlot, to: CycleSlot) => {
    for (const direction of directions) {
      const point = { q: from.q + direction.q, r: from.r + direction.r };
      if (axialDistance(point, to) === 1 && !occupied.has(axialKey(point))) frontier.push({ from, to, point, distance: axialDistance(point, start) });
    }
  };
  offer(start, first); offer(first, third); offer(third, start);
  for (let placed = 3; placed < count; placed++) {
    frontier = frontier.filter(({ from, to, point }) => from.next === to && !occupied.has(axialKey(point)));
    if (!frontier.length) return [];
    let best = 0;
    for (let index = 1; index < frontier.length; index++) if (frontier[index].distance < frontier[best].distance) best = index;
    const { from, to, point } = frontier.splice(best, 1)[0];
    const added = { ...point, next: to }; from.next = added; occupied.add(axialKey(added));
    offer(from, added); offer(added, to);
  }
  const coordinates: CycleSlot[] = [];
  let cursor = start;
  do { coordinates.push(cursor); cursor = cursor.next; } while (cursor !== start);
  return coordinates;
}
function perimeterGeometry(count: number): Axial[] {
  if (count === 3) return [{ q: 0, r: 0 }, { q: 1, r: 0 }, { q: 0, r: 1 }];
  const half = Math.floor(count / 2), width = Math.floor(half / 2), height = half - width;
  const points: Axial[] = [];
  for (let q = 0; q < width; q++) points.push({ q, r: 0 });
  for (let r = 0; r < height; r++) points.push({ q: width, r });
  for (let q = width; q > 0; q--) points.push({ q, r: height });
  for (let r = height; r > 0; r--) points.push({ q: 0, r });
  if (count % 2) points.splice(1, 0, { q: 1, r: -1 });
  return points;
}
function sparseSlots(root: string, members: string[], adjacency: Map<string, Set<string>>): Map<string, Slot> | null {
  const ids = [root, ...members], allowed = new Set(ids);
  const neighbors = new Map(ids.map(id => [id, [...(adjacency.get(id) ?? [])].filter(other => allowed.has(other)).sort()]));
  const edgeCount = [...neighbors.values()].reduce((sum, values) => sum + values.length, 0) / 2;
  if (edgeCount !== ids.length - 1 && edgeCount !== ids.length) return null;
  const degree = new Map(ids.map(id => [id, neighbors.get(id)!.length]));
  if (edgeCount === ids.length - 1 && ids.length > 2 && [...degree.values()].every(value => value <= 2)) {
    // A cycle with one vertex removed is a compact, collision-free unit-edge path.
    const endpoints = ids.filter(id => degree.get(id) === 1).sort();
    const geometry = cycleGeometry(ids.length + 1);
    if (endpoints.length === 2 && geometry.length === ids.length + 1) {
      const order = [endpoints[0]], visited = new Set(order);
      while (order.length < ids.length) {
        const next = neighbors.get(order.at(-1)!)!.find(id => !visited.has(id));
        if (!next) break;
        order.push(next); visited.add(next);
      }
      if (order.length === ids.length) {
        const origin = geometry[order.indexOf(root)];
        return new Map(order.flatMap((id, index) => id === root ? [] : [[id, slot({ q: geometry[index].q - origin.q, r: geometry[index].r - origin.r })] as [string, Slot]]));
      }
    }
  }
  const queue = ids.filter(id => degree.get(id)! < 2), peeled = new Set<string>();
  for (let index = 0; index < queue.length; index++) {
    const id = queue[index];
    if (peeled.has(id)) continue;
    peeled.add(id);
    for (const other of neighbors.get(id)!) {
      if (peeled.has(other)) continue;
      const next = degree.get(other)! - 1;
      degree.set(other, next);
      if (next < 2) queue.push(other);
    }
  }
  const core = ids.filter(id => !peeled.has(id)), coreSet = new Set(core);
  if (core.length && (core.length < 3 || edgeCount !== ids.length || core.some(id => neighbors.get(id)!.filter(other => coreSet.has(other)).length !== 2))) return null;
  if (!core.length && edgeCount !== ids.length - 1) return null;
  const cycle: string[] = [];
  if (core.length) {
    const start = [...core].sort()[0];
    cycle.push(start);
    let previous = start, current = neighbors.get(start)!.find(id => coreSet.has(id))!;
    while (current !== start && cycle.length < core.length) {
      cycle.push(current);
      const next = neighbors.get(current)!.find(id => coreSet.has(id) && id !== previous)!;
      previous = current; current = next;
    }
    if (current !== start || cycle.length !== core.length) return null;
  }
  const seeds = core.length ? core : [root], visited = new Set(seeds), parent = new Map<string, string>(), levels: string[][] = [];
  let front = seeds;
  while (front.length) {
    const next: string[] = [];
    for (const id of front) for (const other of neighbors.get(id)!) {
      if (visited.has(other)) continue;
      visited.add(other); parent.set(other, id); next.push(other);
    }
    if (next.length) levels.push(next.sort());
    front = next;
  }
  if (visited.size !== ids.length) return null;
  const branchCounts = cycle.map(id => neighbors.get(id)!.filter(other => !coreSet.has(other)).length);
  const geometries = core.length ? [cycleGeometry(core.length), perimeterGeometry(core.length)] : [[]];
  for (const geometry of geometries) {
    if (core.length && geometry.length !== core.length) continue;
    const occupiedCore = new Set(geometry.map(axialKey));
    const freeCoreNeighbors = geometry.map(value => directions.filter(direction => !occupiedCore.has(axialKey({ q: value.q + direction.q, r: value.r + direction.r }))).length);
    const tries = core.length ? core.length * 2 : 1;
    for (let attempt = 0; attempt < tries; attempt++) {
    const placed = new Map<string, Axial>();
    if (core.length) {
      const offset = Math.floor(attempt / 2), reverse = attempt % 2 === 1;
      if (branchCounts.some((needed, index) => needed > freeCoreNeighbors[(offset + (reverse ? core.length - index : index)) % core.length])) continue;
      for (const [index, id] of cycle.entries()) placed.set(id, geometry[(offset + (reverse ? core.length - index : index)) % core.length]);
    } else placed.set(root, { q: 0, r: 0 });
    const occupied = new Set([...placed.values()].map(axialKey));
    let possible = true;
    for (const level of levels) {
      const options = new Map(level.map(id => {
        const at = placed.get(parent.get(id)!)!;
        const candidates = directions.map(direction => ({ q: at.q + direction.q, r: at.r + direction.r }))
          .filter(value => !occupied.has(axialKey(value)))
          .sort((a, b) => axialDistance(b, { q: 0, r: 0 }) - axialDistance(a, { q: 0, r: 0 }));
        return [id, candidates] as const;
      }));
      const owner = new Map<string, string>(), assigned = new Map<string, Axial>();
      const assign = (id: string, seen: Set<string>): boolean => {
        for (const value of options.get(id)!) {
          const key = axialKey(value);
          if (seen.has(key)) continue;
          seen.add(key);
          const prior = owner.get(key);
          if (!prior || assign(prior, seen)) { owner.set(key, id); assigned.set(id, value); return true; }
        }
        return false;
      };
      for (const id of [...level].sort((a, b) => options.get(a)!.length - options.get(b)!.length)) {
        if (!assign(id, new Set())) { possible = false; break; }
      }
      if (!possible) break;
      for (const [id, value] of assigned) { placed.set(id, value); occupied.add(axialKey(value)); }
    }
      if (!possible || placed.size !== ids.length) continue;
      const origin = placed.get(root)!;
      return new Map([...placed].filter(([id]) => id !== root).map(([id, value]) => [id, slot({ q: value.q - origin.q, r: value.r - origin.r })]));
    }
  }
  return null;
}
function cactusSlots(root: string, members: string[], adjacency: Map<string, Set<string>>, deadline: number, limit: number): Map<string, Slot> | null {
  const ids = [root, ...members], allowed = new Set(ids);
  const neighbors = new Map(ids.map(id => [id, [...(adjacency.get(id) ?? [])].filter(other => allowed.has(other) && other !== id).sort()]));
  const edgeKey = (a: string, b: string) => JSON.stringify(a < b ? [a, b] : [b, a]);
  const discovery = new Map<string, number>(), low = new Map<string, number>(), bridges = new Set<string>();
  let clock = 0;
  const visit = (id: string, parent: string | null) => {
    discovery.set(id, ++clock); low.set(id, clock);
    for (const other of neighbors.get(id)!) {
      if (other === parent) continue;
      if (!discovery.has(other)) {
        visit(other, id);
        low.set(id, Math.min(low.get(id)!, low.get(other)!));
        if (low.get(other)! > discovery.get(id)!) bridges.add(edgeKey(id, other));
      } else low.set(id, Math.min(low.get(id)!, discovery.get(other)!));
    }
  };
  visit(root, null);
  if (discovery.size !== ids.length || !bridges.size) return null;
  const componentOf = new Map<string, number>(), components: string[][] = [];
  for (const id of ids) {
    if (componentOf.has(id)) continue;
    const component: string[] = [], queue = [id], index = components.length;
    componentOf.set(id, index);
    for (let head = 0; head < queue.length; head++) {
      const current = queue[head]; component.push(current);
      for (const other of neighbors.get(current)!) {
        if (bridges.has(edgeKey(current, other)) || componentOf.has(other)) continue;
        componentOf.set(other, index); queue.push(other);
      }
    }
    components.push(component);
  }
  type Block = { order: string[]; shapes: Axial[][]; links: { block: number; from: string; to: string }[] };
  const blocks: Block[] = [];
  for (const component of components) {
    if (component.length === 1) { blocks.push({ order: component, shapes: [[{ q: 0, r: 0 }]], links: [] }); continue; }
    const inside = new Set(component);
    if (component.length < 3 || component.some(id => neighbors.get(id)!.filter(other => inside.has(other)).length !== 2)) return null;
    const start = [...component].sort()[0], order = [start];
    let previous = start, current = neighbors.get(start)!.find(id => inside.has(id))!;
    while (current !== start && order.length < component.length) {
      order.push(current);
      const next = neighbors.get(current)!.find(id => inside.has(id) && id !== previous)!;
      previous = current; current = next;
    }
    if (current !== start || order.length !== component.length) return null;
    blocks.push({ order, shapes: [cycleGeometry(order.length), perimeterGeometry(order.length)].filter(shape => shape.length === order.length), links: [] });
  }
  for (const id of ids) for (const other of neighbors.get(id)!) {
    if (id > other || !bridges.has(edgeKey(id, other))) continue;
    const a = componentOf.get(id)!, b = componentOf.get(other)!;
    blocks[a].links.push({ block: b, from: id, to: other });
    blocks[b].links.push({ block: a, from: other, to: id });
  }
  for (const block of blocks) {
    if (block.shapes.length > 1 && block.links.some(link => block.links.filter(other => other.from === link.from).length > 1)) block.shapes.reverse();
  }
  const rootBlock = componentOf.get(root)!, placed = new Map<string, Axial>(), occupied = new Set<string>(), stack: string[] = [];
  let proposals = 0;
  const rollback = (mark: number) => {
    while (stack.length > mark) {
      const id = stack.pop()!;
      occupied.delete(axialKey(placed.get(id)!)); placed.delete(id);
    }
  };
  const place = (blockIndex: number, parentBlock: number | null, attachment: { from: string; to: string } | null): boolean => {
    const block = blocks[blockIndex], children = block.links.filter(link => link.block !== parentBlock)
      .sort((a, b) => blocks[b.block].order.length - blocks[a.block].order.length || a.from.localeCompare(b.from));
    const targets = attachment ? directions.map(direction => {
      const from = placed.get(attachment.from)!;
      return { q: from.q + direction.q, r: from.r + direction.r };
    }).filter(value => !occupied.has(axialKey(value)))
      .sort((a, b) => axialDistance(b, { q: 0, r: 0 }) - axialDistance(a, { q: 0, r: 0 })) : [{ q: 0, r: 0 }];
    if (!targets.length) return false;
    for (const shape of block.shapes) {
      const localOccupied = new Set(shape.map(axialKey));
      const free = shape.map(value => directions.filter(direction => !localOccupied.has(axialKey({ q: value.q + direction.q, r: value.r + direction.r }))).length);
      const center = shape.reduce((sum, value) => ({ q: sum.q + value.q / shape.length, r: sum.r + value.r / shape.length }), { q: 0, r: 0 });
      const choices: { offset: number; reverse: boolean; target: Axial; score: number }[] = [];
      for (let offset = 0; offset < block.order.length; offset++) for (const reverse of [false, true]) {
        if (block.order.some((id, index) => block.links.filter(link => link.from === id).length > free[(offset + (reverse ? block.order.length - index : index)) % block.order.length])) continue;
        const anchorIndex = block.order.indexOf(attachment?.to ?? root);
        const reference = shape[(offset + (reverse ? block.order.length - anchorIndex : anchorIndex)) % block.order.length];
        for (const target of targets) {
          const translation = { q: target.q - reference.q, r: target.r - reference.r };
          choices.push({ offset, reverse, target, score: axialDistance({ q: center.q + translation.q, r: center.r + translation.r }, { q: 0, r: 0 }) });
        }
      }
      if (attachment) choices.sort((a, b) => a.score - b.score);
      for (const { offset, reverse, target } of choices) {
        if (++proposals > limit || performance.now() > deadline) return false;
        const anchorIndex = block.order.indexOf(attachment?.to ?? root);
        const reference = shape[(offset + (reverse ? block.order.length - anchorIndex : anchorIndex)) % block.order.length];
        const translation = { q: target.q - reference.q, r: target.r - reference.r }, mark = stack.length;
        let clear = true;
        for (const [index, id] of block.order.entries()) {
          const value = shape[(offset + (reverse ? block.order.length - index : index)) % block.order.length];
          const coordinate = { q: value.q + translation.q, r: value.r + translation.r }, key = axialKey(coordinate);
          if (occupied.has(key)) { clear = false; break; }
          placed.set(id, coordinate); occupied.add(key); stack.push(id);
        }
        if (clear) {
          const needed = new Map<string, number>();
          for (const child of children) needed.set(child.from, (needed.get(child.from) ?? 0) + 1);
          for (const [id, count] of needed) {
            const at = placed.get(id)!;
            if (directions.filter(direction => !occupied.has(axialKey({ q: at.q + direction.q, r: at.r + direction.r }))).length < count) { clear = false; break; }
          }
        }
        if (clear && children.every(link => place(link.block, blockIndex, link))) return true;
        rollback(mark);
      }
    }
    return false;
  };
  if (!place(rootBlock, null, null) || placed.size !== ids.length) return null;
  return new Map([...placed].filter(([id]) => id !== root).map(([id, value]) => [id, slot(value)]));
}
function constraintSlots(root: string, members: string[], adjacency: Map<string, Set<string>>, deadline: number, limit: number): Map<string, Slot> | null {
  const ids = [root, ...members], allowed = new Set(ids);
  const neighbors = new Map(ids.map(id => [id, [...(adjacency.get(id) ?? [])].filter(other => allowed.has(other)).sort()]));
  if ([...neighbors.values()].some(values => values.length > 6)) return null;
  const byDegree = (a: string, b: string) => neighbors.get(b)!.length - neighbors.get(a)!.length || a.localeCompare(b);
  const pivot = [...ids].sort(byDegree)[0];
  const first = [...neighbors.get(pivot)!].sort(byDegree)[0];
  if (!first) return null;
  const placed = new Map<string, Axial>([[pivot, { q: 0, r: 0 }], [first, { q: 1, r: 0 }]]), occupied = new Set(["0,0", "1,0"]);
  let attempts = 0;
  const search = (): boolean => {
    if (placed.size === ids.length) return true;
    if (++attempts > limit || performance.now() > deadline) return false;
    for (const [id, value] of placed) {
      const needed = neighbors.get(id)!.filter(other => !placed.has(other)).length;
      if (needed > directions.filter(direction => !occupied.has(axialKey({ q: value.q + direction.q, r: value.r + direction.r }))).length) return false;
    }
    let next: { id: string; candidates: Axial[]; placedNeighbors: number } | null = null;
    for (const id of ids) {
      if (placed.has(id)) continue;
      const fixed = neighbors.get(id)!.filter(other => placed.has(other));
      if (!fixed.length) continue;
      const anchor = placed.get(fixed[0])!;
      const remaining = neighbors.get(id)!.length - fixed.length;
      const candidates = directions.map(direction => ({ q: anchor.q + direction.q, r: anchor.r + direction.r }))
        .filter(value => !occupied.has(axialKey(value)) && fixed.every(other => axialDistance(value, placed.get(other)!) === 1)
          && directions.filter(direction => !occupied.has(axialKey({ q: value.q + direction.q, r: value.r + direction.r }))).length >= remaining)
        .sort((a, b) => axialDistance(a, { q: 0, r: 0 }) - axialDistance(b, { q: 0, r: 0 }) || b.q - a.q || b.r - a.r);
      if (!candidates.length) return false;
      if (!next || fixed.length > next.placedNeighbors || (fixed.length === next.placedNeighbors && candidates.length < next.candidates.length)) next = { id, candidates, placedNeighbors: fixed.length };
    }
    if (!next) return false;
    for (const value of next.candidates) {
      const key = axialKey(value);
      placed.set(next.id, value); occupied.add(key);
      if (search()) return true;
      placed.delete(next.id); occupied.delete(key);
    }
    return false;
  };
  if (!search()) return null;
  const origin = placed.get(root)!;
  placed.delete(root);
  return new Map([...placed].map(([id, value]) => [id, slot({ q: value.q - origin.q, r: value.r - origin.r })]));
}
function validSlots(root: string, members: string[], adjacency: Map<string, Set<string>>, placed: Map<string, Slot> | null): placed is Map<string, Slot> {
  if (!placed || placed.size !== members.length) return false;
  const points = new Map<string, Slot>([[root, { x: 0, y: 0 }], ...placed]);
  const entries = [...points];
  for (let index = 0; index < entries.length; index++) {
    const [id, a] = entries[index];
    if (!Number.isFinite(a.x) || !Number.isFinite(a.y)) return false;
    for (let next = index + 1; next < entries.length; next++) {
      const b = entries[next][1];
      if ((a.x - b.x) ** 2 + (a.y - b.y) ** 2 < 1 - 1e-9) return false;
    }
    for (const other of adjacency.get(id) ?? []) {
      const b = points.get(other);
      if (b && Math.abs((a.x - b.x) ** 2 + (a.y - b.y) ** 2 - 1) > 1e-9) return false;
    }
  }
  return true;
}
function ringSlots(count: number, extraRing = false): Slot[] {
  const slots: Slot[] = [];
  const addRing = (ring: number) => {
    let q = ring, r = 0;
    for (const [dq, dr] of [[0, -1], [-1, 0], [-1, 1], [0, 1], [1, 0], [1, -1]]) {
      for (let step = 0; step < ring; step++) {
        slots.push({ x: q + r / 2, y: r * Math.sqrt(3) / 2 }); q += dq; r += dr;
      }
    }
  };
  let ring = 1;
  while (slots.length < count) addRing(ring++);
  if (extraRing) addRing(ring);
  return slots;
}
function packedSlots(root: string, members: string[], adjacency: Map<string, Set<string>>, projected: Map<string, Slot>): Map<string, Slot> {
  const ids = [root, ...members], allowed = new Set(ids);
  const degree = (id: string) => [...(adjacency.get(id) ?? [])].filter(other => allowed.has(other)).length;
  const pivot = [...ids].sort((a, b) => degree(b) - degree(a) || a.localeCompare(b))[0];
  const atPivot = projected.get(pivot)!;
  const distance = (id: string) => {
    const at = projected.get(id)!;
    return Math.hypot(at.x - atPivot.x, at.y - atPivot.y);
  };
  const order = [pivot], seen = new Set(order);
  for (let index = 0; index < order.length; index++) {
    const neighbors = [...(adjacency.get(order[index]) ?? [])].filter(id => allowed.has(id) && !seen.has(id))
      .sort((a, b) => distance(a) - distance(b) || a.localeCompare(b));
    for (const id of neighbors) { seen.add(id); order.push(id); }
  }
  for (const id of ids.filter(id => !seen.has(id)).sort((a, b) => distance(a) - distance(b) || a.localeCompare(b))) order.push(id);
  const slots = ringSlots(ids.length - 1), placed = new Map<string, Slot>([[pivot, { x: 0, y: 0 }]]);
  for (const [index, id] of order.slice(1).entries()) placed.set(id, slots[index]);
  const origin = placed.get(root)!;
  return new Map([...placed].filter(([id]) => id !== root).map(([id, at]) => [id, { x: at.x - origin.x, y: at.y - origin.y }]));
}
export function compactSlots(root: string, members: string[], adjacency: Map<string, Set<string>>, timeoutMs = 24, limit = Math.min(10000, 8000 + (members.length + 1) * 10)): Map<string, Slot> | null {
  const allowed = new Set([root, ...members]);
  if ([...allowed].every(id => ![...(adjacency.get(id) ?? [])].some(other => allowed.has(other)))) {
    const slots = ringSlots(members.length);
    return new Map(members.map((id, index) => [id, slots[index]]));
  }
  const deadline = performance.now() + timeoutMs;
  const sparse = sparseSlots(root, members, adjacency);
  if (validSlots(root, members, adjacency, sparse)) return sparse;
  if (performance.now() > deadline) return null;
  const cactus = cactusSlots(root, members, adjacency, deadline, limit);
  if (validSlots(root, members, adjacency, cactus)) return cactus;
  if (performance.now() > deadline) return null;
  const exact = constraintSlots(root, members, adjacency, deadline, limit);
  if (validSlots(root, members, adjacency, exact)) return exact;
  if (performance.now() > deadline) return null;
  // Exact triangular lattice: every nearest-neighbor spacing is one unit.
  // Leave the next ring available when direct neighbors need more than six slots.
  const slots = ringSlots(members.length, true);
  const visited = new Set([root]), order = [root];
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
    if (performance.now() > deadline) return null;
    const neighbors = [...(adjacency.get(id) ?? [])].flatMap(other => placed.has(other) ? [placed.get(other)!] : []);
    const center = neighbors.length ? neighbors.reduce((sum, slot) => ({ x: sum.x + slot.x / neighbors.length, y: sum.y + slot.y / neighbors.length }), { x: 0, y: 0 }) : { x: 0, y: 0 };
    // Distance to the centroid minimizes summed squared lengths to placed neighbors.
    // A tiny center preference breaks ties into a compact, deterministic constellation.
    const balance = [...placed.values()].reduce((sum, slot) => ({ x: sum.x + slot.x / placed.size, y: sum.y + slot.y / placed.size }), { x: 0, y: 0 });
    let best = 0, score = Infinity;
    for (const [index, slot] of slots.entries()) {
      const edgeExcess = neighbors.reduce((sum, other) => sum + Math.max(0, (slot.x - other.x) ** 2 + (slot.y - other.y) ** 2 - 1), 0);
      const candidate = 1e6 * edgeExcess + (slot.x - center.x) ** 2 + (slot.y - center.y) ** 2 + .001 * (slot.x ** 2 + slot.y ** 2) + .000001 * (slot.x * balance.x + slot.y * balance.y);
      if (candidate < score - 1e-9) { score = candidate; best = index; }
    }
    placed.set(id, slots.splice(best, 1)[0]);
  }
  placed.delete(root);
  return validSlots(root, members, adjacency, placed) ? placed : null;
}
type Settle = { anchor: Point; followers: Map<string, Follower>; velocities: Map<string, Point>; clearance: Clearance | null; tolerance: number; last: number; deadline: number };
type PendingLayout = { token: number; root: string; right: Point; up: Point; spacing: number; clearance: Clearance | null; reduced: boolean };
// Session-only coordinates. Membership and the automatic layout remain owned by reconcile().
export class Positions {
  revision = 0;
  private nodes = new Map<string, PositionedNode>();
  private satellites = new Map<string, string>();
  private adjacency = new Map<string, Set<string>>();
  private baseline = new Map<string, Point>();
  private observed = new Map<string, Point>();
  private gesture: Pull | null = null;
  private settle: Settle | null = null;
  private worker: Worker | null = null;
  private pending: PendingLayout | null = null;
  private token = 0;
  get dragging() { return this.gesture !== null; }
  get settling() { return this.settle !== null; }
  private clearedPositions(clearance: Clearance, desired: Map<string, Point>, linked: ReadonlySet<string>, gap = 10): Map<string, Point> {
    const discs = clearance.visible.flatMap(id => {
      const node = this.nodes.get(id), value = desired.get(id);
      if (!node || !value) return [];
      const at = clearance.project(value), radius = clearance.radius(node, at.depth);
      return at.depth > 0 && [at.x, at.y, radius].every(Number.isFinite) && radius > 0 && clearance.isVisible(at, radius) ? [{ id, x: at.x, y: at.y, radius }] : [];
    });
    const separated = separateDiscs(discs, clearance.held, linked, gap);
    const next = new Map(desired);
    for (const disc of discs) {
      if (disc.id === clearance.held) continue;
      const at = separated.get(disc.id)!;
      const scale = clearance.worldPerPixel(clearance.project(desired.get(disc.id)!).depth);
      if (!Number.isFinite(scale) || scale <= 0) continue;
      const dx = (at.x - disc.x) * scale, dy = (at.y - disc.y) * scale;
      if (dx || dy) next.set(disc.id, add(add(desired.get(disc.id)!, clearance.right, dx), clearance.up, dy));
    }
    return next;
  }
  private enforceClearance(clearance: Clearance | null, linked: ReadonlySet<string>, softFraction = 0): Set<string> {
    const changed = new Set<string>();
    if (!clearance) return changed;
    const current = new Map(clearance.visible.flatMap(id => {
      const node = this.nodes.get(id);
      return node ? [[id, point(node)] as const] : [];
    }));
    let desired = current;
    if (softFraction > 0) {
      const soft = this.clearedPositions(clearance, current, linked, 14);
      desired = new Map([...current].map(([id, value]) => [id, id === clearance.held ? value : add(value, add(soft.get(id) ?? value, value, -1), softFraction)]));
    }
    for (const [id, value] of this.clearedPositions(clearance, desired, linked)) {
      const node = this.nodes.get(id)!;
      if (node.x === value.x && node.y === value.y && node.z === value.z) continue;
      fixPosition(node, value); this.observed.set(id, point(value)); changed.add(id);
    }
    if (changed.size) this.revision++;
    return changed;
  }
  private startSettle(root: string, followers: Map<string, Follower>, clearance: Clearance | null, tolerance: number, now: number) {
    const anchor = point(this.nodes.get(root)!);
    if (clearance) {
      const desired = new Map(clearance.visible.flatMap(id => {
        const node = this.nodes.get(id), follower = followers.get(id);
        return node ? [[id, follower ? add(anchor, follower.offset) : point(node)] as const] : [];
      }));
      for (const [id, target] of this.clearedPositions(clearance, desired, new Set(followers.keys()))) {
        if (id !== root && (followers.has(id) || target.x !== this.nodes.get(id)!.x || target.y !== this.nodes.get(id)!.y || target.z !== this.nodes.get(id)!.z)) {
          followers.set(id, { offset: add(target, anchor, -1) });
        }
      }
    }
    this.settle = { anchor, followers, velocities: new Map(), clearance, tolerance, last: now, deadline: now + 2000 };
  }
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
    this.observed = new Map(model.nodes.map(n => [n.id, point(n)]));
    this.revision++;
  }
  reset() {
    this.cancel();
    for (const [id, value] of this.baseline) fixPosition(this.nodes.get(id)!, value);
    this.observed = new Map([...this.nodes].map(([id, node]) => [id, point(node)]));
    this.revision++;
  }
  begin(id: string, unitsPerPixel: number, plane: DragPlane = { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24 }) {
    this.cancel();
    const node = this.nodes.get(id);
    if (!node || !Number.isFinite(unitsPerPixel) || unitsPerPixel <= 0) return;
    const start = this.observed.get(id) ?? point(node);
    const visualCluster = (n: PositionedNode) => this.nodes.get(this.satellites.get(n.id) ?? n.id)!.cluster;
    const groups = new Map<string, string[]>();
    for (const member of this.nodes.values()) {
      const cluster = visualCluster(member);
      const ids = groups.get(cluster) ?? [];
      ids.push(member.id); groups.set(cluster, ids);
    }
    // Follow every visual cluster reached through a stored link, including past evidence.
    // Hidden members also move so clearing a display filter cannot reveal a stretched edge.
    const included = new Set([id]), queue = [id];
    for (let index = 0; index < queue.length; index++) {
      const current = queue[index];
      for (const other of [...(groups.get(visualCluster(this.nodes.get(current)!)) ?? []), ...(this.adjacency.get(current) ?? [])]) {
        if (!included.has(other)) { included.add(other); queue.push(other); }
      }
    }
    const members = [...included].filter(other => other !== id).sort();
    const followers = new Map<string, Follower>();
    // Link slack starts the pull; screen footprints can require longer visible edges.
    const slackPixels = Math.max(24, plane.spacingPixels);
    const project = plane.project ?? ((value: Point) => ({
      x: (value.x * plane.right.x + value.y * plane.right.y + value.z * plane.right.z) / unitsPerPixel,
      y: (value.x * plane.up.x + value.y * plane.up.y + value.z * plane.up.z) / unitsPerPixel,
      depth: 1,
    }));
    const visibleIds = new Set(plane.visible ?? []);
    const radii = plane.radius ? [...included].flatMap(member => {
      const node = this.nodes.get(member)!;
      if (!visibleIds.has(member)) return [];
      const at = project(node), radius = plane.radius!(node, at.depth);
      return at.depth > 0 && Number.isFinite(radius) && radius > 0 && (!plane.isVisible || plane.isVisible(at, radius)) ? [radius] : [];
    }) : [];
    const spacingPixels = Math.max(slackPixels, 2 * Math.max(0, ...radii) + (radii.length ? 10 : 0));
    const spacing = spacingPixels * unitsPerPixel;
    const clearance = plane.visible && plane.radius ? {
      held: id, visible: [...new Set(plane.visible)].filter(member => this.nodes.has(member)).sort(),
      right: point(plane.right), up: point(plane.up), project, radius: plane.radius,
      worldPerPixel: plane.worldPerPixel ?? (() => unitsPerPixel), isVisible: plane.isVisible ?? (() => true),
    } : null;
    // Reuse an already valid screen-plane shape (at any scale) before searching.
    const projected = new Map([...included].map(member => {
      const delta = add(member === id ? start : point(this.nodes.get(member)!), start, -1);
      return [member, { x: delta.x * plane.right.x + delta.y * plane.right.y + delta.z * plane.right.z,
        y: delta.x * plane.up.x + delta.y * plane.up.y + delta.z * plane.up.z }] as const;
    }));
    let priorStep = 0, edgeCount = 0, overfull = false;
    for (const member of included) {
      let degree = 0;
      for (const other of this.adjacency.get(member) ?? []) {
        if (!included.has(other)) continue;
        degree++;
        if (member < other) edgeCount++;
        const a = projected.get(member)!, b = projected.get(other)!;
        if (!priorStep) priorStep = Math.hypot(a.x - b.x, a.y - b.y);
      }
      if (degree > 6) overfull = true;
    }
    // Small and sparse graphs keep their canonical layout; large multi-cycle graphs can reuse a valid existing shape.
    const prior = included.size >= 20 && edgeCount > included.size && priorStep > 1e-9 ? new Map(members.map(member => {
      const at = projected.get(member)!;
      return [member, { x: at.x / priorStep, y: at.y / priorStep }] as const;
    })) : null;
    // Large searches run in a worker. Preserve the current edge lengths until an
    // exact layout arrives; a speculative packed layout can stretch linked nodes.
    const defer = typeof Worker !== "undefined" && (included.size >= 128 || (included.size >= 32 && edgeCount > included.size));
    const exact = overfull ? null : validSlots(id, members, this.adjacency, prior) ? prior : defer && edgeCount > 0 ? null : compactSlots(id, members, this.adjacency);
    const layout = exact ?? (overfull ? packedSlots(id, members, this.adjacency, projected) : null);
    for (const member of members) {
      const target = layout?.get(member);
      const offset = target ? add(add(zero(), plane.right, target.x * spacing), plane.up, target.y * spacing) : add(point(this.nodes.get(member)!), start, -1);
      followers.set(member, { offset });
    }
    const depth = new Map([[id, 0]]), traversal = [id], virtual: [string, string][] = [];
    let head = 0;
    const visit = () => {
      while (head < traversal.length) {
        const from = traversal[head++];
        for (const to of [...(this.adjacency.get(from) ?? [])].filter(other => included.has(other)).sort()) {
          if (depth.has(to)) continue;
          depth.set(to, depth.get(from)! + 1); traversal.push(to);
        }
      }
    };
    visit();
    // A visual group can contain nodes without stored links; its loose members follow the held node.
    for (const member of members) {
      if (depth.has(member)) continue;
      virtual.push([id, member]); depth.set(member, 1); traversal.push(member); visit();
    }
    const edges: TensionEdge[] = [];
    for (const from of included) for (const to of this.adjacency.get(from) ?? []) {
      if (from >= to || !included.has(to)) continue;
      const source = depth.get(from)! < depth.get(to)! || (depth.get(from) === depth.get(to) && from < to) ? from : to;
      const target = source === from ? to : from;
      const a = project(source === id ? start : this.nodes.get(source)!), b = project(target === id ? start : this.nodes.get(target)!);
      edges.push({ from: source, to: target, limit: Math.hypot(a.x - b.x, a.y - b.y) + slackPixels });
    }
    for (const [from, to] of virtual) {
      const a = project(from === id ? start : this.nodes.get(from)!), b = project(this.nodes.get(to)!);
      edges.push({ from, to, limit: Math.hypot(a.x - b.x, a.y - b.y) + slackPixels });
    }
    edges.sort((a, b) => depth.get(a.to)! - depth.get(b.to)! || depth.get(a.from)! - depth.get(b.from)! || a.from.localeCompare(b.from) || a.to.localeCompare(b.to));
    this.gesture = { id, start, threshold: unitsPerPixel * 6, tolerance: unitsPerPixel * .05, followers, edges, project, clearance, moved: false, last: null };
    if (!exact && !overfull && members.length) this.searchLater(id, members, included, plane, spacing, unitsPerPixel * .05);
  }
  private searchLater(root: string, members: string[], included: Set<string>, plane: { right: Point; up: Point }, spacing: number, tolerance: number) {
    if (typeof Worker === "undefined") return;
    const token = ++this.token;
    const pending: PendingLayout = { token, root, right: point(plane.right), up: point(plane.up), spacing, clearance: this.gesture?.clearance ?? null, reduced: false };
    const edges: [string, string][] = [];
    for (const id of included) for (const other of this.adjacency.get(id) ?? []) if (id < other && included.has(other)) edges.push([id, other]);
    try {
      const worker = new Worker(new URL("./layout.worker.ts", import.meta.url), { type: "module" });
      this.worker = worker; this.pending = pending;
      worker.onmessage = (event: MessageEvent<{ token: number; slots: [string, Slot][] | null }>) => {
        if (this.pending?.token !== token || event.data.token !== token) return;
        const layout = event.data.slots ? new Map(event.data.slots) : null;
        worker.terminate(); this.worker = null; this.pending = null;
        if (!validSlots(root, members, this.adjacency, layout)) return;
        const followers = new Map(members.map(id => {
          const at = layout.get(id)!;
          return [id, { offset: add(add(zero(), pending.right, at.x * pending.spacing), pending.up, at.y * pending.spacing) }] as const;
        }));
        if (this.gesture?.id === root) {
          this.gesture.followers = followers;
          this.revision++;
        } else {
          const now = performance.now();
          this.startSettle(root, followers, pending.clearance, tolerance, now);
          if (pending.reduced) this.advance(now, true);
          else this.revision++;
        }
      };
      worker.onerror = () => {
        if (this.pending?.token !== token) return;
        worker.terminate(); this.worker = null; this.pending = null;
      };
      worker.postMessage({ token, root, members, edges });
    } catch {
      this.worker?.terminate(); this.worker = null; this.pending = null;
    }
  }
  move(id: string, value: Point) {
    const drag = this.gesture;
    if (!drag || drag.id !== id || ![value.x, value.y, value.z].every(Number.isFinite)) return;
    fixPosition(this.nodes.get(id)!, value);
    this.observed.set(id, point(value));
    const delta = add(value, drag.start, -1);
    if (Math.hypot(delta.x, delta.y, delta.z) >= drag.threshold) drag.moved = true;
    this.revision++;
    this.enforceClearance(drag.clearance, new Set(drag.followers.keys()));
  }
  release(now: number, reduced: boolean) {
    const drag = this.gesture;
    this.gesture = null;
    if (!drag?.moved || !Number.isFinite(now)) { this.cancel(); return; }
    if (this.pending?.root === drag.id) this.pending.reduced = reduced;
    this.startSettle(drag.id, drag.followers, drag.clearance, drag.tolerance, now);
    if (reduced) this.advance(now, true);
  }
  advance(now: number, reduced: boolean) {
    if (!Number.isFinite(now)) return;
    const drag = this.gesture?.moved ? this.gesture : null;
    if (drag) {
      const dt = drag.last === null ? 0 : Math.max(0, (now - drag.last) / 1000);
      drag.last = Math.max(drag.last ?? now, now);
      const fraction = reduced ? 1 : 1 - Math.exp(-18 * dt);
      let changed = false;
      for (const edge of drag.edges) {
        const from = this.nodes.get(edge.from)!, to = this.nodes.get(edge.to)!;
        const a = drag.project(from), b = drag.project(to), distance = Math.hypot(b.x - a.x, b.y - a.y);
        if (distance <= edge.limit || !Number.isFinite(distance) || !fraction) continue;
        const delta = add(to, from, -1);
        const visibleFraction = edge.limit / distance;
        // Perspective interpolation needs camera depth to reach the requested screen distance.
        const denominator = (1 - visibleFraction) * b.depth + visibleFraction * a.depth;
        const travel = a.depth > 0 && b.depth > 0 && denominator > 0 ? visibleFraction * a.depth / denominator : visibleFraction;
        const target = add(from, delta, travel);
        const next = add(to, add(target, to, -1), fraction);
        fixPosition(to, next); this.observed.set(edge.to, point(next)); changed = true;
      }
      if (changed) this.revision++;
      this.enforceClearance(drag.clearance, new Set(drag.followers.keys()), fraction);
      return;
    }
    const settle = this.settle;
    if (!settle) return;
    const dt = Math.max(0, (now - settle.last) / 1000);
    settle.last = Math.max(settle.last, now);
    const snap = reduced || now >= settle.deadline;
    if (!snap && !dt) return;
    // A short damped ripple carries each node into a clearance-sized slot without a release jump.
    const damping = 9.5, frequency = Math.sqrt(14 * 14 - damping * damping);
    const decay = Math.exp(-damping * dt), cosine = Math.cos(frequency * dt), sine = Math.sin(frequency * dt);
    const step = (value: number, target: number, velocity: number): [number, number] => {
      const displacement = value - target;
      return [target + decay * (displacement * cosine + (velocity + damping * displacement) / frequency * sine),
        decay * (velocity * cosine - (14 * 14 * displacement + damping * velocity) / frequency * sine)];
    };
    let changed = false, complete = true;
    for (const [id, follower] of settle.followers) {
      const node = this.nodes.get(id)!, target = add(settle.anchor, follower.offset), velocity = settle.velocities.get(id) ?? zero();
      const previous = point(node);
      const [x, vx] = snap ? [target.x, 0] : step(node.x, target.x, velocity.x);
      const [y, vy] = snap ? [target.y, 0] : step(node.y, target.y, velocity.y);
      const [z, vz] = snap ? [target.z, 0] : step(node.z, target.z, velocity.z);
      const next = { x, y, z }, nextVelocity = { x: vx, y: vy, z: vz };
      if (Math.hypot(target.x - x, target.y - y, target.z - z) <= settle.tolerance && Math.hypot(vx, vy, vz) <= settle.tolerance * 14) {
        fixPosition(node, target); settle.velocities.delete(id);
      } else { fixPosition(node, next); settle.velocities.set(id, nextVelocity); complete = false; }
      this.observed.set(id, point(node));
      if (previous.x !== node.x || previous.y !== node.y || previous.z !== node.z) changed = true;
    }
    if (changed) this.revision++;
    const displaced = this.enforceClearance(settle.clearance, new Set(settle.followers.keys()));
    for (const id of displaced) settle.velocities.delete(id);
    if (displaced.size && !snap) complete = false;
    if (complete) this.settle = null;
  }
  cancel() {
    // Freeze current coordinates on cancellation; do not teleport to a target.
    this.worker?.terminate(); this.worker = null; this.pending = null; this.token++;
    this.gesture = null; this.settle = null;
  }
}
