import { UndirectedGraph } from "graphology";
import louvain from "graphology-communities-louvain";
import { fileName, nodePresentation, repositoryNames } from "./presentation.ts";

export type Scope = "meenseek" | "personal";
export type NodeKind = "document" | "memory" | "topic" | "subject" | "area";
export type LinkKind = "related" | "evidence" | "topic" | "subject" | "area";
export type Request = <T>(url: string, options?: RequestInit) => Promise<T>;
export type GraphNode = {
  id: string; scope: Scope; kind: NodeKind; label: string;
  revision?: string; relation_digest?: string; content_digest?: string | null; source_revision?: string | null;
  generation?: string; status?: string; present?: boolean; current?: boolean; source_kind?: string; repository?: string;
  temporal?: "future" | "expired" | "current"; supported?: boolean;
  support?: string; subject_id?: string | null; memory_kind?: string;
  excerpt?: string | null; historical_match?: boolean; matched_revision?: string; search_match?: boolean;
  last_success_at?: string | null; observed_at?: string | null;
  context_scope?: string | null; context_path?: string | null;
};
export type GraphLink = { source: string; target: string; kind: LinkKind; current: boolean };
export type Snapshot = {
  scope: Scope; query: string; focus: { id: string | null; found: boolean };
  nodes: GraphNode[]; links: GraphLink[]; matched: number;
  totals: { documents: number; memories: number; markers: number; links: number };
  returned: { knowledge: number; markers: number; links: number };
  omitted: { nodes: number; links: number }; eligible: { nodes: number; links: number };
  limits: { nodes: number; links: number; response_bytes: number; byte_limited: boolean };
  truncated: boolean;
};
export type PositionedNode = GraphNode & {
  x: number; y: number; z: number; fx: number; fy: number; fz: number;
  cluster: string; color: string; taxonomyColor?: string; changed: boolean; signature: string; repositoryLabel?: string;
};
export type Cluster = { id: string; label: string; color: string; members: string[]; knowledge: number };
export type Model = { scope: Scope; nodes: PositionedNode[]; links: GraphLink[]; clusters: Cluster[] };
export const kindName: Record<NodeKind, string> = { document: "문서", memory: "기록", topic: "문서 태그", subject: "기록 묶음", area: "회사 분야" };
export const linkName: Record<LinkKind, string> = { related: "관련 자료", evidence: "출처 근거", topic: "문서 태그", subject: "기록 묶음", area: "회사 분야" };
export const linkColor: Record<LinkKind, string> = { related: "#b2c5f0", evidence: "#e7bb76", topic: "#97bbde", subject: "#ad98d4", area: "#7ecab7" };
const colors = ["#91b8ff", "#ba9aef", "#7bd6c2", "#ecc68f", "#df9dbc", "#8accdc", "#cad990"];
const compare = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
export const knowledge = (n: GraphNode) => n.kind === "document" || n.kind === "memory";
export function active(n: GraphNode): boolean {
  if (n.kind === "document") return n.status === "ok" && n.present === true && n.current === true;
  if (n.kind === "memory") return n.status === "accepted" && n.temporal === "current" && n.supported !== false;
  return true;
}
export function stateName(n: GraphNode): string {
  if (n.source_kind === "original") return "원문 보존";
  if (n.kind === "document") return n.status === "failed" ? "출처 확인 실패" : n.present ? n.current ? "출처 확인" : "갱신 대기" : "원문 부재";
  if (n.kind !== "memory") return "분류 표식";
  const parts = [{ accepted: "저장됨", proposed: "제안", withdrawn: "철회" }[n.status ?? ""] ?? "기록"];
  if (n.temporal === "future") parts.push("미래");
  if (n.temporal === "expired") parts.push("만료");
  if (n.supported === false) parts.push("근거 재확인 필요");
  return parts.join(" · ");
}
function hash(text: string): number {
  let value = 2166136261;
  for (const char of text) value = Math.imul(value ^ (char.codePointAt(0) ?? 0), 16777619);
  return value >>> 0;
}
function random(seed: number): () => number {
  let state = seed;
  return () => { state = (Math.imul(1664525, state) + 1013904223) >>> 0; return state / 4294967296; };
}
const linkKey = (l: GraphLink) => `${l.kind}:${l.source}:${l.target}:${l.current}`;

