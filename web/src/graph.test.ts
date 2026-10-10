/// <reference types="node" />
import assert from "node:assert/strict";
import { test } from "node:test";
import { Positions, compactSlots, fixPosition } from "./positions.ts";
import { separateDiscs } from "./clearance.ts";
import { PerspectiveCamera, Vector3 } from "three";
import { active, constellationView, denseConstellationCores, diagramLinks, expandedCoreCameraFrame, graphUrl, isNativeOriginal, knowledge, listNodes, nativeFolders, parseLocation, reconcile, sameGraphLocation, searchResults, stateName, visibleClusterOptions, visibleGraph, visualSatellites } from "./graph.ts";
import { nucleusLabelIds, nucleusLevel, nucleusView } from "./nuclei.ts";
import type { GraphLink, GraphNode, Snapshot } from "./graph.ts";
import type { ProjectedLabel } from "./presentation.ts";
import { CameraMotion, zoomCameraPose } from "./camera-motion.ts";
import { miniMapTransform } from "./MiniMap.tsx";
import { REVEAL_DURATION, retargetReveal, revealOpacity, revealState } from "./reveal-transition.ts";
const doc = (id: string): GraphNode => ({ id, scope: "meenseek", kind: "document", label: id, revision: "1", generation: "1", content_digest: "digest", source_revision: "revision", status: "ok", present: true, current: true });
const memory = (id: string): GraphNode => ({ id, scope: "meenseek", kind: "memory", label: id, revision: "1", status: "accepted", temporal: "current", supported: true });
const edge = (source: string, target: string, current = true): GraphLink => ({ source, target, kind: "related", current });
test("minimap zoom preserves a panned 3D viewing direction and respects camera limits", () => {
  const pose = { position: { x: 40, y: 60, z: 90 }, target: { x: 10, y: 20, z: 30 } };
  const zoomed = zoomCameraPose(pose, 1.25, 1, 1000);
  assert.deepEqual(zoomed.target, pose.target);
  assert.deepEqual(zoomed.position, { x: 34, y: 52, z: 78 });
  assert.deepEqual(zoomCameraPose(zoomed, 1 / 1.25, 1, 1000), pose);
  const distance = (p: typeof pose) => Math.hypot(p.position.x - p.target.x, p.position.y - p.target.y, p.position.z - p.target.z);
  assert.ok(Math.abs(distance(zoomCameraPose(pose, 100, 20, 100)) - 20) < 1e-10);
  assert.ok(Math.abs(distance(zoomCameraPose(pose, 0.01, 20, 100)) - 100) < 1e-10);
  assert.deepEqual(zoomCameraPose(pose, 0), pose);
});
function snapshot(nodes: GraphNode[], links: GraphLink[] = []): Snapshot {
  return { scope: "meenseek", query: "", focus: { id: null, found: false }, nodes, links, matched: nodes.length, totals: { documents: nodes.filter(n => n.kind === "document").length, memories: nodes.filter(n => n.kind === "memory").length, markers: nodes.filter(n => n.kind !== "document" && n.kind !== "memory").length, links: links.length }, returned: { knowledge: nodes.length, markers: 0, links: links.length }, omitted: { nodes: 0, links: 0 }, eligible: { nodes: nodes.length, links: links.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
}
test("stored purposes retain identity across names, missing markers, removed first members and unrelated relations", () => {
  const member = (id: string) => ({ ...doc(id), subject_id: "p_fixed", subject_name: "실제 목적", purpose_total: 8 });
  const input = snapshot([member("a"), member("b"), doc("isolated")], [edge("a", "isolated")]);
  const before = reconcile(input, undefined, false, "purpose");
  const group = before.nodes.find(node => node.id === "b")!.cluster;
  assert.equal(group, "meenseek:purpose:p_fixed");
  const after = reconcile(snapshot([{ ...member("b"), subject_name: "바뀐 이름" }, doc("new")], [edge("b", "new")]), before, false, "purpose");
  assert.equal(after.nodes.find(node => node.id === "b")!.cluster, group);
  assert.equal(after.clusters.find(cluster => cluster.id === group)!.label, "바뀐 이름");
  assert.equal(after.clusters.find(cluster => cluster.id === group)!.totalKnowledge, 8);
  assert.equal(after.nodes.find(node => node.id === "new")!.cluster, "meenseek:item:new");
  const options = visibleClusterOptions(after, { kind: "knowledge", state: "all", cluster: null });
  assert.equal(options.listed[0].knowledge, 1);
  assert.equal(options.listed[0].totalKnowledge, 8, "full membership is not a filtered count");
});
test("an unclassified native purpose keeps its canonical material identity across binding display IDs", () => {
  const material_id = "00000000-0000-4000-8000-000000000101";
  const original = { ...doc(`c_${material_id}`), material_id };
  const a = reconcile(snapshot([original]), undefined, false, "purpose");
  const b = reconcile(snapshot([{ ...original, id: "e_bound-source" }]), a, false, "purpose");
  assert.equal(a.nodes[0].cluster, `meenseek:item:${material_id}`);
  assert.equal(b.nodes[0].cluster, a.nodes[0].cluster);
  assert.equal(reconcile(snapshot([{ ...original, id: "e_bound-source" }])).nodes[0].cluster, "meenseek:e_bound-source", "relationship IDs retain their existing contract");
});
test("purpose compression follows stored membership while relationship compression retains real adjacency", () => {
  for (const count of [22, 40, 75, 120]) for (const membership of ["same", "mixed", "none"]) {
    const nodes = Array.from({ length: count }, (_, i) => ({ ...doc(`n${String(i).padStart(3, "0")}`),
      ...(membership === "none" ? {} : { subject_id: membership === "same" || i % 2 ? "p_one" : "p_two", subject_name: "합성 목적" }) }));
    const input = snapshot(nodes, nodes.slice(1).map(node => edge(nodes[0].id, node.id)));
    const purpose = reconcile(input, undefined, false, "purpose");
    const relation = reconcile(input, purpose, false, "relationships");
    assert.deepEqual(purpose.links, relation.links, "switching diagrams preserves all source relations");
    assert.deepEqual(diagramLinks(purpose), [], "references cannot become purpose memberships");
    const expected = membership === "none" ? [] : membership === "same" ? [count] : count / 2 > 18 ? [Math.floor(count / 2), Math.ceil(count / 2)] : [];
    const purposeful = constellationView(purpose.nodes, diagramLinks(purpose), null, null, 0, purpose.view);
    assert.deepEqual(purposeful.cores.map(core => core.count), expected);
    for (const core of purposeful.cores) {
      const expanded = constellationView(purpose.nodes, diagramLinks(purpose), null, core.hub, 0, purpose.view);
      assert.equal(expanded.disclosure?.pages ?? 1, core.count > 36 ? Math.ceil((core.count - 1) / 12) : 1);
      assert.ok([...core.members].every(id => purpose.nodes.find(node => node.id === id)!.subject_id === purpose.nodes.find(node => node.id === core.hub)!.subject_id));
    }
    {
      const model = relation;
      const collapsed = constellationView(model.nodes, diagramLinks(model), null, null, 0, model.view);
      assert.equal(collapsed.nodes.length, 1);
      assert.equal(collapsed.counts.get(nodes[0].id), count);
      const expanded = constellationView(model.nodes, diagramLinks(model), null, nodes[0].id, 0, model.view);
      assert.equal(expanded.nodes.length, count > 36 ? 13 : count);
      assert.equal(expanded.disclosure?.pages ?? 1, count > 36 ? Math.ceil((count - 1) / 12) : 1);
      if (expanded.disclosure) {
        const last = constellationView(model.nodes, diagramLinks(model), null, nodes[0].id, expanded.disclosure.pages - 1, model.view);
        assert.ok(last.nodes.length <= 13);
      }
    }
    const roundTrip = reconcile(input, relation, false, "purpose");
    assert.deepEqual(roundTrip.nodes.map(node => [node.id, node.cluster, node.x, node.y, node.z]), purpose.nodes.map(node => [node.id, node.cluster, node.x, node.y, node.z]));
  }
});
test("purpose markers, filtered representatives and physical pages share one stable membership projection", () => {
  const subjects = ["p_one", "p_two"], counts = [120, 23];
  const markers = subjects.map((id, i): GraphNode => ({ id, scope: "meenseek", kind: "subject", label: `목적 ${i}` }));
  const members = counts.flatMap((count, group) => Array.from({ length: count }, (_, i) => ({
    ...doc(`g${group}-${String(i).padStart(3, "0")}`), subject_id: subjects[group], subject_name: `목적 ${group}`,
  })));
  const memberships: GraphLink[] = members.map(node => ({ source: node.id, target: node.subject_id, kind: "subject", current: true }));
  const references = members.map(node => ({ ...edge("unclassified", node.id), kind: "reference" as const }));
  const input = snapshot([...markers, ...members, doc("unclassified")], [...memberships, ...references]);
  const model = reconcile(input, undefined, false, "purpose"), links = diagramLinks(model);
  assert.equal(model.links.length, memberships.length + references.length);
  assert.ok(links.every(link => link.kind === "subject"));
  const overview = constellationView(model.nodes, links, null, null, 0, model.view);
  assert.deepEqual(overview.cores.map(core => [core.hub, core.count]), [[subjects[0], 120], [subjects[1], 23]]);
  assert.deepEqual(overview.nodes.map(node => node.id).sort(), [...subjects, "unclassified"].sort());
  const positions = new Positions(); positions.install(model, true); positions.showCore(null, 0, true, 0);
  const collapsed = members.map(node => [node.id, positions.layoutTarget(node.id)]);
  for (const node of members) assert.deepEqual(positions.layoutTarget(node.id), positions.layoutTarget(node.subject_id));
  const page = constellationView(model.nodes, links, null, subjects[0], 0, model.view);
  assert.equal(page.disclosure!.pages, 10);
  assert.equal(page.nodes.filter(node => node.subject_id === subjects[0]).length, 12);
  const selected = constellationView(model.nodes, links, "g0-119", null, 0, model.view);
  assert.equal(selected.disclosure!.index, 9);
  assert.ok(selected.nodes.some(node => node.id === "g0-119"));
  positions.showCore(subjects[0], 1, true, 0);
  positions.showCore(subjects[0], 2, true, 9);
  positions.showCore(null, 3, true, 0);
  assert.deepEqual(members.map(node => [node.id, positions.layoutTarget(node.id)]), collapsed);
  const epoch = positions.structureEpoch;
  const refreshed = reconcile(snapshot(input.nodes, memberships), model, false, "purpose");
  positions.install(refreshed, true);
  assert.equal(positions.structureEpoch, epoch, "only reference changes cannot invalidate purpose geometry");
  assert.deepEqual(constellationView(refreshed.nodes, diagramLinks(refreshed), null, null, 0, refreshed.view).cores, overview.cores);
  const changedStates = model.nodes.map(node => knowledge(node) ? { ...node, current: false, status: "proposed" } : node);
  assert.deepEqual(constellationView(changedStates, links, null, null, 0, model.view).cores, overview.cores, "stored membership does not depend on active relation eligibility");
  const filtered = visibleGraph(model, { kind: "knowledge", state: "all", cluster: null });
  const withoutMarker = constellationView(filtered.nodes, filtered.links, null, null, 0, model.view);
  assert.deepEqual(withoutMarker.cores.map(core => [core.hub, core.count]), [["g0-000", 120], ["g1-000", 23]]);
  assert.equal(withoutMarker.nodes.find(node => node.id === "g0-000")!.subject_name, "목적 0");
});
test("Louvain separates two dense groups across a single bridge and is permutation invariant", () => {
  const nodes = Array.from({ length: 16 }, (_, i) => doc(`d${i}`));
  const links: GraphLink[] = [];
  for (const start of [0, 8]) for (let i = start; i < start + 8; i++) for (let j = i + 1; j < start + 8; j++) links.push(edge(`d${i}`, `d${j}`));
  links.push(edge("d7", "d8"));
  const a = reconcile(snapshot(nodes, links));
  const b = reconcile(snapshot([...nodes].reverse(), [...links].reverse()));
  assert.equal(a.clusters.length, 2);
  assert.deepEqual(a.clusters.map(c => c.members.length), [8, 8]);
  assert.deepEqual(a, b);
  assert.equal(a.nodes.find(n => n.id === "d0")?.cluster, a.nodes.find(n => n.id === "d7")?.cluster);
  assert.notEqual(a.nodes.find(n => n.id === "d7")?.cluster, a.nodes.find(n => n.id === "d8")?.cluster);
  assert.equal(links[0].source, "d0", "input links remain metadata, not renderer objects");
});
test("native folder ancestry is structural and cannot replace relationship communities", () => {
  const original = (id: string, context_scope: string, context_path: string, source_kind = "original") =>
    ({ ...doc(id), scope: "personal" as const, source_kind, context_scope, context_path });
  const nodes = [
    original("a", "personal", "writing/2026/company/a.md", "context"),
    original("b", "personal", "writing/2026/company/b.md"),
    original("c", "personal", "writing/2025/company/c.md"),
    original("d", "work/common", "writing/2026/company/d.md"),
    original("e", "personal", "root-a.md"), original("f", "personal", "root-b.md"),
  ];
  const input = snapshot(nodes, [edge("a", "c")]); input.scope = "personal";
  const model = reconcile(input), overlay = reconcile(input, undefined, true);
  const byId = new Map(model.nodes.map(node => [node.id, node]));
  assert.equal(isNativeOriginal(nodes[0]), true, "bound originals have the same structural ancestry");
  assert.equal(stateName(nodes[0]), "원문 보존");
  assert.equal(byId.get("a")!.cluster, byId.get("c")!.cluster, "a real relation crosses folder boundaries");
  assert.notEqual(byId.get("a")!.cluster, byId.get("b")!.cluster, "sharing a folder cannot invent relatedness");
  const folders = nativeFolders(model.nodes);
  const parent = folders.find(node => node.context_scope === "personal" && node.context_path === "writing/2026/company")!;
  assert.deepEqual(parent.members, ["a", "b"]);
  const root = folders.find(node => node.context_scope === "personal" && node.context_path === ".")!;
  assert.deepEqual(root.members, ["a", "b", "c", "e", "f"]);
  const links = overlay.links.filter(link => link.kind === "parent");
  assert.equal(links.filter(link => link.source === "a").length, 1);
  assert.equal(links.find(link => link.source === "a")!.target, parent.id);
  const writing = folders.find(node => node.context_scope === "personal" && node.context_path === "writing")!;
  assert.equal(links.find(link => link.source === writing.id)!.target, root.id);
  assert.equal(links.some(link => link.source === root.id), false, "a scope root has no invented parent");
  const otherParent = folders.find(node => node.context_scope === "work/common" && node.context_path === parent.context_path)!;
  assert.notEqual(parent.id, otherParent.id, "same paths in different native scopes are different folders");
  assert.equal(links.find(link => link.source === "d")!.target, otherParent.id);
  assert.deepEqual(overlay.links.filter(link => link.kind !== "parent"), model.links);
  assert.equal(overlay.nodes.filter(knowledge).length, nodes.length, "structural folders are not extra knowledge");
  for (const node of model.nodes) assert.equal(overlay.nodes.find(candidate => candidate.id === node.id)!.cluster, node.cluster);
  const filtered = visibleGraph(overlay, { kind: "all", state: "all", cluster: null, folder: writing.id });
  assert.deepEqual(filtered.nodes.filter(knowledge).map(node => node.id), ["a", "b", "c"]);
  assert.ok(filtered.nodes.every(node => node.context_scope === "personal"));
  assert.equal(filtered.links.filter(link => link.kind === "related").length, 1);
  const subfolder = visibleGraph(overlay, { kind: "knowledge", state: "all", cluster: null, folder: parent.id });
  assert.deepEqual(subfolder.nodes.map(node => node.id), ["a", "b"]);
  assert.equal(subfolder.links.length, 0, "siblings have only structural membership");
  assert.deepEqual(visibleClusterOptions(overlay, { kind: "all", state: "all", cluster: null }), visibleClusterOptions(model, { kind: "all", state: "all", cluster: null }));
  assert.deepEqual(overlay, reconcile({ ...input, nodes: [...nodes].reverse(), links: [...input.links].reverse() }, undefined, true));
  assert.deepEqual(overlay.nodes, reconcile(input, overlay, true).nodes, "refresh retains unchanged points");
});
test("folder ancestry does not change Louvain memberships of ordinary or native originals", () => {
  const nodes = [doc("ordinary-a"), doc("ordinary-b"), memory("ordinary-c"),
    { ...doc("original-a"), context_scope: "personal", context_path: "projects/a/one.md", source_kind: "original" },
    { ...doc("original-b"), context_scope: "personal", context_path: "projects/b/two.md", source_kind: "context" }];
  const links = [edge("ordinary-a", "original-a"), edge("ordinary-b", "original-a"), edge("ordinary-c", "original-b"), edge("original-a", "original-b")];
  const baseline = reconcile(snapshot(nodes.map(node => ({ ...node, context_scope: null, context_path: null })), links));
  for (const overlay of [false, true]) {
    const model = reconcile(snapshot(nodes, links), undefined, overlay);
    for (const node of baseline.nodes) assert.equal(model.nodes.find(candidate => candidate.id === node.id)!.cluster, node.cluster);
    assert.deepEqual(model.links.filter(link => link.kind !== "parent"), baseline.links);
  }
});
test("a large native hub across folders stays compact with structure enabled and after refresh", () => {
  const nodes = ["hub", ...Array.from({ length: 120 }, (_, i) => `page-${i}`)].map((id, i) => ({
    ...doc(id), context_scope: "personal", context_path: i ? `knowledge/notion/pages-${i % 3}/${id}.md` : "knowledge/notion/index.md",
  }));
  const links = nodes.slice(1).map(node => edge("hub", node.id)), input = snapshot(nodes, links);
  for (const structure of [false, true]) {
    const model = reconcile(input, undefined, structure);
    const positions = new Positions(); positions.install(model, true);
    const overview = constellationView(model.nodes, model.links, null, null, 0);
    assert.equal(overview.cores.length, 1);
    assert.equal(overview.counts.get("hub"), 121);
    assert.deepEqual(overview.nodes.filter(knowledge).map(node => node.id), ["hub"]);
    const expanded = constellationView(model.nodes, model.links, null, "hub", 0);
    assert.equal(expanded.nodes.filter(knowledge).length, 13, "folder ancestry must not pin every hidden child open");
    assert.equal(expanded.links.filter(link => link.kind === "related").length, 12);
    assert.equal(expanded.disclosure!.pinned.size, 0);
    const selected = constellationView(model.nodes, model.links, "page-99", null, 0);
    assert.ok(selected.nodes.some(node => node.id === "page-99"));
    assert.deepEqual(reconcile(input, model, structure).nodes.map(node => [node.id, node.cluster]), model.nodes.map(node => [node.id, node.cluster]));
    assert.equal(constellationView(model.nodes, model.links, null, null, 0).counts.get("hub"), 121);
  }
});
test("a large folder without semantic relations cannot become a connected knowledge core", () => {
  const nodes = Array.from({ length: 30 }, (_, i) => ({ ...doc(`d${i}`), context_scope: "personal", context_path: `notes/d${i}.md` }));
  const model = reconcile(snapshot(nodes), undefined, true);
  assert.equal(model.links.filter(link => link.kind === "parent").length, 31);
  assert.equal(model.links.filter(link => link.kind === "related").length, 0);
  assert.equal(denseConstellationCores(model.nodes, model.links).length, 0);
  assert.equal(new Set(model.nodes.filter(knowledge).map(node => node.cluster)).size, nodes.length);
});
test("folder summaries page unrelated siblings while retaining current and historical relation endpoints", () => {
  const nodes = [...Array.from({ length: 40 }, (_, i) => ({ ...doc(`d${String(i).padStart(2, "0")}`), context_scope: "personal", context_path: `notes/d${i}.md` })),
    { ...doc("outside"), context_scope: "personal", context_path: "other/outside.md" }];
  const input = snapshot(nodes, [edge("d00", "outside"), edge("d01", "outside", false)]);
  const model = reconcile(input, undefined, true), folder = nativeFolders(nodes).find(node => node.context_path === "notes")!;
  const positions = new Positions(); positions.install(model, true);
  const overview = constellationView(model.nodes, model.links, null, null, 0);
  assert.equal(denseConstellationCores(model.nodes, model.links).length, 0, "folder grouping does not create semantic cores");
  assert.equal(overview.counts.get(folder.id), 39, "the structural representative contains 38 documents and its folder");
  assert.deepEqual(overview.nodes.filter(knowledge).map(node => node.id), ["d00", "d01", "outside"]);
  assert.deepEqual(overview.links.filter(link => link.kind !== "parent"), input.links, "real current and historical endpoints stay visible");
  assert.equal(overview.links.filter(link => link.kind === "parent" && link.target === folder.id).length, 2);
  for (let page = 0; page < 4; page++) {
    const expanded = constellationView(model.nodes, model.links, null, folder.id, page);
    assert.equal(expanded.disclosure!.index, page);
    assert.equal(expanded.disclosure!.pages, 4);
    assert.equal(expanded.nodes.filter(knowledge).length, 3 + (page === 3 ? 2 : 12));
    assert.deepEqual(expanded.links.filter(link => link.kind !== "parent"), input.links);
    positions.showCore(folder.id, page * 100, true, page, expanded.disclosure!.visible);
    const hub = model.nodes.find(node => node.id === folder.id)!;
    for (const id of overview.cores[0].members) {
      if (id === folder.id) continue;
      const node = model.nodes.find(candidate => candidate.id === id)!;
      const distance = Math.hypot(node.x - hub.x, node.y - hub.y, node.z - hub.z);
      assert.ok(expanded.disclosure!.visible.has(id) ? distance > 0 : distance < 1e-7, "shown folder children have separate slots; other pages remain at their parent");
    }
  }
  const selected = constellationView(model.nodes, model.links, "d39", null, 0);
  assert.equal(selected.disclosure!.index, 3);
  assert.ok(selected.nodes.some(node => node.id === "d39"));
  assert.deepEqual(constellationView(reconcile(input, model, true).nodes, model.links, null, null, 0).cores, overview.cores);
  assert.equal(constellationView(reconcile(input).nodes, input.links, null, null, 0).cores.length, 0, "structure disabled leaves independent documents intact");
});
test("one explicit taxonomy marker shares its star color without recoloring ambiguous members", async () => {
  const topicA: GraphNode = { id: "topic-a", scope: "meenseek", kind: "topic", label: "A" };
  const topicB: GraphNode = { id: "topic-b", scope: "meenseek", kind: "topic", label: "B" };
  const links: GraphLink[] = [
    { source: "a", target: "topic-a", kind: "topic", current: true },
    { source: "b", target: "topic-a", kind: "topic", current: true },
    { source: "c", target: "topic-a", kind: "topic", current: true },
    { source: "c", target: "topic-b", kind: "topic", current: true },
  ];
  const model = reconcile(snapshot([doc("a"), doc("b"), doc("c"), topicA, topicB], links));
  const byId = new Map(model.nodes.map(node => [node.id, node]));
  const { starColor } = await import("./presentation.ts");
  assert.equal(byId.get("a")!.taxonomyColor, byId.get("topic-a")!.taxonomyColor);
  assert.equal(starColor(byId.get("a")!), starColor(byId.get("b")!));
  assert.equal(byId.get("c")!.taxonomyColor, undefined, "multiple taxonomy memberships keep a neutral star color");
  assert.deepEqual(model, reconcile(snapshot([topicB, topicA, doc("c"), doc("b"), doc("a")], [...links].reverse())));
});
test("unclassified stars have stable but varied colors and silhouettes", async () => {
  const { starColor, starShape } = await import("./presentation.ts");
  const ids = Array.from({ length: 24 }, (_, index) => `star-${index}`);
  const colors = ids.map(id => starColor({ id, kind: "document" }));
  const shapes = ids.map(starShape);
  assert.ok(new Set(colors).size >= 3);
  assert.ok(new Set(Array.from({ length: 100 }, (_, index) => starColor({ id: `spectrum-${index}`, kind: "document" }))).size >= 7);
  assert.deepEqual(new Set(shapes), new Set([0, 1, 2, 3]));
  assert.deepEqual(ids.map(id => starColor({ id, kind: "document" })), colors);
  assert.deepEqual(ids.map(starShape), shapes);
});
test("summary glyph samples real members into a stable, bounded star cloud", async () => {
  const { summaryGlyphStars } = await import("./summary-glyph.ts");
  const members = Array.from({ length: 120 }, (_, index) => ({ id: `member-${index}`, kind: "document", taxonomyColor: index === 0 ? "#abc123" : undefined }));
  const stars = summaryGlyphStars("group-a", members);
  assert.equal(stars.length, 24);
  assert.deepEqual(stars, summaryGlyphStars("group-a", [...members].reverse()));
  assert.ok(stars.every(star => Number.isFinite(star.x) && Number.isFinite(star.y) && Math.hypot(star.x, star.y) < 1 && star.depth >= -1 && star.depth <= 1));
  assert.ok(new Set(stars.map(star => star.color)).size > 1);
  assert.ok(new Set(stars.map(star => star.shape)).size > 1);
  const mixed = summaryGlyphStars("small", members.slice(0, 6).map((member, index) => ({ ...member, opacity: index === 0 ? .4 : 1 })));
  assert.equal(mixed.length, 6);
  assert.ok(mixed.some(star => star.color === "#abc123" && star.opacity === .4));
  assert.ok(mixed.some(star => star.opacity === 1));
});
test("empty, isolated and larger disconnected inputs have finite deterministic positions", () => {
  for (const size of [0, 1, 400]) {
    const nodes = Array.from({ length: size }, (_, i) => doc(`isolated-${i}`));
    const a = reconcile(snapshot(nodes));
    assert.equal(a.nodes.length, size); assert.equal(a.clusters.length, size);
    assert.ok(a.nodes.every(n => [n.x, n.y, n.z].every(Number.isFinite)));
    assert.deepEqual(a, reconcile(snapshot([...nodes].reverse())));
  }
});
test("duplicate names across document, memory, tag and subject remain distinct", () => {
  const raw: GraphNode[] = [doc("doc"), memory("memory"), { id: "tag", kind: "topic", scope: "meenseek", label: "same" }, { id: "subject", kind: "subject", scope: "meenseek", label: "same" }];
  const nodes = raw.map(n => ({ ...n, label: "same" }));
  const model = reconcile(snapshot(nodes, [{ ...edge("doc", "tag"), kind: "topic" }, { ...edge("memory", "subject"), kind: "subject" }]));
  assert.equal(model.nodes.length, 4); assert.equal(new Set(model.nodes.map(n => n.id)).size, 4);
  assert.deepEqual(model.clusters.map(c => c.knowledge), [1, 1]);
  const shown = visibleGraph(model, { kind: "knowledge", state: "all", cluster: null });
  assert.equal(shown.nodes.length, 2); assert.equal(shown.links.length, 0);
});
test("scope boundaries drop foreign nodes, links and previous coordinates", () => {
  const first = reconcile(snapshot([doc("same")], []));
  first.nodes[0].x = 9999;
  const input = snapshot([{ ...doc("same"), scope: "personal" }, doc("foreign")], [edge("same", "foreign")]); input.scope = "personal";
  const next = reconcile(input, first);
  assert.equal(next.nodes.length, 1); assert.equal(next.links.length, 0); assert.notEqual(next.nodes[0].x, 9999);
  assert.ok(next.clusters.every(c => c.id.startsWith("personal:")));
});
test("past evidence and nonusable memories do not connect current clusters", () => {
  const nodes = [doc("source"), memory("stale"), { ...memory("withdrawn"), status: "withdrawn" }, { ...memory("future"), temporal: "future" as const }, { ...memory("expired"), temporal: "expired" as const }, { ...memory("proposal"), status: "proposed" }];
  const links = nodes.slice(1).map(n => ({ ...edge(n.id, "source", n.id !== "stale"), kind: "evidence" as const }));
  const model = reconcile(snapshot(nodes, links));
  assert.equal(model.clusters.length, 6); assert.equal(model.links.length, 5, "historical links remain inspectable");
  assert.equal(active(nodes[2]), false); assert.match(stateName(nodes[2]), /철회/);
  assert.match(stateName(nodes[3]), /미래/); assert.match(stateName(nodes[4]), /만료/); assert.match(stateName(nodes[5]), /제안/);
  assert.match(stateName({ ...memory("unsupported"), supported: false }), /재확인/);
});
test("two-level filters preserve membership and endpoints; deleted nodes lose selection and coordinates", () => {
  const a = reconcile(snapshot([doc("a"), doc("b"), doc("c"), doc("d")], [edge("a", "b"), edge("c", "d")]));
  const group = a.nodes.find(n => n.id === "a")!.cluster;
  const inner = visibleGraph(a, { kind: "all", state: "all", cluster: group });
  assert.deepEqual(inner.nodes.map(n => n.id), ["a", "b"]);
  assert.equal(inner.links.length, 1);
  assert.ok(inner.nodes.every(n => n.cluster === group));
  const remembered = { x: 144, y: 23, z: -22 }; Object.assign(a.nodes.find(n => n.id === "b")!, remembered);
  const b = reconcile(snapshot([{ ...doc("b"), relation_digest: "authoritative relation removed" }, doc("c"), doc("d")], [edge("c", "d")]), a);
  assert.equal(b.nodes.find(n => n.id === "a"), undefined);
  assert.equal(b.clusters.flatMap(c => c.members).includes("a"), false);
  assert.ok(b.links.every(l => l.source !== "a" && l.target !== "a"));
  assert.deepEqual({ x: b.nodes[0].x, y: b.nodes[0].y, z: b.nodes[0].z }, remembered);
  assert.equal(b.nodes[0].changed, true, "relation removal is a real change");
});
test("checking source timestamps never highlights content changes; revisions and generations do", () => {
  const original = snapshot([doc("a"), memory("b")], [edge("a", "b")]);
  const a = reconcile(original);
  const checked = snapshot(original.nodes.map(n => ({ ...n, observed_at: "later", last_success_at: "later" })), original.links);
  const b = reconcile(checked, a);
  assert.ok(b.nodes.every(n => !n.changed));
  const changed = snapshot([{ ...doc("a"), generation: "2" }, { ...memory("b"), revision: "2" }], original.links);
  const c = reconcile(changed, b);
  assert.ok(c.nodes.every(n => n.changed));
  assert.deepEqual(c.nodes.map(n => [n.x, n.y, n.z]), b.nodes.map(n => [n.x, n.y, n.z]));
});
test("URL focus and Unicode search are explicit and round-trip without executing text", () => {
  const id = `e_${"a".repeat(64)}`;
  const url = graphUrl("personal", "한글 <script>& question", id);
  assert.deepEqual(parseLocation(url.slice(1)), { scope: "personal", q: "한글 <script>& question", focus: id });
  assert.deepEqual(parseLocation(""), { scope: "personal", q: "", focus: null });
  assert.equal(parseLocation("?scope=other&focus=<script>").scope, "personal");
  assert.deepEqual(parseLocation(graphUrl("meenseek", "사업 & 계획", id).slice(1)), { scope: "meenseek", q: "사업 & 계획", focus: id });
  assert.equal(parseLocation("?focus=<script>").focus, null);
  assert.equal([...parseLocation(`?q=${"가".repeat(121)}`).q].length, 120);
  const model = reconcile(snapshot([{ ...doc(id), label: "한글 <script>" }]));
  assert.equal(model.nodes.find(n => n.id === parseLocation(url.slice(1)).focus)?.label, "한글 <script>");
  assert.equal(visibleGraph(model, { kind: "memory", state: "all", cluster: null }).nodes.length, 0);
});

test("search, focus and cap windows cannot invent relation changes", () => {
  const first = reconcile(snapshot([{ ...doc("a"), relation_digest: "full-scope-a" }, { ...doc("b"), relation_digest: "full-scope-b" }], [edge("a", "b")]));
  const search = snapshot([{ ...doc("a"), relation_digest: "full-scope-a" }]); search.query = "a";
  assert.equal(reconcile(search, first).nodes[0].changed, false);
  const cap = snapshot([{ ...doc("a"), relation_digest: "full-scope-a" }, { ...doc("b"), relation_digest: "full-scope-b" }]); cap.truncated = true;
  assert.ok(reconcile(cap, first).nodes.every(n => !n.changed));
  const actual = snapshot([{ ...doc("a"), relation_digest: "changed-full-scope-a" }]);
  assert.equal(reconcile(actual, first).nodes[0].changed, true);
});
test("a delayed outside-focus response cannot replace a newer selection or its displayed window", async () => {
  for (const size of [2, 40]) {
    const requested = { scope: "meenseek" as const, q: "", focus: "outside" };
    let current: ReturnType<typeof parseLocation> = requested;
    let model = reconcile(snapshot(Array.from({ length: size }, (_, i) => doc(`inside-${i}`))));
    const displayed = model;
    let resolve!: (value: Snapshot) => void;
    const delayed = new Promise<Snapshot>(done => { resolve = done; });
    const response = delayed.then(value => {
      if (!sameGraphLocation(requested, current)) return;
      model = reconcile(value, model);
      if (!model.nodes.some(n => n.id === current.focus)) current = { ...current, focus: null };
    });
    current = { ...requested, focus: "inside-1" };
    const outside = snapshot([doc("outside")]); outside.truncated = true;
    resolve(outside); await response;
    assert.equal(current.focus, "inside-1");
    assert.equal(model, displayed, "a capped response for another focus never replaces the displayed graph");
    assert.equal(sameGraphLocation(requested, requested), true, "the requested outside focus can still load");
    assert.equal(sameGraphLocation(requested, { ...requested, q: "new search" }), false);
    assert.equal(sameGraphLocation(requested, { ...requested, scope: "personal" }), false);
  }
});

test("presentation retains disambiguating paths and never treats a folder as ownership", async () => {
  const { nodePresentation, fileName } = await import("./presentation.ts");
  const first = nodePresentation({ kind: "document", label: "one/README.md" });
  const second = nodePresentation({ kind: "document", label: "two/README.md" });
  assert.equal(first.title, second.title); assert.notEqual(first.subtitle, second.subtitle);
  assert.deepEqual(nodePresentation({ kind: "memory", label: "a/b is my literal title" }), { title: "a/b is my literal title", subtitle: "" });
  assert.equal(fileName("C:\\local\\note.md"), "note.md");
  assert.deepEqual(nodePresentation({ kind: "document", label: "meenseek/private/notes.md" }), { title: "notes.md", subtitle: "meenseek/private/notes.md" });
  assert.deepEqual(nodePresentation({ kind: "document", label: "projects/sample.md", title: "Sample 프로젝트" }), { title: "Sample 프로젝트", subtitle: "projects/sample.md" });
});
test("broad year searches surface a matching filename before deep path and body matches", () => {
  const nodes = [
    { ...doc("a"), label: "writing/applications/2026/company/jd.md", search_match: true },
    { ...doc("b"), label: "decisions/2026-job-search.md", search_match: true },
    { ...doc("c"), label: "knowledge/2026-market.md", search_match: true },
    { ...doc("d"), label: "decisions/other.md", excerpt: "2026 activity", search_match: true },
    { ...doc("e"), label: "decisions/2026-hidden.md", search_match: false },
  ];
  assert.deepEqual(searchResults(nodes, "2026").map(node => node.id), ["c", "b", "a", "d"]);
  assert.deepEqual(nodes.map(node => node.id), ["a", "b", "c", "d", "e"], "input order is untouched");
});
test("multiword searches put filenames containing every term before path and body matches", () => {
  const nodes = [
    { ...doc("body"), label: "a.md", excerpt: "2026 지원", search_match: true },
    { ...doc("path"), label: "2026/notes/지원.md", search_match: true },
    { ...doc("name"), label: "2026-지원.md", search_match: true },
    { ...doc("hidden"), label: "2026-지원-hidden.md", search_match: false },
  ];
  for (const query of ["2026 지원", "지원   2026"]) {
    assert.deepEqual(searchResults(nodes, query).map(node => node.id), ["name", "path", "body"]);
  }
});
test("dotfile names rank ahead of matches found only in a folder", () => {
  const nodes = [
    { ...doc("path"), label: "env/config.txt", search_match: true },
    { ...doc("name"), label: "config/.env", search_match: true },
  ];
  assert.deepEqual(searchResults(nodes, "env").map(node => node.id), ["name", "path"]);
});
test("an exact dotfile name ranks ahead of a stem match", () => {
  const nodes = [
    { ...doc("suffix"), label: "config/.env.md", search_match: true },
    { ...doc("exact"), label: "config/.env", search_match: true },
  ];
  assert.deepEqual(searchResults(nodes, ".env").map(node => node.id), ["exact", "suffix"]);
});
test("sprite projection keeps visual and picking sizes independent across viewport and camera changes", async () => {
  const { spriteScale } = await import("./presentation.ts");
  for (const height of [230, 844, 1200]) for (const projection of [1, 2.1445, 3]) {
    assert.ok(Math.abs(spriteScale(36, height, projection) * height * projection / 2 - 36) < 1e-10);
    assert.ok(spriteScale(24, height, projection) < spriteScale(36, height, projection));
  }
  for (const bad of [0, -1, NaN, Infinity]) assert.equal(spriteScale(36, bad, 2), 0);
});

test("snapshot repository suffixes distinguish identical filenames without truncating source identity", async () => {
  const { repositoryNames, nodePresentation } = await import("./presentation.ts");
  const repositories = ["/local/one/shared", "/local/two/shared", "/local/other", "/shared"];
  const names = repositoryNames(repositories);
  assert.equal(names.get(repositories[0]), "one/shared");
  assert.equal(names.get(repositories[1]), "two/shared");
  assert.equal(names.get(repositories[2]), "other");
  assert.equal(names.get(repositories[3]), "/shared");
  assert.equal(new Set(names.values()).size, repositories.length);
  assert.deepEqual(names, repositoryNames([...repositories, repositories[0]]));
  const path = `${"archive/".repeat(40)}actual-filename.md`;
  const model = reconcile(snapshot(repositories.map((repository, i) => ({ ...doc(`identity-${i}`), repository, label: path }))));
  for (const node of model.nodes) {
    assert.equal(nodePresentation(node).title, "actual-filename.md");
    assert.equal(nodePresentation(node).subtitle, `${node.repository} · ${path}`);
    assert.equal(nodePresentation(node, true).subtitle, `${names.get(node.repository!)} · ${path}`);
  }
  assert.equal(new Set(model.nodes.map(n => nodePresentation(n, true).subtitle)).size, repositories.length);
});

test("projected labels reconsider nodes beyond 24 after camera movement and share names across kinds", async () => {
  const { visibleLabels, MAX_VISIBLE_LABELS } = await import("./presentation.ts");
  const candidates = Array.from({ length: 60 }, (_, i) => ({ id: `${i < 40 ? "d" : "m"}-${i}`, kind: i < 40 ? "document" : "memory", active: i < 40, x: 35 + (i % 10) * 120, y: 45 + Math.floor(i / 10) * 100, depth: 0, radius: 12, width: 40, height: 30 }));
  const before = candidates.map((n, i) => ({ ...n, depth: i < 24 ? 0 : 2 }));
  assert.ok(visibleLabels(before, 1300, 700, null, null).every(box => Number(box.id.split("-")[1]) < 24));
  const after = candidates.map((n, i) => ({ ...n, depth: i < 24 ? 2 : 0 }));
  const moved = visibleLabels(after, 1300, 700, "m-59", "d-39");
  assert.equal(moved[0].id, "m-59"); assert.ok(moved.some(box => box.id === "d-39"));
  assert.ok(moved.some(box => box.id.startsWith("d-")));
  assert.ok(moved.some(box => box.id.startsWith("m-")));
  assert.equal(moved.length, MAX_VISIBLE_LABELS);
  const all = visibleLabels(candidates, 1300, 700, null, null);
  assert.equal(all.length, MAX_VISIBLE_LABELS);
  assert.ok(all.some(box => box.id.startsWith("m-")), "document IDs cannot starve even inactive memories");
  for (const [i, box] of all.entries()) {
    assert.ok(box.left >= 0 && box.top >= 0 && box.right <= 1300 && box.bottom <= 700);
    assert.ok(all.slice(i + 1).every(other => box.right + 8 <= other.left || other.right + 8 <= box.left || box.bottom + 6 <= other.top || other.bottom + 6 <= box.top));
  }
  const overlap = candidates.slice(0, 6).map(n => ({ ...n, x: 650, y: 350 }));
  const picked = visibleLabels(overlap, 1300, 700, overlap[2].id, null);
  assert.equal(picked[0].id, overlap[2].id);
  assert.ok(picked.length < overlap.length, "colliding label boxes are rejected");
  assert.deepEqual(visibleLabels([{ ...candidates[0], y: -1 }, { ...candidates[1], depth: 2 }], 1300, 700, null, null), []);
});

test("hovering any visible label preserves every box and its order", async () => {
  const { visibleLabels } = await import("./presentation.ts");
  const candidates = Array.from({ length: 36 }, (_, i) => ({ id: `label-${i}`, kind: i % 2 ? "memory" : "document", active: i % 3 !== 0, x: 40 + i % 9 * 130, y: 60 + Math.floor(i / 9) * 140, depth: 0, radius: 13, width: 80, height: 40 }));
  const original = structuredClone(candidates);
  for (const selected of [null, "label-35"]) {
    const base = visibleLabels(candidates, 1300, 700, selected, null);
    for (const box of base) assert.deepEqual(visibleLabels(candidates, 1300, 700, selected, box.id), base);
    assert.deepEqual(visibleLabels([...candidates].reverse(), 1300, 700, selected, null), base);
  }
  assert.deepEqual(candidates, original, "layout does not retain hover state in the input");
});
test("collapsed group labels stay visible among crowded document labels", async () => {
  const { visibleLabels } = await import("./presentation.ts");
  const candidates: ProjectedLabel[] = Array.from({ length: 40 }, (_, index) => ({ id: `a-${index}`, kind: "document", active: true,
    x: 600 + index % 8 * 12, y: 300 + Math.floor(index / 8) * 12, depth: 0, radius: 12, width: 110, height: 30 }));
  candidates.push({ id: "z-summary", kind: "document", active: true, summary: true,
    x: 630, y: 330, depth: 0, radius: 12, width: 110, height: 30 });
  assert.ok(visibleLabels(candidates, 1280, 720, null, null).some(box => box.id === "z-summary"));
});
test("a hidden hover replaces only local collisions, protects selection and restores the base on exit", async () => {
  const { visibleLabels } = await import("./presentation.ts");
  for (const viewport of [{ width: 1300, height: 700, radius: 13, labelWidth: 80, labelHeight: 40 }, { width: 390, height: 300, radius: 80, labelWidth: 145, labelHeight: 45 }]) {
    const { width, height, radius, labelWidth, labelHeight } = viewport;
    const candidates = Array.from({ length: 8 }, (_, i) => ({ id: `dense-${i}`, kind: "document", active: true, x: width / 2, y: height / 2, depth: 0, radius, width: labelWidth, height: labelHeight }));
    const base = visibleLabels(candidates, width, height, "dense-0", null);
    assert.ok(base.length >= 2 && base.length < candidates.length);
    const hidden = candidates.find(n => !base.some(box => box.id === n.id))!;
    const hovered = visibleLabels(candidates, width, height, "dense-0", hidden.id);
    assert.equal(hovered.at(-1)!.id, hidden.id);
    assert.deepEqual(hovered[0], base[0], "selected label cannot be displaced");
    const survivors = hovered.slice(0, -1);
    assert.deepEqual(survivors, base.filter(box => survivors.some(other => other.id === box.id)));
    assert.equal(base.length - survivors.length, 1, "an equal-position fixture needs only one local replacement");
    const inserted = hovered.at(-1)!;
    const removed = base.find(box => !survivors.some(other => other.id === box.id))!;
    assert.deepEqual({ ...inserted, id: removed.id }, removed, "equal collision ties retain the existing position order");
    assert.ok(inserted.left >= 0 && inserted.top >= 0 && inserted.right <= width && inserted.bottom <= height);
    assert.ok(inserted.left >= hidden.x + radius + 5 || inserted.right <= hidden.x - radius - 5 || inserted.top >= hidden.y + radius + 5 || inserted.bottom <= hidden.y - radius - 5);
    assert.deepEqual(visibleLabels(candidates, width, height, "dense-0", null), base);
  }
});
test("a hidden hover at the 24-label budget removes only the last ordinary label", async () => {
  const { visibleLabels, MAX_VISIBLE_LABELS } = await import("./presentation.ts");
  const candidates = Array.from({ length: 60 }, (_, i) => ({ id: `budget-${i}`, kind: "document", active: true, x: 35 + i % 10 * 120, y: 45 + Math.floor(i / 10) * 100, depth: 0, radius: 12, width: 40, height: 30 }));
  const base = visibleLabels(candidates, 1300, 700, "budget-59", null);
  assert.equal(base.length, MAX_VISIBLE_LABELS);
  const hidden = candidates.find(n => !base.some(box => box.id === n.id))!;
  const hovered = visibleLabels(candidates, 1300, 700, "budget-59", hidden.id);
  assert.equal(hovered.length, MAX_VISIBLE_LABELS);
  assert.equal(hovered.at(-1)!.id, hidden.id);
  assert.deepEqual(hovered.slice(0, -1), base.slice(0, -1));
  assert.deepEqual(visibleLabels(candidates, 1300, 700, "budget-59", null), base);
});
test("invalid, offscreen or unplaceable hovers retain the base labels", async () => {
  const { visibleLabels } = await import("./presentation.ts");
  const node = { id: "selected", kind: "document", active: true, x: 150, y: 100, depth: 0, radius: 13, width: 145, height: 45 };
  for (const hidden of [{ ...node, id: "hidden", width: 500 }, { ...node, id: "hidden", x: -1 }, { ...node, id: "hidden", depth: 2 }, { ...node, id: "hidden", radius: NaN }]) {
    const candidates = [node, hidden];
    const base = visibleLabels(candidates, 390, 230, node.id, null);
    assert.ok(base.length);
    assert.deepEqual(visibleLabels(candidates, 390, 230, node.id, hidden.id), base);
    assert.deepEqual(visibleLabels(candidates, 390, 230, node.id, "missing"), base);
  }
  const low = { ...node, x: 195, y: 60, radius: 80 };
  const candidates = [low, { ...low, id: "hidden" }];
  const base = visibleLabels(candidates, 390, 200, low.id, null);
  assert.equal(base.length, 1);
  assert.deepEqual(visibleLabels(candidates, 390, 200, low.id, "hidden"), base, "the only fitting position belongs to the selected label");
});

const coordinates = (n: { x: number; y: number; z: number }) => [n.x, n.y, n.z];
function assertSeparated(model: ReturnType<typeof reconcile>) {
  for (const [index, node] of model.nodes.entries()) {
    assert.ok(model.nodes.slice(index + 1).every(other => Math.hypot(node.x - other.x, node.y - other.y) >= 40 - 1e-9), "default front-view positions stay separated");
  }
}
test("sparse initial placement is planar and keeps actual singleton memberships", () => {
  for (const count of [2, 4, 8, 24]) {
    const input = snapshot(Array.from({ length: count }, (_, i) => doc(`sparse-${i}`)));
    const model = reconcile(input);
    assert.equal(model.clusters.length, count);
    assert.ok(model.nodes.every(n => n.z === 0));
    assertSeparated(model);
    assert.deepEqual(model, reconcile({ ...input, nodes: [...input.nodes].reverse() }));
  }
});
test("lower-sorting additions reserve retained singleton coordinates, including a joining cluster", () => {
  for (const connected of [false, true]) {
    const retained = reconcile(snapshot([doc("b")]));
    assert.deepEqual(coordinates(retained.nodes[0]), [0, 0, 0]);
    const input = snapshot([doc("a"), doc("b")], connected ? [edge("a", "b")] : []);
    const expanded = reconcile(input, retained);
    assert.deepEqual(coordinates(expanded.nodes.find(n => n.id === "b")!), [0, 0, 0]);
    assertSeparated(expanded);
    assert.equal(expanded.clusters.length, connected ? 1 : 2);
    assert.deepEqual(expanded, reconcile({ ...input, nodes: [...input.nodes].reverse() }, retained));
    assert.deepEqual(expanded, reconcile(input, expanded), "subsequent refresh preserves new and retained coordinates");
  }
});
test("same-scope query expansion and reintroduction place new nodes around all retained points", () => {
  const initial = snapshot([doc("b")]); initial.query = "b";
  let model = reconcile(initial);
  const expanded = snapshot([doc("a"), doc("b"), doc("c")]);
  model = reconcile(expanded, model); assertSeparated(model);
  const retainedA = coordinates(model.nodes.find(n => n.id === "a")!);
  const narrow = snapshot([doc("a")]); narrow.query = "a";
  model = reconcile(narrow, model);
  model = reconcile(expanded, model);
  assert.deepEqual(coordinates(model.nodes.find(n => n.id === "a")!), retainedA);
  assertSeparated(model);
  assert.deepEqual(model, reconcile(expanded, model));
});

test("knowledge stars grow monotonically with perspective and retain bounded overview and closeup sizes", async () => {
  const { nodeScreenSize, spriteScale } = await import("./presentation.ts");
  for (const kind of ["document", "memory"]) {
    const sizes = [10000, 1000, 400, 200, 100, 20, 1].map(depth => nodeScreenSize(kind, depth, 800, 2));
    assert.equal(sizes[0], 10); assert.equal(sizes.at(-1), 140);
    assert.ok(sizes.every((size, index) => size >= 10 && size <= 140 && (!index || size >= sizes[index - 1])));
    assert.ok(sizes[3] > sizes[2], "approaching within the unclamped range visibly enlarges a star");
    const base = nodeScreenSize(kind, 400, 800, 2);
    assert.ok(Math.abs(nodeScreenSize(kind, 400, 1200, 2) - base * 1.5) < 1e-10);
    assert.ok(Math.abs(nodeScreenSize(kind, 400, 800, 3) - base * 1.5) < 1e-10);
    for (const height of [230, 844, 1200]) for (const projection of [1, 2.1445, 3]) {
      const pixels = nodeScreenSize(kind, 400, height, projection);
      assert.ok(Math.abs(spriteScale(pixels, height, projection) * height * projection / 2 - pixels) < 1e-10);
    }
  }
  for (const kind of ["topic", "subject", "area"]) for (const depth of [1, 1000]) assert.equal(nodeScreenSize(kind, depth, 800, 2), 10);
});
test("invalid or behind-camera stars have no visible, ring or picking footprint", async () => {
  const { nodeScreenSize, nodeScreenMetrics } = await import("./presentation.ts");
  for (const kind of ["document", "memory", "topic"]) for (const bad of [0, -1, NaN, Infinity, -Infinity]) {
    for (const inputs of [[bad, 800, 2], [400, bad, 2], [400, 800, bad]]) {
      const size = nodeScreenSize(kind, inputs[0], inputs[1], inputs[2]);
      assert.equal(size, 0);
      assert.deepEqual(nodeScreenMetrics(size, true, true), { body: 0, selection: 0, change: 0, hit: 0, radius: 0 });
    }
    assert.ok(Object.values(nodeScreenMetrics(bad, true, true)).every(value => value === 0));
  }
});
test("enlarged stars share padded ring, hit and label clearance at overview and closeup", async () => {
  const { nodeScreenMetrics, visibleLabels } = await import("./presentation.ts");
  for (const size of [10, 26, 56, 140]) for (const selected of [false, true]) for (const changed of [false, true]) {
    const metrics = nodeScreenMetrics(size, selected, changed);
    assert.equal(metrics.body, size);
    assert.ok(Math.abs(metrics.selection * 112 / 128 - Math.max(27, size + 12)) < 1e-10);
    assert.ok(Math.abs(metrics.change * 112 / 128 - Math.max(36, size + 24)) < 1e-10);
    assert.ok(metrics.change * 112 / 128 > metrics.selection * 116 / 128, "change ring clears the selection stroke");
    assert.ok(metrics.hit >= 36 && metrics.hit >= metrics.radius * 2);
    assert.ok(metrics.radius >= size / 2);
    const node = { id: "star", kind: "document", active: true, x: 500, y: 300, depth: 0, radius: metrics.radius, width: 145, height: 45 };
    const [box] = visibleLabels([node], 1000, 600, selected ? node.id : null, null);
    assert.ok(box);
    assert.ok(box.left >= node.x + metrics.radius + 5, "horizontal label clears the entire footprint");
  }
});
test("summary sizes share count boundaries with collision, hit and label footprints at every depth", async () => {
  const { nodeScreenMetrics, nodeVisualRadius, summaryAppearance } = await import("./presentation.ts");
  assert.deepEqual([0, 6, 9, 10, 29, 30, 49, 50, 99, 100, 120, 800].map(count => summaryAppearance(count).pixels),
    [0, 24, 24, 32, 32, 40, 40, 50, 50, 64, 64, 64]);
  assert.deepEqual([NaN, Infinity, -1].map(summaryAppearance), Array(3).fill({ pixels: 0, samples: 0 }));
  for (const size of [10, 36, 72, 140]) {
    assert.equal(nodeVisualRadius(size, false, false), size / 2);
    for (const count of [6, 10, 30, 50, 120]) for (const [selected, changed] of [[false, false], [true, false], [false, true], [true, true]]) {
      const metrics = nodeScreenMetrics(summaryAppearance(count).pixels, selected, changed);
      assert.equal(nodeVisualRadius(size, selected, changed, count), metrics.radius);
      assert.ok(metrics.hit >= metrics.body, "the actual glyph is fully clickable at any camera depth");
    }
  }
  assert.equal(nodeVisualRadius(0, false, false, 120), 0);
});
test("390px closeups keep the selected name visible above or below its enlarged rings", async () => {
  const { nodeScreenMetrics, nodeScreenSize, visibleLabels } = await import("./presentation.ts");
  const metrics = nodeScreenMetrics(nodeScreenSize("document", 20, 300, 2), true, true);
  const node = { id: "closeup", kind: "document", active: true, x: 195, y: 150, depth: 0, radius: metrics.radius, width: 145, height: 45 };
  const [above] = visibleLabels([node], 390, 300, node.id, null);
  assert.ok(above, "neither horizontal side fits, but the name remains visible");
  assert.ok(above.left >= 0 && above.right <= 390 && above.top >= 0 && above.bottom <= 300);
  assert.ok(above.bottom <= node.y - metrics.radius - 5);
  const nearTop = { ...node, y: 100 };
  const [below] = visibleLabels([nearTop], 390, 300, node.id, null);
  assert.ok(below && below.top >= nearTop.y + metrics.radius + 5 && below.bottom <= 300);
  const overlapping = Array.from({ length: 6 }, (_, i) => ({ ...node, id: `star-${i}` }));
  const boxes = visibleLabels(overlapping, 390, 300, "star-5", "star-4");
  assert.deepEqual(boxes.map(box => box.id), ["star-5", "star-4"]);
  assert.ok(boxes[0].bottom + 6 <= boxes[1].top, "vertical fallback retains collision culling and priority");
  assert.deepEqual(visibleLabels([{ ...node, radius: 0 }, { ...node, id: "invalid", radius: NaN }], 390, 300, null, null), []);
});

test("stellar distance blend is smooth, finite, bounded and leaves footprints unchanged", async () => {
  const { starMotion, nodeScreenMetrics } = await import("./presentation.ts");
  let previous = 0;
  for (let pixels = 0; pixels <= 180; pixels += .25) {
    const motion = starMotion(pixels, 1.7, 13);
    assert.ok(Object.values(motion).every(Number.isFinite));
    assert.ok(motion.detail >= previous && motion.detail >= 0 && motion.detail <= 1);
    assert.ok(motion.detail - previous < .006);
    previous = motion.detail;
  }
  for (const edge of [36, 100]) assert.ok(Math.abs(starMotion(edge + .001, 0, 0).detail - starMotion(edge - .001, 0, 0).detail) < 1e-8);
  for (const bad of [NaN, Infinity, -Infinity]) assert.ok(Object.values(starMotion(bad, bad, bad)).every(Number.isFinite));
  for (const pixels of [26, 56, 100, 140]) {
    const before = nodeScreenMetrics(pixels, true, true);
    for (const time of [0, 2, 7, 84, 923]) starMotion(pixels, 2, time);
    assert.deepEqual(nodeScreenMetrics(pixels, true, true), before);
    if (pixels >= 100) assert.equal(starMotion(pixels, 2, 7).shimmer, 1);
  }
});
test("stellar phases are deterministic, independent and keep shimmer below status separation", async () => {
  const { starPhase, starMotion } = await import("./presentation.ts");
  const ids = Array.from({ length: 500 }, (_, i) => `star-${i}`);
  const phases = ids.map(starPhase);
  assert.deepEqual(phases, [...ids].reverse().map(starPhase).reverse());
  assert.equal(new Set(phases).size, ids.length);
  assert.ok(phases.every(phase => phase >= 0 && phase < Math.PI * 2));
  assert.notEqual(starMotion(26, phases[0], 2).shimmer, starMotion(26, phases[1], 2).shimmer);
  for (const phase of phases) for (const time of [0, 2, 6, 22, 84, 923]) {
    const motion = starMotion(26, phase, time);
    assert.ok(motion.shimmer >= .7 && motion.shimmer <= 1);
    assert.ok(.35 * motion.shimmer < .7, "inactive remains distinctly dimmer than active");
  }
});
test("renderer motion clock freezes both effects and resumes without a suspension jump", async () => {
  const { advanceStarClock, starMotion } = await import("./presentation.ts");
  const clock = { seconds: 0, lastTime: null as number | null };
  assert.equal(advanceStarClock(clock, 100, false), 0);
  assert.equal(advanceStarClock(clock, 200, false), .1);
  const frozenFar = starMotion(26, 1.4, clock.seconds), frozenNear = starMotion(140, 1.4, clock.seconds);
  for (const now of [300, 400, 10000]) {
    advanceStarClock(clock, now, true);
    assert.deepEqual(starMotion(26, 1.4, clock.seconds), frozenFar);
    assert.deepEqual(starMotion(140, 1.4, clock.seconds), frozenNear);
  }
  assert.equal(advanceStarClock(clock, 1000000, false), .1);
  assert.equal(advanceStarClock(clock, 1000100, false), .2);
  assert.equal(advanceStarClock(clock, NaN, false), .2);
  assert.equal(advanceStarClock(clock, 1000050, false), .2);
  // Visibility pause/resume clears only the timestamp, even for a sub-250ms gap.
  clock.lastTime = null;
  assert.equal(advanceStarClock(clock, 1000150, false), .2);
  assert.ok(Math.abs(advanceStarClock(clock, 1000250, false) - .3) < 1e-12);
  clock.seconds = 923.95;
  const before = starMotion(26, 1.4, clock.seconds);
  const continued = advanceStarClock(clock, 1000350, false);
  assert.ok(continued > 924 && continued < 924.1, "elapsed time never wraps at the former common period");
  const after = starMotion(26, 1.4, continued);
  assert.ok(Math.abs(before.shimmer - after.shimmer) < .025);
  assert.ok(Math.abs(before.rotation - after.rotation) < .02);
});
test("summary halo moves only with its held group and settles after release", async () => {
  const { summaryHaloScale } = await import("./presentation.ts");
  const halo = { id: "group", startedAt: 100, releasedAt: null as number | null, releaseScale: 1 };
  assert.equal(summaryHaloScale(halo, "other", 300, false), 1);
  assert.equal(summaryHaloScale(halo, "group", 300, true), 1);
  assert.ok(summaryHaloScale(halo, "group", 300, false) > 1);
  halo.releaseScale = summaryHaloScale(halo, "group", 300, false);
  halo.releasedAt = 300;
  assert.equal(summaryHaloScale(halo, "group", 300, false), halo.releaseScale);
  assert.equal(summaryHaloScale(halo, "group", 750, false), 1);
});
test("summary members drift independently inside the fixed cloud and retain their resting positions", async () => {
  const { summaryGlyphStars, summaryGlyphPosition } = await import("./summary-glyph.ts");
  const stars = summaryGlyphStars("group", Array.from({ length: 120 }, (_, index) => ({ id: `member-${index}`, kind: "document" })));
  const baseline = structuredClone(stars);
  for (const star of stars) {
    assert.deepEqual(summaryGlyphPosition(star, 2, 0), { x: star.x, y: star.y });
    for (let frame = 0; frame < 600; frame++) {
      const at = summaryGlyphPosition(star, frame / 30, 1);
      assert.ok(Math.hypot(at.x, at.y) <= .97 + 1e-12, "each star stays inside the summary disc");
      assert.ok(Math.hypot(at.x - star.x, at.y - star.y) <= .1, "drift stays small at every group size");
    }
    assert.notDeepEqual(summaryGlyphPosition(star, 2, 1), summaryGlyphPosition(star, 2.5, 1));
  }
  const displacements = stars.map(star => { const at = summaryGlyphPosition(star, 2, 1); return `${(at.x - star.x).toFixed(4)}:${(at.y - star.y).toFixed(4)}`; });
  assert.equal(new Set(displacements).size, stars.length, "members cannot share one rigid group translation");
  assert.deepEqual(stars, baseline, "drawing motion never rewrites resting coordinates or appearance");
});

test("each ID retains a varied bounded rotation period and visible axial tilt across refresh and reorder", async () => {
  const { starPhase, starMotion } = await import("./presentation.ts");
  const ids = Array.from({ length: 500 }, (_, i) => `stellar-${i}`);
  const traits = (id: string) => {
    const { period, tilt } = starMotion(140, starPhase(id), 0);
    return { id, period, tilt };
  };
  const samples = ids.map(traits);
  assert.deepEqual(samples, [...ids].reverse().map(traits).reverse());
  assert.ok(samples.every(({ period, tilt }) => period >= 32 && period <= 56 && Math.abs(tilt) <= 32 * Math.PI / 180));
  assert.ok(Math.max(...samples.map(s => s.period)) - Math.min(...samples.map(s => s.period)) > 22);
  assert.ok(samples.some(s => s.tilt < -.5) && samples.some(s => s.tilt > .5));
  const turn = Math.PI * 2;
  const angularDistance = (a: number, b: number) => Math.abs(Math.atan2(Math.sin(a - b), Math.cos(a - b)));
  for (const { id, period, tilt } of samples) {
    const phase = starPhase(id), start = starMotion(140, phase, 3), later = starMotion(140, phase, 5);
    const movement = angularDistance(start.rotation, later.rotation);
    assert.ok(movement >= 2 * turn / 56 - 1e-12 && movement <= 2 * turn / 32 + 1e-12);
    assert.ok(angularDistance(start.rotation, starMotion(140, phase, 3 + period).rotation) < 1e-12);
    for (const time of [-1, 0, 923.99, 924.01, 1e9]) {
      const near = starMotion(140, phase, time), far = starMotion(26, phase, time);
      assert.equal(near.period, period); assert.equal(near.tilt, tilt);
      assert.equal(far.period, period); assert.equal(far.tilt, tilt);
      assert.ok(near.rotation >= 0 && near.rotation < turn);
    }
    assert.ok(angularDistance(starMotion(140, phase, 923.99).rotation, starMotion(140, phase, 924.01).rotation) < .004);
  }
});

test("distant twinkle repeats smoothly every seven seconds with one restrained crest", async () => {
  const { starMotion } = await import("./presentation.ts");
  for (const phase of [0, 1.7, 4.1]) {
    const values = Array.from({ length: 701 }, (_, i) => starMotion(26, phase, i / 100).shimmer);
    assert.ok(Math.max(...values) - Math.min(...values) > .29);
    assert.ok(values.filter(value => value > .94).length / values.length < .2);
    for (let i = 1; i < values.length; i++) assert.ok(Math.abs(values[i] - values[i - 1]) < .004);
    for (const time of [0, 1.2, 4, 17]) assert.ok(Math.abs(starMotion(26, phase, time).shimmer - starMotion(26, phase, time + 7).shimmer) < 1e-12);
    for (const time of [0, 1.2, 17, 923, 925]) {
      const priorPulse = (.5 + .5 * Math.sin(time * Math.PI * 2 / 7 + phase)) ** 4;
      assert.ok(Math.abs(starMotion(26, phase, time).shimmer - (1 - .3 * (1 - priorPulse))) < 1e-12, "far twinkle retains the previous curve and phase");
    }
  }
});

test("current, stale, failed and missing Context documents retain their honest states", () => {
  const current = { ...doc("context"), source_kind: "context" };
  const stale = { ...current, current: false };
  assert.equal(active(current), true); assert.equal(active(stale), false);
  assert.equal(stateName(stale), "갱신 대기");
  assert.equal(stateName({ ...stale, status: "failed" }), "출처 확인 실패");
  assert.equal(stateName({ ...stale, status: "missing", present: false }), "원문 부재");
  const model = reconcile(snapshot([stale, doc("other")], [edge("context", "other")]));
  assert.equal(model.links.length, 1); assert.equal(model.clusters.length, 2);
});

test("each stretched link waits for its own slack before drawing a neighbor, including past evidence", () => {
  for (const current of [true, false]) {
    const model = reconcile(snapshot([doc("a"), doc("b"), doc("other")], [edge("a", "b", current)]));
    const positions = new Positions(); positions.install(model);
    const [a, b, other] = model.nodes, baseline = model.nodes.map(n => ({ ...n }));
    positions.begin(a.id, 1);
    positions.move(a.id, { x: 8, y: 0, z: 0 }); positions.advance(0, false); positions.advance(16, false);
    assert.deepEqual(b, baseline[1], "a short pull leaves the linked endpoint in place");
    positions.move(a.id, { x: 200, y: 0, z: 0 }); positions.advance(32, false);
    assert.ok(b.x > baseline[1].x, "a stretched link draws its endpoint while held");
    const duringDrag = b.x;
    positions.release(32, false); assert.equal(b.x, duringDrag, "release does not teleport");
    positions.advance(160, false);
    assert.ok(b.x > duringDrag, "follower heads toward the compact slot after release");
    positions.advance(2100, false); assert.equal(b.x, 224); assert.equal(a.x, 200);
    assert.deepEqual(other, baseline[2]);
  }
});
test("screen clearance keeps a dragged node fixed and separates coincident neighbors deterministically", () => {
  const discs = [{ id: "held", x: 0, y: 0, radius: 30 }, { id: "a", x: 0, y: 0, radius: 18 }, { id: "b", x: 0, y: 0, radius: 18 }, { id: "c", x: 10, y: 0, radius: 48 }];
  const run = (values: typeof discs) => separateDiscs(values, "held", new Set(["a"]));
  const placed = run(discs);
  assert.deepEqual(placed, run([...discs].reverse()));
  assert.deepEqual(placed.get("held"), { x: 0, y: 0 });
  for (const [index, a] of discs.entries()) for (const b of discs.slice(index + 1)) {
    const first = placed.get(a.id)!, second = placed.get(b.id)!;
    assert.ok(Math.hypot(first.x - second.x, first.y - second.y) >= a.radius + b.radius + 1 - 1e-4);
  }
});
test("screen clearance lets an unrelated star yield to a fixed constellation spoke", () => {
  const discs = [
    { id: "hub", x: 0, y: 0, radius: 20 },
    { id: "spoke", x: 80, y: 0, radius: 20 },
    { id: "unrelated", x: 80, y: 0, radius: 20 },
  ];
  const placed = separateDiscs(discs, "hub", new Set(["spoke"]), 1, new Set(["hub", "spoke"]));
  assert.deepEqual(placed.get("hub"), { x: 0, y: 0 });
  assert.deepEqual(placed.get("spoke"), { x: 80, y: 0 });
  assert.ok(Math.hypot(placed.get("unrelated")!.x - 80, placed.get("unrelated")!.y) >= 41);
});
test("a neighboring node moves only at the visible one-pixel clearance boundary", () => {
  const model = reconcile(snapshot([doc("held"), doc("other")], []));
  for (const node of model.nodes) {
    const x = node.id === "held" ? 0 : 75;
    Object.assign(node, { x, y: 0, z: 0, fx: x, fy: 0, fz: 0 });
  }
  const positions = new Positions(); positions.install(model);
  positions.begin("held", 1, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: model.nodes.map(node => node.id), radius: () => 20,
    project: value => ({ x: value.x, y: value.y, depth: 1 }) });
  positions.move("held", { x: 33, y: 0, z: 0 });
  const other = model.nodes.find(node => node.id === "other")!;
  assert.equal(other.x, 75, "the visible clearance boundary has not been reached");
  positions.advance(0, false); positions.advance(16, false);
  assert.equal(other.x, 75, "the neighboring node waits outside the visible clearance range");
  positions.move("held", { x: 34, y: 0, z: 0 });
  assert.equal(other.x, 75, "touching the one-pixel boundary does not push the neighbor");
  assert.equal(positions.collisionReaction("other", performance.now(), false), null, "proximity alone has no visual reaction");
  positions.move("held", { x: 34.1, y: 0, z: 0 });
  assert.equal(other.x, 75, "the pointer cannot teleport its neighbor");
  positions.advance(32, false);
  assert.ok(other.x > 75 && positions.collisionReaction("other", performance.now(), false), "fractional contact also reacts");
  positions.move("held", { x: 35, y: 0, z: 0 });
  positions.advance(48, false);
  assert.ok(other.x > 75 && other.x < 77, "the neighbor eases after crossing the boundary");
  positions.release(48, false); positions.advance(2048, false);
  assert.ok(other.x - 35 >= 41, "the 1px hard boundary applies at rest");
  const now = performance.now(), reaction = positions.collisionReaction("other", now, false);
  assert.ok(reaction && reaction.x > 0 && reaction.glow > 0, "contact gives the displaced star a directed response");
  assert.equal(positions.collisionReaction("other", now, true), null, "reduced motion suppresses the response");
  const frozen = { x: other.x, y: other.y, z: other.z };
  assert.equal(positions.collisionReaction("other", now + 500, false), null, "the response expires");
  assert.deepEqual({ x: other.x, y: other.y, z: other.z }, frozen, "reading the visual recoil never changes geometry");
});
test("an ordinary star yields at its visible one-pixel boundary, regardless of its hit target", async () => {
  const { nodeScreenMetrics } = await import("./presentation.ts");
  const metrics = nodeScreenMetrics(26, false, false);
  assert.equal(metrics.radius, 13);
  assert.equal(metrics.hit, 36);
  const model = reconcile(snapshot([doc("held"), doc("other")], []));
  for (const node of model.nodes) {
    const x = node.id === "held" ? 0 : 45;
    Object.assign(node, { x, y: 0, z: 0, fx: x, fy: 0, fz: 0 });
  }
  const positions = new Positions(); positions.install(model);
  positions.begin("held", 1, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: model.nodes.map(node => node.id), radius: () => metrics.radius,
    project: value => ({ x: value.x, y: value.y, depth: 1 }) });
  const other = model.nodes.find(node => node.id === "other")!;
  positions.move("held", { x: 18, y: 0, z: 0 });
  positions.advance(0, false); positions.advance(16, false);
  assert.equal(other.x, 45, "a 27px center distance is exactly the 1px boundary");
  positions.move("held", { x: 19, y: 0, z: 0 });
  positions.advance(32, false);
  assert.ok(other.x > 45 && other.x < 47, "the star moves only after crossing that boundary");
  positions.release(32, false); positions.advance(2032, false);
  assert.ok(other.x - 19 >= 27, "the visible 26px stars retain a 1px final gap");
});
test("a small drag waits for contact and cancellation restores the neighboring node", async () => {
  const { nodeScreenMetrics } = await import("./presentation.ts");
  const radius = nodeScreenMetrics(26, false, false).radius;
  const model = reconcile(snapshot([doc("held"), doc("other")], []));
  for (const node of model.nodes) {
    const x = node.id === "held" ? 0 : 31;
    Object.assign(node, { x, y: 0, z: 0, fx: x, fy: 0, fz: 0 });
  }
  const positions = new Positions(); positions.install(model);
  positions.begin("held", 1, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: model.nodes.map(node => node.id), radius: () => radius,
    project: value => ({ x: value.x, y: value.y, depth: 1 }) });
  positions.move("held", { x: 1, y: 0, z: 0 });
  const other = model.nodes.find(node => node.id === "other")!;
  assert.equal(other.x, 31, "the one-pixel drag remains outside visible contact");
  positions.advance(0, false); positions.advance(16, false);
  assert.equal(other.x, 31, "no early motion runs on animation frames");
  positions.move("held", { x: 5, y: 0, z: 0 });
  positions.advance(32, false);
  assert.ok(other.x > 31 && other.x < 33, "contact displaces the neighbor even below the click threshold");
  assert.ok(positions.collisionReaction("other", performance.now(), false), "contact reacts before the click threshold");
  positions.release(32, false);
  assert.equal(model.nodes.find(node => node.id === "held")!.x, 0, "a sub-threshold gesture remains a click");
  assert.equal(other.x, 31, "click jitter does not leave the neighbor displaced");
  assert.equal(positions.collisionReaction("other", performance.now(), false), null, "canceling the click clears its visual reaction");
  positions.begin("held", 1, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: model.nodes.map(node => node.id), radius: () => radius,
    project: value => ({ x: value.x, y: value.y, depth: 1 }) });
  positions.move("held", { x: 5, y: 0, z: 0 }); positions.advance(0, false); positions.advance(16, false);
  positions.cancel();
  assert.equal(model.nodes.find(node => node.id === "held")!.x, 0);
  assert.equal(other.x, 31, "pointer cancellation also restores click jitter");
});
test("released linked stars rest at their visible one-pixel clearance", async () => {
  const { nodeScreenMetrics } = await import("./presentation.ts");
  const radius = nodeScreenMetrics(26, false, false).radius;
  const model = reconcile(snapshot([doc("held"), doc("other")], [edge("held", "other")]));
  const positions = new Positions(); positions.install(model);
  positions.begin("held", 1, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: model.nodes.map(node => node.id), radius: () => radius,
    project: value => ({ x: value.x, y: value.y, depth: 1 }) });
  positions.move("held", { x: 100, y: 0, z: 0 }); positions.release(0, true);
  const held = model.nodes.find(node => node.id === "held")!, other = model.nodes.find(node => node.id === "other")!;
  assert.ok(Math.abs(Math.hypot(held.x - other.x, held.y - other.y) - 27) < 1e-6);
});
test("one enlarged ring does not spread ordinary linked stars on release", () => {
  const model = reconcile(snapshot([doc("held"), doc("normal"), doc("ring")], [edge("held", "normal"), edge("normal", "ring")]));
  const positions = new Positions(); positions.install(model);
  positions.begin("held", 1, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: model.nodes.map(node => node.id), radius: node => node.id === "ring" ? 29 : 13,
    project: value => ({ x: value.x, y: value.y, depth: 1 }) });
  positions.move("held", { x: 1000, y: 0, z: 0 }); positions.release(0, true);
  const byId = new Map(model.nodes.map(node => [node.id, node]));
  const distance = (a: string, b: string) => Math.hypot(byId.get(a)!.x - byId.get(b)!.x, byId.get(a)!.y - byId.get(b)!.y);
  assert.ok(Math.abs(distance("held", "normal") - 27) < 1e-6, "ordinary pair keeps a one-pixel visible gap");
  assert.ok(distance("normal", "ring") >= 43 - 1e-4, "the enlarged ring gets only its own clearance");
});
test("visible linked and unlinked nodes flow during drag and clear after release", () => {
  for (const reduced of [false, true]) {
    const model = reconcile(snapshot([doc("held"), doc("linked"), doc("other")], [edge("held", "linked")]));
    for (const node of model.nodes) {
      const x = node.id === "held" ? 0 : node.id === "linked" ? 160 : 80;
      Object.assign(node, { x, y: 0, z: 0, fx: x, fy: 0, fz: 0 });
    }
    const positions = new Positions(); positions.install(model);
    const plane = { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
      visible: model.nodes.map(node => node.id), radius: () => 20,
      project: (value: { x: number; y: number; z: number }) => ({ x: value.x, y: value.y, depth: 1 }) };
    const clear = () => {
      for (const [index, a] of model.nodes.entries()) for (const b of model.nodes.slice(index + 1)) {
        assert.ok(Math.hypot(a.x - b.x, a.y - b.y) >= 41 - 1e-4, `${a.id} and ${b.id} remain distinct`);
      }
    };
    positions.begin("held", 1, plane);
    positions.move("held", { x: 80, y: 0, z: 0 });
    assert.equal(model.nodes.find(node => node.id === "held")!.x, 80);
    assert.equal(model.nodes.find(node => node.id === "other")!.x, 80, "a held star can pass through before its neighbors flow");
    for (const time of [0, 16, 32]) positions.advance(time, reduced);
    assert.notEqual(model.nodes.find(node => node.id === "other")!.x, 80, "unlinked neighbor yields during the drag");
    const atRelease = model.nodes.map(node => ({ ...node }));
    positions.release(32, reduced);
    if (!reduced) assert.deepEqual(model.nodes, atRelease, "release does not jump any node or edge endpoint");
    for (const time of [48, 80, 160, 500, 1000, 2100]) positions.advance(time, reduced);
    clear();
    const frozen = model.nodes.map(node => ({ ...node }));
    positions.cancel(); positions.advance(3000, reduced); assert.deepEqual(model.nodes, frozen);
  }
});
test("clearance follows perspective, zoom, and different visible footprints", () => {
  const model = reconcile(snapshot([doc("held"), doc("near"), doc("far")], []));
  const initial = new Map([["held", { x: 0, z: 0 }], ["near", { x: 5000, z: 50 }], ["far", { x: 7200, z: 20 }]]);
  for (const node of model.nodes) {
    const value = initial.get(node.id)!;
    Object.assign(node, { x: value.x, y: 0, z: value.z, fx: value.x, fy: 0, fz: value.z });
  }
  const positions = new Positions(); positions.install(model);
  const radii = new Map([["held", 42], ["near", 30], ["far", 18]]);
  let zoom = 1;
  const project = (value: { x: number; y: number; z: number }) => {
    const depth = 100 - value.z;
    return { x: value.x / depth * zoom, y: value.y / depth * zoom, depth };
  };
  const separated = () => {
    for (const [index, a] of model.nodes.entries()) for (const b of model.nodes.slice(index + 1)) {
      const left = project(a), right = project(b);
      assert.ok(Math.hypot(left.x - right.x, left.y - right.y) >= radii.get(a.id)! + radii.get(b.id)! + 1 - 1e-4);
    }
  };
  positions.begin("held", 100, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: model.nodes.map(node => node.id), radius: node => radii.get(node.id)!, worldPerPixel: depth => depth / zoom,
    project, viewKey: () => String(zoom) });
  positions.move("held", { x: 7000, y: 0, z: 0 }); positions.advance(0, false); positions.advance(16, false);
  positions.release(16, false);
  zoom = .5;
  for (const time of [32, 80, 240, 1000, 2100]) positions.advance(time, false);
  separated();
});
test("a camera turn during settling retargets clearance in the current screen plane", () => {
  const model = reconcile(snapshot([doc("held"), doc("other")], []));
  fixPosition(model.nodes[0], { x: 0, y: 0, z: 0 });
  fixPosition(model.nodes[1], { x: 75, y: 0, z: 0 });
  const positions = new Positions(); positions.install(model);
  let turned = false, zoom = 1;
  const project = (value: { x: number; y: number; z: number }) => ({
    x: (turned ? -value.y : value.x) * zoom, y: (turned ? value.x : value.y) * zoom, depth: 1,
  });
  positions.begin("held", 1, {
    right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: ["held", "other"], radius: () => 20, project, worldPerPixel: () => 1 / zoom,
    viewKey: () => `${turned}:${zoom}`,
    basis: () => turned ? { right: { x: 0, y: -1, z: 0 }, up: { x: 1, y: 0, z: 0 } } :
      { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 } },
  });
  positions.move("held", { x: 34.1, y: 0, z: 0 });
  positions.release(0, false);
  positions.advance(16, false);
  turned = true; zoom = .5;
  for (const time of [32, 80, 240, 1000, 2100]) positions.advance(time, false);
  const a = project(model.nodes[0]), b = project(model.nodes[1]);
  assert.ok(Math.hypot(a.x - b.x, a.y - b.y) >= 41 - 1e-4,
    "the finished stars remain one screen pixel apart after the camera turns and zooms");
});
test("clicks stay unchanged; a drag returning near its origin still rearranges on release", () => {
  const model = reconcile(snapshot([doc("a"), doc("b")], [edge("a", "b")]));
  const positions = new Positions(); positions.install(model); const before = model.nodes.map(n => ({ ...n }));
  positions.begin("a", 1); positions.release(0, false); positions.advance(1500, false); assert.deepEqual(model.nodes, before);
  positions.begin("a", 1); positions.move("a", { x: 8, y: 0, z: 0 }); positions.move("a", { x: 1, y: 0, z: 0 });
  positions.release(0, true); assert.equal(model.nodes[1].x, 25);
});
test("adjacency maps stars and chains to equal nearest-neighbor lattice edges", () => {
  for (const shape of ["star", "chain"]) {
    const nodes = Array.from({ length: shape === "star" ? 7 : 20 }, (_, i) => doc(`n${String(i).padStart(2, "0")}`));
    const links = nodes.slice(1).map((n, i) => edge(shape === "star" ? nodes[0].id : nodes[i].id, n.id));
    const model = reconcile(snapshot(nodes, links));
    for (const node of model.nodes) node.cluster = "frozen-test-group";
    const positions = new Positions(); positions.install(model); positions.begin(nodes[0].id, 1);
    positions.move(nodes[0].id, { x: 200, y: 0, z: 0 }); positions.release(0, true);
    const placed = new Map(model.nodes.map(n => [n.id, n]));
    for (const link of links) {
      const a = placed.get(link.source)!, b = placed.get(link.target)!;
      assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z) - 24) < 1e-9, `${shape} edge has uniform spacing`);
    }
    assert.deepEqual(model.links, links, "semantic edges are unchanged");
  }
});
test("seven historical spokes compact without overlap even though seven exact 24px spokes are impossible", () => {
  for (const picked of ["center", "record0"]) {
    const records = Array.from({ length: 7 }, (_, index) => ({ ...memory(`record${index}`), supported: false }));
    const links: GraphLink[] = records.map(record => ({ ...edge("center", record.id, false), kind: "evidence" }));
    const model = reconcile(snapshot([doc("center"), ...records], links));
    const positions = new Positions(); positions.install(model);
    const center = model.nodes.find(node => node.id === "center")!;
    const before = Math.max(...model.nodes.filter(node => node !== center).map(node => Math.hypot(node.x - center.x, node.y - center.y)));
    assert.ok(before > 48, "automatic layout starts wider than a two-ring constellation");
    positions.begin(picked, 1); positions.move(picked, { x: 1000, y: 0, z: 0 }); positions.release(0, true);
    const after = Math.max(...model.nodes.filter(node => node !== center).map(node => Math.hypot(node.x - center.x, node.y - center.y)));
    assert.ok(after <= 48 + 1e-9, "every brown edge shrinks into two 24px rings");
    for (const [index, node] of model.nodes.entries()) for (const other of model.nodes.slice(index + 1)) {
      assert.ok(Math.hypot(node.x - other.x, node.y - other.y) >= 24 - 1e-9, "nodes stay distinct");
    }
  }
});
test("a crowded hub keeps its constellation together when its center or a spoke is dragged", () => {
  const leaves = Array.from({ length: 64 }, (_, index) => doc(`spoke-${String(index).padStart(2, "0")}`));
  const makeModel = () => reconcile(snapshot([doc("hub"), ...leaves, doc("unrelated")],
    leaves.map(leaf => ({ ...edge("hub", leaf.id, false), kind: "evidence" as const }))));
  const dragPlane = (model: ReturnType<typeof makeModel>) => ({
    right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: model.nodes.map(node => node.id), radius: () => 20, worldPerPixel: () => 1,
    project: (value: { x: number; y: number; z: number }) => ({ x: value.x, y: value.y, depth: 1 }),
  });
  for (const held of ["hub", leaves[0].id]) for (const reduced of [false, true]) {
    const model = makeModel();
    const positions = new Positions(); positions.install(model, true);
    // This case checks group translation; keep the unrelated star outside its drag path.
    const unrelated = model.nodes.find(node => node.id === "unrelated")!;
    unrelated.x = unrelated.fx = 1000;
    const before = new Map(model.nodes.map(node => [node.id, { x: node.x, y: node.y, z: node.z }]));
    const start = before.get(held)!;
    positions.begin(held, 1, dragPlane(model));
    positions.move(held, { ...start, x: start.x + 200 });
    positions.advance(0, false); positions.advance(16, false);
    positions.release(16, reduced);
    if (!reduced && held === "hub") {
      const hub = model.nodes.find(node => node.id === "hub")!;
      const widest = () => Math.max(...leaves.map(({ id }) => {
        const leaf = model.nodes.find(node => node.id === id)!;
        return Math.hypot(leaf.x - hub.x, leaf.y - hub.y);
      }));
      const releaseWidth = widest();
      for (const time of [32, 80, 240, 1000, 2100]) {
        positions.advance(time, false);
        assert.ok(widest() <= releaseWidth + 1, "the normal release does not fling spokes farther out");
      }
    } else if (!reduced) for (const time of [32, 80, 240, 1000, 2100]) positions.advance(time, false);
    for (const node of model.nodes) {
      const original = before.get(node.id)!;
      const translation = node.id === "unrelated" ? 0 : 200;
      assert.ok(Math.abs(node.x - original.x - translation) < 1e-8 && Math.abs(node.y - original.y) < 1e-8,
        `${held} drag preserves ${node.id}'s place in the constellation`);
    }
  }
  for (const reduced of [false, true]) {
    const model = makeModel(), positions = new Positions(); positions.install(model, true);
    const hub = model.nodes.find(node => node.id === "hub")!, unrelated = model.nodes.find(node => node.id === "unrelated")!;
    const leaf = model.nodes.find(node => node.id === leaves[0].id)!;
    const originalOffset = { x: leaf.x - hub.x, y: leaf.y - hub.y };
    const destination = { x: unrelated.x, y: unrelated.y };
    const separated = () => assert.ok(Math.hypot(hub.x - unrelated.x, hub.y - unrelated.y) >= 41 - 1e-6,
      "an unrelated star still yields at the one-pixel boundary");
    positions.begin("hub", 1, dragPlane(model)); positions.move("hub", { ...destination, z: 0 });
    positions.release(0, reduced);
    if (!reduced) for (const time of [16, 80, 240, 2100]) positions.advance(time, false);
    separated();
    assert.ok(Math.abs(leaf.x - hub.x - originalOffset.x) < 1e-8 &&
      Math.abs(leaf.y - hub.y - originalOffset.y) < 1e-8,
    "unrelated-star clearance does not pull connected spokes out of their constellation");
  }
  for (const reduced of [false, true]) {
    const model = makeModel(), positions = new Positions(); positions.install(model, true);
    const hub = model.nodes.find(node => node.id === "hub")!, unrelated = model.nodes.find(node => node.id === "unrelated")!;
    const spoke = model.nodes.filter(node => node.id.startsWith("spoke-")).sort((a, b) =>
      Math.hypot(b.x - hub.x, b.y - hub.y) - Math.hypot(a.x - hub.x, a.y - hub.y))[0];
    const offset = { x: spoke.x - hub.x, y: spoke.y - hub.y };
    const destination = { x: unrelated.x - offset.x, y: unrelated.y - offset.y, z: hub.z };
    positions.begin("hub", 1, dragPlane(model));
    positions.move("hub", destination);
    positions.advance(0, false); positions.advance(16, false);
    positions.release(16, reduced);
    if (!reduced) for (const time of [80, 240, 1000, 2100]) positions.advance(time, false);
    assert.ok(Math.abs(spoke.x - hub.x - offset.x) < 1e-8 && Math.abs(spoke.y - hub.y - offset.y) < 1e-8,
      "a dense constellation keeps its spoke offset after release");
    assert.ok(Math.hypot(spoke.x - unrelated.x, spoke.y - unrelated.y) >= 41 - 1e-6,
      "an unrelated star yields to the moving spoke, even when the hub does not touch it");
  }
});
test("tension travels through a chain only after each successive edge stretches", () => {
  const ids = ["a", "b", "c"];
  const model = reconcile(snapshot(ids.map(doc), [edge("a", "b"), edge("b", "c")]));
  for (const [index, node] of model.nodes.entries()) Object.assign(node, { x: index * 24, y: 0, z: 0, fx: index * 24, fy: 0, fz: 0 });
  const positions = new Positions(); positions.install(model); positions.begin("a", 1);
  positions.move("a", { x: -70, y: 0, z: 0 }); positions.advance(0, false); positions.advance(16, false);
  assert.ok(model.nodes[1].x < 24, "direct neighbor follows after its edge exceeds 48px");
  assert.equal(model.nodes[2].x, 48, "second neighbor still waits for its own edge");
  positions.advance(300, false);
  assert.ok(model.nodes[2].x < 48, "second neighbor follows once that edge stretches");
  positions.release(300, true);
  for (const link of model.links) {
    const a = model.nodes.find(node => node.id === link.source)!, b = model.nodes.find(node => node.id === link.target)!;
    assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z) - 24) < 1e-9);
  }
});
test("tension uses each edge's visible starting length, including short and depth-tilted edges", () => {
  for (const baseline of [{ x: 12, z: 0 }, { x: 0, z: 100 }]) {
    const model = reconcile(snapshot([doc("a"), doc("b")], [edge("a", "b")]));
    Object.assign(model.nodes[0], { x: 0, y: 0, z: 0 });
    Object.assign(model.nodes[1], { x: baseline.x, y: 0, z: baseline.z });
    const positions = new Positions(); positions.install(model); positions.begin("a", 1);
    positions.move("a", { x: -20, y: 0, z: 0 }); positions.advance(0, false); positions.advance(16, false);
    assert.equal(model.nodes[1].x, baseline.x, "edge remains slack before its visible length gains 24px");
    positions.move("a", { x: -30, y: 0, z: 0 }); positions.advance(32, false);
    assert.ok(model.nodes[1].x < baseline.x, "edge follows after its visible length gains 24px");
  }
  const model = reconcile(snapshot([doc("a"), doc("b")], [edge("a", "b")]));
  Object.assign(model.nodes[0], { x: 0, y: 0, z: 0 });
  Object.assign(model.nodes[1], { x: 12, y: 0, z: 0 });
  const positions = new Positions(); positions.install(model);
  positions.begin("a", 1, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    project: value => ({ x: value.x * 2, y: value.y, depth: 1 }) });
  positions.move("a", { x: -20, y: 0, z: 0 }); positions.advance(0, false); positions.advance(16, false);
  assert.ok(model.nodes[1].x < 12, "the camera's projected pixel scale determines the threshold");
});
test("perspective drag reaches the screen-space edge limit across unequal depths", () => {
  const model = reconcile(snapshot([doc("a"), doc("b")], [edge("a", "b")]));
  Object.assign(model.nodes[0], { x: 0, y: 0, z: 10 });
  Object.assign(model.nodes[1], { x: 100, y: 0, z: 100 });
  const positions = new Positions(); positions.install(model);
  const project = (value: { x: number; y: number; z: number }) => ({ x: 100 * value.x / value.z, y: 100 * value.y / value.z, depth: value.z });
  positions.begin("a", 1, { right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24, project });
  positions.move("a", { x: -50, y: 0, z: 10 }); positions.advance(0, true);
  const a = project(model.nodes[0]), b = project(model.nodes[1]);
  assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y) - 124) < 1e-9);
});
test("release ripples into 24px slots without a jump and remains frame-rate independent", () => {
  const make = () => {
    const model = reconcile(snapshot([doc("a"), doc("b")], [edge("a", "b")]));
    const positions = new Positions(); positions.install(model); positions.begin("a", 1);
    positions.move("a", { x: 200, y: 20, z: 30 }); positions.advance(0, false);
    return { positions, model };
  };
  const { positions, model } = make(), single = make();
  const held = { ...model.nodes[0] };
  for (let t = 16; t <= 160; t += 16) positions.advance(t, false);
  single.positions.advance(160, false);
  assert.ok(model.nodes[1].x < 224, "held neighbor remains outside its final slot");
  assert.ok(Math.abs(model.nodes[1].x - single.model.nodes[1].x) < 1e-9);
  assert.deepEqual(model.nodes[0], held, "held node never participates in follower motion");
  const atRelease = model.nodes[1].x;
  positions.release(160, false); assert.equal(model.nodes[1].x, atRelease);
  single.positions.release(160, false);
  const samples: number[] = [];
  for (let t = 176; t <= 1000; t += 16) { positions.advance(t, false); samples.push(model.nodes[1].x); }
  single.positions.advance(1000, false);
  assert.ok(Math.abs(model.nodes[1].x - single.model.nodes[1].x) < 1e-8);
  assert.ok(samples.some(x => x > 224), "small damped overshoot gives the release its ripple");
  assert.ok(Math.max(...samples) < 224 + (224 - atRelease) * .08, "ripple remains restrained");
  positions.advance(2160, false);
  assert.deepEqual([model.nodes[1].x, model.nodes[1].y, model.nodes[1].z], [224, 20, 30]);
  const stable = model.nodes.map(n => ({ ...n })); positions.advance(10000, false); assert.deepEqual(model.nodes, stable);
});
test("an 800-node path closes into a compact constellation with 24px links", () => {
  const ids = Array.from({ length: 800 }, (_, index) => `n${String(index).padStart(3, "0")}`);
  const adjacency = new Map(ids.map((id, index) => [id, new Set([...(index ? [ids[index - 1]] : []), ...(index < ids.length - 1 ? [ids[index + 1]] : [])])]));
  for (const root of [ids[0], ids[400]]) {
    const slots = compactSlots(root, ids.filter(id => id !== root), adjacency);
    assert.equal(slots?.size, 799);
    const placed = new Map([[root, { x: 0, y: 0 }], ...slots!]);
    const values = [...placed.values()];
    assert.ok((Math.max(...values.map(p => p.x)) - Math.min(...values.map(p => p.x))) * 24 < 1000);
    assert.ok((Math.max(...values.map(p => p.y)) - Math.min(...values.map(p => p.y))) * 24 < 1000);
    for (let index = 1; index < ids.length; index++) {
      const a = placed.get(ids[index - 1])!, b = placed.get(ids[index])!;
      assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y) - 1) < 1e-9);
    }
  }
});
test("new drag recalculates 24 screen pixels after zoom while normal zoom scales placed nodes", () => {
  const model = reconcile(snapshot([doc("a"), doc("b")], [edge("a", "b")]));
  const positions = new Positions(); positions.install(model);
  positions.begin("a", .25); positions.move("a", { x: 200, y: 0, z: 0 }); positions.release(0, true);
  assert.equal(Math.abs(model.nodes[1].x - model.nodes[0].x) / .25, 24);
  assert.equal(Math.abs(model.nodes[1].x - model.nodes[0].x) / .5, 12, "subsequent zoom changes graph spacing normally");
  positions.begin("a", .5); positions.move("a", { x: 220, y: 0, z: 0 }); positions.release(0, true);
  assert.equal(Math.abs(model.nodes[1].x - model.nodes[0].x) / .5, 24);
});
test("compact targets grow in screen-plane rings with minimum spacing at different zooms", () => {
  for (const scale of [.25, 3]) {
    const nodes = Array.from({ length: 40 }, (_, i) => doc(`n${String(i).padStart(2, "0")}`));
    const model = reconcile(snapshot(nodes));
    for (const node of model.nodes) node.cluster = "frozen-test-group";
    const positions = new Positions(); positions.install(model);
    positions.begin("n00", scale, { right: { x: 0, y: 0, z: 1 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24 });
    positions.move("n00", { x: 1000, y: 0, z: 0 }); positions.release(0, true);
    const held = model.nodes[0];
    for (const [i, node] of model.nodes.entries()) {
      assert.equal(node.x, held.x, "rotated camera plane is respected");
      assert.ok(Math.hypot(node.y, node.z) <= 96 * scale + 1e-9, "40 nodes fit in four compact rings");
      for (const other of model.nodes.slice(i + 1)) assert.ok(Math.hypot(node.y - other.y, node.z - other.z) >= 24 * scale - 1e-9);
    }
    const stable = model.nodes.map(n => ({ ...n }));
    positions.release(0, true); positions.advance(100, false); assert.deepEqual(model.nodes, stable, "reduced motion has no tail");
  }
});
test("cancellation freezes an active flow and invalid input cannot poison coordinates", () => {
  const model = reconcile(snapshot([doc("a"), doc("b")], [edge("a", "b")]));
  const positions = new Positions(); positions.install(model); positions.begin("a", 1);
  positions.move("a", { x: 200, y: 0, z: 0 }); positions.release(0, false); positions.advance(50, false);
  const frozen = model.nodes.map(n => ({ ...n }));
  positions.move("a", { x: Infinity, y: 0, z: 0 }); assert.deepEqual(model.nodes, frozen);
  positions.cancel(); positions.advance(10000, false); positions.move("a", { x: 0, y: 0, z: 0 });
  assert.deepEqual(model.nodes, frozen); assert.equal(positions.dragging, false);
});
test("display filtering retains hidden positions; reset and fresh query sessions restore automatic layout", () => {
  const auto = reconcile(snapshot([doc("a"), memory("b")], [edge("a", "b")]));
  const baseline = auto.nodes.map(n => ({ ...n }));
  const positions = new Positions(); positions.install(auto);
  const visible = visibleGraph(auto, { kind: "document", state: "all", cluster: null });
  assert.equal(visible.nodes.length, 1);
  positions.begin("a", 1); positions.move("a", { x: 200, y: 0, z: 0 }); positions.release(0, true);
  assert.notEqual(auto.nodes[1].x, baseline[1].x, "hidden cluster member follows");
  const refreshed = reconcile(snapshot([doc("a"), memory("b")], [edge("a", "b")])), newSession = new Positions();
  positions.install(refreshed); assert.equal(refreshed.nodes[0].x, 200);
  assert.equal(positions.dragging, false);
  positions.reset(); assert.deepEqual(refreshed.nodes, baseline);
  const otherQuery = reconcile(snapshot([doc("a"), memory("b")], [edge("a", "b")]));
  newSession.install(otherQuery); assert.deepEqual(otherQuery.nodes, baseline);
});

test("overlapping hit targets prefer the nearest screen center across depth, zoom and aspect", async () => {
  const { screenPickDistance, nodeScreenMetrics } = await import("./presentation.ts");
  const { PerspectiveCamera, Vector3 } = await import("three");
  for (const [width, height] of [[1000, 500], [400, 800]]) for (const zoom of [1, 3]) {
    const camera = new PerspectiveCamera(60, width / height, .1, 2000); camera.zoom = zoom; camera.updateProjectionMatrix(); camera.updateMatrixWorld();
    const centerAt = (pixels: number, depth: number) => new Vector3(pixels * 2 * depth / (width * camera.projectionMatrix.elements[0]), 0, -depth).project(camera);
    const nearerCamera = centerAt(0, 50), fartherCamera = centerAt(24, 500);
    for (const cursorPixels of [11, 12, 13]) {
      const cursor = { x: cursorPixels * 2 / width, y: 0 };
      const a = screenPickDistance(nearerCamera, cursor, width, height), b = screenPickDistance(fartherCamera, cursor, width, height);
      assert.ok(a <= 18 && b <= 18, "both original 36px hit targets overlap");
      if (cursorPixels < 12) assert.ok(a < b);
      else if (cursorPixels > 12) assert.ok(b < a, "farther-depth node wins near its center");
      else assert.ok(Math.abs(a - b) < 1e-9, "midpoint is an equal-distance boundary");
    }
    assert.equal(nodeScreenMetrics(26, false, false).hit, 36, "picking footprint is retained");
    assert.equal(screenPickDistance({ x: 0, y: 24 / height, z: 0 }, { x: 0, y: 0 }, width, height), 12, "vertical distance uses viewport height");
  }
  assert.equal(screenPickDistance({ x: 0, y: 0, z: 2 }, { x: 0, y: 0 }, 100, 100), Infinity);
  assert.equal(screenPickDistance({ x: NaN, y: 0, z: 0 }, { x: 0, y: 0 }, 100, 100), Infinity);
  assert.equal(screenPickDistance({ x: 0, y: 0, z: 0 }, { x: 0, y: 0 }, 0, 100), Infinity);
});

function satelliteFixture() {
  const orphan = { ...memory("z_record"), supported: false };
  const nodes = [doc("a0"), doc("a1"), doc("a2"), doc("b0"), doc("b1"), doc("b2"), orphan, memory("zz_unlinked")];
  const links: GraphLink[] = [edge("a0", "a1"), edge("a0", "a2"), edge("a1", "a2"), edge("b0", "b1"), edge("b0", "b2"), edge("b1", "b2"),
    { source: orphan.id, target: "a0", kind: "evidence", current: false }, { source: orphan.id, target: "b0", kind: "evidence", current: false }];
  return snapshot(nodes, links);
}
test("the first live view packs current and brown edges without changing semantic groups", () => {
  const source = satelliteFixture(), model = reconcile(source);
  const groups = new Map(model.nodes.map(node => [node.id, node.cluster]));
  const positions = new Positions(); positions.install(model, true);
  assert.equal(positions.hasCompactInitialLayout, true);
  const byId = new Map(model.nodes.map(node => [node.id, node]));
  for (const link of model.links) {
    const a = byId.get(link.source)!, b = byId.get(link.target)!;
    assert.ok(Math.hypot(a.x - b.x, a.y - b.y) <= 31 + 1e-9, `${link.source} → ${link.target} starts compact`);
  }
  for (const [index, a] of model.nodes.entries()) for (const b of model.nodes.slice(index + 1)) {
    assert.ok(Math.hypot(a.x - b.x, a.y - b.y) >= 31 - 1e-9, `${a.id} and ${b.id} do not overlap`);
  }
  assert.deepEqual(new Map(model.nodes.map(node => [node.id, node.cluster])), groups);
  const original = model.nodes.map(node => ({ ...node }));
  const previous = { ...model, nodes: original };
  const refreshed = reconcile(source, previous); positions.install(refreshed, true);
  assert.deepEqual(refreshed.nodes, original, "an unchanged refresh retains the first layout");
  const reversed = reconcile(snapshot([...source.nodes].reverse(), [...source.links].reverse()));
  new Positions().install(reversed, true);
  assert.deepEqual(reversed.nodes, original, "first-view layout ignores source order");
});
test("a personal graph with hundreds of unrelated documents opens as a constellation", () => {
  const nodes = Array.from({ length: 536 }, (_, index) => ({ ...doc(`personal-${String(index).padStart(3, "0")}`), scope: "personal" as const }));
  const links = Array.from({ length: 128 }, (_, index) => edge(nodes[0].id, nodes[index + 1].id));
  const source = snapshot(nodes, links); source.scope = "personal";
  const model = reconcile(source), positions = new Positions(); positions.install(model, true);
  assert.equal(positions.hasCompactInitialLayout, true);
  const overview = constellationView(model.nodes, model.links, null, null);
  const core = overview.cores[0], hub = model.nodes.find(node => node.id === core.hub)!;
  assert.equal(core.count, 129);
  assert.ok([...core.members].every(id => {
    const node = model.nodes.find(value => value.id === id)!;
    return node.x === hub.x && node.y === hub.y && node.z === hub.z;
  }), "collapsed spokes share the visible nucleus until expanded");
  const isolated = model.nodes.filter(node => Number(node.id.slice(-3)) > 128);
  assert.ok(new Set(isolated.map(node => node.y.toFixed(2))).size > 300, "unrelated stars do not form shelf rows");
  for (const [index, node] of overview.nodes.entries()) for (const other of overview.nodes.slice(index + 1)) {
    assert.ok(Math.hypot(node.x - other.x, node.y - other.y) >= 31 - 1e-8, "visible initial stars do not overlap");
  }
  assert.ok(Math.min(...isolated.map(node => Math.hypot(node.x - hub.x, node.y - hub.y))) < 40,
    "only visible stars reserve space around the collapsed group");
  const reordered = reconcile({ ...source, nodes: [...nodes].reverse(), links: [...links].reverse() });
  new Positions().install(reordered, true);
  assert.deepEqual(reordered.nodes.map(node => [node.id, node.x, node.y, node.z]), model.nodes.map(node => [node.id, node.x, node.y, node.z]));
});
test("distant isolated stars form several bounded nuclei without hiding any relationship", () => {
  const nodes = Array.from({ length: 536 }, (_, index) => ({ ...doc(`personal-${String(index).padStart(3, "0")}`), scope: "personal" as const }));
  const links = Array.from({ length: 128 }, (_, index) => edge(nodes[0].id, nodes[index + 1].id));
  const source = { ...snapshot(nodes, links), scope: "personal" as const };
  const model = reconcile(source), positions = new Positions(); positions.install(model, true);
  const semantic = constellationView(model.nodes, model.links, null, null);
  const original = new Map(model.nodes.map(node => [node.id, coordinates(node)]));
  const distant = nucleusView(semantic.nodes, model.links, 1, null);
  assert.ok(distant.groups.size >= 5 && distant.groups.size <= 60, "the overview has several compact nuclei");
  assert.ok(distant.nodes.length < semantic.nodes.length / 2, "a distant view actually reduces visual density");
  assert.equal(semantic.counts.get(nodes[0].id), 129, "the linked core remains a separate summary");
  const shown = new Set(distant.nodes.map(node => node.id));
  for (const link of links) {
    if (semantic.nodes.some(node => node.id === link.source)) assert.ok(shown.has(link.source));
    if (semantic.nodes.some(node => node.id === link.target)) assert.ok(shown.has(link.target));
  }
  for (const [id, group] of distant.groups) {
    assert.ok(group.members.length >= 6 && group.members.length <= 60);
    assert.ok(shown.has(id) && group.members.includes(id), "the nucleus is a real member");
    assert.ok(group.radius > 0 && Number.isFinite(group.radius));
    assert.ok(group.members.every(member => !links.some(link => link.source === member || link.target === member)), "linked nodes never disappear");
  }
  assert.deepEqual(new Map(model.nodes.map(node => [node.id, coordinates(node)])), original, "LOD does not move knowledge");
  const opened = [...distant.groups.values()][0], pinned = new Set(opened.members);
  for (const level of [1, 2, 3]) {
    const expanded = nucleusView(semantic.nodes, model.links, level, null, pinned);
    const visibleIds = new Set(expanded.nodes.map(node => node.id));
    assert.ok(opened.members.every(id => visibleIds.has(id)), "a clicked group's real members remain visible while zooming");
    assert.ok(expanded.groups.size > 0, "opening one group leaves unrelated groups summarized");
    assert.ok(![...expanded.groups.values()].some(group => group.members.some(id => pinned.has(id))), "a changed representative cannot hide an opened member");
  }
  assert.deepEqual(nucleusView(semantic.nodes, model.links, 0, null).nodes, semantic.nodes, "approaching restores every visible star");
  const focused = nucleusView(semantic.nodes, model.links, 1, nodes[300].id);
  assert.ok(focused.nodes.some(node => node.id === nodes[300].id), "focus keeps the selected star visible");
  assert.ok(focused.nodes.length < semantic.nodes.length / 2, "selection keeps unrelated distant nuclei summarized");
  assert.ok(![...focused.groups.values()].some(group => group.members.includes(nodes[300].id)), "focus does not hide the selected star inside a nucleus");
  const reversed = nucleusView([...semantic.nodes].reverse(), [...model.links].reverse(), 1, null);
  assert.deepEqual([...reversed.groups].sort(), [...distant.groups].sort(), "source order cannot change nuclei");
  const draggedMember = model.nodes.find(node => node.id === opened.members[0])!;
  positions.begin(draggedMember.id, 1);
  positions.move(draggedMember.id, { x: draggedMember.x + 45, y: draggedMember.y, z: draggedMember.z });
  positions.release(100, true);
  assert.ok(nucleusView(semantic.nodes, model.links, 1, null, pinned).nodes.some(node => node.id === draggedMember.id), "a dragged member stays visible after regrouping");
  assert.ok(nucleusView(semantic.nodes, model.links, 1, null).groups.size > 0, "closing restores compact groups");
});
test("a spatial summary moves its exact members together and stays grouped across cell boundaries", () => {
  const nodes = Array.from({ length: 64 }, (_, index) => doc(`free-${String(index).padStart(2, "0")}`));
  const model = reconcile(snapshot(nodes)), positions = new Positions(); positions.install(model, true);
  const original = nucleusView(model.nodes, model.links, 1, null);
  const group = [...original.groups.values()][0];
  assert.ok(group && original.groups.size > 1);
  const before = new Map(model.nodes.map(node => [node.id, coordinates(node)]));
  const representative = model.nodes.find(node => node.id === group.representative)!;
  const start = { x: representative.x, y: representative.y, z: representative.z };
  positions.beginGroup(group.representative, group.members, 1);
  positions.move(group.representative, { ...start, x: start.x + 500 });
  assert.equal(positions.dragging, true);
  assert.equal(positions.release(100, false), true);
  for (const node of model.nodes) {
    const [x, y, z] = before.get(node.id)!;
    assert.deepEqual(coordinates(node), group.members.includes(node.id) ? [x + 500, y, z] : [x, y, z]);
  }
  const second = [...original.groups.values()].find(value => value.representative !== group.representative)!;
  const other = model.nodes.find(node => node.id === second.representative)!;
  positions.beginGroup(other.id, second.members, 1);
  positions.move(other.id, { x: other.x - 500, y: other.y, z: other.z });
  assert.equal(positions.release(200, true), true);
  const locks = new Map([[group.representative, group.members], [second.representative, second.members]]);
  const locked = nucleusView(model.nodes, model.links, 1, null, new Set(), locks);
  assert.deepEqual(locked.groups.get(group.representative)?.members, group.members);
  assert.deepEqual(locked.groups.get(second.representative)?.members, second.members, "multiple moved groups remain separate");
  assert.equal(locked.nodes.filter(node => group.members.includes(node.id)).length, 1);
  const expanded = nucleusView(model.nodes, model.links, 1, null, new Set(group.members), new Map([[group.representative, group.members]]));
  assert.equal(expanded.nodes.filter(node => group.members.includes(node.id)).length, group.members.length);
  const linked = nucleusView(model.nodes, [edge(group.members[0], group.members[1])], 1, null, new Set(), new Map([[group.representative, group.members]]));
  assert.notDeepEqual(linked.groups.get(group.representative)?.members, group.members, "a changed relationship invalidates the old summary");
  assert.ok(group.members.slice(0, 2).every(id => linked.nodes.some(node => node.id === id)), "newly linked members remain visible");
  assert.equal(positions.settling, false, "dropping an unlinked group does not repack its members");
});
test("a spatial summary click-sized gesture restores its members, while a return drag remains a drag", () => {
  const model = reconcile(snapshot(Array.from({ length: 30 }, (_, index) => doc(`free-${index}`))));
  const positions = new Positions(); positions.install(model, true);
  const group = [...nucleusView(model.nodes, model.links, 2, null).groups.values()][0];
  const before = new Map(model.nodes.map(node => [node.id, coordinates(node)]));
  const star = model.nodes.find(node => node.id === group.representative)!;
  const start = { x: star.x, y: star.y, z: star.z };
  positions.beginGroup(star.id, group.members, 1);
  positions.move(star.id, { ...start, x: start.x + 3 });
  assert.equal(positions.release(10, false), false);
  assert.deepEqual(new Map(model.nodes.map(node => [node.id, coordinates(node)])), before);
  positions.beginGroup(star.id, group.members, 1);
  positions.move(star.id, { ...start, x: start.x + 12 });
  positions.move(star.id, start);
  assert.equal(positions.release(20, false), true);
  assert.deepEqual(new Map(model.nodes.map(node => [node.id, coordinates(node)])), before);
});
test("dropping a spatial group on another star clears the one-pixel boundary as a rigid cohort", () => {
  const model = reconcile(snapshot(Array.from({ length: 64 }, (_, index) => doc(`free-${String(index).padStart(2, "0")}`))));
  const positions = new Positions(); positions.install(model, true);
  const groups = [...nucleusView(model.nodes, model.links, 1, null).groups.values()];
  assert.ok(groups.length > 1);
  const moving = groups[0], target = groups[1];
  const held = model.nodes.find(node => node.id === moving.representative)!;
  const blocker = model.nodes.find(node => node.id === target.representative)!;
  const start = { x: held.x, y: held.y, z: held.z };
  const original = new Map(moving.members.map(id => {
    const node = model.nodes.find(value => value.id === id)!;
    return [id, { x: node.x - start.x, y: node.y - start.y, z: node.z - start.z }] as const;
  }));
  const visible = groups.map(group => group.representative);
  const cohorts = new Map(groups.map(group => [group.representative, group.members] as const));
  const targetMember = model.nodes.find(node => node.id === target.members.find(id => id !== target.representative))!;
  const targetOffset = { x: targetMember.x - blocker.x, y: targetMember.y - blocker.y };
  positions.beginGroup(held.id, moving.members, 1, {
    right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible, cohorts, radius: () => 18, worldPerPixel: () => 1, isVisible: () => true,
    project: node => ({ x: node.x, y: node.y, depth: 1 }),
  });
  positions.move(held.id, { x: blocker.x, y: blocker.y, z: start.z });
  positions.advance(0, false); positions.advance(16, false);
  assert.notEqual(blocker.x, held.x, "the nearby group starts to yield while held");
  const atRelease = new Map(model.nodes.map(node => [node.id, { x: node.x, y: node.y, z: node.z }]));
  assert.equal(positions.release(16, false), true);
  assert.deepEqual(new Map(model.nodes.map(node => [node.id, { x: node.x, y: node.y, z: node.z }])), atRelease,
    "dropping a group does not teleport either cohort");
  for (const time of [32, 80, 240, 1000, 2016]) positions.advance(time, false);
  for (const id of visible.filter(id => id !== held.id)) {
    const node = model.nodes.find(value => value.id === id)!;
    assert.ok(Math.hypot(held.x - node.x, held.y - node.y) >= 37 - 1e-4, `representative ${id} remains pickable`);
  }
  for (const id of moving.members) {
    const node = model.nodes.find(value => value.id === id)!, offset = original.get(id)!;
    assert.ok(Math.hypot(node.x - held.x - offset.x, node.y - held.y - offset.y, node.z - held.z - offset.z) < 1e-7, `member ${id} keeps its group offset`);
  }
  assert.ok(Math.abs(targetMember.x - blocker.x - targetOffset.x) < 1e-7 &&
    Math.abs(targetMember.y - blocker.y - targetOffset.y) < 1e-7, "a pushed group's hidden member stays attached");
});
test("a hidden member cannot be dropped on a visible star even when the representative is clear", () => {
  const model = reconcile(snapshot([doc("held"), doc("far-member"), doc("blocker")]));
  fixPosition(model.nodes.find(node => node.id === "held")!, { x: 0, y: 0, z: 0 });
  fixPosition(model.nodes.find(node => node.id === "far-member")!, { x: 60, y: 0, z: 0 });
  fixPosition(model.nodes.find(node => node.id === "blocker")!, { x: 200, y: 0, z: 0 });
  const positions = new Positions(); positions.install(model);
  positions.beginGroup("held", ["held", "far-member"], 1, {
    right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, spacingPixels: 24,
    visible: ["held", "blocker"], radius: () => 18, worldPerPixel: () => 1,
    isVisible: () => true, project: node => ({ x: node.x, y: node.y, depth: 1 }),
  });
  positions.move("held", { x: 140, y: 0, z: 0 });
  assert.equal(positions.release(100, false), true);
  positions.advance(2100, false);
  const held = model.nodes.find(node => node.id === "held")!;
  const member = model.nodes.find(node => node.id === "far-member")!;
  const blocker = model.nodes.find(node => node.id === "blocker")!;
  assert.ok(Math.hypot(held.x - blocker.x, held.y - blocker.y) >= 37 - 1e-5);
  assert.ok(Math.hypot(member.x - blocker.x, member.y - blocker.y) >= 37 - 1e-5);
  assert.equal(member.x - held.x, 60, "the hidden member stays attached to the group");
});
test("group locks expire when graph structure or the automatic layout changes", () => {
  const model = reconcile(snapshot(Array.from({ length: 24 }, (_, index) => doc(`free-${index}`))));
  const positions = new Positions(); positions.install(model, true);
  const initial = positions.groupEpoch;
  positions.install(reconcile(snapshot([...model.nodes].reverse().map(node => doc(node.id)))), true);
  assert.equal(positions.groupEpoch, initial, "response order does not invalidate an unchanged group");
  positions.reset();
  assert.ok(positions.groupEpoch > initial, "reset invalidates moved groups");
  const afterReset = positions.groupEpoch;
  positions.install(reconcile(snapshot([...model.nodes.map(node => doc(node.id)), doc("new-star")])), true);
  assert.ok(positions.groupEpoch > afterReset, "new membership invalidates moved groups");
});
test("historical links and classification markers stay visible across visual LOD", () => {
  const nodes = [doc("past-a"), doc("past-b"), ...Array.from({ length: 30 }, (_, index) => doc(`free-${index}`)), { ...doc("marker"), kind: "topic" as const }];
  const links = [{ ...edge("past-a", "past-b", false), kind: "evidence" as const }];
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  const distant = nucleusView(model.nodes, model.links, 3, null), ids = new Set(distant.nodes.map(node => node.id));
  assert.ok(ids.has("past-a") && ids.has("past-b") && ids.has("marker"));
  assert.ok(distant.groups.size > 0, "unlinked knowledge can still be summarized");
  const filtered = nucleusView(model.nodes.filter(node => node.id.startsWith("free-")), model.links, 3, null);
  assert.ok(filtered.nodes.every(node => node.id.startsWith("free-")), "filters cannot reintroduce excluded items");
});
test("camera LOD uses hysteresis instead of toggling at one zoom boundary", () => {
  assert.equal(nucleusLevel(15, 0), 1);
  assert.equal(nucleusLevel(22, 1), 1);
  assert.equal(nucleusLevel(27, 1), 0);
  assert.equal(nucleusLevel(9, 0), 2);
  assert.equal(nucleusLevel(12, 2), 2);
  assert.equal(nucleusLevel(14, 2), 1);
  assert.equal(nucleusLevel(4, 0), 3);
  assert.equal(nucleusLevel(7, 3), 2);
});
test("offscreen nuclei cannot consume the visible label budget after an orbit", () => {
  const counts = new Map(Array.from({ length: 12 }, (_, index) => [`group-${index}`, 30 - index] as const));
  const projected = Array.from({ length: 12 }, (_, index) => ({ id: `group-${index}`, x: index < 9 ? 2000 : 20 + (index - 9) * 40, y: 50, depth: 0 }));
  assert.deepEqual([...nucleusLabelIds(projected, counts, 300, 100)], ["group-9", "group-10", "group-11"]);
  projected[9].depth = 2;
  assert.deepEqual([...nucleusLabelIds(projected, counts, 300, 100)], ["group-10", "group-11"]);
});
test("an overview nucleus expands and closes without keeping an empty halo or losing a dragged offset", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 119 }, (_, index) => doc(`spoke-${index}`)),
    ...Array.from({ length: 80 }, (_, index) => doc(`outside-${index}`))];
  const links = nodes.slice(1, 120).map(node => edge("hub", node.id));
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  const byId = new Map(model.nodes.map(node => [node.id, node]));
  const hub = byId.get("hub")!, spoke = byId.get("spoke-0")!, outside = nodes.slice(120).map(node => byId.get(node.id)!);
  const near = () => Math.min(...outside.map(node => Math.hypot(node.x - hub.x, node.y - hub.y)));
  const compactDistance = near();
  assert.ok(compactDistance < 70);
  assert.equal(Math.hypot(spoke.x - hub.x, spoke.y - hub.y), 0);
  positions.showCore("hub", 0, false);
  const targetHub = positions.layoutTarget("hub")!, targetSpoke = positions.layoutTarget("spoke-0")!;
  assert.ok(Math.hypot(targetSpoke.x - targetHub.x, targetSpoke.y - targetHub.y) >= 31 - 1e-8,
    "camera framing can use the expanded destination before nodes finish moving");
  positions.advance(325, false);
  assert.ok(Math.hypot(spoke.x - hub.x, spoke.y - hub.y) > 0, "spokes flow outward during expansion");
  positions.advance(650, false);
  assert.equal(positions.layoutMoving, false);
  const expandedHubX = hub.x;
  const expanded = constellationView(model.nodes, model.links, null, "hub").nodes;
  for (const [index, node] of expanded.entries()) for (const other of expanded.slice(index + 1)) {
    assert.ok(Math.hypot(node.x - other.x, node.y - other.y) >= 31 - 1e-8, "expanded stars do not overlap");
  }
  assert.ok(near() > compactDistance + 50, "expanding makes room for the real constellation");
  positions.showCore(null, 700, false);
  positions.advance(1025, false); positions.advance(1350, false);
  assert.ok(Math.abs(near() - compactDistance) < 1e-7);
  assert.ok(Math.hypot(spoke.x - hub.x, spoke.y - hub.y) < 1e-7);
  const originalX = hub.x;
  positions.begin("hub", 1);
  positions.move("hub", { x: originalX + 90, y: hub.y, z: hub.z });
  positions.advance(1400, false); positions.advance(1416, false); positions.release(1416, true);
  positions.showCore("hub", 1500, true);
  assert.ok(Math.abs(hub.x - expandedHubX - 90) < 1e-7, "expansion retains the nucleus drag");
  assert.ok(Math.hypot(spoke.x - hub.x, spoke.y - hub.y) >= 31 - 1e-7,
    "hidden members expand around the dragged nucleus");
  positions.showCore(null, 1600, true);
  assert.ok(Math.hypot(spoke.x - hub.x, spoke.y - hub.y) < 1e-7);
});
test("interrupting a constellation transition completes its layout before dragging", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 25 }, (_, index) => doc(`spoke-${index}`))];
  const links = nodes.slice(1).map(node => edge("hub", node.id));
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  const hub = model.nodes.find(node => node.id === "hub")!, spoke = model.nodes.find(node => node.id === "spoke-0")!;
  positions.showCore("hub", 0, false); positions.advance(325, false);
  positions.begin("hub", 1);
  assert.equal(positions.layoutMoving, false);
  assert.ok(Math.hypot(spoke.x - hub.x, spoke.y - hub.y) >= 31 - 1e-8, "a gesture cannot freeze the expanded layout halfway");
  positions.cancel();
  positions.showCore(null, 700, false); positions.advance(800, false); positions.cancel();
  assert.equal(positions.layoutMoving, false);
  assert.equal(Math.hypot(spoke.x - hub.x, spoke.y - hub.y), 0, "cancel completes the collapsed layout");
});
test("filter-exposed spokes receive distinct positions when their nucleus is absent", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 25 }, (_, index) => doc(`spoke-${index}`))];
  const model = reconcile(snapshot(nodes, nodes.slice(1).map(node => edge("hub", node.id))));
  const positions = new Positions(); positions.install(model, true);
  const hub = model.nodes.find(node => node.id === "hub")!, spokes = model.nodes.filter(node => node.id !== "hub");
  assert.ok(spokes.every(node => node.x === hub.x && node.y === hub.y));
  positions.showCore("*", 0, true);
  for (const [index, node] of spokes.entries()) for (const other of spokes.slice(index + 1)) {
    assert.ok(Math.hypot(node.x - other.x, node.y - other.y) >= 31 - 1e-8);
  }
});
test("a high-degree component has one stable overview core and expands without losing links", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 119 }, (_, index) => doc(`spoke-${index}`)), doc("independent")];
  const links = nodes.slice(1, 120).map(node => edge("hub", node.id));
  const first = denseConstellationCores(nodes, links);
  const reordered = denseConstellationCores([...nodes].reverse(), [...links].reverse());
  assert.deepEqual(first, reordered);
  assert.equal(first.length, 1);
  assert.equal(first[0].hub, "hub");
  assert.equal(first[0].count, 120);
  assert.equal(first[0].members.has("independent"), false);
  const model = reconcile(snapshot(nodes, links)), positioned = model.nodes;
  const overview = constellationView(positioned, links, null, null);
  assert.deepEqual(overview.nodes.map(node => node.id), ["hub", "independent"]);
  assert.equal(overview.links.length, 0);
  assert.equal(overview.counts.get("hub"), 120);
  for (const expanded of [constellationView(positioned, links, null, "hub"), constellationView(positioned, links, "spoke-0", null)]) {
    assert.equal(expanded.nodes.length, 121);
    assert.equal(expanded.links.length, 119, "expansion restores every relationship");
    assert.equal(expanded.counts.size, 0);
  }
  assert.deepEqual(constellationView(positioned, links, null, null).nodes.map(node => node.id), overview.nodes.map(node => node.id),
    "closing an expansion returns to the same core");
  const positions = new Positions(); positions.install(model, true);
  const lengths = links.map(link => {
    const source = positioned.find(node => node.id === link.source)!, target = positioned.find(node => node.id === link.target)!;
    return Math.hypot(source.x - target.x, source.y - target.y, source.z - target.z);
  });
  const hub = positioned.find(node => node.id === "hub")!;
  positions.begin(hub.id, 1);
  positions.move(hub.id, { x: hub.x + 200, y: hub.y + 140, z: hub.z });
  positions.advance(0, false); positions.advance(16, false); positions.release(16, true);
  for (const [index, link] of links.entries()) {
    const source = positioned.find(node => node.id === link.source)!, target = positioned.find(node => node.id === link.target)!;
    assert.ok(Math.abs(Math.hypot(source.x - target.x, source.y - target.y, source.z - target.z) - lengths[index]) < 1e-5,
      "moving a collapsed core keeps its hidden members and link geometry together");
  }
  assert.equal(constellationView(positioned, links, null, null).nodes.length, 2, "drag and release retain the core summary");
  assert.equal(denseConstellationCores(nodes.slice(0, 19), links.slice(0, 18)).length, 0);
  const pastOnly = links.map(link => ({ ...link, current: false }));
  assert.equal(denseConstellationCores(nodes, pastOnly).length, 0, "historical evidence cannot create a summary core");
  assert.equal(constellationView(positioned, pastOnly, null, null).links.length, 119, "historical edges remain inspectable");
  assert.equal(denseConstellationCores([{ ...doc("hub"), current: false }, ...nodes.slice(1)], links).length, 0,
    "a nonusable node cannot make unrelated records into a core");
  const externalEvidence = [...links, edge("spoke-0", "independent", false)];
  assert.equal(constellationView(positioned, externalEvidence, null, null).counts.get("hub"), 120,
    "one outside historical edge does not expose the entire core");
  assert.equal(constellationView(positioned, externalEvidence, null, null).links.length, 2,
    "the outside historical edge and its member's hub edge keep their actual endpoints visible");
  const hubEvidence = [...links, edge("hub", "independent", false)];
  assert.equal(constellationView(positioned, hubEvidence, null, null).counts.get("hub"), 120);
  assert.equal(constellationView(positioned, hubEvidence, null, null).links.length, 1,
    "a relationship attached directly to the visible core remains visible");
});
test("a large connected core opens in compact pages without losing access to any member or relation", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 119 }, (_, index) => doc(`spoke-${String(index).padStart(3, "0")}`)), doc("outside")];
  const links = nodes.slice(1, 120).map(node => edge("hub", node.id));
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  const seenNodes = new Set<string>(), seenLinks = new Set<string>();
  const first = constellationView(model.nodes, links, null, "hub", 0);
  assert.equal(first.disclosure?.pages, 10);
  assert.equal(first.nodes.length, 14, "the first view has the hub, twelve spokes and the unrelated star");
  for (let page = 0; page < first.disclosure!.pages; page++) {
    const view = constellationView(model.nodes, links, null, "hub", page);
    assert.ok(view.nodes.length <= 14 && view.links.length <= 12, "one page cannot form a 119-spoke fan");
    for (const node of view.nodes) seenNodes.add(node.id);
    for (const link of view.links) seenLinks.add(`${link.source}|${link.target}`);
  }
  assert.deepEqual(seenNodes, new Set(nodes.map(node => node.id)), "every member remains reachable through the pages");
  assert.deepEqual(seenLinks, new Set(links.map(link => `${link.source}|${link.target}`)), "every relation remains reachable");
  positions.showCore("hub", 0, true, 0, first.disclosure?.visible);
  const hub = model.nodes.find(node => node.id === "hub")!;
  const outside = model.nodes.find(node => node.id === "outside")!;
  const outsideAt = { x: outside.x, y: outside.y, z: outside.z };
  assert.ok(first.nodes.filter(node => node.id.startsWith("spoke-")).every(node => Math.hypot(node.x - hub.x, node.y - hub.y) < 120),
    "the shown page stays around its hub instead of inheriting the full fan radius");
  const hidden = model.nodes.find(node => node.id === "spoke-090")!;
  assert.ok(Math.hypot(hidden.x - hub.x, hidden.y - hub.y) < 1e-7, "unshown members remain in the nucleus");
  const focused = constellationView(model.nodes, links, "spoke-090", null, 0);
  assert.ok(focused.nodes.some(node => node.id === hidden.id), "a direct selection opens the selected member's page");
  positions.showCore("hub", 100, true, focused.disclosure?.index, focused.disclosure?.visible);
  assert.ok(Math.hypot(hidden.x - hub.x, hidden.y - hub.y) > 0, "switching pages reveals the selected real star");
  assert.ok(Math.hypot(outside.x - outsideAt.x, outside.y - outsideAt.y, outside.z - outsideAt.z) < 1e-8,
    "paging leaves unrelated components in place, including the shorter final page");
  const last = constellationView(model.nodes, links, null, "hub", 9);
  positions.showCore("hub", 150, true, last.disclosure?.index, last.disclosure?.visible);
  assert.ok(Math.hypot(outside.x - outsideAt.x, outside.y - outsideAt.y, outside.z - outsideAt.z) < 1e-8);
  const filteredNodes = model.nodes.filter(node => node.id === "hub" || node.id === "outside" || node.id >= "spoke-030");
  const filteredIds = new Set(filteredNodes.map(node => node.id));
  const filtered = constellationView(filteredNodes, links.filter(link => filteredIds.has(link.source) && filteredIds.has(link.target)), null, "hub", 0);
  positions.showCore("hub", 175, true, filtered.disclosure?.index, filtered.disclosure?.visible);
  assert.ok(filtered.nodes.filter(node => node.id.startsWith("spoke-")).every(node => Math.hypot(node.x - hub.x, node.y - hub.y) > 0),
    "a filtered page positions its actual visible members, not the full source's page members");
  positions.reset();
  assert.ok(filtered.nodes.filter(node => node.id.startsWith("spoke-")).every(node => Math.hypot(node.x - hub.x, node.y - hub.y) > 0),
    "reset keeps the active filtered page layout");
  assert.equal(constellationView(model.nodes, links, null, "hub").links.length, 119,
    "explicit full expansion still exposes every relation at once");
});
test("outside relations keep their real endpoints while the rest of a large core stays compact", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 119 }, (_, index) => doc(`spoke-${String(index).padStart(3, "0")}`)), doc("outside")];
  const links = [...nodes.slice(1, 120).map(node => edge("hub", node.id)), edge("spoke-090", "outside", false)];
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  const overview = constellationView(model.nodes, model.links, null, null);
  assert.equal(overview.counts.get("hub"), 120);
  assert.deepEqual(new Set(overview.nodes.map(node => node.id)), new Set(["hub", "spoke-090", "outside"]));
  assert.ok(overview.links.some(link => link.source === "spoke-090" && link.target === "outside"));
  const hub = model.nodes.find(node => node.id === "hub")!, pinned = model.nodes.find(node => node.id === "spoke-090")!;
  assert.ok(Math.hypot(pinned.x - hub.x, pinned.y - hub.y) > 0, "the outside relation's endpoint stays visible and separated");
  const first = constellationView(model.nodes, model.links, null, "hub", 0);
  assert.equal(first.disclosure?.pages, 10);
  assert.equal(first.nodes.length, 15, "the exposed core shows twelve ordinary members and one pinned member");
  assert.ok(first.links.some(link => link.source === "spoke-090" && link.target === "outside"));
  positions.showCore("hub", 0, true, first.disclosure?.index, first.disclosure?.visible);
  assert.ok(Math.hypot(pinned.x - hub.x, pinned.y - hub.y) > 0);
  const pinnedAt = { x: pinned.x, y: pinned.y, z: pinned.z };
  const next = constellationView(model.nodes, model.links, null, "hub", 1);
  positions.showCore("hub", 10, true, next.disclosure?.index, next.disclosure?.visible);
  assert.deepEqual({ x: pinned.x, y: pinned.y, z: pinned.z }, pinnedAt,
    "the endpoint of an outside relation stays put when ordinary members change pages");
});
test("reset after a filtered page and a same-route refresh drops obsolete layout scope", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 50 }, (_, index) => doc(`spoke-${String(index).padStart(3, "0")}`))];
  const links = nodes.slice(1).map(node => edge("hub", node.id));
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  const filteredNodes = model.nodes.filter(node => node.id !== "spoke-045");
  const filteredLinks = links.filter(link => link.target !== "spoke-045");
  const page = constellationView(filteredNodes, filteredLinks, null, "hub", 3);
  positions.showCore("hub", 0, true, page.disclosure?.index, page.disclosure?.visible, { nodes: filteredNodes, links: filteredLinks });
  const updatedNodes = nodes.filter(node => node.id !== "spoke-040" && node.id !== "spoke-045");
  const updatedLinks = links.filter(link => updatedNodes.some(node => node.id === link.target));
  const updated = reconcile(snapshot(updatedNodes, updatedLinks), model);
  positions.install(updated, true);
  assert.doesNotThrow(() => positions.reset());
  assert.ok(updated.nodes.every(node => [node.x, node.y, node.z].every(Number.isFinite)));
});
test("a filtered core still opens compactly when its hub changes or another hub disappears", () => {
  const nodes = [doc("a"), doc("b"), ...Array.from({ length: 40 }, (_, index) => doc(`a-${index}`)),
    ...Array.from({ length: 38 }, (_, index) => doc(`b-${index}`))];
  const links = [edge("a", "b"), ...nodes.filter(node => node.id.startsWith("a-")).map(node => edge("a", node.id)),
    ...nodes.filter(node => node.id.startsWith("b-")).map(node => edge("b", node.id))];
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  assert.equal(denseConstellationCores(model.nodes, links)[0]?.hub, "a");
  const filteredNodes = model.nodes.filter(node => !/^a-[0-9]$/.test(node.id));
  const ids = new Set(filteredNodes.map(node => node.id));
  const filteredLinks = links.filter(link => ids.has(link.source) && ids.has(link.target));
  const view = constellationView(filteredNodes, filteredLinks, null, "b", 0);
  assert.equal(view.disclosure?.hub, "b");
  positions.showCore("b", 0, true, view.disclosure?.index, view.disclosure?.visible, { nodes: filteredNodes, links: filteredLinks });
  const hub = model.nodes.find(node => node.id === "b")!;
  assert.ok(view.nodes.filter(node => node.id !== "b").every(node => Math.hypot(node.x - hub.x, node.y - hub.y) > 0),
    "filtered visible stars must not stack at the old source hub");

  const otherNodes = [doc("p"), doc("q"), ...Array.from({ length: 50 }, (_, index) => doc(`p-${index}`)),
    ...Array.from({ length: 50 }, (_, index) => doc(`q-${index}`))];
  const otherLinks = otherNodes.filter(node => node.id.startsWith("p-")).map(node => edge("p", node.id)).concat(
    otherNodes.filter(node => node.id.startsWith("q-")).map(node => edge("q", node.id)));
  const other = reconcile(snapshot(otherNodes, otherLinks)), place = new Positions(); place.install(other, true);
  const partialNodes = other.nodes.filter(node => node.id !== "q"), partialIds = new Set(partialNodes.map(node => node.id));
  const partialLinks = otherLinks.filter(link => partialIds.has(link.source) && partialIds.has(link.target));
  const partial = constellationView(partialNodes, partialLinks, null, "p", 0);
  assert.equal(partial.disclosure?.hub, "p");
  place.showCore("p", 0, true, partial.disclosure?.index, partial.disclosure?.visible, { nodes: partialNodes, links: partialLinks });
  const q0 = other.nodes.find(node => node.id === "q-0")!, q1 = other.nodes.find(node => node.id === "q-1")!;
  assert.ok(Math.hypot(q0.x - q1.x, q0.y - q1.y) > 1,
    "stars exposed by removing a different hub retain independent positions");
});
test("an expanded core refits for changed relationships but retains the camera after drag and unchanged refresh", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 25 }, (_, index) => doc(`spoke-${index}`)), doc("late")];
  const links = nodes.slice(1, 26).map(node => edge("hub", node.id));
  const model = reconcile(snapshot(nodes, links)), positions = new Positions();
  positions.install(model, true);
  const firstEpoch = positions.structureEpoch;
  const hub = model.nodes.find(node => node.id === "hub")!;
  const late = model.nodes.find(node => node.id === "late")!;
  late.x = hub.x + 1000;
  const firstCore = constellationView(model.nodes, model.links, null, "hub").cores.find(core => core.hub === "hub")!;
  const first = expandedCoreCameraFrame(model.nodes, model.links, "hub", firstCore.members)!;
  const changedLinks = [...links, edge("hub", "late")];
  const refreshed = reconcile(snapshot(nodes, changedLinks), { ...model, nodes: model.nodes.map(node => ({ ...node })) });
  positions.install(refreshed, true);
  assert.ok(positions.structureEpoch > firstEpoch, "relationship changes invalidate spatial camera framing too");
  const changedEpoch = positions.structureEpoch;
  assert.deepEqual(refreshed.nodes.map(node => node.id), model.nodes.map(node => node.id));
  const nextCore = constellationView(refreshed.nodes, refreshed.links, null, "hub").cores.find(core => core.hub === "hub")!;
  const next = expandedCoreCameraFrame(refreshed.nodes, refreshed.links, "hub", nextCore.members)!;
  assert.notEqual(next.key, first.key, "new core membership invalidates the previous camera fit");
  assert.ok(next.radius > first.radius, "the newly connected distant node is included in the fit");
  assert.equal(expandedCoreCameraFrame([...refreshed.nodes].reverse(), [...refreshed.links].reverse(), "hub", nextCore.members)!.key, next.key,
    "response order alone does not move the camera");
  const movedHub = refreshed.nodes.find(node => node.id === "hub")!, originalX = movedHub.x;
  positions.begin("hub", 1);
  positions.move("hub", { x: originalX + 200, y: movedHub.y, z: movedHub.z });
  positions.advance(0, false); positions.advance(16, false); positions.release(16, true);
  assert.equal(positions.structureEpoch, changedEpoch, "dragging does not retrigger camera framing");
  assert.notEqual(movedHub.x, originalX);
  assert.equal(expandedCoreCameraFrame(refreshed.nodes, refreshed.links, "hub", nextCore.members)!.key, next.key,
    "dragging the core does not invalidate its camera fit");
  const unchanged = reconcile(snapshot(nodes, changedLinks), { ...refreshed, nodes: refreshed.nodes.map(node => ({ ...node })) });
  positions.install(unchanged, true);
  assert.equal(positions.structureEpoch, changedEpoch, "an unchanged refresh keeps the camera stable");
  const reordered = reconcile(snapshot([...nodes].reverse(), [...changedLinks].reverse()), unchanged);
  positions.install(reordered, true);
  assert.equal(positions.structureEpoch, changedEpoch, "response order alone keeps the camera stable");
  const unchangedCore = constellationView(unchanged.nodes, unchanged.links, null, "hub").cores.find(core => core.hub === "hub")!;
  assert.equal(expandedCoreCameraFrame(unchanged.nodes, unchanged.links, "hub", unchangedCore.members)!.key, next.key,
    "an unchanged refresh after drag retains the current camera fit");
  const rewiredLinks = [...changedLinks, edge("spoke-0", "spoke-1")];
  const rewired = reconcile(snapshot(nodes, rewiredLinks), unchanged);
  positions.install(rewired, true);
  assert.ok(positions.structureEpoch > changedEpoch);
  const rewiredCore = constellationView(rewired.nodes, rewired.links, null, "hub").cores.find(core => core.hub === "hub")!;
  assert.notEqual(expandedCoreCameraFrame(rewired.nodes, rewired.links, "hub", rewiredCore.members)!.key, next.key,
    "changed internal relationships invalidate the fit even with unchanged membership");
});
test("two dense groups joined at their hubs keep separate expandable cores and their bridge", () => {
  const nodes = ["a", "b"].flatMap(hub => [doc(hub), ...Array.from({ length: 25 }, (_, index) => doc(`${hub}-${index}`))]);
  const links = ["a", "b"].flatMap(hub => Array.from({ length: 25 }, (_, index) => edge(hub, `${hub}-${index}`)));
  links.push(edge("a", "b"));
  const model = reconcile(snapshot(nodes, links));
  const cores = denseConstellationCores(model.nodes, model.links);
  assert.deepEqual(cores.map(core => [core.hub, core.count]), [["a", 26], ["b", 26]]);
  const overview = constellationView(model.nodes, model.links, null, null);
  assert.deepEqual(overview.nodes.map(node => node.id), ["a", "b"]);
  assert.deepEqual(overview.links, [edge("a", "b")]);
  assert.equal(constellationView(model.nodes, model.links, null, "a").nodes.length, 27);
  assert.equal(constellationView(model.nodes, model.links, null, "a").links.length, 26);
  const switched = constellationView(model.nodes, model.links, "b-0", "a");
  assert.equal(switched.nodes.length, 27, "selection in another core closes the previously expanded one");
  assert.equal(switched.counts.has("a"), true);
  assert.equal(switched.counts.has("b"), false);
  const positions = new Positions(); positions.install(model, true); positions.showCore("b", 0, true);
  const b = model.nodes.find(node => node.id === "b")!, selected = model.nodes.find(node => node.id === "b-0")!;
  assert.ok(Math.hypot(selected.x - b.x, selected.y - b.y) >= 31 - 1e-8);
});
test("separate collapsed groups each sit among unrelated stars without a reserved halo", () => {
  const nodes = ["a", "b"].flatMap(hub => [doc(hub), ...Array.from({ length: 25 }, (_, index) => doc(`${hub}-${index}`))])
    .concat(Array.from({ length: 100 }, (_, index) => doc(`outside-${index}`)));
  const links = ["a", "b"].flatMap(hub => Array.from({ length: 25 }, (_, index) => edge(hub, `${hub}-${index}`)));
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  const overview = constellationView(model.nodes, model.links, null, null);
  assert.deepEqual([...overview.counts], [["a", 26], ["b", 26]]);
  const outside = overview.nodes.filter(node => node.id.startsWith("outside-"));
  for (const hubId of overview.counts.keys()) {
    const hub = model.nodes.find(node => node.id === hubId)!;
    assert.ok(Math.min(...outside.map(node => Math.hypot(node.x - hub.x, node.y - hub.y))) < 40,
      `${hubId} has unrelated stars within one visible slot`);
  }
  const before = new Map(model.nodes.map(node => [node.id, coordinates(node)]));
  positions.showCore("a", 0, true, 0);
  const expanded = constellationView(model.nodes, model.links, null, "a", 0);
  for (const node of expanded.nodes.filter(node => node.id.startsWith("a-"))) {
    for (const other of expanded.nodes.filter(other => other.id !== node.id)) {
      assert.ok(Math.hypot(node.x - other.x, node.y - other.y) >= 31 - 1e-7, "local expansion avoids fixed background stars");
    }
  }
  for (const node of model.nodes.filter(node => !node.id.startsWith("a-"))) assert.deepEqual(coordinates(node), before.get(node.id), "only newly revealed members move");
});
test("overview packing closes group gaps at different zooms without overlapping footprints or losing hidden members", async () => {
  const { summaryAppearance } = await import("./presentation.ts");
  const sizes = [22, 40, 75, 120];
  const nodes = sizes.flatMap((count, group) => Array.from({ length: count }, (_, index) => doc(`g${group}-${String(index).padStart(3, "0")}`)))
    .concat(Array.from({ length: 10 }, (_, index) => doc(`near-${index}`)), doc("historical-endpoint"));
  const links = sizes.flatMap((count, group) => Array.from({ length: count - 1 }, (_, index) => edge(`g${group}-000`, `g${group}-${String(index + 1).padStart(3, "0")}`)));
  links.push(edge("g0-000", "historical-endpoint", false));
  for (const scale of [.5, 2]) {
    const input = snapshot(nodes, links), model = reconcile(input), positions = new Positions(); positions.install(model, true);
    const originalLinks = model.links.map(link => ({ ...link }));
    const semantic = constellationView(model.nodes, model.links, null, null);
    const cohorts = new Map(semantic.cores.map(core => [core.hub, [...core.members]]));
    cohorts.set("near-0", nodes.filter(node => node.id.startsWith("near-")).map(node => node.id));
    const ids = [...cohorts.keys(), "historical-endpoint"];
    const byId = new Map(model.nodes.map(node => [node.id, node]));
    for (const [index, id] of ids.entries()) fixPosition(byId.get(id)!, { x: index * 500 * scale, y: 0, z: byId.get(id)!.z });
    const before = new Map(model.nodes.map(node => [node.id, coordinates(node)]));
    const discs = ids.map((id, index) => ({ id, x: byId.get(id)!.x / scale, y: byId.get(id)!.y / scale, depth: 100,
      radius: index < 4 ? summaryAppearance(sizes[index]).pixels / 2 : id === "near-0" ? summaryAppearance(10).pixels / 2 : 5 }));
    assert.equal(positions.packOverview(discs, semantic.links, cohorts, {
      right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, worldPerPixel: () => scale,
    }, 0, true), true);
    for (let i = 0; i < discs.length; i++) for (let j = i + 1; j < discs.length; j++) {
      const a = byId.get(discs[i].id)!, b = byId.get(discs[j].id)!;
      assert.ok(Math.hypot(a.x - b.x, a.y - b.y) / scale >= discs[i].radius + discs[j].radius + 6 - 1e-7, "actual group bodies retain a small clear gap");
    }
    const xs = ids.map(id => byId.get(id)!.x / scale), ys = ids.map(id => byId.get(id)!.y / scale);
    assert.ok(Math.max(...xs) - Math.min(...xs) < 300 && Math.max(...ys) - Math.min(...ys) < 300, "visible groups occupy a compact region instead of their original empty slots");
    for (const [id, members] of cohorts) for (const member of members) {
      const delta = before.get(member)!.map((value, axis) => value - before.get(id)![axis]);
      assert.deepEqual(coordinates(byId.get(member)!).map((value, axis) => Math.round((value - coordinates(byId.get(id)!)[axis]) * 1e7)), delta.map(value => Math.round(value * 1e7)), "hidden cohort geometry follows its representative");
    }
    assert.deepEqual(model.links, originalLinks, "packing cannot add, hide or rewrite stored relationships");
    const packed = new Map(model.nodes.map(node => [node.id, coordinates(node)]));
    const assertPacked = (current: typeof model.nodes, message: string) => {
      for (const node of current) assert.ok(Math.hypot(...coordinates(node).map((value, axis) => value - packed.get(node.id)![axis])) < 1e-7, message);
    };
    positions.showCore("g0-000", 100, true);
    assert.equal(constellationView(model.nodes, model.links, null, "g0-000", 0).nodes.filter(node => node.id.startsWith("g0-")).length, 22);
    for (const node of model.nodes.filter(node => !node.id.startsWith("g0-"))) {
      assert.ok(Math.hypot(...coordinates(node).map((value, axis) => value - packed.get(node.id)![axis])) < 1e-7,
        "opening one core leaves unrelated cores and stars in their overview positions");
    }
    assert.deepEqual(coordinates(byId.get("g0-000")!), packed.get("g0-000"), "expansion stays anchored at the existing hub");
    positions.showCore(null, 200, true);
    assertPacked(model.nodes, "closing restores the compact overview");
    positions.reset();
    assertPacked(model.nodes, "reset retains the packed overview");
    const refreshed = reconcile(input, model); positions.install(refreshed, true);
    assertPacked(refreshed.nodes, "unchanged refresh retains compact placement");
  }
});
test("overview, orbit, focus, refresh and reset keep a dense personal constellation readable", async () => {
  const { focusedCameraDistance, nodeScreenSize, nodeVisualRadius, starMotion } = await import("./presentation.ts");
  const nodes = Array.from({ length: 536 }, (_, index) => ({ ...doc(`personal-${String(index).padStart(3, "0")}`), scope: "personal" as const }));
  const links = Array.from({ length: 128 }, (_, index) => edge(nodes[0].id, nodes[index + 1].id));
  const source = { ...snapshot(nodes, links), scope: "personal" as const };
  const model = reconcile(source), positions = new Positions(); positions.install(model, true);
  const initial = new Map(model.nodes.map(node => [node.id, coordinates(node)]));
  const overviewView = constellationView(model.nodes, model.links, null, null);
  const overview = overviewView.nodes;
  const width = 1200, height = 800, camera = new PerspectiveCamera(60, width / height, .1, 10000);
  const bounds = overview.reduce((value, node) => ({
    minX: Math.min(value.minX, node.x), maxX: Math.max(value.maxX, node.x),
    minY: Math.min(value.minY, node.y), maxY: Math.max(value.maxY, node.y),
    minZ: Math.min(value.minZ, node.z), maxZ: Math.max(value.maxZ, node.z),
  }), { minX: Infinity, maxX: -Infinity, minY: Infinity, maxY: -Infinity, minZ: Infinity, maxZ: -Infinity });
  const center = new Vector3((bounds.minX + bounds.maxX) / 2, (bounds.minY + bounds.maxY) / 2, (bounds.minZ + bounds.maxZ) / 2);
  const radius = Math.max(18, ...overview.map(node => Math.hypot(node.x - center.x, node.y - center.y, node.z - center.z) + 6));
  const distance = Math.max(radius * 1.15 / Math.sin(camera.fov * Math.PI / 360), height * camera.projectionMatrix.elements[5] / 2);
  const project = () => overview.map(node => {
    const point = new Vector3(node.x, node.y, node.z).applyMatrix4(camera.matrixWorldInverse);
    const size = nodeScreenSize(node.kind, -point.z, height, camera.projectionMatrix.elements[5]);
    point.applyMatrix4(camera.projectionMatrix);
    return { x: (point.x + 1) * width / 2, y: (1 - point.y) * height / 2,
      radius: nodeVisualRadius(size, false, false, overviewView.counts.get(node.id)) };
  });
  camera.position.copy(center).add(new Vector3(0, 0, distance)); camera.lookAt(center); camera.updateMatrixWorld();
  const front = project();
  let overlaps = 0;
  for (let i = 0; i < front.length; i++) for (let j = i + 1; j < front.length; j++) {
    if (Math.hypot(front[i].x - front[j].x, front[i].y - front[j].y) < front[i].radius + front[j].radius) overlaps++;
  }
  assert.ok(overlaps < 100, `${overlaps} projected bodies overlap in the first view`);
  camera.up.set(0, 0, 1); camera.position.copy(center).add(new Vector3(0, distance, 0)); camera.lookAt(center); camera.updateMatrixWorld();
  const side = project(), sideHeight = Math.max(...side.map(node => node.y)) - Math.min(...side.map(node => node.y));
  const sideWidth = Math.max(...side.map(node => node.x)) - Math.min(...side.map(node => node.x));
  assert.ok(sideHeight > height / 4 && sideHeight > sideWidth * .55, "orbiting across the old plane retains visible depth instead of a line");
  const focus = focusedCameraDistance(distance, height, camera.projectionMatrix.elements[5]);
  assert.ok(nodeScreenSize("document", focus, height, camera.projectionMatrix.elements[5]) >= 72);
  assert.ok(starMotion(72, 1, 1).detail > .5, "a focused star reveals its rotating surface");
  assert.equal(focusedCameraDistance(focus / 2, height, camera.projectionMatrix.elements[5]), focus / 2, "closer user zoom is retained");
  const refreshed = reconcile(source, model); positions.install(refreshed, true);
  assert.deepEqual(new Map(refreshed.nodes.map(node => [node.id, coordinates(node)])), initial, "refresh retains the volumetric layout");
  const root = refreshed.nodes[0]; positions.begin(root.id, 1);
  positions.move(root.id, { x: root.x + 200, y: root.y, z: root.z }); positions.advance(0, false); positions.advance(16, false);
  positions.release(16, false); positions.advance(2100, false);
  assert.ok(refreshed.nodes.every(node => [node.x, node.y, node.z].every(Number.isFinite)), "drag and settle keep finite coordinates");
  positions.reset();
  assert.deepEqual(new Map(refreshed.nodes.map(node => [node.id, coordinates(node)])), initial, "reset restores the first view");
});
test("unlinked first views also have depth when orbited", () => {
  const model = reconcile(snapshot(Array.from({ length: 60 }, (_, index) => doc(`single-${index}`))));
  const positions = new Positions(); positions.install(model, true);
  assert.ok(new Set(model.nodes.map(node => node.z.toFixed(2))).size > 20);
  assert.equal(positions.hasCompactInitialLayout, true);
});
test("past-evidence singleton is initially near one host without changing semantic clusters or state", () => {
  const source = satelliteFixture(), model = reconcile(source), byId = new Map(model.nodes.map(n => [n.id, n]));
  const record = byId.get("z_record")!, host = byId.get("a0")!;
  assert.equal(visualSatellites(model).get(record.id), host.id);
  assert.ok(Math.hypot(record.x - host.x, record.y - host.y) <= 126);
  for (const [index, node] of model.nodes.entries()) for (const other of model.nodes.slice(index + 1)) assert.ok(Math.hypot(node.x - other.x, node.y - other.y) >= 40 - 1e-9);
  assert.notEqual(record.cluster, host.cluster); assert.notEqual(host.cluster, byId.get("b0")!.cluster);
  assert.equal(model.clusters.find(c => c.id === record.cluster)!.members.length, 1);
  assert.equal(record.supported, false); assert.equal(active(record), false); assert.match(stateName(record), /근거 재확인/);
  assert.deepEqual(model.links.filter(l => l.kind === "evidence"), source.links.filter(l => l.kind === "evidence"));
  assert.equal(visualSatellites(model).has("zz_unlinked"), false);
  const reversed = reconcile(snapshot([...source.nodes].reverse(), [...source.links].reverse()));
  assert.deepEqual(reversed, model, "anchor and layout ignore input order");
  assert.deepEqual(reconcile(source, model), model, "unchanged refresh preserves every retained coordinate");
});
test("past evidence bridges move both clusters and a hidden satellite into one visual constellation", () => {
  for (const picked of ["a0", "z_record"]) {
    const model = reconcile(satelliteFixture()), before = model.nodes.map(n => ({ ...n }));
    const positions = new Positions(); positions.install(model);
    const shown = visibleGraph(model, { kind: "document", state: "all", cluster: null });
    assert.equal(shown.nodes.some(n => n.id === "z_record"), false);
    positions.begin(picked, 1); positions.move(picked, { x: 1500, y: 1500, z: 20 }); positions.advance(0, true);
    assert.notDeepEqual(model.nodes.find(n => n.id === "b0"), before.find(n => n.id === "b0"), "far end of the brown edge follows during drag");
    positions.release(0, true);
    for (const node of model.nodes) {
      if (node.id.startsWith("a") || node.id.startsWith("b") || node.id === "z_record") assert.ok(Math.hypot(node.x - 1500, node.y - 1500, node.z - 20) <= 48 + 1e-9);
      else assert.deepEqual(node, before.find(n => n.id === node.id), "unlinked orphan stays fixed");
    }
    for (const link of model.links.filter(link => link.kind === "evidence")) {
      const source = model.nodes.find(node => node.id === link.source)!, target = model.nodes.find(node => node.id === link.target)!;
      assert.ok(Math.hypot(source.x - target.x, source.y - target.y, source.z - target.z) <= 24 + 1e-9, "brown edge is compact");
    }
    assert.deepEqual(model.nodes.map(n => n.cluster), before.map(n => n.cluster));
    assert.deepEqual(visibleGraph(model, { kind: "document", state: "all", cluster: null }).links, shown.links, "display filtering stays unchanged");
    positions.reset(); assert.deepEqual(model.nodes, before);
  }
});
test("a past record with several cross-cluster links keeps every brown edge at 24 pixels", () => {
  const source = satelliteFixture();
  source.links.push(
    { source: "z_record", target: "a1", kind: "evidence", current: false },
    { source: "z_record", target: "b1", kind: "evidence", current: false },
  );
  const model = reconcile(source), positions = new Positions(); positions.install(model);
  positions.begin("a0", 1); positions.move("a0", { x: 1500, y: 1500, z: 0 }); positions.release(0, true);
  for (const link of model.links.filter(link => link.kind === "evidence")) {
    const a = model.nodes.find(node => node.id === link.source)!, b = model.nodes.find(node => node.id === link.target)!;
    assert.ok(Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z) <= 24 + 1e-9, `${link.source} → ${link.target} remains 24px or closer`);
  }
});
test("two past records bridging different documents keep both constellations compact", () => {
  const nodes = [doc("a0"), doc("a1"), doc("a2"), doc("b0"), doc("b1"), doc("b2"), { ...memory("z0"), supported: false }, { ...memory("z1"), supported: false }];
  const links: GraphLink[] = [edge("a0", "a1"), edge("a0", "a2"), edge("a1", "a2"), edge("b0", "b1"), edge("b0", "b2"), edge("b1", "b2"),
    { source: "z0", target: "a0", kind: "evidence", current: false }, { source: "z0", target: "b0", kind: "evidence", current: false },
    { source: "z1", target: "a2", kind: "evidence", current: false }, { source: "z1", target: "b2", kind: "evidence", current: false }];
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model);
  positions.begin("a0", 1); positions.move("a0", { x: 1000, y: 0, z: 0 }); positions.release(0, true);
  for (const link of model.links) {
    const a = model.nodes.find(node => node.id === link.source)!, b = model.nodes.find(node => node.id === link.target)!;
    assert.ok(Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z) <= 24 + 1e-9, `${link.source} → ${link.target} is one lattice step`);
  }
});
test("brown evidence cycles keep 24px edges in a compact constellation at small and capped sizes", () => {
  for (const count of [10, 100, 400]) {
    const documents = Array.from({ length: count }, (_, index) => doc(`d${String(index).padStart(3, "0")}`));
    const records = Array.from({ length: count }, (_, index) => ({ ...memory(`m${String(index).padStart(3, "0")}`), supported: false }));
    const links: GraphLink[] = records.flatMap((record, index) => [index, (index + 1) % count].map(target => ({ source: record.id, target: documents[target].id, kind: "evidence", current: false })));
    const model = reconcile(snapshot([...documents, ...records], links)), positions = new Positions(); positions.install(model);
    positions.begin(documents[0].id, 1); positions.move(documents[0].id, { x: 1000, y: 0, z: 0 }); positions.release(0, true);
    const byId = new Map(model.nodes.map(node => [node.id, node]));
    for (const link of model.links) {
      const a = byId.get(link.source)!, b = byId.get(link.target)!;
      assert.ok(Math.hypot(a.x - b.x, a.y - b.y, a.z - b.z) <= 24 + 1e-9, `${link.source} → ${link.target} is 24px`);
    }
    const xs = model.nodes.map(node => node.x), ys = model.nodes.map(node => node.y);
    const bound = 24 * (Math.ceil(Math.sqrt(model.nodes.length)) + 3);
    assert.ok(Math.max(...xs) - Math.min(...xs) <= bound, `${model.nodes.length} nodes stay compact horizontally`);
    assert.ok(Math.max(...ys) - Math.min(...ys) <= bound, `${model.nodes.length} nodes stay compact vertically`);
  }
});
test("odd cycles also keep distinct nodes and equal 24px edges", () => {
  for (const count of [3, 5, 7]) {
    const nodes = Array.from({ length: count }, (_, index) => doc(`cycle-${index}`));
    const links = nodes.map((node, index) => edge(node.id, nodes[(index + 1) % count].id));
    const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model);
    positions.begin(nodes[0].id, 1); positions.move(nodes[0].id, { x: 1000, y: 0, z: 0 }); positions.release(0, true);
    const byId = new Map(model.nodes.map(node => [node.id, node]));
    assert.equal(new Set(model.nodes.map(node => `${node.x},${node.y}`)).size, count);
    for (const link of model.links) {
      const a = byId.get(link.source)!, b = byId.get(link.target)!;
      assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y) - 24) < 1e-9);
    }
  }
});
test("a cycle with a leaf keeps every kind of edge at 24px", () => {
  const verify = (count: number, kind: GraphLink["kind"], picked: string) => {
    const nodes = [...Array.from({ length: count }, (_, index) => doc(`n${index}`)), doc("leaf")];
    const links: GraphLink[] = Array.from({ length: count }, (_, index) => ({ ...edge(`n${index}`, `n${(index + 1) % count}`, kind === "related"), kind }));
    links.push({ ...edge("n0", "leaf", kind === "related"), kind });
    const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model);
    positions.begin(picked, 1); positions.move(picked, { x: 10000, y: 0, z: 0 }); positions.release(0, true);
    const byId = new Map(model.nodes.map(node => [node.id, node]));
    assert.equal(new Set(model.nodes.map(node => `${node.x},${node.y}`)).size, nodes.length, "nodes do not overlap");
    for (const link of model.links) {
      const a = byId.get(link.source)!, b = byId.get(link.target)!;
      assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y) - 24) < 1e-9, `${kind}: ${link.source} → ${link.target} is 24px`);
    }
    const xs = model.nodes.map(node => node.x), ys = model.nodes.map(node => node.y);
    const bound = 24 * (Math.ceil(Math.sqrt(nodes.length)) + 3);
    assert.ok(Math.max(...xs) - Math.min(...xs) <= bound && Math.max(...ys) - Math.min(...ys) <= bound, "leaf stays in a compact constellation");
  };
  for (const kind of ["related", "evidence", "topic", "subject", "area"] as const) for (const picked of ["n0", "n10", "leaf"]) verify(20, kind, picked);
  for (const count of [100, 400]) verify(count, "evidence", "leaf");
});
test("cycle branches and two bridged cycles share the same 24px edge layout", () => {
  const check = (nodes: GraphNode[], links: GraphLink[], picked: string) => {
    const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model);
    positions.begin(picked, 1); positions.move(picked, { x: 10000, y: 0, z: 0 }); positions.release(0, true);
    const byId = new Map(model.nodes.map(node => [node.id, node]));
    assert.equal(new Set(model.nodes.map(node => `${node.x},${node.y}`)).size, nodes.length, "all nodes occupy distinct slots");
    for (const link of model.links) {
      const a = byId.get(link.source)!, b = byId.get(link.target)!;
      assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y) - 24) < 1e-9, `${link.source} → ${link.target} is 24px`);
    }
  };
  const first = Array.from({ length: 20 }, (_, index) => doc(`a${index}`));
  const second = Array.from({ length: 20 }, (_, index) => doc(`b${index}`));
  const cycle = (prefix: string) => Array.from({ length: 20 }, (_, index) => ({ ...edge(`${prefix}${index}`, `${prefix}${(index + 1) % 20}`, false), kind: "evidence" as const }));
  const leaves = [doc("leaf0"), doc("leaf10"), doc("tip0"), doc("tip10")];
  const branched = [...cycle("a"), { ...edge("a0", "leaf0", false), kind: "evidence" as const }, { ...edge("a10", "leaf10", false), kind: "evidence" as const }, edge("leaf0", "tip0"), edge("leaf10", "tip10")];
  for (const picked of ["a0", "a10", "leaf0", "tip10"]) check([...first, ...leaves], branched, picked);
  const bridged = [...cycle("a"), ...cycle("b"), { ...edge("a0", "b0", false), kind: "evidence" as const }];
  for (const picked of ["a0", "b10"]) check([...first, ...second], bridged, picked);
});
test("an impossible planar unit-edge graph follows a drag without stretching existing edges", () => {
  const nodes = [doc("a"), doc("b"), doc("c"), doc("d")];
  const links = nodes.flatMap((node, index) => nodes.slice(index + 1).map(other => edge(node.id, other.id)));
  const model = reconcile(snapshot(nodes, links)), before = new Map(model.nodes.map(node => [node.id, { ...node }]));
  const positions = new Positions(); positions.install(model);
  model.nodes.find(node => node.id === "a")!.x = 1000; // DragControls moves the held node before the first callback.
  positions.begin("a", 1); positions.move("a", { x: 1000, y: 0, z: 0 }); positions.release(0, true);
  const shift = model.nodes.find(node => node.id === "a")!.x - before.get("a")!.x;
  for (const node of model.nodes) {
    assert.equal(node.x, before.get(node.id)!.x + shift);
    assert.equal(node.y, before.get(node.id)!.y);
    assert.equal(node.z, before.get(node.id)!.z);
  }
});
test("feasible multi-cycle lattices keep every link at 24px", () => {
  for (const [width, seed] of [[4, 28], [4, 35], [6, 28], [6, 35], [8, 25]]) {
    const coordinates = Array.from({ length: width }, (_, q) => Array.from({ length: 5 }, (_, r) => ({ id: `${q},${r}`, q, r }))).flat();
    const nodes = coordinates.map(({ id }) => doc(id));
    let state = seed;
    const random = () => ((state = (Math.imul(state, 1664525) + 1013904223) >>> 0) / 4294967296);
    const links: GraphLink[] = [];
    for (const { id, q, r } of coordinates) for (const [dq, dr] of [[1, 0], [0, 1], [1, -1]]) {
      const other = `${q + dq},${r + dr}`;
      if (q + dq >= width || r + dr < 0 || r + dr >= 5) continue;
      if ((dq === 0 && dr === 1) || (r === 0 && dr === 0) || random() < .45) links.push(edge(id, other));
    }
    const model = reconcile(snapshot(nodes, links));
    for (const node of model.nodes) {
      const [q, r] = node.id.split(",").map(Number);
      node.x = 100 * (q + r / 2); node.y = 100 * r * Math.sqrt(3) / 2; node.z = 0;
    }
    const positions = new Positions(); positions.install(model);
    model.nodes.find(node => node.id === "0,0")!.x = 10000;
    positions.begin("0,0", 1); positions.move("0,0", { x: 10000, y: 0, z: 0 }); positions.release(0, true);
    const byId = new Map(model.nodes.map(node => [node.id, node]));
    assert.equal(new Set(model.nodes.map(node => `${node.x},${node.y}`)).size, nodes.length);
    for (const link of links) {
      const a = byId.get(link.source)!, b = byId.get(link.target)!;
      assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y) - 24) < 1e-9, `${width}×5 seed ${seed}: ${link.source} → ${link.target} is 24px`);
    }
  }
});
test("a background layout preserves linked geometry until exact slots arrive", async () => {
  const width = 8, height = 5, coordinates = Array.from({ length: width }, (_, q) => Array.from({ length: height }, (_, r) => ({ id: `${q},${r}`, q, r }))).flat();
  let state = 25;
  const random = () => ((state = (Math.imul(state, 1664525) + 1013904223) >>> 0) / 4294967296);
  const links: GraphLink[] = [];
  for (const { id, q, r } of coordinates) for (const [dq, dr] of [[1, 0], [0, 1], [1, -1]]) {
    if (q + dq >= width || r + dr < 0 || r + dr >= height) continue;
    if ((dq === 0 && dr === 1) || (r === 0 && dr === 0) || random() < .45) links.push(edge(id, `${q + dq},${r + dr}`));
  }
  const model = reconcile(snapshot(coordinates.map(({ id }) => doc(id)), links));
  const originalWorker = globalThis.Worker;
  let requested = 0;
  class LayoutWorker {
    onmessage: ((event: MessageEvent<{ token: number; slots: [string, { x: number; y: number }][] | null }>) => void) | null = null;
    onerror: (() => void) | null = null;
    stopped = false;
    postMessage(request: { token: number; root: string; members: string[]; edges: [string, string][] }) {
      requested++;
      queueMicrotask(() => {
        if (this.stopped) {
          this.onmessage?.({ data: { token: request.token, slots: [] } } as MessageEvent<{ token: number; slots: [] }>);
          return;
        }
        const adjacency = new Map([request.root, ...request.members].map(id => [id, new Set<string>()]));
        for (const [a, b] of request.edges) { adjacency.get(a)!.add(b); adjacency.get(b)!.add(a); }
        const solved = compactSlots(request.root, request.members, adjacency, 1000, 100000);
        this.onmessage?.({ data: { token: request.token, slots: solved ? [...solved] : null } } as MessageEvent<{ token: number; slots: [string, { x: number; y: number }][] | null }>);
      });
    }
    terminate() { this.stopped = true; }
  }
  globalThis.Worker = LayoutWorker as unknown as typeof Worker;
  try {
    const positions = new Positions(); positions.install(model);
    const held = model.nodes.find(node => node.id === "0,0")!;
    const before = new Map(model.nodes.map(node => [node.id, { ...node }]));
    held.x = 10000;
    positions.begin(held.id, 1); positions.move(held.id, { x: 10000, y: 0, z: 0 }); positions.release(0, true);
    assert.equal(requested, 1, "expensive layout is delegated after the first movement");
    const origin = before.get(held.id)!;
    const shift = { x: held.x - origin.x, y: held.y - origin.y, z: held.z - origin.z };
    for (const node of model.nodes) {
      const original = before.get(node.id)!;
      assert.ok(Math.abs(node.x - original.x - shift.x) < 1e-8 && Math.abs(node.y - original.y - shift.y) < 1e-8 && Math.abs(node.z - original.z - shift.z) < 1e-8,
        "pending work preserves every edge's original geometry");
    }
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    const byId = new Map(model.nodes.map(node => [node.id, node]));
    for (const link of links) {
      const a = byId.get(link.source)!, b = byId.get(link.target)!;
      assert.ok(Math.abs(Math.hypot(a.x - b.x, a.y - b.y) - 24) < 1e-9, `${link.source} → ${link.target} settles at 24px`);
    }
    const next = reconcile(snapshot(coordinates.map(({ id }) => doc(id)), links));
    const second = new Positions(); second.install(next);
    const nextHeld = next.nodes.find(node => node.id === "0,0")!;
    nextHeld.x = 10000;
    second.begin(nextHeld.id, 1); second.move(nextHeld.id, { x: 10000, y: 0, z: 0 }); second.release(0, true);
    assert.equal(requested, 2);
    second.reset();
    const restored = next.nodes.map(node => ({ ...node }));
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    assert.deepEqual(next.nodes, restored, "a result queued before reset cannot move the new graph");
  } finally {
    globalThis.Worker = originalWorker;
  }
});
test("a failed background layout preserves a large impossible component", async () => {
  const cycle = Array.from({ length: 40 }, (_, index) => `ring${index}`), clique = ["k0", "k1", "k2", "k3"];
  const links = [...cycle.map((id, index) => edge(id, cycle[(index + 1) % cycle.length])),
    ...clique.flatMap((id, index) => clique.slice(index + 1).map(other => edge(id, other))), edge(cycle[0], clique[0])];
  const model = reconcile(snapshot([...cycle, ...clique].map(doc), links));
  const originalWorker = globalThis.Worker;
  let requested = 0;
  class NoLayoutWorker {
    onmessage: ((event: MessageEvent<{ token: number; slots: null }>) => void) | null = null;
    onerror: (() => void) | null = null;
    postMessage(request: { token: number }) {
      requested++;
      queueMicrotask(() => this.onmessage?.({ data: { token: request.token, slots: null } } as MessageEvent<{ token: number; slots: null }>));
    }
    terminate() {}
  }
  globalThis.Worker = NoLayoutWorker as unknown as typeof Worker;
  try {
    const positions = new Positions(); positions.install(model);
    const before = new Map(model.nodes.map(node => [node.id, { ...node }]));
    const held = model.nodes.find(node => node.id === cycle[0])!;
    const drop = { x: held.x + 600, y: held.y, z: held.z };
    positions.begin(held.id, 1); positions.move(held.id, drop); positions.release(0, true);
    assert.equal(requested, 1, "the large cyclic graph takes the worker path");
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    positions.advance(10000, false);
    for (const node of model.nodes) {
      const original = before.get(node.id)!;
      assert.ok(Math.abs(node.x - original.x - 600) < 1e-8 && Math.abs(node.y - original.y) < 1e-8 && Math.abs(node.z - original.z) < 1e-8,
        "an unsatisfiable worker result keeps the existing edge lengths");
    }
  } finally {
    globalThis.Worker = originalWorker;
  }
});
test("singleton-only anchors do not form cycles and deleted anchors leave no visual membership", () => {
  const source = snapshot([doc("a"), doc("b"), { ...memory("m"), supported: false }], [edge("a", "b", false), { source: "m", target: "a", kind: "evidence", current: false }]);
  const model = reconcile(source), anchors = visualSatellites(model);
  assert.equal(anchors.get("b"), "a"); assert.equal(anchors.get("m"), "a"); assert.equal(anchors.has("a"), false);
  for (const host of anchors.values()) assert.equal(anchors.has(host), false);
  const removed = reconcile(snapshot(source.nodes.filter(n => n.id !== "a"), source.links), model);
  assert.equal(visualSatellites(removed).size, 0);
});
test("new and reanchored satellites follow retained temporary hosts while reset uses automatic coordinates", () => {
  const source = satelliteFixture(), initial = reconcile(snapshot(source.nodes.filter(n => n.id !== "z_record"), source.links));
  const positions = new Positions(); positions.install(initial); positions.begin("a0", 1); positions.move("a0", { x: 1000, y: 1000, z: 0 }); positions.release(0, true);
  const automatic = reconcile(source), baseline = automatic.nodes.map(n => ({ ...n }));
  positions.install(automatic);
  const host = automatic.nodes.find(n => n.id === "a0")!, record = automatic.nodes.find(n => n.id === "z_record")!;
  assert.ok(Math.hypot(record.x - host.x, record.y - host.y) <= 126);
  positions.reset(); assert.deepEqual(automatic.nodes, baseline);
  const movedSource = { ...source, links: source.links.filter(l => !(l.source === "z_record" && l.target === "a0")) };
  const reanchored = reconcile(movedSource, automatic); positions.install(reanchored);
  assert.equal(visualSatellites(reanchored).get("z_record"), "b0");
  const b = reanchored.nodes.find(n => n.id === "b0")!, moved = reanchored.nodes.find(n => n.id === "z_record")!;
  assert.ok(Math.hypot(moved.x - b.x, moved.y - b.y) <= 126);
});

