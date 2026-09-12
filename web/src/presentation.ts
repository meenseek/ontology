/** Keep labels literal: path segments identify a file, never its owner. */
export function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}
/** The shortest unique local suffix is derived from this snapshot, not ownership or configuration. */
export function repositoryNames(repositories: string[]): Map<string, string> {
  const entries = [...new Set(repositories)].map(repository => ({ repository, parts: repository.split(/[\\/]/).filter(Boolean) }));
  return new Map(entries.map(({ repository, parts }) => {
    for (let depth = 1; depth <= parts.length; depth++) {
      const suffix = parts.slice(-depth).join("/");
      if (entries.every(other => other.repository === repository || other.parts.slice(-depth).join("/") !== suffix)) return [repository, suffix];
    }
    return [repository, repository];
  }));
}
export function nodePresentation(node: { kind: string; label: string; repository?: string; repositoryLabel?: string }, compact = false): { title: string; subtitle: string } {
  if (node.kind !== "document") return { title: node.label, subtitle: "" };
  const title = fileName(node.label);
  const repository = compact ? node.repositoryLabel ?? node.repository : node.repository;
  return { title, subtitle: repository ? `${repository} · ${node.label}` : title !== node.label ? node.label : "" };
}
export const MAX_VISIBLE_LABELS = 24;
export type ProjectedLabel = { id: string; kind: string; active: boolean; x: number; y: number; depth: number; radius: number; width: number; height: number };
type LabelBox = { id: string; left: number; top: number; right: number; bottom: number };
/** Rank the current projection before bounding the visible set. No ID window excludes later nodes. */
export function visibleLabels(candidates: ProjectedLabel[], width: number, height: number, selected: string | null, hovered: string | null): LabelBox[] {
  const visible = candidates.filter(n => [n.x, n.y, n.depth, n.radius, n.width, n.height].every(Number.isFinite) && n.radius > 0 && n.depth >= -1 && n.depth <= 1 && n.x >= 0 && n.x <= width && n.y >= 0 && n.y <= height);
  const distance = (n: ProjectedLabel) => Math.hypot(n.x - width / 2, n.y - height / 2);
  const nearby = (a: ProjectedLabel, b: ProjectedLabel) => distance(a) - distance(b) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const priority = visible.filter(n => n.id === selected);
  const ordinary = visible.filter(n => n.id !== selected);
  const ordered = [...priority];
  const readable = (a: ProjectedLabel, b: ProjectedLabel) => Number(b.active) - Number(a.active) || nearby(a, b);
  const documents = ordinary.filter(n => n.kind === "document").sort(readable);
  const memories = ordinary.filter(n => n.kind === "memory").sort(readable);
  // Each kind gets a turn before the other kind can consume the label budget.
  for (let i = 0; i < Math.max(documents.length, memories.length); i++) {
    if (documents[i]) ordered.push(documents[i]);
    if (memories[i]) ordered.push(memories[i]);
  }
  ordered.push(...ordinary.filter(n => n.kind !== "document" && n.kind !== "memory").sort(nearby));
  const positionsFor = (node: ProjectedLabel): LabelBox[] => {
    const clearance = node.radius + 5;
    const sides = node.x > width * .66 ? [-1, 1] : [1, -1];
    const positions = sides.map(side => ({ left: node.x + (side === -1 ? -clearance - node.width : clearance), top: node.y - node.height / 2 }));
    const centered = Math.max(0, Math.min(width - node.width, node.x - node.width / 2));
    positions.push({ left: centered, top: node.y - clearance - node.height }, { left: centered, top: node.y + clearance });
    return positions.map(({ left, top }) => ({ id: node.id, left, top, right: left + node.width, bottom: top + node.height }));
  };
  const inBounds = (box: LabelBox) => box.left >= 0 && box.top >= 0 && box.right <= width && box.bottom <= height;
  const overlaps = (a: LabelBox, b: LabelBox) => a.left < b.right + 8 && a.right + 8 > b.left && a.top < b.bottom + 6 && a.bottom + 6 > b.top;
  // Hover never participates in the base ranking or placement.
  const boxes: LabelBox[] = [];
  for (const node of ordered) {
    const box = positionsFor(node).find(position => inBounds(position) && !boxes.some(other => overlaps(position, other)));
    if (box) boxes.push(box);
    if (boxes.length === MAX_VISIBLE_LABELS) break;
  }
  if (!hovered || boxes.some(box => box.id === hovered)) return boxes;
  const node = visible.find(candidate => candidate.id === hovered);
  if (!node) return boxes;
  let insertion: LabelBox | undefined;
  let fewestCollisions = Infinity;
  for (const position of positionsFor(node)) {
    if (!inBounds(position) || boxes.some(box => box.id === selected && overlaps(position, box))) continue;
    const collisions = boxes.filter(box => overlaps(position, box)).length;
    if (collisions < fewestCollisions) {
      insertion = position;
      fewestCollisions = collisions;
    }
  }
  if (!insertion) return boxes;
  const survivors = boxes.filter(box => !overlaps(insertion, box));
  if (survivors.length === MAX_VISIBLE_LABELS) {
    const lastOrdinary = survivors.map(box => box.id !== selected).lastIndexOf(true);
    survivors.splice(lastOrdinary, 1);
  }
  return [...survivors, insertion];
}
export const memoryKindName: Record<string, string> = { record: "일반 기록", fact: "사실", decision: "결정", preference: "선호", idea: "아이디어" };
/** SpriteMaterial.sizeAttenuation=false projects these local units to CSS pixels. */
export function spriteScale(pixels: number, viewportHeight: number, projectionY: number): number {
  if (![pixels, viewportHeight, projectionY].every(Number.isFinite) || pixels <= 0 || viewportHeight <= 0 || projectionY <= 0) return 0;
  return 2 * pixels / (viewportHeight * projectionY);
}