/** Visual proximity only: singletons may follow one directly linked host, never merge semantic clusters. */
export function visualSatellites(model: Pick<Model, "nodes" | "links">): Map<string, string> {
  const byId = new Map(model.nodes.map(node => [node.id, node]));
  const sizes = new Map<string, number>();
  for (const node of model.nodes) sizes.set(node.cluster, (sizes.get(node.cluster) ?? 0) + 1);
  const rank = (node: PositionedNode) => [Number(node.kind === "document" && active(node)), Number(sizes.get(node.cluster)! > 1), Number(active(node)), Number(node.kind === "document")];
  const order = (a: PositionedNode, b: PositionedNode) => {
    const ar = rank(a), br = rank(b);
    for (let i = 0; i < ar.length; i++) if (ar[i] !== br[i]) return br[i] - ar[i];
    return compare(a.id, b.id);
  };
  const neighbors = new Map(model.nodes.map(node => [node.id, new Set<string>()]));
  for (const link of model.links) {
    if (link.source === link.target || !byId.has(link.source) || !byId.has(link.target)) continue;
    neighbors.get(link.source)!.add(link.target); neighbors.get(link.target)!.add(link.source);
  }
  const satellites = new Map<string, string>();
  for (const node of [...model.nodes].sort(order)) {
    if (sizes.get(node.cluster)! > 1) continue;
    const host = [...neighbors.get(node.id)!].map(id => byId.get(id)!)
      .filter(other => !satellites.has(other.id) && (sizes.get(other.cluster)! > 1 || order(other, node) < 0))
      .sort(order)[0];
    if (host) satellites.set(node.id, host.id);
  }
  return satellites;
}