test("a trapped revealed star finds nearby clearance without being ejected past distant stars", () => {
  const blockers = Array.from({ length: 7 }, (_, x) => Array.from({ length: 7 }, (_, y) =>
    ({ id: `fixed-${x}-${y}`, x: (x - 3) * 31, y: (y - 3) * 31, radius: 15 }))).flat();
  const revealed = { id: "reveal", x: 15.5, y: 15.5, radius: 15 };
  const solve = (distant: boolean) => {
    const fixed = distant ? [...blockers, { id: "distant", x: 10000, y: 0, radius: 15 }] : blockers;
    const result = separateDiscs([...fixed, revealed], "fixed-3-3", new Set([revealed.id]), 1, new Set(fixed.map(disc => disc.id)));
    const at = result.get(revealed.id)!;
    assert.ok(Math.hypot(at.x - revealed.x, at.y - revealed.y) < 180, "clearance stays near the occupied local region");
    for (const disc of fixed) {
      assert.deepEqual(result.get(disc.id), { x: disc.x, y: disc.y }, "background positions stay fixed");
      assert.ok(Math.hypot(at.x - disc.x, at.y - disc.y) >= disc.radius + revealed.radius + 1 - 1e-8);
    }
    return at;
  };
  assert.deepEqual(solve(true), solve(false), "a remote blocker cannot enlarge a local edge");
});
test("default core framing does not magnify a layout step beyond 24 CSS pixels", async () => {
  const { coreCameraDistance } = await import("./presentation.ts");
  for (const [width, height] of [[390, 800], [1280, 900]]) for (const fov of [45, 60, 80]) {
    const camera = new PerspectiveCamera(fov, width / height, .1, 20000);
    for (const fit of [80, 1200, 9000]) {
      const distance = coreCameraDistance(fit, height, camera.projectionMatrix.elements[5]);
      camera.position.set(0, 0, distance); camera.lookAt(0, 0, 0); camera.updateMatrixWorld();
      const a = new Vector3(0, 0, 0).project(camera), b = new Vector3(31, 0, 0).project(camera);
      assert.ok(Math.hypot((b.x - a.x) * width / 2, (b.y - a.y) * height / 2) <= 24 + 1e-9,
        `${width}×${height}, fov ${fov}: automatic framing keeps the default link scale`);
      assert.ok(distance >= fit, "a large core still fits within its requested frame");
    }
  }
});


