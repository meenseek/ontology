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
export function nodePresentation(node: { kind: string; label: string; title?: string | null; repository?: string; repositoryLabel?: string }, compact = false): { title: string; subtitle: string } {
  if (node.kind !== "document") return { title: node.label, subtitle: "" };
  const title = node.title?.trim() || fileName(node.label);
  const repository = compact ? node.repositoryLabel ?? node.repository : node.repository;
  return { title, subtitle: repository ? `${repository} · ${node.label}` : title !== node.label ? node.label : "" };
}
/** Omit a duplicate leading Markdown heading; keep the remaining text and line breaks. */
export function searchExcerpt(excerpt: string, title: string): string {
  const heading = excerpt.match(/^((?:…|\.\.\.)?)[ ]{0,3}#{1,6}[ \t]+([^\r\n]+)\r?\n([\s\S]+)$/);
  if (!heading || heading[2].trim().replace(/[ \t]+#+$/, "") !== title.trim() || !heading[3].trim()) return excerpt;
  return heading[1] + heading[3].replace(/^(?:[ \t]*\r?\n)+/, "");
}
export const MAX_VISIBLE_LABELS = 24;
/** Group names lead the overview; individual titles appear through interaction, proximity or disclosure. */
export const MAX_RESTING_STAR_LABELS = 0;
// A star this close reads as an individual body, so its title is not rationed.
const CLOSE_STAR_LABEL_RADIUS = 9;
export type ProjectedLabel = { id: string; kind: string; active: boolean; summary?: boolean; importance?: number; x: number; y: number; depth: number; radius: number; width: number; height: number;
  /** Drawn body radius without selection or change rings; defaults to radius. */
  body?: number };
export type LabelBox = { id: string; left: number; top: number; right: number; bottom: number; guide?: LabelLeader[] };
export type LabelLeader = { x1: number; y1: number; x2: number; y2: number; length: number };
/** The same visible segment is used by placement and drawing, outside the body's halo. */
export function labelLeader(star: Pick<ProjectedLabel, "x" | "y" | "radius">, box: LabelBox): LabelLeader | null {
  const x = Math.max(box.left, Math.min(box.right, star.x)), y = Math.max(box.top, Math.min(box.bottom, star.y));
  const dx = x - star.x, dy = y - star.y, distance = Math.hypot(dx, dy), length = distance - star.radius - 6;
  if (length <= 14) return null;
  const x1 = star.x + dx / distance * (star.radius + 3), y1 = star.y + dy / distance * (star.radius + 3);
  return { x1, y1, x2: x1 + dx / distance * length, y2: y1 + dy / distance * length, length };
}
export function leaderHitsBox(line: LabelLeader, box: Omit<LabelBox, "id">, gap = 2): boolean {
  let start = 0, end = 1;
  for (const [at, delta, min, max] of [[line.x1, line.x2 - line.x1, box.left - gap, box.right + gap], [line.y1, line.y2 - line.y1, box.top - gap, box.bottom + gap]]) {
    if (Math.abs(delta) < 1e-8) { if (at < min || at > max) return false; }
    else { const a = (min - at) / delta, b = (max - at) / delta; start = Math.max(start, Math.min(a, b)); end = Math.min(end, Math.max(a, b)); }
    if (start > end) return false;
  }
  return true;
}
export function leadersCross(a: LabelLeader, b: LabelLeader): boolean {
  const cross = (x: number, y: number, u: number, v: number) => x * v - y * u;
  const ax = a.x2 - a.x1, ay = a.y2 - a.y1, bx = b.x2 - b.x1, by = b.y2 - b.y1;
  const denominator = cross(ax, ay, bx, by), dx = b.x1 - a.x1, dy = b.y1 - a.y1;
  if (Math.abs(denominator) < 1e-8) return Math.abs(cross(dx, dy, ax, ay)) < 1e-8 &&
    Math.max(Math.min(a.x1, a.x2), Math.min(b.x1, b.x2)) <= Math.min(Math.max(a.x1, a.x2), Math.max(b.x1, b.x2)) &&
    Math.max(Math.min(a.y1, a.y2), Math.min(b.y1, b.y2)) <= Math.min(Math.max(a.y1, a.y2), Math.max(b.y1, b.y2));
  const t = cross(dx, dy, bx, by) / denominator, u = cross(dx, dy, ax, ay) / denominator;
  return t >= 0 && t <= 1 && u >= 0 && u <= 1;
}
export type LabelEnvironment = {
  stars?: readonly Pick<ProjectedLabel, "id" | "x" | "y" | "depth" | "radius">[];
  obstacles?: readonly { left: number; top: number; right: number; bottom: number }[];
  previous?: ReadonlyMap<string, LabelBox>;
  /** Resting summary names yield when no free spot is this close to the footprint, in CSS pixels.
   * The selection and hover keep their names; this never moves a star. */
  restingSummaryGap?: number;
  /** Small star titles beyond the limit yield. Group names, the selection, the hovered star and exempt stars are not counted;
   * close stars are not counted unless an exempt set (an opened group) is given. */
  starTitles?: { limit: number; exempt?: ReadonlySet<string>; reach?: number };
};
/** Rank the current projection before bounding the visible set. No ID window excludes later nodes. */
export function visibleLabels(candidates: ProjectedLabel[], width: number, height: number, selected: string | null, hovered: string | null, environment: LabelEnvironment = {}): LabelBox[] {
  const edgeGap = width <= 700 ? 8 : 0;
  const visible = candidates.filter(n => [n.x, n.y, n.depth, n.radius, n.width, n.height].every(Number.isFinite) && n.radius > 0 && n.depth >= -1 && n.depth <= 1 && n.x >= 0 && n.x <= width && n.y >= 0 && n.y <= height);
  const distance = (n: ProjectedLabel) => Math.hypot(n.x - width / 2, n.y - height / 2);
  const nearby = (a: ProjectedLabel, b: ProjectedLabel) => distance(a) - distance(b) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const priority = visible.filter(n => n.id === selected);
  const ordinary = visible.filter(n => n.id !== selected && !n.summary);
  const disclosed = ordinary.filter(n => environment.starTitles?.exempt?.has(n.id)).sort(nearby);
  const ordered = [...priority, ...disclosed, ...visible.filter(n => n.id !== selected && n.summary).sort((a, b) => (b.importance ?? 0) - (a.importance ?? 0) || nearby(a, b))];
  const readable = (a: ProjectedLabel, b: ProjectedLabel) => Number(b.active) - Number(a.active) || nearby(a, b);
  const documents = ordinary.filter(n => n.kind === "document" && !environment.starTitles?.exempt?.has(n.id)).sort(readable);
  const memories = ordinary.filter(n => n.kind === "memory" && !environment.starTitles?.exempt?.has(n.id)).sort(readable);
  // Each kind gets a turn before the other kind can consume the label budget.
  for (let i = 0; i < Math.max(documents.length, memories.length); i++) {
    if (documents[i]) ordered.push(documents[i]);
    if (memories[i]) ordered.push(memories[i]);
  }
  ordered.push(...ordinary.filter(n => n.kind !== "document" && n.kind !== "memory").sort(nearby));
  const fromStar = (node: ProjectedLabel, box: LabelBox) => Math.hypot(node.x - Math.max(box.left, Math.min(box.right, node.x)), node.y - Math.max(box.top, Math.min(box.bottom, node.y)));
  const restingReach = (node: ProjectedLabel) => node.id === selected ? Infinity : environment.starTitles?.exempt?.has(node.id) ? environment.starTitles.reach ?? Infinity :
    node.summary ? environment.restingSummaryGap === undefined ? Infinity : node.radius + environment.restingSummaryGap : node.radius + 18;
  const positionsFor = (node: ProjectedLabel): LabelBox[] => {
    const clearance = node.radius + 5;
    const sides = node.x > width * .66 ? [-1, 1] : [1, -1];
    const positions = sides.map(side => ({ left: node.x + (side === -1 ? -clearance - node.width : clearance), top: node.y - node.height / 2 }));
    const centered = Math.max(edgeGap, Math.min(width - edgeGap - node.width, node.x - node.width / 2));
    positions.push({ left: centered, top: node.y - clearance - node.height }, { left: centered, top: node.y + clearance });
    if (environment.stars || environment.obstacles) {
      // Keep the label near its star, but let it slide along an occupied side.
      const shifts = environment.starTitles?.exempt?.has(node.id) ? [-24, 24, -48, 48, -72, 72, -96, 96, -120, 120, -144, 144, -168, 168] : [-24, 24, -48, 48, -72, 72, -96, 96];
      for (const shift of shifts) {
        for (const side of sides) positions.push({ left: node.x + (side === -1 ? -clearance - node.width : clearance), top: node.y - node.height / 2 + shift });
        positions.push({ left: Math.max(edgeGap, Math.min(width - edgeGap - node.width, centered + shift)), top: node.y - clearance - node.height },
          { left: Math.max(edgeGap, Math.min(width - edgeGap - node.width, centered + shift)), top: node.y + clearance });
      }
    }
    if (node.summary && environment.stars) for (const gap of [24, 48, 72, 96, 128, 160, 208]) {
      for (const side of sides) for (const shift of [0, -gap / 2, gap / 2]) positions.push({ left: node.x + (side === -1 ? -clearance - gap - node.width : clearance + gap), top: node.y - node.height / 2 + shift });
      positions.push({ left: centered, top: node.y - clearance - gap - node.height }, { left: centered, top: node.y + clearance + gap });
    }
    const boxes = positions.map(({ left, top }) => ({ id: node.id, left, top, right: left + node.width, bottom: top + node.height }));
    // A close-up ring can leave only diagonal pockets inside the mobile gutter.
    if (edgeGap) for (const top of [edgeGap, height - edgeGap - node.height])
      for (const left of [edgeGap, width - edgeGap - node.width]) {
        const box = { id: node.id, left, top, right: left + node.width, bottom: top + node.height };
        if (fromStar(node, box) >= clearance) boxes.push(box);
      }
    // A projection has one answer, including its first frame. History-dependent
    // candidate order made a hover commit a different resting arrangement.
    if (node.summary && environment.restingSummaryGap !== undefined)
      boxes.sort((a, b) => {
        const difference = fromStar(node, a) - fromStar(node, b);
        // Equal sides can differ by floating-point roundoff after camera motion.
        // Preserve their authored order below a thousandth of a CSS pixel.
        return Math.abs(difference) < .001 ? 0 : difference;
      });
    return boxes;
  };
  const inBounds = (box: LabelBox) => box.left >= edgeGap && box.top >= edgeGap && box.right <= width - edgeGap && box.bottom <= height - edgeGap;
  const overlaps = (a: LabelBox, b: LabelBox) => a.left < b.right + 8 && a.right + 8 > b.left && a.top < b.bottom + 6 && a.bottom + 6 > b.top;
  const starCells = new Map<string, NonNullable<LabelEnvironment["stars"]>[number][]>();
  let largestStar = 0;
  for (const star of environment.stars ?? []) {
    if (![star.x, star.y, star.depth, star.radius].every(Number.isFinite) || star.depth < -1 || star.depth > 1 || star.radius <= 0) continue;
    largestStar = Math.max(largestStar, star.radius);
    const key = `${Math.floor(star.x / 64)},${Math.floor(star.y / 64)}`;
    const bucket = starCells.get(key) ?? []; bucket.push(star); starCells.set(key, bucket);
  }
  const hitsStar = (box: LabelBox) => {
    const reach = largestStar + 4;
    for (let x = Math.floor((box.left - reach) / 64); x <= Math.floor((box.right + reach) / 64); x++)
      for (let y = Math.floor((box.top - reach) / 64); y <= Math.floor((box.bottom + reach) / 64); y++)
        if ((starCells.get(`${x},${y}`) ?? []).some(star => star.id !== box.id &&
          Math.hypot(star.x - Math.max(box.left, Math.min(box.right, star.x)), star.y - Math.max(box.top, Math.min(box.bottom, star.y))) < star.radius + 4)) return true;
    return false;
  };
  const clear = (box: LabelBox) => inBounds(box) &&
    !(environment.obstacles ?? []).some(other => overlaps(box, { ...other, id: "chrome" })) &&
    !hitsStar(box);
  const byId = new Map(visible.map(node => [node.id, node]));
  const leaderCache = new Map<LabelBox, LabelLeader | null>();
  const leader = (box: LabelBox) => {
    if (!leaderCache.has(box)) leaderCache.set(box, labelLeader(byId.get(box.id)!, box));
    return leaderCache.get(box)!;
  };
  const guide = (box: LabelBox) => box.guide ?? (leader(box) ? [leader(box)!] : []);
  const passesStar = (line: LabelLeader, id: string) => {
    const reach = largestStar + 4;
    for (let x = Math.floor((Math.min(line.x1, line.x2) - reach) / 64); x <= Math.floor((Math.max(line.x1, line.x2) + reach) / 64); x++)
      for (let y = Math.floor((Math.min(line.y1, line.y2) - reach) / 64); y <= Math.floor((Math.max(line.y1, line.y2) + reach) / 64); y++)
        for (const star of starCells.get(`${x},${y}`) ?? []) {
          if (star.id === id) continue;
          const dx = line.x2 - line.x1, dy = line.y2 - line.y1;
          const t = Math.max(0, Math.min(1, ((star.x - line.x1) * dx + (star.y - line.y1) * dy) / Math.max(.001, dx * dx + dy * dy)));
          if (Math.hypot(star.x - line.x1 - t * dx, star.y - line.y1 - t * dy) < star.radius + 3) return true;
        }
    return false;
  };
  const fitsTogether = (box: LabelBox, placed: readonly LabelBox[]) => {
    const lines = guide(box), star = byId.get(box.id)!;
    if (lines.some(line => passesStar(line, box.id))) return false;
    // A covered source still needs an accessible name outside the covering control.
    if (lines.length && (environment.obstacles ?? []).some(other => {
      const coversSource = star.x >= other.left && star.x <= other.right && star.y >= other.top && star.y <= other.bottom;
      return !coversSource && lines.some(line => leaderHitsBox(line, other));
    })) return false;
    return placed.every(other => {
      if (overlaps(box, other)) return false;
      const otherLines = guide(other);
      return lines.every(line => !leaderHitsBox(line, other)) && otherLines.every(line => !leaderHitsBox(line, box)) &&
        lines.every(line => otherLines.every(otherLine => !leadersCross(line, otherLine)));
    });
  };
  // Bound geometric work, not elapsed time: identical projections keep identical names.
  // Crowded disclosures may defer a title; keyboard selection and the list remain available.
  let remainingRoutePaths = MAX_VISIBLE_LABELS * 3000;
  // Only a disclosed title that has no straight association may turn around a name.
  // Small, bounded elbow candidates preserve its title without inventing a graph edge.
  const routed = (node: ProjectedLabel, box: LabelBox, placed: readonly LabelBox[]): LabelBox | undefined => {
    if (remainingRoutePaths <= 0 || placed.some(other => overlaps(box, other) || guide(other).some(line => leaderHitsBox(line, box)))) return;
    const clamp = (at: number, min: number, max: number) => Math.max(min, Math.min(max, at));
    const ports = [{ x: box.left - 3, y: clamp(node.y, box.top + 3, box.bottom - 3) },
      { x: box.right + 3, y: clamp(node.y, box.top + 3, box.bottom - 3) },
      { x: clamp(node.x, box.left + 3, box.right - 3), y: box.top - 3 },
      { x: clamp(node.x, box.left + 3, box.right - 3), y: box.bottom + 3 }];
    let best: LabelBox | undefined, shortest = Infinity;
    const barriers = [...placed, box, ...(environment.stars ?? []).filter(star => star.id !== node.id && star.depth >= -1 && star.depth <= 1 && star.radius > 0 &&
      star.x + star.radius >= Math.min(node.x, box.left) - 24 && star.x - star.radius <= Math.max(node.x, box.right) + 24 &&
      star.y + star.radius >= Math.min(node.y, box.top) - 24 && star.y - star.radius <= Math.max(node.y, box.bottom) + 24)
      .map(star => ({ left: star.x - star.radius, right: star.x + star.radius, top: star.y - star.radius, bottom: star.y + star.radius }))];
    const xs = [...new Set(barriers.flatMap(other => [other.left - 5, other.right + 5]))];
    const ys = [...new Set(barriers.flatMap(other => [other.top - 5, other.bottom + 5]))];
    for (const port of ports) {
      const paths = [[{ x: node.x, y: port.y }], [{ x: port.x, y: node.y }],
        ...xs.map(x => [{ x, y: node.y }, { x, y: port.y }]), ...ys.map(y => [{ x: node.x, y }, { x: port.x, y }])];
      for (const bends of paths) {
        if (remainingRoutePaths-- <= 0) return best;
        const points = [{ x: node.x, y: node.y }, ...bends, port].filter((point, index, all) => !index || Math.hypot(point.x - all[index - 1].x, point.y - all[index - 1].y) > .001);
        if (points.length < 3 || points.some(point => point.x < 2 || point.x > width - 2 || point.y < 2 || point.y > height - 2)) continue;
        const first = points[1], distance = Math.hypot(first.x - node.x, first.y - node.y);
        if (distance <= node.radius + 3) continue;
        points[0] = { x: node.x + (first.x - node.x) / distance * (node.radius + 3), y: node.y + (first.y - node.y) / distance * (node.radius + 3) };
        const segments = points.slice(1).map((point, index) => ({ x1: points[index].x, y1: points[index].y, x2: point.x, y2: point.y, length: Math.hypot(point.x - points[index].x, point.y - points[index].y) }));
        const length = segments.reduce((sum, line) => sum + line.length, 0);
        const candidate = { ...box, guide: segments };
        if (length > Math.min(240, Math.max(width, height), restingReach(node)) || length >= shortest || segments.some(line => leaderHitsBox(line, box, 0)) || !fitsTogether(candidate, placed)) continue;
        best = candidate; shortest = length;
      }
    }
    return best;
  };
  // Resolve constrained summaries first; a flexible earlier label can yield its side.
  const options = new Map<string, LabelBox[]>();
  const optionsFor = (node: ProjectedLabel) => {
    let available = options.get(node.id);
    if (!available) { available = positionsFor(node).filter(box => clear(box) && fromStar(node, box) <= restingReach(node) && fitsTogether(box, [])); options.set(node.id, available); }
    return available;
  };
  if (environment.stars) {
    const summaries = ordered.filter(node => node.summary && node.id !== selected).slice(0, MAX_VISIBLE_LABELS * 2).sort((a, b) =>
      (b.importance ?? 0) - (a.importance ?? 0) || optionsFor(a).length - optionsFor(b).length || nearby(a, b));
    ordered.splice(priority.length + disclosed.length, summaries.length, ...summaries);
  }
  // The fallback keeps the label off its own body, then takes the nearest grid spot within reach.
  const nearestFree = (node: ProjectedLabel, placed: readonly LabelBox[], allowTurn = false): LabelBox | undefined => {
    if (allowTurn && remainingRoutePaths <= 0) return;
    const usable = (box: LabelBox) => fromStar(node, box) >= node.radius + 4 && clear(box) && !placed.some(other => overlaps(box, other));
    const arrange = (box: LabelBox) => usable(box) ? fitsTogether(box, placed) ? box : allowTurn ? routed(node, box, placed) : undefined : undefined;
    const reach = Math.min(240, Math.max(width, height), restingReach(node)), spots: LabelBox[] = [];
    for (let top = Math.max(edgeGap, Math.floor((node.y - reach - node.height) / 12) * 12); top <= Math.min(height - edgeGap - node.height, node.y + reach); top += 12)
      for (let left = Math.max(edgeGap, Math.floor((node.x - reach - node.width) / 12) * 12); left <= Math.min(width - edgeGap - node.width, node.x + reach); left += 12)
        spots.push({ id: node.id, left, top, right: left + node.width, bottom: top + node.height });
    for (const { box } of spots.map(box => ({ box, distance: fromStar(node, box) })).filter(spot => spot.distance <= reach).sort((a, b) => a.distance - b.distance)) {
      if (allowTurn && remainingRoutePaths <= 0) break;
      const arranged = arrange(box); if (arranged) return arranged;
    }
  };
  // Keep the base ranking stable; interaction can reveal a deferred name.
  const boxes: LabelBox[] = [];
  const rationed = (node: ProjectedLabel) => !!environment.starTitles && !node.summary && node.id !== selected &&
    !environment.starTitles.exempt?.has(node.id) && (!!environment.starTitles.exempt || (node.body ?? node.radius) < CLOSE_STAR_LABEL_RADIUS);
  let rationedTitles = 0;
  const important = !environment.stars ? [] : disclosed.length
    ? disclosed.slice(0, MAX_VISIBLE_LABELS - priority.length).sort((a, b) => optionsFor(a).length - optionsFor(b).length || nearby(a, b))
    : ordered.filter(node => node.summary && (node.importance ?? 0) > 0 && node.id !== selected).slice(0, 8);
  if (important.length > 1) {
    for (const node of priority) { const box = optionsFor(node)[0]; if (box) boxes.push(box); }
    // Resolve opened titles or prominent group names together, including their leaders.
    let states = [{ boxes: [...boxes], cost: 0 }];
    for (const node of important) {
      const next = states.flatMap(state => [
        ...optionsFor(node).slice(0, 52).flatMap((box, index) => !fitsTogether(box, state.boxes) ? [] : [{ boxes: [...state.boxes, box], cost: state.cost + index + (leader(box)?.length ?? 0) / 12 }]),
        { boxes: state.boxes, cost: state.cost + 10000 },
      ]);
      states = next.sort((a, b) => a.cost - b.cost).slice(0, 24);
    }
    boxes.splice(0, boxes.length, ...states[0].boxes);
  }
  for (const [attempt, node] of ordered.entries()) {
    if (attempt >= MAX_VISIBLE_LABELS * 4) break;
    if (boxes.some(box => box.id === node.id)) continue;
    if (rationed(node) && rationedTitles >= environment.starTitles!.limit) continue;
    let box = optionsFor(node).find(position => fitsTogether(position, boxes));
    if (!box && node.summary && environment.stars) for (const candidate of optionsFor(node)) {
      const collisions = boxes.filter(other => overlaps(candidate, other));
      if (collisions.length > 2 || collisions.some(other => other.id === selected || (visible.find(candidate => candidate.id === other.id)?.importance ?? 0) > (node.importance ?? 0))) continue;
      const shifted: LabelBox[] = [];
      for (const other of collisions) {
        const alternative = options.get(other.id)?.find(position => !overlaps(position, candidate) &&
          !boxes.some(existing => !collisions.includes(existing) && overlaps(position, existing)) && !shifted.some(existing => overlaps(position, existing)));
        if (!alternative) break;
        shifted.push(alternative);
      }
      const next = [...boxes.filter(existing => !collisions.includes(existing)), ...shifted];
      if (shifted.length === collisions.length && fitsTogether(candidate, next) && next.every((box, index) => fitsTogether(box, next.slice(0, index)))) {
        for (const replacement of shifted) boxes[boxes.findIndex(existing => existing.id === replacement.id)] = replacement;
        box = candidate; break;
      }
    }
    // Fill a close diagonal pocket before deferring a title or a prominent group name.
    if (!box && environment.stars && (!node.summary || (node.importance ?? 0) > 0)) box = nearestFree(node, boxes);
    if (!box && environment.starTitles?.exempt?.has(node.id)) {
      box = nearestFree(node, boxes, true);
      // A newly constrained guide may need a pocket already taken by another
      // disclosed title. Reconsider one such title before deferring either name.
      if (!box && environment.starTitles.reach === undefined && remainingRoutePaths > 0) for (const other of boxes.filter(other => other.id !== selected && environment.starTitles?.exempt?.has(other.id))) {
        if (remainingRoutePaths <= 0) break;
        const rest = boxes.filter(existing => existing !== other);
        const candidate = nearestFree(node, rest, true);
        if (!candidate) continue;
        const replacement = nearestFree(byId.get(other.id)!, [...rest, candidate], true);
        if (!replacement) continue;
        boxes[boxes.indexOf(other)] = replacement; box = candidate; break;
      }
    }
    if (box) { boxes.push(box); if (rationed(node)) rationedTitles++; }
    if (boxes.length === MAX_VISIBLE_LABELS) break;
  }
  if (!hovered || boxes.some(box => box.id === hovered)) return boxes;
  const node = visible.find(candidate => candidate.id === hovered);
  if (!node) return boxes;
  let insertion: LabelBox | undefined;
  let fewestCollisions = Infinity;
  for (const position of positionsFor(node)) {
    if (!clear(position) || boxes.some(box => (box.id === selected || byId.get(box.id)?.summary) && overlaps(position, box)) || !fitsTogether(position, boxes.filter(box => !overlaps(position, box)))) continue;
    const collisions = boxes.filter(box => overlaps(position, box)).length;
    if (collisions < fewestCollisions) {
      insertion = position;
      fewestCollisions = collisions;
    }
  }
  if (!insertion) return boxes;
  const survivors = boxes.filter(box => !overlaps(insertion, box));
  if (survivors.length === MAX_VISIBLE_LABELS) {
    const lastOrdinary = survivors.map(box => box.id !== selected && !byId.get(box.id)?.summary).lastIndexOf(true);
    if (lastOrdinary < 0) return boxes;
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

// Distant stars must be able to shrink with their projected spacing. A fixed
// 26px floor turned a fitted dense constellation into a solid white disc.
export const MIN_KNOWLEDGE_STAR_PIXELS = 4;
export const FOCUSED_KNOWLEDGE_STAR_PIXELS = 72;
/** Count controls one bounded visual scale, shared by glyphs, hit areas and packing. */
export function summaryAppearance(count: number): { pixels: number; samples: number } {
  if (!Number.isFinite(count) || count <= 0) return { pixels: 0, samples: 0 };
  if (count >= 100) return { pixels: 136, samples: 24 };
  if (count >= 50) return { pixels: 92, samples: 20 };
  if (count >= 30) return { pixels: 64, samples: 16 };
  if (count >= 10) return { pixels: 42, samples: 12 };
  return { pixels: 28, samples: 10 };
}
const KNOWLEDGE_STAR_WORLD_SIZE = 14;
/** Unclamped visual diameter; even unresolved stars keep their true depth in the light. */
export function starPerspectiveSize(depth: number, viewportHeight: number, projectionY: number): number {
  if (![depth, viewportHeight, projectionY].every(Number.isFinite) || depth <= 0 || viewportHeight <= 0 || projectionY <= 0) return 0;
  return KNOWLEDGE_STAR_WORLD_SIZE * (viewportHeight / depth) * projectionY / 2;
}
/** Continuous camera-depth contrast, including beyond the distant size floor. */
export function stellarDepth(perspectivePixels: number): number {
  return Number.isFinite(perspectivePixels) && perspectivePixels > 0 ? perspectivePixels / (perspectivePixels + KNOWLEDGE_STAR_WORLD_SIZE) : 0;
}
/** Perspective diameter in CSS pixels, shared by rendering, picking and projected labels. */
export function nodeScreenSize(kind: string, depth: number, viewportHeight: number, projectionY: number): number {
  const perspective = starPerspectiveSize(depth, viewportHeight, projectionY);
  if (perspective <= 0) return 0;
  if (kind !== "document" && kind !== "memory") return 10;
  return Math.min(140, Math.max(MIN_KNOWLEDGE_STAR_PIXELS, perspective));
}
export const LAYOUT_WORLD_SPACING = 31;
export const DEFAULT_LINK_PIXELS = 24;
/** Keep a readable existing zoom; otherwise bound magnification by the ordinary step. Large content still fits. */
export function coreCameraDistance(fitDistance: number, viewportHeight: number, projectionY: number, currentDistance = Infinity): number {
  return Math.max(fitDistance, Math.min(currentDistance, LAYOUT_WORLD_SPACING * viewportHeight * projectionY / (2 * DEFAULT_LINK_PIXELS)));
}
/** Keep an already close camera; otherwise reach the visible surface on selection. */
export function focusedCameraDistance(currentDistance: number, viewportHeight: number, projectionY: number): number {
  if (![currentDistance, viewportHeight, projectionY].every(Number.isFinite) || currentDistance <= 0 || viewportHeight <= 0 || projectionY <= 0) return currentDistance;
  return Math.min(currentDistance, KNOWLEDGE_STAR_WORLD_SIZE * viewportHeight * projectionY / (2 * FOCUSED_KNOWLEDGE_STAR_PIXELS));
}
/** Clear the corona and padded ring textures with one footprint for labels and picking. */
export function nodeScreenMetrics(size: number, selected: boolean, changed: boolean) {
  if (!Number.isFinite(size) || size <= 0) return { body: 0, selection: 0, change: 0, hit: 0, radius: 0 };
  // The ring's 112px circle occupies only part of its 128px texture.
  const selection = Math.max(27, size + 12) * (128 / 112), change = Math.max(36, size + 24) * (128 / 112);
  const extent = changed ? change : selected ? selection : size;
  return { body: size, selection, change, hit: Math.max(36, extent), radius: extent / 2 };
}
/** A summary's body, rings and labels all use its count-based screen footprint. */
export function nodeVisualRadius(size: number, selected: boolean, changed: boolean, summaryCount = 0): number {
  if (!Number.isFinite(size) || size <= 0) return 0;
  return nodeScreenMetrics(summaryCount > 0 ? summaryAppearance(summaryCount).pixels : size, selected, changed).radius;
}
/** A summary's ring responds to a grab and settles after release without moving its stars or hit area. */
export type SummaryHaloMotion = { id: string; startedAt: number; releasedAt: number | null; releaseScale: number };
export function summaryHaloScale(motion: SummaryHaloMotion | null, id: string, now: number, reduced: boolean): number {
  if (!motion || motion.id !== id || reduced || !Number.isFinite(now)) return 1;
  if (motion.releasedAt !== null) {
    const elapsed = Math.max(0, now - motion.releasedAt);
    return elapsed >= 450 ? 1 : 1 + (motion.releaseScale - 1) * Math.exp(-elapsed / 170) * Math.cos(elapsed / 85);
  }
  const elapsed = Math.max(0, now - motion.startedAt);
  return 1 + .08 * (1 - Math.exp(-elapsed / 120)) + .025 * Math.sin(elapsed / 70);
}
/** Cosmetic motion is independent of graph status, identity, layout and hit bounds. */
export function starPhase(id: string): number {
  let hash = 2166136261;
  for (let i = 0; i < id.length; i++) hash = Math.imul(hash ^ id.charCodeAt(i), 16777619);
  hash = Math.imul(hash ^ (hash >>> 16), 0x7feb352d);
  hash = Math.imul(hash ^ (hash >>> 15), 0x846ca68b);
  return ((hash ^ (hash >>> 16)) >>> 0) / 4294967296 * Math.PI * 2;
}
// A bounded spectrum keeps the stars luminous on the dark sky. Identity and
// taxonomy, rather than render timing, choose each color.
const documentStars = ["#8ecbff", "#6cc6ef", "#6bd5be", "#9bcf82", "#a0baff", "#bc9ee9", "#d694d1", "#79cfdb"];
const memoryStars = ["#f2ca79", "#f3ac77", "#e9969f", "#c89bdf", "#96b9e9", "#8ad0a7", "#e7a8b9", "#e4d17d"];
/** Stable variation; an explicit single taxonomy marker gives its members one color. */
export function starColor(node: { id: string; kind: string; taxonomyColor?: string }): string {
  if (node.taxonomyColor) return node.taxonomyColor;
  const palette = node.kind === "memory" ? memoryStars : documentStars;
  return palette[Math.floor(starPhase(node.id) / (Math.PI * 2) * palette.length)];
}
export function starShape(id: string): number {
  return Math.floor(starPhase(`${id}|shape`) / (Math.PI * 2) * 4);
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

/** Rank already-valid sprite hits in CSS pixels, independent of depth and viewport aspect. */
export function screenPickDistance(center: { x: number; y: number; z: number }, cursor: { x: number; y: number }, width: number, height: number): number {
  if (![center.x, center.y, center.z, cursor.x, cursor.y, width, height].every(Number.isFinite) || width <= 0 || height <= 0 || center.z < -1 || center.z > 1) return Infinity;
  return Math.hypot((center.x - cursor.x) * width / 2, (center.y - cursor.y) * height / 2);
}
