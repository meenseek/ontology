import type { GraphNode, Scope, Snapshot } from "./graph";

/** Synthetic API shared by the dev preview and production-bundle visual review. */
export function createMapFixture(sparse = false) {
  const names = ["제품 전략과 의사결정", "보안 플랫폼 개발·검증과 사용자 경험을 함께 개선하기", "개발과 품질", "프로젝트 실행", "글쓰기와 지식 정리", "새로운 실험"];
  const counts = sparse ? [6, 5, 4, 3, 2, 2] : [225, 39, 22, 21, 20, 12];
  const titles = ["사용자의 맥락을 이해하는 방법", "작은 결정을 오래 기억하기", "제품의 방향을 정하는 질문", "읽기와 탐색의 균형", "이번 주 실험에서 배운 것", "반복 작업의 부담을 줄이기", "팀이 같은 그림을 보는 순간", "아이디어에서 실제 결과까지", "오래 쓰는 도구의 조건", "자연스럽게 이어지는 인터랙션", "기록을 다시 찾기 위한 기준", "변화하는 지식을 연결하는 방식"];
  // Members after the document chain mix in memories so kind colors, evidence and inactive states are visible.
  const isMemory = (index: number) => index >= 13 && index % 3 === 2;
  const inactive: Record<string, Partial<GraphNode>> = { "preview-1-14": { status: "proposed" }, "preview-1-17": { status: "withdrawn" }, "preview-1-20": { temporal: "expired" } };
  const memory = (id: string, title: string, extra: Partial<GraphNode> = {}): GraphNode => ({ id, scope: "personal", kind: "memory", title, label: title, memory_kind: "record", status: "accepted", temporal: "current", supported: true, ...extra, ...inactive[id] });
  const nodes: GraphNode[] = names.flatMap((name, group) => {
    const subject = `preview-purpose-${group}`;
    return [{ id: subject, scope: "personal", kind: "subject", label: name } as GraphNode,
      ...Array.from({ length: counts[group] }, (_, index): GraphNode => isMemory(index)
        ? memory(`preview-${group}-${index}`, titles[index % titles.length], { subject_id: subject, subject_name: name })
        : { id: `preview-${group}-${index}`, scope: "personal", kind: "document", title: titles[index % titles.length], label: `synthetic/${name}/${index}.md`,
          subject_id: subject, subject_name: name, source_kind: "original", context_scope: "personal", context_path: `synthetic/${group}/${index}.md`, status: "ok", present: true, current: true })];
  });
  for (let index = 0; index < (sparse ? 6 : 23); index++) nodes.push(index % 4 === 3 ? memory(`preview-single-${index}`, titles[index % titles.length]) : { id: `preview-single-${index}`, scope: "personal", kind: "document", title: titles[index % titles.length], label: `synthetic/ungrouped/${index}.md`,
    source_kind: "original", context_scope: "personal", context_path: `synthetic/ungrouped/${index}.md`, status: "ok", present: true, current: true });
  nodes.push({ id: "preview-topic-shared", scope: "personal", kind: "topic", label: "공통 관점" });
  const links: Snapshot["links"] = nodes.filter(node => node.subject_id).map(node => ({ source: node.id, target: node.subject_id!, kind: "subject", current: true }));
  for (let group = 0; group < 6; group++) for (let index = 0; index < Math.min(12, counts[group] - 1); index++) links.push({ source: `preview-${group}-${index}`, target: `preview-${group}-${index + 1}`, kind: index % 2 ? "related" : "reference", current: true });
  links.push({ source: "preview-0-0", target: "preview-1-0", kind: "reference", current: true });
  // Each memory cites the document before it; a few citations in the large group refer to past revisions.
  for (let group = 0; group < 6; group++) for (let index = 13; index < counts[group]; index++) {
    if (!isMemory(index) || (group === 0 && index % 9 !== 2)) continue;
    links.push({ source: `preview-${group}-${index}`, target: `preview-${group}-${index - 1}`, kind: "evidence", current: !(group === 0 && index % 27 === 2) });
  }
  for (const member of ["preview-2-0", "preview-2-1", "preview-2-2", "preview-3-0", "preview-3-1", "preview-3-2"].filter(id => nodes.some(node => node.id === id))) links.push({ source: member, target: "preview-topic-shared", kind: "topic", current: true });
  const json = (value: unknown, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
  const respond = (url: URL, body?: string | null) => {
    if (url.pathname === "/api/session") return json({ csrf: "synthetic-only", areas: [] });
    if (url.pathname === "/api/graph") {
      const scope: Scope = url.searchParams.get("scope") === "meenseek" ? "meenseek" : "personal", query = url.searchParams.get("q") ?? "", focus = url.searchParams.get("focus");
      const included = nodes.filter(node => !query || `${node.label} ${node.title}`.includes(query) || node.id === focus).map(node => ({ ...node, scope, excerpt: query ? `합성 자료에서 ${query} 검색어와 일치하는 내용입니다.` : undefined }));
      const ids = new Set(included.map(node => node.id)), includedLinks = links.filter(link => ids.has(link.source) && ids.has(link.target));
      return json({ scope, query, focus: { id: focus, found: !!focus && ids.has(focus) }, nodes: included, links: includedLinks, matched: included.length,
        totals: { documents: included.filter(node => node.kind === "document").length, memories: included.filter(node => node.kind === "memory").length, markers: included.filter(node => node.kind === "subject" || node.kind === "topic").length, links: includedLinks.length },
        returned: { knowledge: included.filter(node => node.kind === "document" || node.kind === "memory").length, markers: included.filter(node => node.kind === "subject" || node.kind === "topic").length, links: includedLinks.length },
        omitted: { nodes: 0, links: 0 }, eligible: { nodes: included.length, links: includedLinks.length }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false } satisfies Snapshot);
    }
    if (url.pathname === "/api/context/read") {
      const node = nodes.find(node => node.context_path === url.searchParams.get("path"));
      return json({ metadata: { scope: "personal", path: node?.context_path, revision: 1, content_digest: "synthetic", byte_len: 200 }, title: node?.title,
        content: `# ${node?.title ?? "합성 원문"}\n\n이 화면은 지도와 모션을 확인하기 위한 합성 자료입니다.\n\n## 탐색의 흐름\n\n자료를 읽고 연결을 따라간 뒤 같은 지도로 돌아옵니다.\n\n- 자료를 선택하면 별을 가까이에서 확인합니다.\n- 묶음을 펼치고 접거나 키보드로 탐색합니다.\n- 직접 배치한 위치는 세션 동안 유지합니다.` });
    }
    if (url.pathname === "/api/context/history") return json({ items: [], next_before: null });
    if (url.pathname === "/api/sync") return json({ enabled: false, running: false, error: null, report: null });
    if (url.pathname === "/api/brain" && typeof body === "string" && JSON.parse(body).op === "subjects") return json({ items: [], next_after: null });
    return json({ error: "합성 미리보기에서는 자료를 저장하지 않습니다." }, 403);
  };
  return { nodes, respond };
}
