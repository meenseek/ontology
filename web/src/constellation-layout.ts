import type { GraphLink } from "./graph";

type Slot = { x: number; y: number };
/** These edges describe membership, not a relationship between two knowledge items. */
export const classificationLink = (kind: string) => kind === "subject" || kind === "topic" || kind === "area" || kind === "parent";

/** Only actual edges shape the interior. Membership still groups the surrounding region. */
export function topologySlots(ids: readonly string[], links: readonly GraphLink[], radii: ReadonlyMap<string, number>, spacing: number): Map<string, Slot> {
  if (!ids.length) return new Map();
  const ordered = [...ids].sort(), included = new Set(ordered);
  const edges = links.filter(link => !classificationLink(link.kind) && included.has(link.source) && included.has(link.target))
    .map(link => [link.source, link.target].sort() as [string, string])
    .sort((a, b) => a[0].localeCompare(b[0]) || a[1].localeCompare(b[1]));
  const adjacency = new Map(ordered.map(id => [id, new Set<string>()]));
  for (const [a, b] of edges) { adjacency.get(a)!.add(b); adjacency.get(b)!.add(a); }
  const visited = new Set<string>(), components: { ids: string[]; points: Map<string, Slot>; radius: number }[] = [];
  const radius = (id: string) => (radii.get(id) ?? spacing / 2) / spacing;
  for (const seed of ordered) {
    if (visited.has(seed)) continue;
    const members = [seed]; visited.add(seed);
    for (let i = 0; i < members.length; i++) for (const other of [...adjacency.get(members[i])!].sort()) {
      if (!visited.has(other)) { visited.add(other); members.push(other); }
    }
    const memberSet = new Set(members), localEdges = edges.filter(([a]) => memberSet.has(a));
    // A traversal seed breaks circular symmetry without inventing connecting edges.
    const degree = (id: string) => adjacency.get(id)!.size;
    const path = members.every(id => degree(id) <= 2) && members.some(id => degree(id) === 1);
    const cycle = members.length > 2 && members.every(id => degree(id) === 2);
    const traversal: string[] = [];
    if (path || cycle) {
      let id = (path ? members.filter(id => degree(id) === 1) : members).sort()[0];
      const seen = new Set<string>();
      while (id && !seen.has(id)) { traversal.push(id); seen.add(id); id = [...adjacency.get(id)!].sort().find(other => !seen.has(other))!; }
    }
    const sequence = traversal.length === members.length ? traversal : members;
    const points = new Map(sequence.map((id, i) => {
      const angle = cycle ? i * 2 * Math.PI / members.length : path ? i * .5 : i * 2.399963;
      const reach = cycle ? Math.max(1, members.length * 1.35 / (2 * Math.PI)) * (1 + .1 * Math.sin(i * 1.7)) : path ? 2.5 + i * .13 : Math.sqrt(i + 1);
      return [id, { x: Math.cos(angle) * reach, y: Math.sin(angle) * reach }];
    }));
    const steps = members.length <= 48 ? 96 : 40;
    const largest = Math.max(...members.map(radius)), cellSize = Math.max(1, 2 * largest + .5);
    for (let step = 0; step < steps && members.length > 1; step++) {
      const delta = new Map(members.map(id => [id, { x: 0, y: 0 }]));
      for (const [a, b] of localEdges) {
        if (a === b) continue;
        const p = points.get(a)!, q = points.get(b)!, dx = q.x - p.x, dy = q.y - p.y, d = Math.max(.001, Math.hypot(dx, dy));
        const desired = Math.max(1.25, radius(a) + radius(b) + .55), pull = (d - desired) * .075 / Math.sqrt(Math.max(adjacency.get(a)!.size, adjacency.get(b)!.size));
        const da = delta.get(a)!, db = delta.get(b)!;
        da.x += dx / d * pull; da.y += dy / d * pull; db.x -= dx / d * pull; db.y -= dy / d * pull;
      }
      const cells = new Map<string, string[]>();
      for (const id of members) {
        const p = points.get(id)!, column = Math.floor(p.x / cellSize), row = Math.floor(p.y / cellSize);
        for (let x = column - 1; x <= column + 1; x++) for (let y = row - 1; y <= row + 1; y++) for (const other of cells.get(`${x},${y}`) ?? []) {
          const q = points.get(other)!, dx = p.x - q.x, dy = p.y - q.y, d = Math.max(.001, Math.hypot(dx, dy)), minimum = radius(id) + radius(other) + .4;
          if (d >= minimum) continue;
          const push = (minimum - d) * .24, a = delta.get(id)!, b = delta.get(other)!;
          a.x += dx / d * push; a.y += dy / d * push; b.x -= dx / d * push; b.y -= dy / d * push;
        }
        const key = `${column},${row}`; cells.set(key, [...(cells.get(key) ?? []), id]);
      }
      for (const id of members) {
        const p = points.get(id)!, d = delta.get(id)!;
        p.x += Math.max(-.4, Math.min(.4, d.x - p.x * .002)); p.y += Math.max(-.4, Math.min(.4, d.y - p.y * .002));
      }
    }
    const mean = [...points.values()].reduce((a, p) => ({ x: a.x + p.x / members.length, y: a.y + p.y / members.length }), { x: 0, y: 0 });
    let extent = 0;
    for (const [id, p] of points) { p.x -= mean.x; p.y -= mean.y; extent = Math.max(extent, Math.hypot(p.x, p.y) + radius(id)); }
    components.push({ ids: members, points, radius: extent });
  }
  components.sort((a, b) => b.ids.length - a.ids.length || a.ids[0].localeCompare(b.ids[0]));
  const result = new Map<string, Slot>(), cells = new Map<string, (Slot & { radius: number })[]>(), cursors = new Map<number, number>();
  const largest = Math.max(...components.map(component => component.radius)), cellSize = Math.max(1, largest * 2 + .4);
  for (const component of components) {
    let center = { x: 0, y: 0 }, cursor = result.size ? cursors.get(component.radius) ?? 1 : 0;
    for (;; cursor++) {
      const distance = 1.1 * Math.sqrt(cursor), angle = cursor * 2.399963;
      center = { x: Math.cos(angle) * distance, y: Math.sin(angle) * distance };
      const reach = component.radius + largest + .4;
      let free = true;
      for (let x = Math.floor((center.x - reach) / cellSize); x <= Math.floor((center.x + reach) / cellSize) && free; x++)
        for (let y = Math.floor((center.y - reach) / cellSize); y <= Math.floor((center.y + reach) / cellSize) && free; y++)
          if ((cells.get(`${x},${y}`) ?? []).some(other => Math.hypot(center.x - other.x, center.y - other.y) < component.radius + other.radius + .4)) free = false;
      if (free) break;
    }
    cursors.set(component.radius, cursor + 1);
    const key = `${Math.floor(center.x / cellSize)},${Math.floor(center.y / cellSize)}`;
    cells.set(key, [...(cells.get(key) ?? []), { ...center, radius: component.radius }]);
    for (const [id, p] of component.points) result.set(id, { x: p.x + center.x, y: p.y + center.y });
  }
  const origin = result.get(ordered[0])!;
  return new Map([...result].map(([id, p]) => [id, { x: p.x - origin.x, y: p.y - origin.y }]));
}
