import { UndirectedGraph } from "graphology";
import louvain from "graphology-communities-louvain";
import { fileName, nodePresentation, repositoryNames } from "./presentation.ts";

export type Scope = "meenseek" | "personal";
export type GraphView = "purpose" | "relationships";
export type SubjectDefinition = { purpose: string; include: string; exclude: string };
export type NodeKind = "document" | "memory" | "topic" | "subject" | "area" | "folder";
export type LinkKind = "related" | "reference" | "evidence" | "topic" | "subject" | "area" | "parent";
export type Request = <T>(url: string, options?: RequestInit) => Promise<T>;
export type GraphNode = {
  id: string; scope: Scope; kind: NodeKind; label: string; title?: string | null;
  revision?: string; relation_digest?: string; content_digest?: string | null; source_revision?: string | null;
  generation?: string; status?: string; present?: boolean; current?: boolean; source_kind?: string; repository?: string;
  temporal?: "future" | "expired" | "current"; supported?: boolean;
  support?: string; subject_id?: string | null; memory_kind?: string;
  excerpt?: string | null; historical_match?: boolean; matched_revision?: string; search_match?: boolean;
  last_success_at?: string | null; observed_at?: string | null;
  created_at?: string | null; content_updated_at?: string | null;
  context_scope?: string | null; context_path?: string | null;
  material_id?: string | null; purpose_source_revision?: string | null;
  subject_name?: string | null; subject_revision?: number; definition?: SubjectDefinition | null;
  classification_revision?: number; classification_review_needed?: boolean; purpose_total?: number;
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
  cluster: string; relationshipCluster?: string; color: string; taxonomyColor?: string; changed: boolean; signature: string; repositoryLabel?: string;
};
export type Cluster = { id: string; label: string; color: string; members: string[]; knowledge: number; purpose?: boolean; totalKnowledge?: number };
export type Model = { scope: Scope; view?: GraphView; nodes: PositionedNode[]; links: GraphLink[]; clusters: Cluster[] };
export const kindName: Record<NodeKind, string> = { document: "문서", memory: "기록", topic: "문서 태그", subject: "목적 묶음", area: "회사 분야", folder: "폴더" };
export const linkName: Record<LinkKind, string> = { related: "관련 자료", reference: "원문 링크", evidence: "출처 근거", topic: "문서 태그", subject: "목적 묶음", area: "회사 분야", parent: "부모·소속" };
export const linkColor: Record<LinkKind, string> = { related: "#b2c5f0", reference: "#7db8bf", evidence: "#e7bb76", topic: "#97bbde", subject: "#ad98d4", area: "#7ecab7", parent: "#648d99" };
const colors = ["#91b8ff", "#ba9aef", "#7bd6c2", "#ecc68f", "#df9dbc", "#8accdc", "#cad990"];
const compare = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
const nameCollator = new Intl.Collator("ko", { numeric: true });
export const compareNames = (a: string, b: string) => nameCollator.compare(a, b);
export type ListSort = "updated" | "created" | "name";
export function listTimestamp(node: GraphNode, sort: ListSort): number | null {
  const value = sort === "created" ? node.created_at : node.content_updated_at ?? node.created_at;
  const parsed = value ? Date.parse(value) : NaN;
  return Number.isFinite(parsed) ? parsed : null;
}
/** Display sorting never mutates the graph model or its layout order. */
export function listNodes<T extends GraphNode>(nodes: readonly T[], sort: ListSort = "updated"): T[] {
  const timestamp = (node: GraphNode) => listTimestamp(node, sort) ?? -Infinity;
  return [...nodes].sort((a, b) => {
    const group = Number(knowledge(b)) - Number(knowledge(a));
    const time = sort !== "name" && knowledge(a) && knowledge(b) ? timestamp(b) - timestamp(a) : 0;
    const left = nodePresentation(a), right = nodePresentation(b);
    return group || time || compareNames(left.title, right.title) || compareNames(left.subtitle, right.subtitle) || compare(a.id, b.id);
  });
}
export const knowledge = (n: GraphNode) => n.kind === "document" || n.kind === "memory";
export const isNativeOriginal = (n: GraphNode) => n.kind === "document" && !!n.context_scope && !!n.context_path;
export function active(n: GraphNode): boolean {
  if (n.kind === "document") return n.status === "ok" && n.present === true && n.current === true;
  if (n.kind === "memory") return n.status === "accepted" && n.temporal === "current" && n.supported !== false;
  return true;
}
export function stateName(n: GraphNode): string {
  if (n.kind === "folder") return "경로에서 계산한 구조";
  if (isNativeOriginal(n)) return "원문 보존";
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

const parentPath = (path: string) => path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : ".";
const folderId = (scope: Scope, contextScope: string, path: string) => `folder:${scope}:${JSON.stringify([contextScope, path])}`;
export type Folder = GraphNode & { kind: "folder"; context_scope: string; context_path: string; members: string[] };
/** Structural ancestry comes only from returned native paths, never from document meaning. */
export function nativeFolders(nodes: readonly GraphNode[]): Folder[] {
  const folders = new Map<string, Folder>();
  for (const node of nodes.filter(isNativeOriginal)) {
    for (let path = parentPath(node.context_path!);;) {
      const id = folderId(node.scope, node.context_scope!, path);
      const folder = folders.get(id) ?? { id, scope: node.scope, kind: "folder", label: `${node.context_scope} · ${path === "." ? "최상위" : path}`, context_scope: node.context_scope!, context_path: path, members: [] };
      folder.members.push(node.id); folders.set(id, folder);
      if (path === ".") break;
      path = parentPath(path);
    }
  }
  return [...folders.values()].sort((a, b) => compareNames(a.label, b.label) || compare(a.id, b.id)).map(folder => ({ ...folder, members: folder.members.sort(compare) }));
}
function folderStructure(nodes: readonly GraphNode[]) {
  const folders = nativeFolders(nodes);
  const members = [...nodes.filter(isNativeOriginal), ...folders.filter(folder => folder.context_path !== ".")];
  const links: GraphLink[] = members.map(node => ({ source: node.id,
    target: folderId(node.scope, node.context_scope!, parentPath(node.context_path!)), kind: "parent", current: true }));
  return { nodes: folders, links };
}

/** Visual proximity only: singletons may follow one directly linked host, never merge semantic clusters. */
export function visualSatellites(model: Pick<Model, "nodes" | "links">): Map<string, string> {
  const byId = new Map(model.nodes.map(node => [node.id, node]));
  const sizes = new Map<string, number>();
  const community = (node: PositionedNode) => node.relationshipCluster ?? node.cluster;
  for (const node of model.nodes) sizes.set(community(node), (sizes.get(community(node)) ?? 0) + 1);
  const rank = (node: PositionedNode) => [Number(node.kind === "document" && active(node)), Number(sizes.get(community(node))! > 1), Number(active(node)), Number(node.kind === "document")];
  const order = (a: PositionedNode, b: PositionedNode) => {
    const ar = rank(a), br = rank(b);
    for (let i = 0; i < ar.length; i++) if (ar[i] !== br[i]) return br[i] - ar[i];
    return compare(a.id, b.id);
  };
  const neighbors = new Map(model.nodes.map(node => [node.id, new Set<string>()]));
  for (const link of model.links) {
    if (link.kind === "parent" || link.source === link.target || !byId.has(link.source) || !byId.has(link.target)) continue;
    neighbors.get(link.source)!.add(link.target); neighbors.get(link.target)!.add(link.source);
  }
  const satellites = new Map<string, string>();
  for (const node of [...model.nodes].sort(order)) {
    if (sizes.get(community(node))! > 1) continue;
    const host = [...neighbors.get(node.id)!].map(id => byId.get(id)!)
      .filter(other => !satellites.has(other.id) && (sizes.get(community(other))! > 1 || order(other, node) < 0))
      .sort(order)[0];
    if (host) satellites.set(node.id, host.id);
  }
  return satellites;
}

// This is the only owner of display memberships, initial positions and refresh reconciliation.
// Persisted subjects own purposes; Louvain still owns transient relation summaries.
// Optional folder ancestry never changes either membership or stored relations.
export function reconcile(snapshot: Snapshot, previous?: Model, showFolders = false, view: GraphView = "relationships"): Model {
  const originals = snapshot.nodes.filter(n => n.scope === snapshot.scope).sort((a, b) => compare(a.id, b.id));
  const structure = showFolders ? folderStructure(originals) : { nodes: [], links: [] };
  const nodes = [...originals, ...structure.nodes].sort((a, b) => compare(a.id, b.id));
  const byId = new Map(nodes.map(n => [n.id, n]));
  const links = [...snapshot.links, ...structure.links].filter(l => byId.has(l.source) && byId.has(l.target)).sort((a, b) => compare(linkKey(a), linkKey(b)));
  const graph = new UndirectedGraph();
  for (const node of originals) graph.addNode(node.id);
  for (const link of links) {
    if (link.kind === "parent" || link.source === link.target || !link.current || !active(byId.get(link.source)!) || !active(byId.get(link.target)!)) continue;
    if (graph.hasEdge(link.source, link.target)) graph.updateEdgeAttribute(link.source, link.target, "weight", (weight: number) => weight + 1);
    else graph.addEdge(link.source, link.target, { weight: 1 });
  }
  const seed = hash(`${snapshot.scope}|${originals.map(n => n.id).join("|")}|${links.filter(link => link.kind !== "parent").map(linkKey).join("|")}`);
  const memberships: Record<string, number> = graph.size ? louvain(graph, { rng: random(seed), randomWalk: false }) : Object.fromEntries(originals.map((n, i) => [n.id, i]));
  const communities = new Map<number, GraphNode[]>();
  for (const node of originals) { const group = memberships[node.id]; communities.set(group, [...(communities.get(group) ?? []), node]); }
  const relationshipClusters = new Map([...communities.values()].flatMap(members => members.map(node => [node.id, `${snapshot.scope}:${members[0].id}`])));
  const groups = new Map<string, GraphNode[]>();
  for (const node of originals) {
    const subject = node.kind === "subject" ? node.id : knowledge(node) ? node.subject_id : null;
    const identity = node.kind === "document" && node.material_id ? node.material_id : node.id;
    const id = view === "relationships" ? relationshipClusters.get(node.id)! : subject ? `${snapshot.scope}:purpose:${subject}` : `${snapshot.scope}:item:${identity}`;
    groups.set(id, [...(groups.get(id) ?? []), node]);
  }
  const repositories = repositoryNames(nodes.filter(n => n.kind === "document" && n.repository).map(n => n.repository!));
  const clusters: Cluster[] = [...groups].map(([id, members]) => {
    const purpose = view === "purpose" && id.startsWith(`${snapshot.scope}:purpose:`);
    const markers = members.filter(n => !knowledge(n));
    const first = markers[0] ?? members[0];
    const repositoryLabel = repositories.get(first.repository ?? "");
    const label = purpose ? members.find(node => node.kind === "subject")?.label ?? members.find(node => node.subject_name)?.subject_name ?? "목적 묶음" : first.kind === "document" ? [repositoryLabel, nodePresentation(first).title].filter(Boolean).join(" · ") : first.label;
    return { id, label, color: colors[hash(id) % colors.length], members: members.map(n => n.id), knowledge: members.filter(knowledge).length,
      ...(purpose ? { purpose: true, totalKnowledge: Math.max(members.filter(knowledge).length, ...members.map(node => node.purpose_total ?? 0)) } : {}) };
  });
  for (const folder of structure.nodes) clusters.push({ id: folder.id, label: folder.label,
    color: linkColor.parent, members: [folder.id], knowledge: 0 });
  clusters.sort((a, b) => compare(a.id, b.id));
  const old = new Map((previous?.scope === snapshot.scope ? previous.nodes : []).map(n => [n.id, n]));
  const taxonomyColors = new Map(nodes.filter(n => !knowledge(n)).map(n => [n.id, n.kind === "folder" ? linkColor.parent : colors[hash(`${snapshot.scope}|taxonomy|${n.kind}|${n.id}`) % colors.length]]));
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
      const signature = JSON.stringify([node.revision, node.content_digest, node.source_revision, node.generation, node.relation_digest, node.classification_revision]);
      const groups = taxonomyMembership.get(id);
      const taxonomyColor = knowledge(node) ? groups?.size === 1 ? taxonomyColors.get([...groups][0]) : undefined : taxonomyColors.get(id);
      positioned.set(id, { ...node, repositoryLabel: repositories.get(node.repository ?? ""), x, y, z, fx: x, fy: y, fz: z, cluster: cluster.id, relationshipCluster: relationshipClusters.get(id), color: cluster.color, taxonomyColor, signature, changed: !!prior && signature !== prior.signature });
    }
  }
  const model = { scope: snapshot.scope, view, nodes: nodes.map(n => positioned.get(n.id)!), links, clusters };
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
export type Filters = { kind: "all" | "knowledge" | NodeKind; state: "all" | "active" | "proposed" | "withdrawn" | "attention"; cluster: string | null; sourceScope?: string | null; folder?: string | null };
/** Project the selected diagram while preserving the complete source relationship model. */
export function diagramLinks(model: Pick<Model, "view" | "links">): GraphLink[] {
  return model.view === "purpose" ? model.links.filter(link => link.kind === "subject" || link.kind === "parent") : model.links;
}
export function visibleGraph(model: Model, filters: Filters): { nodes: PositionedNode[]; links: GraphLink[] } {
  const folder = filters.folder ? nativeFolders(model.nodes).find(node => node.id === filters.folder) : null;
  const nodes = model.nodes.filter(n => (!filters.cluster || n.cluster === filters.cluster) &&
    (!filters.folder || !!folder && n.context_scope === folder.context_scope && !!n.context_path &&
      (folder.context_path === "." || n.context_path === folder.context_path || n.context_path.startsWith(`${folder.context_path}/`))) &&
    (!filters.sourceScope || n.context_scope === filters.sourceScope) &&
    (filters.kind === "all" || (filters.kind === "knowledge" ? knowledge(n) : n.kind === filters.kind)) &&
    (filters.state === "all" || (filters.state === "active" ? active(n) : filters.state === "attention" ? knowledge(n) && !active(n) : n.status === filters.state)));
  const ids = new Set(nodes.map(n => n.id));
  return { nodes, links: diagramLinks(model).filter(l => ids.has(l.source) && ids.has(l.target)) };
}
export function visibleClusterOptions(model: Model, filters: Filters): { listed: Cluster[]; standalone: number } {
  const visible = new Set(visibleGraph(model, { ...filters, cluster: null, folder: null }).nodes.map(node => node.id));
  const nodes = new Map(model.nodes.map(node => [node.id, node]));
  const present = model.clusters.filter(cluster => !cluster.id.startsWith("folder:")).flatMap(cluster => {
    const members = cluster.members.filter(id => visible.has(id));
    return members.length ? [{ ...cluster, members, knowledge: members.filter(id => knowledge(nodes.get(id)!)).length }] : [];
  });
  const listed = present.filter(cluster => cluster.purpose || cluster.members.length > 1).sort((a, b) => compareNames(a.label, b.label) || compare(a.id, b.id));
  return { listed, standalone: present.length - listed.length };
}
/** Summarize only components whose hub cannot fit its neighbors in two compact rings. */
export function denseConstellationCores(nodes: readonly (GraphNode & { cluster?: string; relationshipCluster?: string })[], links: readonly GraphLink[], threshold = 18) {
  const byId = new Map(nodes.map(node => [node.id, node]));
  const neighbors = new Map(nodes.map(node => [node.id, new Set<string>()]));
  for (const link of links) {
    const source = byId.get(link.source), target = byId.get(link.target);
    if (link.kind === "parent" || link.source === link.target || !link.current || !source || !target || !active(source) || !active(target) ||
      ((source.relationshipCluster ?? source.cluster) && (target.relationshipCluster ?? target.cluster) && (source.relationshipCluster ?? source.cluster) !== (target.relationshipCluster ?? target.cluster))) continue;
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
function purposeCores(nodes: readonly PositionedNode[], threshold = 18) {
  const markers = new Map(nodes.filter(node => node.kind === "subject").map(node => [node.id, node]));
  const groups = new Map<string, string[]>();
  for (const node of nodes) {
    if (!knowledge(node) || !node.subject_id) continue;
    const members = groups.get(node.subject_id) ?? [];
    members.push(node.id); groups.set(node.subject_id, members);
  }
  return [...groups].sort(([a], [b]) => compare(a, b)).flatMap(([subject, members]) => {
    if (members.length <= threshold) return [];
    members.sort(compare);
    const hub = markers.has(subject) ? subject : members[0];
    return [{ hub, members: new Set([hub, ...members]), count: members.length }];
  });
}
export function constellationView(nodes: PositionedNode[], links: GraphLink[], selected: string | null, expandedCore: string | null, page?: number, view: GraphView = "relationships") {
  const cores = view === "purpose" ? purposeCores(nodes) : denseConstellationCores(nodes, links);
  // Folder summaries hide only direct original children with no semantic edge.
  // Real relation endpoints and nested folder structure remain individually visible.
  const related = new Set(links.filter(link => link.kind !== "parent").flatMap(link => [link.source, link.target]));
  const byId = new Map(nodes.map(node => [node.id, node]));
  const owned = new Set(cores.flatMap(core => [...core.members]));
  const children = new Map<string, Set<string>>();
  for (const link of links) {
    const child = byId.get(link.source);
    if (link.kind !== "parent" || !link.current || related.has(link.source) || owned.has(link.source) || !child || !isNativeOriginal(child) || byId.get(link.target)?.kind !== "folder") continue;
    if (!children.has(link.target)) children.set(link.target, new Set());
    children.get(link.target)!.add(link.source);
  }
  for (const [hub, members] of [...children].sort(([a], [b]) => compare(a, b))) {
    if (members.size > 18) cores.push({ hub, members: new Set([hub, ...[...members].sort()]), count: members.size + 1 });
  }
  const exposedCore = selected ? cores.find(core => core.members.has(selected))?.hub ?? null : expandedCore;
  const exposed = cores.find(core => core.hub === exposedCore);
  // Keep the real endpoint of every outside relation visible while summarizing the other members.
  const exposedMembers = (core: (typeof cores)[number]) => new Set(links.flatMap(link => {
    if (link.kind === "parent") return [];
    if (core.members.has(link.source) === core.members.has(link.target)) return [];
    const member = core.members.has(link.source) ? link.source : link.target;
    return member === core.hub ? [] : [member];
  }));
  const collapsed = cores.filter(core => core.hub !== exposedCore);
  const counts = new Map(collapsed.map(core => [core.hub, core.count]));
  const hidden = new Set(collapsed.flatMap(core => {
    const pinned = exposedMembers(core);
    return [...core.members].filter(id => id !== core.hub && !pinned.has(id));
  }));
  let disclosure: { hub: string; index: number; pages: number; visible: ReadonlySet<string>; pinned: ReadonlySet<string>; total: number } | null = null;
  if (page !== undefined && exposed && exposed.count > 36) {
    const pinned = exposedMembers(exposed);
    const members = [...exposed.members].filter(id => id !== exposed.hub && !pinned.has(id)).sort();
    if (members.length > 12) {
      const pages = Math.ceil(members.length / 12);
      const selectedIndex = selected ? members.indexOf(selected) : -1;
      const index = selectedIndex >= 0 ? Math.floor(selectedIndex / 12) : Math.max(0, Math.min(pages - 1, Math.trunc(page) || 0));
      const visible = new Set([exposed.hub, ...pinned, ...members.slice(index * 12, (index + 1) * 12)]);
      for (const id of exposed.members) if (!visible.has(id)) hidden.add(id);
      disclosure = { hub: exposed.hub, index, pages, visible, pinned, total: exposed.count };
    }
  }
  return { nodes: nodes.filter(node => !hidden.has(node.id)),
    links: links.filter(link => !hidden.has(link.source) && !hidden.has(link.target)), counts, cores, disclosure };
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