test("a filtered small core packs only the displayed graph instead of reserving hidden spokes", () => {
  const nodes = [memory("hub"), ...Array.from({ length: 100 }, (_, i) => doc(`a-${String(i).padStart(3, "0")}`)),
    ...Array.from({ length: 19 }, (_, i) => memory(`z-${String(i).padStart(3, "0")}`))];
  const links = nodes.slice(1).map(node => edge("hub", node.id));
  const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
  const scope = visibleGraph(model, { kind: "memory", state: "all", cluster: null, folder: null });
  const opened = constellationView(scope.nodes, scope.links, null, "hub", 0);
  assert.equal(opened.disclosure, null, "small filtered cores do not use the pagination path");
  const hiddenBefore = model.nodes.filter(node => node.kind === "document").map(coordinates);
  positions.showCore("*", 0, true, undefined, undefined, scope);
  const hub = positions.layoutTarget("hub")!;
  const longest = Math.max(...scope.links.map(link => {
    const target = positions.layoutTarget(link.target)!;
    return Math.hypot(target.x - hub.x, target.y - hub.y);
  }));
  assert.ok(longest <= 93 + 1e-6, "nineteen displayed spokes require three rings, not the hidden full fan");
  assert.deepEqual(model.nodes.filter(node => node.kind === "document").map(coordinates), hiddenBefore,
    "a scoped layout leaves omitted source nodes in place");
  positions.showCore(null, 1, true, undefined, undefined, scope);
  assert.equal(positions.packOverview([{ id: "hub", x: 0, y: 0, depth: 100, radius: 16 },
    { id: "z-000", x: 40, y: 0, depth: 100, radius: 5 }], [], new Map(), {
      right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 }, worldPerPixel: () => 1,
    }, 2, true), true, "a filtered overview still uses the shared screen-footprint packer");
});