/** Perspective diameter in CSS pixels, shared by rendering, picking and projected labels. */
export function nodeScreenSize(kind: string, depth: number, viewportHeight: number, projectionY: number): number {
  if (![depth, viewportHeight, projectionY].every(Number.isFinite) || depth <= 0 || viewportHeight <= 0 || projectionY <= 0) return 0;
  if (kind !== "document" && kind !== "memory") return 10;
  return Math.min(140, Math.max(26, 14 * (viewportHeight / depth) * projectionY / 2));
}
/** Clear the corona and padded ring textures with one footprint for labels and picking. */
export function nodeScreenMetrics(size: number, selected: boolean, changed: boolean) {
  if (!Number.isFinite(size) || size <= 0) return { body: 0, selection: 0, change: 0, hit: 0, radius: 0 };
  // The ring's 112px circle occupies only part of its 128px texture.
  const selection = Math.max(27, size + 12) * (128 / 112), change = Math.max(36, size + 24) * (128 / 112);
  const extent = changed ? change : selected ? selection : size;
  return { body: size, selection, change, hit: Math.max(36, extent), radius: extent / 2 };
}
/** Cosmetic motion is independent of graph status, identity, layout and hit bounds. */
export function starPhase(id: string): number {
  let hash = 2166136261;
  for (let i = 0; i < id.length; i++) hash = Math.imul(hash ^ id.charCodeAt(i), 16777619);
  hash = Math.imul(hash ^ (hash >>> 16), 0x7feb352d);
  hash = Math.imul(hash ^ (hash >>> 15), 0x846ca68b);
  return ((hash ^ (hash >>> 16)) >>> 0) / 4294967296 * Math.PI * 2;
}
export type StarClock = { seconds: number; lastTime: number | null };
/** Use the renderer's clock; discard suspension gaps and freeze at the current surface. */
export function advanceStarClock(clock: StarClock, now: number, reduced: boolean): number {
  if (!Number.isFinite(now)) return clock.seconds;
  const delta = clock.lastTime === null ? 0 : (now - clock.lastTime) / 1000;
  clock.lastTime = now;
  if (!reduced && delta > 0 && delta <= .25) clock.seconds += delta;
  return clock.seconds;
}
/** A smooth size blend moves from a distant core/glint pulse to a resolved stellar surface. */
export function starMotion(pixels: number, phase: number, seconds: number) {
  const t = Number.isFinite(pixels) ? Math.max(0, Math.min(1, (pixels - 36) / 64)) : 0;
  const detail = t * t * (3 - 2 * t);
  const time = Number.isFinite(seconds) ? seconds : 0;
  const initial = Number.isFinite(phase) ? phase : 0;
  const turn = Math.PI * 2;
  const seed = ((initial / turn) % 1 + 1) % 1;
  const period = 32 + 24 * seed;
  // A brief, gently eased crest every 7 seconds, offset independently for each star.
  const pulse = (.5 + .5 * Math.sin((time % 7) * turn / 7 + initial % turn)) ** 4;
  return {
    detail,
    period,
    tilt: Math.sin(seed * turn * 3 + .7) * 32 * Math.PI / 180,
    rotation: ((seed * turn + (time % period) * turn / period) % turn + turn) % turn,
    shimmer: 1 - .3 * (1 - detail) * (1 - pulse),
  };
}
