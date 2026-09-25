export type Disc = { id: string; x: number; y: number; radius: number };
export type ScreenPosition = { x: number; y: number };
export const COLLISION_GAP = 1;

/** Place the held node first, then linked nodes, without moving a placed node again. */
export function separateDiscs(discs: Disc[], held: string, linked: ReadonlySet<string>, gap = COLLISION_GAP): Map<string, ScreenPosition> {
  const anchor = discs.find(disc => disc.id === held);
  const ordered = [...discs].sort((a, b) => {
    if (a.id === held) return -1;
    if (b.id === held) return 1;
    if (linked.has(a.id) !== linked.has(b.id)) return linked.has(a.id) ? -1 : 1;
    const da = anchor ? Math.hypot(a.x - anchor.x, a.y - anchor.y) : 0;
    const db = anchor ? Math.hypot(b.x - anchor.x, b.y - anchor.y) : 0;
    return da - db || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  });
  const placed: Disc[] = [];
  const largest = Math.max(0, ...ordered.filter(disc => Number.isFinite(disc.radius) && disc.radius > 0).map(disc => disc.radius));
  const cell = Math.max(48, Math.min(128, largest * 2));
  const grid = new Map<string, Disc[]>();
  const key = (x: number, y: number) => `${x},${y}`;
  const nearby = (x: number, y: number, radius: number): Disc[] => {
    const reach = radius + largest + gap, candidates: Disc[] = [];
    for (let gx = Math.floor((x - reach) / cell); gx <= Math.floor((x + reach) / cell); gx++) {
      for (let gy = Math.floor((y - reach) / cell); gy <= Math.floor((y + reach) / cell); gy++) {
        candidates.push(...(grid.get(key(gx, gy)) ?? []));
      }
    }
    return candidates;
  };
  const result = new Map<string, ScreenPosition>();
  for (const disc of ordered) {
    if (![disc.x, disc.y, disc.radius].every(Number.isFinite) || disc.radius <= 0) continue;
    let x = disc.x, y = disc.y;
    if (disc.id !== held) {
      let clear = false;
      for (let attempt = 0; attempt < 32; attempt++) {
        let blocker: Disc | null = null, penetration = 0;
        for (const other of nearby(x, y, disc.radius)) {
          const distance = disc.radius + other.radius + gap;
          const dx = x - other.x, dy = y - other.y;
          if (Math.abs(dx) >= distance || Math.abs(dy) >= distance) continue;
          const overlap = distance - Math.hypot(dx, dy);
          if (overlap > penetration) { blocker = other; penetration = overlap; }
        }
        if (!blocker) { clear = true; break; }
        let dx = x - blocker.x, dy = y - blocker.y, length = Math.hypot(dx, dy);
        if (length < 1e-9) {
          let hash = 2166136261;
          for (const letter of `${disc.id}|${blocker.id}`) hash = Math.imul(hash ^ letter.charCodeAt(0), 16777619);
          const angle = (hash >>> 0) / 4294967296 * Math.PI * 2;
          dx = Math.cos(angle); dy = Math.sin(angle); length = 1;
        }
        const distance = disc.radius + blocker.radius + gap + 1e-6;
        x = blocker.x + dx / length * distance;
        y = blocker.y + dy / length * distance;
      }
      if (!clear) {
        // A finite set of discs always has free space beyond this bound.
        const reach = Math.max(0, ...placed.map(other => Math.hypot(other.x - disc.x, other.y - disc.y) + other.radius + disc.radius + gap)) + 1;
        x = disc.x + reach; y = disc.y;
      }
    }
    const positioned = { ...disc, x, y };
    placed.push(positioned);
    const cellKey = key(Math.floor(x / cell), Math.floor(y / cell));
    const bucket = grid.get(cellKey) ?? [];
    bucket.push(positioned); grid.set(cellKey, bucket);
    result.set(disc.id, { x, y });
  }
  return result;
}