test("clearance chooses nearby space instead of following a chain of fixed stars", () => {
  const blockers = Array.from({ length: 25 }, (_, index) => ({ id: `fixed-${index}`, x: 31 * index, y: 0, radius: 15 }));
  const revealed = { id: "reveal", x: 1, y: 0, radius: 15 };
  const solve = (remote: boolean) => {
    const fixed = remote ? [...blockers, { id: "remote", x: 10000, y: 0, radius: 15 }] : blockers;
    const result = separateDiscs([...fixed, revealed], "fixed-0", new Set([revealed.id]), 1, new Set(fixed.map(node => node.id)));
    const at = result.get(revealed.id)!;
    assert.ok(Math.hypot(at.x - revealed.x, at.y - revealed.y) < 31, "nearby clearance wins over a distant successful push chain");
    for (const node of fixed) {
      assert.deepEqual(result.get(node.id), { x: node.x, y: node.y });
      assert.ok(Math.hypot(at.x - node.x, at.y - node.y) >= 31);
    }
    return at;
  };
  assert.deepEqual(solve(true), solve(false));
});

test("nearest clearance handles intersecting and containing blocker circles", () => {
  for (const blockers of [
    [{ id: "left", x: -10, y: 0, radius: 15 }, { id: "right", x: 10, y: 0, radius: 15 }],
    [{ id: "inner", x: 0, y: 0, radius: 5 }, { id: "outer", x: 0, y: 0, radius: 40 }],
  ]) {
    const moving = { id: "moving", x: 0, y: 0, radius: 15 };
    const result = separateDiscs([...blockers, moving], blockers[0].id, new Set([moving.id]), 1, new Set(blockers.map(node => node.id)));
    const at = result.get(moving.id)!;
    for (const blocker of blockers) assert.ok(Math.hypot(at.x - blocker.x, at.y - blocker.y) >= blocker.radius + moving.radius + 1);
    const expected = blockers[0].id === "left" ? Math.sqrt(31 * 31 - 10 * 10) : 56;
    assert.ok(Math.abs(Math.hypot(at.x, at.y) - expected) < 1e-4, "the nearest exposed boundary supplies clearance");
    assert.deepEqual(result, separateDiscs([moving, ...blockers].reverse(), blockers[0].id, new Set([moving.id]), 1, new Set(blockers.map(node => node.id))));
  }
});


