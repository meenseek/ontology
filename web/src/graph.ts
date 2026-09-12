import { UndirectedGraph } from "graphology";
import louvain from "graphology-communities-louvain";
import { nodePresentation, repositoryNames } from "./presentation.ts";

export type Scope = "meenseek" | "personal";
export type NodeKind = "document" | "memory" | "topic" | "subject" | "area";
export type LinkKind = "related" | "evidence" | "topic" | "subject" | "area";
export type Request = <T>(url: string, options?: RequestInit) => Promise<T>;
export type GraphNode = {
  id: string; scope: Scope; kind: NodeKind; label: string;
  revision?: string; relation_digest?: string; content_digest?: string | null; source_revision?: string | null;
  generation?: string; status?: string; present?: boolean; source_kind?: string; repository?: string;
  temporal?: "future" | "expired" | "current"; supported?: boolean;
  support?: string; subject_id?: string | null; memory_kind?: string;
  excerpt?: string | null; historical_match?: boolean; matched_revision?: string; search_match?: boolean;
  last_success_at?: string | null; observed_at?: string | null;
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
  cluster: string; color: string; changed: boolean; signature: string; repositoryLabel?: string;
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
  if (n.kind === "document") return n.status === "ok" && n.present === true;
  if (n.kind === "memory") return n.status === "accepted" && n.temporal === "current" && n.supported !== false;
  return true;
}
export function stateName(n: GraphNode): string {
  if (n.kind === "document") return n.status === "failed" ? "출처 확인 실패" : n.present ? "출처 확인" : "원문 부재";
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
      positioned.set(id, { ...node, repositoryLabel: repositories.get(node.repository ?? ""), x, y, z, fx: x, fy: y, fz: z, cluster: cluster.id, color: cluster.color, signature, changed: !!prior && signature !== prior.signature });
    }
  }
  return { scope: snapshot.scope, nodes: nodes.map(n => positioned.get(n.id)!), links, clusters };
}
export type Filters = { kind: "all" | "knowledge" | NodeKind; state: "all" | "active" | "proposed" | "withdrawn" | "attention"; cluster: string | null };
export function visibleGraph(model: Model, filters: Filters): { nodes: PositionedNode[]; links: GraphLink[] } {
  const nodes = model.nodes.filter(n => (!filters.cluster || n.cluster === filters.cluster) &&
    (filters.kind === "all" || (filters.kind === "knowledge" ? knowledge(n) : n.kind === filters.kind)) &&
    (filters.state === "all" || (filters.state === "active" ? active(n) : filters.state === "attention" ? knowledge(n) && !active(n) : n.status === filters.state)));
  const ids = new Set(nodes.map(n => n.id));
  return { nodes, links: model.links.filter(l => ids.has(l.source) && ids.has(l.target)) };
}
export function parseLocation(search: string): { scope: Scope; q: string; focus: string | null } {
  const params = new URLSearchParams(search);
  const q = params.get("q") ?? "";
  const focus = params.get("focus");
  return { scope: params.get("scope") === "personal" ? "personal" : "meenseek", q: [...q.replaceAll("\0", "")].slice(0, 120).join(""), focus: focus && /^(e_[a-f\d]{64}|[mp]_[a-f\d-]{36}|t_[1-9]\d*|a_[a-z-]+)$/.test(focus) ? focus : null };
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
