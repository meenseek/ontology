export type Disc = { id: string; x: number; y: number; radius: number };
export type ScreenPosition = { x: number; y: number };
export const COLLISION_GAP = 1;
const epsilon = 1e-6;

function discIndex(largest: number) {
  const cell = Math.max(48, Math.min(128, largest * 2)), grid = new Map<string, Disc[]>();
  const key = (x: number, y: number) => `${x},${y}`;
  return {
    add(disc: Disc) {
      const at = key(Math.floor(disc.x / cell), Math.floor(disc.y / cell)), bucket = grid.get(at) ?? [];
      bucket.push(disc); grid.set(at, bucket);
    },
    nearby(x: number, y: number, radius: number): Disc[] {
      const reach = radius + largest, candidates: Disc[] = [];
      for (let gx = Math.floor((x - reach) / cell); gx <= Math.floor((x + reach) / cell); gx++) {
        for (let gy = Math.floor((y - reach) / cell); gy <= Math.floor((y + reach) / cell); gy++) {
          candidates.push(...(grid.get(key(gx, gy)) ?? []));
        }
      }
      return candidates;
    },
  };
}

function freePoint(origin: ScreenPosition & { id: string }, blockers: Disc[], nearby: (circle: Disc) => Disc[]): ScreenPosition {
  if (!blockers.length) return { x: origin.x, y: origin.y };
  let best: ScreenPosition | null = null, bestDistance = Infinity;
  const frontier = [...blockers], seen = new Set(blockers.map(other => other.id)), checked = new Set<string>();
  // The nearest free point is on an exposed circle arc or at an arc's
  // intersection. Search the containing union, pruning outside the best ball.
  for (let index = 0; index < frontier.length; index++) {
    const circle = frontier[index], radius = circle.radius + epsilon;
    checked.add(circle.id);
    if (Math.hypot(origin.x - circle.x, origin.y - circle.y) - radius > bestDistance) continue;
    const neighbors = nearby(circle).filter(other => other.id !== circle.id &&
      Math.hypot(other.x - circle.x, other.y - circle.y) <= radius + other.radius + epsilon);
    const offer = (candidate: ScreenPosition) => {
      const distance = Math.hypot(candidate.x - origin.x, candidate.y - origin.y);
      if (distance > bestDistance + 1e-9) return;
      // Any circle covering a point on this boundary must overlap it.
      if (neighbors.some(other => Math.hypot(candidate.x - other.x, candidate.y - other.y) < other.radius)) return;
      if (distance < bestDistance - 1e-9 || !best ||
        (Math.abs(distance - bestDistance) <= 1e-9 && (candidate.x < best.x || candidate.x === best.x && candidate.y < best.y))) {
        best = candidate; bestDistance = distance;
      }
    };
    let dx = origin.x - circle.x, dy = origin.y - circle.y, length = Math.hypot(dx, dy);
    if (length < 1e-9) {
      let hash = 2166136261;
      for (const letter of `${origin.id}|${circle.id}`) hash = Math.imul(hash ^ letter.charCodeAt(0), 16777619);
      const angle = (hash >>> 0) / 4294967296 * Math.PI * 2;
      dx = Math.cos(angle); dy = Math.sin(angle); length = 1;
    }
    offer({ x: circle.x + dx / length * radius, y: circle.y + dy / length * radius });
    for (const other of neighbors) {
      if (!seen.has(other.id) && Math.hypot(origin.x - other.x, origin.y - other.y) - other.radius - epsilon <= bestDistance) {
        seen.add(other.id); frontier.push(other);
      }
      if (checked.has(other.id)) continue;
      const ox = other.x - circle.x, oy = other.y - circle.y, distance = Math.hypot(ox, oy), otherRadius = other.radius + epsilon;
      if (distance < 1e-9 || distance < Math.abs(radius - otherRadius) || distance > radius + otherRadius) continue;
      const along = (radius * radius - otherRadius * otherRadius + distance * distance) / (2 * distance);
      const height = Math.sqrt(Math.max(0, radius * radius - along * along));
      const cx = circle.x + ox / distance * along, cy = circle.y + oy / distance * along;
      offer({ x: cx - oy / distance * height, y: cy + ox / distance * height });
      offer({ x: cx + oy / distance * height, y: cy - ox / distance * height });
    }
  }
  // A finite union of valid discs always has an exposed outer boundary.
  return best!;
}

/** Nearest point outside collision-expanded circles, also used for a moving camera target. */
export function nearestFreePoint(origin: ScreenPosition & { id: string }, circles: readonly Disc[]): ScreenPosition {
  const valid = circles.filter(disc => [disc.x, disc.y, disc.radius].every(Number.isFinite) && disc.radius > 0)
    .sort((a, b) => a.id.localeCompare(b.id));
  const index = discIndex(Math.max(0, ...valid.map(disc => disc.radius)));
  for (const circle of valid) index.add(circle);
  const blockers = index.nearby(origin.x, origin.y, 0).filter(circle => Math.hypot(origin.x - circle.x, origin.y - circle.y) < circle.radius);
  return freePoint(origin, blockers, circle => index.nearby(circle.x, circle.y, circle.radius + 2 * epsilon));
}

/** Place the held node first, then linked nodes, without moving a placed node again. */
export function separateDiscs(discs: Disc[], held: string, linked: ReadonlySet<string>, gap = COLLISION_GAP, fixed: ReadonlySet<string> = new Set([held])): Map<string, ScreenPosition> {
  const anchor = discs.find(disc => disc.id === held);
  const ordered = discs.filter(disc => [disc.x, disc.y, disc.radius].every(Number.isFinite) && disc.radius > 0).sort((a, b) => {
    if (a.id === held) return -1;
    if (b.id === held) return 1;
    if (fixed.has(a.id) !== fixed.has(b.id)) return fixed.has(a.id) ? -1 : 1;
    if (linked.has(a.id) !== linked.has(b.id)) return linked.has(a.id) ? -1 : 1;
    const da = anchor ? Math.hypot(a.x - anchor.x, a.y - anchor.y) : 0;
    const db = anchor ? Math.hypot(b.x - anchor.x, b.y - anchor.y) : 0;
    return da - db || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  });
  const index = discIndex(Math.max(0, ...ordered.map(disc => disc.radius))), result = new Map<string, ScreenPosition>();
  for (const disc of ordered) {
    let at: ScreenPosition = { x: disc.x, y: disc.y };
    if (!fixed.has(disc.id)) {
      const expand = (other: Disc): Disc => ({ ...other, radius: other.radius + disc.radius + gap });
      const blockers = index.nearby(disc.x, disc.y, disc.radius + gap).map(expand)
        .filter(other => Math.hypot(disc.x - other.x, disc.y - other.y) < other.radius);
      at = freePoint(disc, blockers, circle => index.nearby(circle.x, circle.y, circle.radius + disc.radius + gap + 2 * epsilon).map(expand));
    }
    index.add({ ...disc, ...at }); result.set(disc.id, at);
  }
  return result;
}