function assertUniformRings(points: readonly { x: number; y: number }[], hub: { x: number; y: number }) {
  const rings = new Map<number, number[]>();
  for (const node of points) {
    const radius = Math.round(Math.hypot(node.x - hub.x, node.y - hub.y) * 1e6) / 1e6;
    const angles = rings.get(radius) ?? [];
    angles.push((Math.atan2(node.y - hub.y, node.x - hub.x) + 2 * Math.PI) % (2 * Math.PI)); rings.set(radius, angles);
  }
  for (const angles of rings.values()) {
    angles.sort((a, b) => a - b);
    assert.ok(angles.length > 1, "a partially occupied outer ring cannot strand a lone star");
    for (const [index, angle] of angles.entries()) {
      const next = index + 1 < angles.length ? angles[index + 1] : angles[0] + 2 * Math.PI;
      assert.ok(Math.abs(next - angle - 2 * Math.PI / angles.length) < 1e-7, "every ring divides the full circle equally");
    }
  }
  return rings;
}

test("hub stars use uniform angles for small counts and partially occupied outer rings", () => {
  for (const count of [3, 4, 5, 6, 7, 8, 12, 13, 19, 20, 36, 119, 799]) {
    const nodes = [doc("hub"), ...Array.from({ length: count }, (_, i) => doc(`leaf-${String(i).padStart(3, "0")}`))];
    const links = nodes.slice(1).map(node => edge("hub", node.id));
    const source = snapshot(nodes, links), model = reconcile(source), positions = new Positions(); positions.install(model, true);
    positions.showCore("*", 0, true);
    const hub = model.nodes.find(node => node.id === "hub")!, leaves = model.nodes.filter(node => node.id !== "hub");
    const rings = assertUniformRings(leaves, hub);
    if (count <= 12) assert.equal(rings.size, 1, "small hubs expand one balanced ring instead of opening a partial hex ring");
    for (const [index, node] of model.nodes.entries()) for (const other of model.nodes.slice(index + 1)) {
      assert.ok(Math.hypot(node.x - other.x, node.y - other.y) >= 31 - 1e-6, "ring and hub bodies retain their separation");
    }
    assert.deepEqual(model.links, links, "visual distribution never changes stored relationships");
    const reversed = reconcile(snapshot([...nodes].reverse(), [...links].reverse())), other = new Positions(); other.install(reversed, true); other.showCore("*", 0, true);
    assert.deepEqual(model.nodes.map(coordinates), reversed.nodes.map(coordinates), "response order cannot rotate or reassign the rings");
  }
});

