/// <reference types="node" />
import assert from "node:assert/strict";
import { test } from "node:test";
import { Positions } from "./positions.ts";
import { active, graphUrl, parseLocation, reconcile, sameGraphLocation, stateName, visibleGraph } from "./graph.ts";
import type { GraphLink, GraphNode, Snapshot } from "./graph.ts";
const doc = (id: string): GraphNode => ({ id, scope: "meenseek", kind: "document", label: id, revision: "1", generation: "1", content_digest: "digest", source_revision: "revision", status: "ok", present: true, current: true });
const memory = (id: string): GraphNode => ({ id, scope: "meenseek", kind: "memory", label: id, revision: "1", status: "accepted", temporal: "current", supported: true });
const edge = (source: string, target: string, current = true): GraphLink => ({ source, target, kind: "related", current });
function snapshot(nodes: GraphNode[], links: GraphLink[] = []): Snapshot {
  return { scope: "meenseek", query: "", focus: { id: null, found: false }, nodes, links, matched: nodes.length, totals: { documents: nodes.filter(n => n.kind === "document").length, memories: nodes.filter(n => n.kind === "memory").length, markers: nodes.filter(n => n.kind !== "document" && n.kind !== "memory").length, links: links.length }, returned: { knowledge: nodes.length, markers: 0, links: links.length }, omitted: { nodes: 0, links: 0 }, eligible: { nodes: nodes.length, links: links.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
}
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
    assert.equal(sizes[0], 26); assert.equal(sizes.at(-1), 140);
    assert.ok(sizes.every((size, index) => size >= 26 && size <= 140 && (!index || size >= sizes[index - 1])));
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

test("only held node moves during small and large drags; release compacts around the drop", () => {
  for (const distance of [8, 200]) {
    const model = reconcile(snapshot([doc("a"), doc("b"), doc("other")], [edge("a", "b")]));
    const positions = new Positions(); positions.install(model);
    const [a, b, other] = model.nodes, baseline = model.nodes.map(n => ({ ...n }));
    positions.begin(a.id, 1);
    positions.move(a.id, { x: distance, y: 0, z: 0 }); positions.advance(1000, false);
    assert.deepEqual(b, baseline[1], "no followers move while dragging");
    positions.release(1000, false); assert.deepEqual(b, baseline[1], "release starts without teleporting");
    positions.advance(1016, false); assert.notEqual(b.x, baseline[1].x);
    positions.advance(2100, false); assert.equal(b.x, distance + 24); assert.equal(a.x, distance);
    assert.deepEqual(other, baseline[2]);
  }
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
test("followers visibly overshoot while held node stays fixed, then settle independently of frame rate", () => {
  const make = () => {
    const model = reconcile(snapshot([doc("a"), doc("b")], [edge("a", "b")]));
    const positions = new Positions(); positions.install(model); positions.begin("a", 1);
    positions.move("a", { x: 200, y: 20, z: 30 }); positions.release(0, false);
    return { positions, model };
  };
  const { positions, model } = make(), single = make();
  const held = { ...model.nodes[0] };
  for (let t = 16; t <= 160; t += 16) positions.advance(t, false);
  single.positions.advance(160, false);
  assert.ok(model.nodes[1].x > 224, "follower passes its compact target after release");
  assert.ok(Math.abs(model.nodes[1].x - single.model.nodes[1].x) < 1e-9);
  assert.deepEqual(model.nodes[0], held, "held node never participates in spring motion");
  positions.advance(1100, false);
  assert.deepEqual([model.nodes[1].x, model.nodes[1].y, model.nodes[1].z], [224, 20, 30]);
  const stable = model.nodes.map(n => ({ ...n })); positions.advance(10000, false); assert.deepEqual(model.nodes, stable);
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
test("cancellation freezes an active spring and invalid input cannot poison coordinates", () => {
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