// This is the only owner of display memberships, initial positions and refresh reconciliation.
// Louvain sees current relationships between usable knowledge and explicit classification markers.
// No labels/text are interpreted as meaning and no membership is persisted.
export function reconcile(snapshot: Snapshot, previous?: Model): Model {
  const nodes = snapshot.nodes.filter(n => n.scope === snapshot.scope).sort((a, b) => compare(a.id, b.id));
  const byId = new Map(nodes.map(n => [n.id, n]));
  const links = snapshot.links.filter(l => byId.has(l.source) && byId.has(l.target)).sort((a, b) => compare(linkKey(a), linkKey(b)));
  const graph = new UndirectedGraph();
  for (const node of nodes) graph.addNode(node.id);
  for (const link of links) {
    if (link.source === link.target || !link.current || !active(byId.get(link.source)!) || !active(byId.get(link.target)!)) continue;
    if (graph.hasEdge(link.source, link.target)) graph.updateEdgeAttribute(link.source, link.target, "weight", (weight: number) => weight + 1);
    else graph.addEdge(link.source, link.target, { weight: 1 });
  }
  const seed = hash(`${snapshot.scope}|${nodes.map(n => n.id).join("|")}|${links.map(linkKey).join("|")}`);
  const memberships: Record<string, number> = graph.size ? louvain(graph, { rng: random(seed), randomWalk: false }) : Object.fromEntries(nodes.map((n, i) => [n.id, i]));
  const groups = new Map<number, GraphNode[]>();
  for (const node of nodes) { const group = memberships[node.id]; groups.set(group, [...(groups.get(group) ?? []), node]); }
  const repositories = repositoryNames(nodes.filter(n => n.kind === "document" && n.repository).map(n => n.repository!));
  const clusters: Cluster[] = [...groups.values()].map(group => {
    const id = `${snapshot.scope}:${group[0].id}`;
    const markers = group.filter(n => !knowledge(n));
    const first = markers[0] ?? group[0];
    const repositoryLabel = repositories.get(first.repository ?? "");
    const label = first.kind === "document" ? [repositoryLabel, nodePresentation(first).title].filter(Boolean).join(" · ") : first.label;
    return { id, label, color: colors[hash(id) % colors.length], members: group.map(n => n.id), knowledge: group.filter(knowledge).length };
  }).sort((a, b) => compare(a.id, b.id));
  const old = new Map((previous?.scope === snapshot.scope ? previous.nodes : []).map(n => [n.id, n]));
  const taxonomyColors = new Map(nodes.filter(n => !knowledge(n)).map(n => [n.id, colors[hash(`${snapshot.scope}|taxonomy|${n.kind}|${n.id}`) % colors.length]]));
  const taxonomyMembership = new Map<string, Set<string>>();
  for (const link of links) {
    if (!link.current || !["topic", "subject", "area"].includes(link.kind)) continue;
    const from = byId.get(link.source)!, to = byId.get(link.target)!;
    const marker = from.kind === link.kind && knowledge(to) ? from : to.kind === link.kind && knowledge(from) ? to : null;
    const member = marker === from ? to : marker === to ? from : null;
    if (marker && member) {
      const groups = taxonomyMembership.get(member.id) ?? new Set<string>();
      groups.add(marker.id); taxonomyMembership.set(member.id, groups);
    }
  }
  const positioned = new Map<string, PositionedNode>();
  // Reserve every retained point before placing a lower-sorting addition or a reintroduced node.
  const occupied: { x: number; y: number }[] = nodes.flatMap(n => old.has(n.id) ? [old.get(n.id)!] : []);
  const clusterSpacing = 160 + Math.sqrt(Math.max(1, ...clusters.map(c => c.members.length))) * 44;
  for (const [clusterIndex, cluster] of clusters.entries()) {
    const angle = clusterIndex * 2.399963229728653;
    const radius = Math.sqrt(clusterIndex) * clusterSpacing;
    const existing = cluster.members.flatMap(id => old.has(id) ? [old.get(id)!] : []);
    const center = existing.length ? existing.reduce((sum, n) => ({ x: sum.x + n.x / existing.length, y: sum.y + n.y / existing.length, z: sum.z + n.z / existing.length }), { x: 0, y: 0, z: 0 }) : { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius, z: 0 };
    let slot = 0;
    for (const id of cluster.members) {
      const node = byId.get(id)!;
      const prior = old.get(id);
      let x = prior?.x ?? center.x, y = prior?.y ?? center.y;
      const z = prior?.z ?? center.z;
      if (!prior) {
        // A planar spiral separates the default front view; rotations can still overlap.
        do {
          const theta = slot * 2.399963229728653, spread = 42 * Math.sqrt(slot++);
          x = center.x + Math.cos(theta) * spread; y = center.y + Math.sin(theta) * spread;
        } while (occupied.some(point => Math.hypot(point.x - x, point.y - y) < 40));
        occupied.push({ x, y });
      }
      const signature = JSON.stringify([node.revision, node.content_digest, node.source_revision, node.generation, node.relation_digest]);
      const groups = taxonomyMembership.get(id);
      const taxonomyColor = knowledge(node) ? groups?.size === 1 ? taxonomyColors.get([...groups][0]) : undefined : taxonomyColors.get(id);
      positioned.set(id, { ...node, repositoryLabel: repositories.get(node.repository ?? ""), x, y, z, fx: x, fy: y, fz: z, cluster: cluster.id, color: cluster.color, taxonomyColor, signature, changed: !!prior && signature !== prior.signature });
    }
  }
  const model = { scope: snapshot.scope, nodes: nodes.map(n => positioned.get(n.id)!), links, clusters };
  const satellites = visualSatellites(model);
  const priorSatellites = previous?.scope === snapshot.scope ? visualSatellites(previous) : new Map<string, string>();
  const relocating = new Set([...satellites].filter(([id, host]) => !old.has(id) || priorSatellites.get(id) !== host).map(([id]) => id));
  const reserved = model.nodes.filter(node => !relocating.has(node.id));
  for (const node of model.nodes) {
    if (!relocating.has(node.id)) continue;
    const host = positioned.get(satellites.get(node.id)!)!;
    let slot = 1, x: number, y: number;
    do {
      const angle = slot * 2.399963229728653, radius = 42 * Math.sqrt(slot++);
      x = host.x + Math.cos(angle) * radius; y = host.y + Math.sin(angle) * radius;
    } while (reserved.some(other => Math.hypot(other.x - x, other.y - y) < 40));
    node.x = node.fx = x; node.y = node.fy = y; node.z = node.fz = host.z;
    reserved.push(node);
  }
  return model;
}
export type Filters = { kind: "all" | "knowledge" | NodeKind; state: "all" | "active" | "proposed" | "withdrawn" | "attention"; cluster: string | null; sourceScope?: string | null };
export function visibleGraph(model: Model, filters: Filters): { nodes: PositionedNode[]; links: GraphLink[] } {
  const nodes = model.nodes.filter(n => (!filters.cluster || n.cluster === filters.cluster) &&
    (!filters.sourceScope || n.context_scope === filters.sourceScope) &&
    (filters.kind === "all" || (filters.kind === "knowledge" ? knowledge(n) : n.kind === filters.kind)) &&
    (filters.state === "all" || (filters.state === "active" ? active(n) : filters.state === "attention" ? knowledge(n) && !active(n) : n.status === filters.state)));
  const ids = new Set(nodes.map(n => n.id));
  return { nodes, links: model.links.filter(l => ids.has(l.source) && ids.has(l.target)) };
}
/** Summarize only components whose hub cannot fit its neighbors in two compact rings. */
export function denseConstellationCores(nodes: readonly (GraphNode & { cluster?: string })[], links: readonly GraphLink[], threshold = 18) {
  const byId = new Map(nodes.map(node => [node.id, node]));
  const neighbors = new Map(nodes.map(node => [node.id, new Set<string>()]));
  for (const link of links) {
    const source = byId.get(link.source), target = byId.get(link.target);
    if (link.source === link.target || !link.current || !source || !target || !active(source) || !active(target) ||
      (source.cluster && target.cluster && source.cluster !== target.cluster)) continue;
    neighbors.get(link.source)!.add(link.target); neighbors.get(link.target)!.add(link.source);
  }
  const seen = new Set<string>();
  const cores: { hub: string; members: ReadonlySet<string>; count: number }[] = [];
  for (const id of [...neighbors.keys()].sort()) {
    if (seen.has(id)) continue;
    const component = [id]; seen.add(id);
    for (let index = 0; index < component.length; index++) {
      for (const other of [...neighbors.get(component[index])!].sort()) {
        if (!seen.has(other)) { seen.add(other); component.push(other); }
      }
    }
    const hub = component.sort((a, b) => neighbors.get(b)!.size - neighbors.get(a)!.size || compare(a, b))[0];
    if (neighbors.get(hub)!.size > threshold) cores.push({ hub, members: new Set(component), count: component.length });
  }
  return cores;
}
export function constellationView(nodes: PositionedNode[], links: GraphLink[], selected: string | null, expandedCore: string | null) {
  const cores = denseConstellationCores(nodes, links);
  // A visible edge from a hidden member to an outside node must not disappear.
  const collapsed = cores.filter(core => core.hub !== expandedCore && !(selected && core.members.has(selected)) &&
    !links.some(link => core.members.has(link.source) !== core.members.has(link.target) &&
      (core.members.has(link.source) ? link.source : link.target) !== core.hub));
  const counts = new Map(collapsed.map(core => [core.hub, core.count]));
  const hidden = new Set(collapsed.flatMap(core => [...core.members].filter(id => id !== core.hub)));
  return { nodes: nodes.filter(node => !hidden.has(node.id)),
    links: links.filter(link => !hidden.has(link.source) && !hidden.has(link.target)), counts, cores };
}
/** A relation refresh refits an expanded core; coordinate-only drags do not. */
export function expandedCoreCameraFrame(nodes: readonly PositionedNode[], links: readonly GraphLink[], hubId: string, members: ReadonlySet<string>) {
  const hub = nodes.find(node => node.id === hubId);
  if (!hub) return null;
  const radius = Math.max(18, ...nodes.filter(node => members.has(node.id)).map(node => Math.hypot(node.x - hub.x, node.y - hub.y, node.z - hub.z) + 6));
  const internalLinks = links.filter(link => members.has(link.source) && members.has(link.target))
    .map(link => JSON.stringify([link.source, link.target, link.kind, link.current])).sort();
  return { radius, key: JSON.stringify([[...members].sort(), internalLinks]) };
}
/** Put filename matches ahead of broad body/path matches in the searchable list. */
export function searchResults<T extends GraphNode>(nodes: T[], query: string): T[] {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return nodes.filter(node => node.search_match !== false);
  const phrase = terms.join(" ");
  const includesAll = (text: string) => terms.every(term => text.includes(term));
  const score = (node: GraphNode) => {
    const name = node.kind === "document" ? fileName(node.label) : node.label;
    const filename = name.toLocaleLowerCase();
    const stem = filename.replace(/\.[^.]+$/, "");
    const path = node.label.toLocaleLowerCase();
    const match = filename === phrase ? 0 : stem === phrase ? 1 : stem.startsWith(phrase) ? 2 : includesAll(filename) ? 3 : includesAll(path) ? 4 : 5;
    const depth = node.kind === "document" ? node.label.split(/[\\/]/).length : 0;
    return [match, node.kind === "document" || node.kind === "memory" ? 0 : 1, depth, stem.length] as const;
  };
  return nodes.filter(node => node.search_match !== false).sort((a, b) => {
    const left = score(a), right = score(b);
    for (let i = 0; i < left.length; i++) if (left[i] !== right[i]) return left[i] - right[i];
    return compare(a.id, b.id);
  });
}
export function parseLocation(search: string): { scope: Scope; q: string; focus: string | null } {
  const params = new URLSearchParams(search);
  const q = params.get("q") ?? "";
  const focus = params.get("focus");
  return { scope: params.get("scope") === "meenseek" ? "meenseek" : "personal", q: [...q.replaceAll("\0", "")].slice(0, 120).join(""), focus: focus && /^(e_[a-f\d]{64}|[mpc]_[a-f\d-]{36}|t_[1-9]\d*|a_[a-z-]+)$/.test(focus) ? focus : null };
}
export function sameGraphLocation(a: ReturnType<typeof parseLocation>, b: ReturnType<typeof parseLocation>): boolean {
  return a.scope === b.scope && a.q === b.q && a.focus === b.focus;
}
export function graphUrl(scope: Scope, q = "", focus: string | null = null): string {
  const params = new URLSearchParams({ scope });
  if (q) params.set("q", q);
  if (focus) params.set("focus", focus);
  return `/?${params}`;
}