test("grabbing a hub or a leaf retains the common uniform star layout", () => {
  for (const count of [4, 5, 7, 8, 12]) for (const held of ["hub", "leaf-0"]) {
    const nodes = [doc("hub"), ...Array.from({ length: count }, (_, i) => doc(`leaf-${i}`))], links = nodes.slice(1).map(node => edge("hub", node.id));
    const model = reconcile(snapshot(nodes, links)), positions = new Positions(); positions.install(model, true);
    const start = model.nodes.find(node => node.id === held)!;
    positions.begin(held, 1); positions.move(held, { x: start.x + 200, y: start.y, z: start.z }); positions.release(0, true);
    assertUniformRings(model.nodes.filter(node => node.id !== "hub"), model.nodes.find(node => node.id === "hub")!);
    assert.deepEqual(model.links.map(link => JSON.stringify(link)).sort(), links.map(link => JSON.stringify(link)).sort());
  }
});

test("static star clearance turns or expands a whole uniform ring and preserves sufficient clearance", () => {
  const nodes = [doc("hub"), ...Array.from({ length: 8 }, (_, i) => doc(`leaf-${i}`)), doc("outside")];
  const model = reconcile(snapshot(nodes, nodes.slice(1, 9).map(node => edge("hub", node.id)))), positions = new Positions(); positions.install(model, true);
  const desired = new Map(model.nodes.map(node => [node.id, { x: node.id === "outside" ? 24 : 0, y: 0, z: 0 }]));
  const moving = new Set(nodes.slice(1, 9).map(node => node.id));
  let hubRadius = 10;
  const plane = { visible: model.nodes.map(node => node.id), right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 },
    project: (value: { x: number; y: number }) => ({ ...value, depth: 1 }), radius: (node: { id: string }) => node.id === "hub" ? hubRadius : node.id === "outside" ? 7 : 5,
    worldPerPixel: () => 1, isVisible: () => true };
  const clear = positions.clearLayout(desired, moving, "hub", plane), leaves = [...moving].map(id => clear.get(id)!);
  assertUniformRings(leaves, clear.get("hub")!);
  assert.ok(Math.hypot(leaves[0].x, leaves[0].y) > 24 && Math.hypot(leaves[0].x, leaves[0].y) < 40, "necessary clearance may exceed the preferred 24px radius");
  assert.deepEqual(clear.get("hub"), desired.get("hub")); assert.deepEqual(clear.get("outside"), desired.get("outside"));
  assert.deepEqual(positions.clearLayout(desired, moving, "hub", { ...plane, compactReveal: true }), clear,
    "a little breathing room keeps the existing ring instead of invoking compact fallback");
  for (const [index, node] of model.nodes.entries()) for (const other of model.nodes.slice(index + 1)) {
    const a = clear.get(node.id)!, b = clear.get(other.id)!;
    assert.ok(Math.hypot(a.x - b.x, a.y - b.y) >= plane.radius(node) + plane.radius(other) + 1 - 1e-6);
  }
  assert.deepEqual(positions.clearLayout(clear, moving, "hub", plane), clear, "a repeated view retains its settled phase and radius");
  hubRadius = 3;
  assert.deepEqual(positions.clearLayout(clear, moving, "hub", plane), clear, "shrinking a footprint cannot repack a settled ring");
});

