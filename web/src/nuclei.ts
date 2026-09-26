import { knowledge } from "./graph";
import type { GraphLink, PositionedNode } from "./graph";

export type Nucleus = { representative: string; members: readonly string[]; center: { x: number; y: number; z: number }; radius: number };
export type NucleusView = { nodes: PositionedNode[]; counts: Map<string, number>; groups: Map<string, Nucleus> };

/** Keep label slots for nuclei actually inside the current camera view. */
export function nucleusLabelIds<T extends { id: string; x: number; y: number; depth: number }>(candidates: readonly T[], counts: ReadonlyMap<string, number>, width: number, height: number, limit = 8): Set<string> {
  return new Set(candidates.filter(node => counts.has(node.id) &&
    [node.x, node.y, node.depth].every(Number.isFinite) && node.depth >= -1 && node.depth <= 1 &&
    node.x >= 0 && node.x <= width && node.y >= 0 && node.y <= height)
    .sort((a, b) => (counts.get(b.id) ?? 0) - (counts.get(a.id) ?? 0) ||
      Math.hypot(a.x - width / 2, a.y - height / 2) - Math.hypot(b.x - width / 2, b.y - height / 2) || a.id.localeCompare(b.id))
    .slice(0, limit).map(node => node.id));
}

// A camera band changes only after the view has crossed the opposite threshold.
// The 31-unit first-view spacing comes from the canonical layout, not a second layout.
export function nucleusLevel(spacingPixels: number, previous: number): number {
  if (!Number.isFinite(spacingPixels) || spacingPixels <= 0) return previous;
  let level = Math.max(0, Math.min(3, Math.trunc(previous)));
  const enter = [20, 10, 5], leave = [26, 13, 6.5];
  while (level < 3 && spacingPixels < enter[level]) level++;
  while (level > 0 && spacingPixels > leave[level - 1]) level--;
  return level;
}

function hexKey(node: PositionedNode, radius: number): string {
  const q = (Math.sqrt(3) * node.x / 3 - node.y / 3) / radius;
  const r = 2 * node.y / (3 * radius);
  let x = Math.round(q), z = Math.round(r), y = Math.round(-q - r);
  const dx = Math.abs(x - q), dy = Math.abs(y + q + r), dz = Math.abs(z - r);
  if (dx > dy && dx > dz) x = -y - z;
  else if (dy > dz) y = -x - z;
  else z = -x - y;
  return `${x},${z}`;
}

function bounded(groups: PositionedNode[], limit: number): PositionedNode[][] {
  if (groups.length <= limit) return [groups];
  const xs = groups.map(node => node.x), ys = groups.map(node => node.y);
  const axis = Math.max(...xs) - Math.min(...xs) >= Math.max(...ys) - Math.min(...ys) ? "x" : "y";
  const ordered = [...groups].sort((a, b) => a[axis] - b[axis] || a.id.localeCompare(b.id));
  const half = Math.floor(ordered.length / 2);
  return [...bounded(ordered.slice(0, half), limit), ...bounded(ordered.slice(half), limit)];
}

/** Only visually summarize zero-degree knowledge stars; no relationship is rewritten. */
export function nucleusView(nodes: readonly PositionedNode[], allLinks: readonly GraphLink[], level: number, selected: string | null, expanded: ReadonlySet<string> = new Set(), locked: ReadonlyMap<string, readonly string[]> = new Map()): NucleusView {
  const counts = new Map<string, number>(), groups = new Map<string, Nucleus>();
  if (!level) return { nodes: [...nodes], counts, groups };
  const linked = new Set(allLinks.flatMap(link => [link.source, link.target]));
  const radius = 120 * 1.5 ** (level - 1), limit = 60 * 2 ** (level - 1);
  const byId = new Map(nodes.map(node => [node.id, node]));
  const hidden = new Set<string>(), reserved = new Set<string>();
  const summarize = (members: PositionedNode[], representative?: string) => {
    const stable = [...members].sort((a, b) => a.id.localeCompare(b.id));
    const center = stable.reduce((sum, node) => ({ x: sum.x + node.x / stable.length, y: sum.y + node.y / stable.length, z: sum.z + node.z / stable.length }), { x: 0, y: 0, z: 0 });
    const id = representative ?? [...stable].sort((a, b) =>
      Math.hypot(a.x - center.x, a.y - center.y, a.z - center.z) - Math.hypot(b.x - center.x, b.y - center.y, b.z - center.z) || a.id.localeCompare(b.id))[0].id;
    const spread = Math.max(...stable.map(node => Math.hypot(node.x - center.x, node.y - center.y, node.z - center.z))) + 6;
    const nucleus = { representative: id, members: stable.map(node => node.id), center, radius: spread };
    counts.set(id, stable.length); groups.set(id, nucleus);
    for (const node of stable) { reserved.add(node.id); if (node.id !== id) hidden.add(node.id); }
  };
  // A moved group stays intact across hex-cell boundaries until its camera band changes.
  for (const [representative, ids] of locked) {
    if (ids.length < 6 || !ids.includes(representative) || ids.some(id => !byId.has(id) || !knowledge(byId.get(id)!) || linked.has(id) || expanded.has(id) || id === selected || reserved.has(id))) continue;
    summarize(ids.map(id => byId.get(id)!), representative);
  }
  const cells = new Map<string, PositionedNode[]>();
  for (const node of nodes) {
    if (!knowledge(node) || linked.has(node.id) || expanded.has(node.id) || reserved.has(node.id)) continue;
    const key = hexKey(node, radius), bucket = cells.get(key) ?? [];
    bucket.push(node); cells.set(key, bucket);
  }
  for (const key of [...cells.keys()].sort()) for (const members of bounded(cells.get(key)!, limit)) {
    // Keep the focused star and its immediate cell visible during a camera move.
    // Unrelated cells still follow the current camera band.
    if (members.length < 6 || members.some(node => node.id === selected)) continue;
    summarize(members);
  }
  return { nodes: nodes.filter(node => !hidden.has(node.id)), counts, groups };
}