test("a crowded reveal uses a nearby pocket instead of enlarging every spoke", () => {
  const leaves = Array.from({ length: 12 }, (_, i) => doc(`leaf-${i}`));
  const nodes = [doc("hub"), ...leaves, ...["blocked-left", "blocked-top", "blocked-right", "distant"].map(doc)];
  const model = reconcile(snapshot(nodes, leaves.map(node => edge("hub", node.id)))), positions = new Positions(); positions.install(model, true);
  const desired = new Map(model.nodes.map(node => [node.id, { x: 0, y: 0, z: 0 }]));
  desired.set("blocked-left", { x: -46, y: 0, z: 0 });
  desired.set("blocked-top", { x: 0, y: 46, z: 0 });
  desired.set("blocked-right", { x: 46, y: 0, z: 0 });
  desired.set("distant", { x: 0, y: -120, z: 0 });
  for (const [index, leaf] of leaves.entries()) desired.set(leaf.id, { x: 24 * Math.cos(index * Math.PI / 6), y: 24 * Math.sin(index * Math.PI / 6), z: index - 6 });
  const moving = new Set(leaves.map(node => node.id));
  const plane = { visible: nodes.map(node => node.id), right: { x: 1, y: 0, z: 0 }, up: { x: 0, y: 1, z: 0 },
    project: (value: { x: number; y: number }) => ({ x: value.x, y: value.y, depth: 1 }),
    radius: (node: { id: string }) => node.id.startsWith("blocked-") ? 32 : 5, worldPerPixel: () => 1, isVisible: () => true };
  const expanded = positions.clearLayout(desired, moving, "hub", plane);
  const compact = positions.clearLayout(desired, moving, "hub", { ...plane, compactReveal: true });
  const extent = (points: typeof desired) => Math.max(...leaves.map(node => Math.hypot(points.get(node.id)!.x, points.get(node.id)!.y)));
  assert.ok(extent(compact) < extent(expanded) * .8, `local clearance materially reduces the runaway ring in this crowded case (${extent(expanded)} → ${extent(compact)})`);
  assert.ok(extent(compact) < 60, "the free pocket is near the original hub rather than beyond the distant blocker");
  for (const node of nodes) {
    if (!moving.has(node.id)) assert.deepEqual(compact.get(node.id), desired.get(node.id), "the hub and background stay fixed");
    assert.equal(compact.get(node.id)!.z, desired.get(node.id)!.z, "compact screen clearance preserves depth");
    for (const other of nodes) {
      if (other.id === node.id) continue;
      const a = compact.get(node.id)!, b = compact.get(other.id)!;
      assert.ok(Math.hypot(a.x - b.x, a.y - b.y) >= plane.radius(node) + plane.radius(other) + 1 - 1e-6, "compact bodies keep clearance");
    }
  }
  assert.deepEqual(positions.clearLayout(compact, moving, "hub", { ...plane, compactReveal: true }), compact, "the settled pocket does not repack on refresh");
});

test("list sorting uses content dates and names without moving graph nodes", () => {
  const old = { ...memory("z"), label: "기록 10", created_at: "2026-01-01T00:00:00Z", content_updated_at: "2026-01-02T00:00:00Z" };
  const recent = { ...memory("a"), label: "기록 2", created_at: "2025-01-01T00:00:00Z", content_updated_at: "2026-02-01T00:00:00Z" };
  const unknown = { ...doc("b"), title: "원문", content_updated_at: "invalid", observed_at: "2099-01-01T00:00:00Z" };
  const markers: GraphNode[] = [{ id: "z-marker", scope: "meenseek", kind: "subject", label: "가나다" }, { id: "a-marker", scope: "meenseek", kind: "topic", label: "하나" }];
  const model = reconcile(snapshot([unknown, ...markers, old, recent]));
  const before = structuredClone(model);
  assert.deepEqual(listNodes(model.nodes).map(n => n.id), ["a", "z", "b", "z-marker", "a-marker"]);
  assert.deepEqual(listNodes(model.nodes, "created").map(n => n.id), ["z", "a", "b", "z-marker", "a-marker"]);
  assert.deepEqual(listNodes(model.nodes, "name").map(n => n.id), ["a", "z", "b", "z-marker", "a-marker"]);
  assert.deepEqual(model, before, "sorting cannot mutate membership, input order or coordinates");
  const same = [{ ...recent, id: "2", label: "same" }, { ...recent, id: "1", label: "same" }];
  assert.deepEqual(listNodes(same).map(n => n.id), ["1", "2"], "ties have a deterministic final key");
  assert.deepEqual(listNodes([...model.nodes].reverse()), listNodes(model.nodes), "source permutation cannot change display order");
});

test("closing retains outgoing stars for the full return and dissolves near the hub", () => {
  const state = retargetReveal(revealState("open", "view", ["hub", "a", "b"]), "closed", "view", ["hub"], new Set(["hub", "a", "b"]), 1000, false);
  assert.deepEqual([...state.retiring.keys()], ["a", "b"]);
  assert.equal(revealOpacity(state, "a", 1300), 1);
  assert.ok(revealOpacity(state, "a", 1600) > 0);
  assert.equal(revealOpacity(state, "a", 1000 + REVEAL_DURATION), 0);
  const finished = retargetReveal(state, "closed", "view", ["hub"], new Set(["hub", "a", "b"]), 1000 + REVEAL_DURATION, false);
  assert.equal(finished.retiring.size, 0);
});
test("paging and reversing a return preserve continuity without resurrecting filtered content", () => {
  const eligible = new Set(["hub", "a", "b", "c"]);
  const first = retargetReveal(revealState("one", "view", ["hub", "a"]), "two", "view", ["hub", "b"], eligible, 100, false);
  assert.equal(first.retiring.get("a"), 100);
  assert.equal(first.entering.get("b"), 100);
  const reverse = retargetReveal(first, "one", "view", ["hub", "a"], eligible, 200, false);
  assert.equal(reverse.retiring.has("a"), false);
  assert.equal(reverse.entering.has("a"), false);
  const filtered = retargetReveal(reverse, "filter", "different-view", ["c"], new Set(["c"]), 210, false);
  assert.equal(filtered.retiring.size, 0);
  assert.equal(filtered.entering.size, 0);
  const reduced = retargetReveal(first, "closed", "view", ["hub"], eligible, 210, true);
  assert.equal(reduced.retiring.size, 0);
});
test("camera retargeting starts at the observed pose and manual input cancels future writes", () => {
  const camera = new CameraMotion();
  const initial = { position: { x: 0, y: 0, z: 500 }, target: { x: 0, y: 0, z: 0 } };
  const first = { position: { x: 100, y: 50, z: 200 }, target: { x: 100, y: 50, z: 0 } };
  camera.move(initial, first, 0);
  const observed = camera.advance(200)!;
  const second = { position: { x: -90, y: 0, z: 400 }, target: { x: -90, y: 0, z: 0 } };
  assert.deepEqual(camera.move(observed, second, 200), observed);
  assert.deepEqual(camera.advance(200), observed);
  assert.notDeepEqual(observed, first);
  camera.stop(); assert.equal(camera.advance(1000), null); assert.equal(camera.moving, false);
  assert.deepEqual(camera.move(observed, initial, 1000, 0), initial);
  assert.equal(camera.moving, false);
  camera.move(observed, second, 1100);
  assert.deepEqual(camera.finish(), second);
  assert.equal(camera.advance(1200), null);
});
test("minimap coordinates invert precisely for translated scenes, empty scenes and narrow viewports", () => {
  for (const points of [[], [{ x: 1000, y: -900 }, { x: 1100, y: -800 }], [{ x: -20, y: 20 }]]) {
    const transform = miniMapTransform(points, 112, 70);
    for (const point of [{ x: 0, y: 0 }, ...points]) {
      const inverse = transform.toWorld(transform.toPixel(point));
      assert.ok(Math.abs(inverse.x - point.x) < 1e-9 && Math.abs(inverse.y - point.y) < 1e-9);
    }
    assert.ok(Number.isFinite(transform.scale) && transform.scale > 0);
  }
});

test("successive page transitions keep already returning stars until the retargeted layout reaches its hub", () => {
  const source = snapshot([{ id: "purpose-many", scope: "meenseek", kind: "subject", label: "Many" },
    ...Array.from({ length: 75 }, (_, index) => ({ ...doc(`member-${String(index).padStart(3, "0")}`), subject_id: "purpose-many", subject_name: "Many" }))]);
  const model = reconcile(source, undefined, false, "purpose"), positions = new Positions(); positions.install(model, true);
  const hub = [...constellationView(model.nodes, model.links, null, null, undefined, "purpose").counts.keys()][0];
  const first = constellationView(model.nodes, model.links, null, hub, 0, "purpose");
  positions.showCore(hub, 0, true, 0, first.disclosure!.visible);
  const eligible = new Set(model.nodes.map(node => node.id));
  let transition = revealState("0", "same", first.nodes.map(node => node.id));
  const turn = (page: number, now: number) => {
    const next = constellationView(model.nodes, model.links, null, hub, page, "purpose");
    positions.showCore(hub, now, false, page, next.disclosure!.visible);
    transition = retargetReveal(transition, String(page), "same", next.nodes.map(node => node.id), eligible, now, false);
  };
  turn(1, 100); positions.advance(250, false); turn(2, 250);
  const returning = [...transition.retiring.keys()][0];
  assert.equal(transition.retiring.get(returning), 250);
  positions.advance(100 + REVEAL_DURATION, false);
  assert.equal(positions.layoutMoving, true);
  assert.ok(revealOpacity(transition, returning, 100 + REVEAL_DURATION) > 0, "old deadline cannot hide a star whose physical return was restarted");
  positions.advance(250 + REVEAL_DURATION, false);
  assert.equal(positions.layoutMoving, false);
  assert.equal(revealOpacity(transition, returning, 250 + REVEAL_DURATION), 0);
});
