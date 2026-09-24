import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import DocumentPreview, { findDocumentFragment, resolveRelativeContextFilePath, resolveRelativeContextPath } from "./DocumentPreview";

const render = (content: string | null, path = "notes.md", kind: "git" | "vault" = "git") => renderToStaticMarkup(<DocumentPreview path={path} content={content} kind={kind} />);
const preview = (content: string, path?: string, kind?: "git" | "vault") => render(content, path, kind);
const decode = (value: string) => value.replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
function attributes(tag: string): Record<string, string> {
  return Object.fromEntries([...tag.matchAll(/([\w-]+)="([^"]*)"/g)].map(match => [match[1], decode(match[2])]));
}
function anchors(html: string) {
  return [...html.matchAll(/<a\b[^>]*>/g)].map(match => attributes(match[0]));
}
function fragmentRoot(html: string) {
  const targets = [...html.matchAll(/<[a-z][^>]*\bid="[^"]*"[^>]*>/gi)].map(match => {
    const props = attributes(match[0]);
    return { id: props.id, getAttribute: (name: string) => props[name] ?? null } as unknown as HTMLElement;
  });
  const root = { querySelectorAll(selector: string) { assert.equal(selector, "[id]"); return targets; } } as unknown as ParentNode;
  return { root, targets };
}
function noNestedControls(html: string) {
  const stack: string[] = [];
  for (const match of html.matchAll(/<(\/?)(a|button)\b[^>]*>/g)) {
    if (match[1]) assert.equal(stack.pop(), match[2]);
    else { assert.equal(stack.length, 0, `Nested control: ${match[0]}`); stack.push(match[2]); }
  }
  assert.equal(stack.length, 0);
}

test("renders headings, formatted prose, GFM lists/tasks/table and code through the component", () => {
  const html = preview('# 제목\n\n첫 줄\n둘째 줄 **강조** *기울임* ~~취소~~ `a < b`\n\n- 항목\n- [x] 완료\n- [ ] 예정\n\n1. 순서\n\n| 이름 | 값 |\n| --- | --- |\n| 데이터 | 1 |\n\n```js\n  a();\n\n\tb();\n```');
  for (const pattern of [/<h1[^>]*>제목<\/h1>/, /<strong>강조<\/strong>/, /<em>기울임<\/em>/, /<del>취소<\/del>/, /<ul/, /<ol>/, /<table>/, /<th>이름<\/th>/, /<td>데이터<\/td>/, /<code>a &lt; b<\/code>/]) assert.match(html, pattern);
  assert.match(html, /<p>첫 줄\n둘째 줄/);
  assert.doesNotMatch(html, /<br\s*\/>/);
  assert.match(html, /<input type="checkbox" disabled="" checked=""\/>/);
  assert.match(html, /<input type="checkbox" disabled=""\/>/);
  assert.match(html, /<pre><code class="language-js">  a\(\);\n\n\tb\(\);\n<\/code><\/pre>/);
  assert.match(html, /class="document-table" role="region" aria-label="문서 표" tabindex="0"/);
});

test("shows one Markdown preview without duplicating source or front matter", () => {
  const source = '\uFEFF---\r\ntitle: "숨김"\r\n---\r\n# 본문 제목\r\n\r\n <tag> & \t끝  \r\n';
  const html = render(source);
  assert.match(preview(source), /<h1[^>]*>본문 제목<\/h1>/);
  assert.doesNotMatch(preview(source), /숨김/);
  assert.doesNotMatch(html, /원문 보기|document-source|<pre class="source-text">/);
  assert.match(html, /aria-label="문서 미리보기"/);
});

test("original detail preview keeps authored heading without adding another title", () => {
  const html = renderToStaticMarkup(<DocumentPreview path="notes.md" content="문단만 있는 원문" kind="context" suppressGeneratedTitle />);
  assert.doesNotMatch(html, /<h1/);
  assert.match(html, /<p>문단만 있는 원문<\/p>/);
});

test("keeps thematic breaks and fenced front matter examples visible", () => {
  const html = preview('문단\n\n---\n\n# 실제 제목\n\n```yaml\n---\ntitle: 코드\n---\n```');
  assert.match(html, /<hr\/>/);
  assert.match(html, /<h1[^>]*>실제 제목<\/h1>/);
  assert.match(html, /<code class="language-yaml">---\ntitle: 코드\n---\n<\/code>/);
  assert.match(preview('---\n\n# 제목'), /<hr\/>/);
});

test("recognizes only Markdown extensions, case insensitively", () => {
  for (const path of ["docs/notes.md", "docs/notes.MD", "notes.MarkDown"]) assert.match(preview("# 제목", path), /<h1/);
  for (const path of ["notes.txt", "notes.json", "notes.md.txt", "README"]) {
    const html = render('# 제목\n\n**문자** <tag>', path);
    assert.match(html, /<pre class="source-text"># 제목\n\n\*\*문자\*\* &lt;tag&gt;<\/pre>/);
    assert.match(html, /<h1 class="document-title">/);
    assert.doesNotMatch(html, /<strong>|<details/);
  }
});

test("distinguishes unavailable content from empty content", () => {
  for (const path of ["notes.md", "notes.txt"]) {
    assert.match(render(null, path), /성공적으로 읽은 원문이 없습니다/);
    assert.doesNotMatch(render(null, path), /<details|원문이 비어/);
    assert.match(render("", path), /원문이 비어 있습니다/);
  }
  const source = " \t\r\n";
  assert.doesNotMatch(render(source), /document-source|원문 보기/);
});

test("raw HTML stays inert and images never create network-loading elements", () => {
  const html = preview('<script>alert(1)</script>\n\n<img src="https://example.com/track" onerror="alert(1)">\n\n<iframe src="https://example.com/frame"></iframe>\n\n![외부 이미지](https://example.com/image.png)\n\n![상대 이미지](./assets/pic.png)');
  assert.doesNotMatch(html, /<(?:script|img|iframe|object|embed|link|meta|video|audio|source)\b/i);
  assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.match(html, /이미지: 외부 이미지/);
  assert.match(html, /href="https:\/\/example.com\/image.png" target="_blank" rel="noreferrer"/);
  assert.match(html, /\.\/assets\/pic.png · 이 원문 경로는 여기서 열 수 없습니다/);
});

test("only explicit HTTP(S) and mailto destinations become external controls", () => {
  const html = preview('[보안](https://example.com/a?q=1&b=2) [일반](http://example.com) [메일](mailto:hello@example.com)\n\n[스크립트](javascript:alert%281%29) [데이터](data:text/html,test) [파일](file:///tmp/a) [프로토콜 상대](//example.com/a) [문서](../other.md) [절대 경로](/etc/passwd)\n\n![위험 이미지](javascript:alert%281%29)');
  assert.equal(anchors(html).length, 3);
  for (const link of anchors(html)) { assert.equal(link.rel, "noreferrer"); assert.equal(link.target, "_blank"); }
  for (const label of ["스크립트", "데이터", "파일", "프로토콜 상대", "문서", "절대 경로"]) assert.ok(html.includes(label));
  assert.match(html, /\.\.\/other.md · 이 원문 경로는 여기서 열 수 없습니다/);
  assert.match(html, /\/etc\/passwd · 이 원문 경로는 여기서 열 수 없습니다/);
  assert.doesNotMatch(html, /href="(?:javascript:|data:|file:|\/)/i);
});

test("linked context originals open through a verified local relationship", () => {
  const path = "knowledge/notion/index.md";
  const target = "knowledge/notion/originals/pages/[토스] 포트폴리오.md";
  const href = "originals/pages/[토스] 포트폴리오.md";
  assert.equal(resolveRelativeContextPath(path, href), target);
  assert.equal(resolveRelativeContextPath(path, "../../../../profile/private.md"), null);
  assert.equal(resolveRelativeContextPath(path, "javascript%3Aalert(1).md"), null);
  const html = renderToStaticMarkup(<DocumentPreview path={path} kind="context" content={`[원문](<${href}>) [없는 문서](missing.md)`} resolveInternalLink={value => resolveRelativeContextPath(path, value) === target ? () => {} : undefined} />);
  assert.match(html, /<button type="button" class="document-internal-link">원문<\/button>/);
  assert.match(html, /없는 문서.*이 원문 경로는 여기서 열 수 없습니다/);
  assert.doesNotMatch(html, /href="originals/);
});

test("referenced context attachments can be downloaded without loading them automatically", () => {
  const path = "knowledge/notion/originals/pages/portfolio.md";
  const href = "../assets/image.png";
  const target = "knowledge/notion/originals/assets/image.png";
  assert.equal(resolveRelativeContextFilePath(path, href), target);
  assert.equal(resolveRelativeContextFilePath(path, "../../../../../../private.png"), null);
  assert.equal(resolveRelativeContextFilePath(path, "javascript%3Aalert(1).png"), null);
  const local = `/api/context/download?${new URLSearchParams({ scope: "personal", path: target })}`;
  const html = renderToStaticMarkup(<DocumentPreview path={path} kind="context" content={`![그림](${href}) [표](${href})`} resolveInternalDownload={value => value === href ? local : undefined} />);
  assert.equal(anchors(html).length, 2);
  assert(anchors(html).every(link => link.href === local));
  assert.match(html, /이미지 다운로드/);
  assert.doesNotMatch(html, /<img|src=/);
});

for (const parent of ["https://example.com/parent", "#topics"]) {
  for (const formatted of ["**![도표](https://example.com/image.png)**", "*깊게 **![도표](https://example.com/image.png)** 감싸기*"]) {
    test(`formatted linked images keep parent and image destinations separate: ${parent}, ${formatted}`, () => {
      const html = preview(`# Topics\n\n[${formatted}](${parent})`);
      noNestedControls(html);
      assert.deepEqual(anchors(html).map(link => link.href), ["https://example.com/image.png", parent]);
    });
  }
  for (const label of ["본문 [^한글]", "*본문 **![도표](https://example.com/image.png)** [^한글]*"]) {
    test(`nested footnote references keep all destinations separate: ${parent}, ${label}`, () => {
      const html = preview(`# Topics\n\n[${label}](${parent})\n\n[^한글]: 각주 내용`);
      noNestedControls(html);
      const links = anchors(html);
      assert.equal(links.filter(link => link.href === parent).length, 1);
      const reference = links.find(link => link["data-footnote-ref"]);
      assert.ok(reference);
      assert.equal(reference["aria-describedby"], "footnote-label");
      const back = links.find(link => "data-footnote-backref" in link);
      assert.ok(back);
      assert.equal(back.href, `#${reference.id}`);
      assert.match(back["aria-label"], /각주로 돌아가기/);
      const { root } = fragmentRoot(html);
      assert.equal(findDocumentFragment(root, reference.href)?.id, reference.href.slice(1));
      assert.equal(findDocumentFragment(root, back.href)?.id, reference.id);
      if (label.includes("![")) assert.equal(links.filter(link => link.href === "https://example.com/image.png").length, 1);
    });
  }
}

test("heading IDs cannot shadow app fields, and encoded Korean fragments resolve locally", () => {
  const html = preview('# Topics\n\n# Topics\n\n# 한글 제목\n\n[태그](#topics) [한글](#%ED%95%9C%EA%B8%80-%EC%A0%9C%EB%AA%A9)');
  const { root, targets } = fragmentRoot(html);
  assert.ok(targets.every(target => target.id.startsWith("document-")));
  assert.equal(new Set(targets.map(target => target.id)).size, targets.length);
  assert.ok(targets.every(target => target.id !== "topics"));
  assert.equal(findDocumentFragment(root, "#topics")?.getAttribute("data-heading-key"), "topics");
  assert.equal(findDocumentFragment(root, "#topics-1")?.getAttribute("data-heading-key"), "topics-1");
  assert.equal(findDocumentFragment(root, "#%ED%95%9C%EA%B8%80-%EC%A0%9C%EB%AA%A9")?.getAttribute("data-heading-key"), "한글-제목");
  assert.equal(findDocumentFragment(root, "#graph-search"), null);
  const pair = renderToStaticMarkup(<><DocumentPreview path="a.md" content="# Topics" kind="git" /><DocumentPreview path="b.md" content="# Topics" kind="git" /></>);
  const headings = [...pair.matchAll(/<h1 id="([^"]+)"/g)].map(match => match[1]);
  assert.equal(new Set(headings).size, 2);
});

test("actual fragment lookup resolves Korean, percent-encoded and malformed-percent footnotes and their return links", () => {
  const html = preview('본문 [^한글] [^c%20d] [^bad%zz] 다시 [^한글]\n\n[^한글]: 한글 각주\n\n[^c%20d]: 퍼센트 각주\n\n[^bad%zz]: 퍼센트 오류 각주');
  const { root } = fragmentRoot(html);
  const links = anchors(html);
  assert.equal(links.length, 8);
  assert.match(html, /id="footnote-label">각주<\/h2>/);
  for (const link of links) {
    assert.ok(link.href.startsWith("#"));
    assert.equal(findDocumentFragment(root, link.href)?.id, link.href.slice(1), link.href);
  }
  assert.ok(links.some(link => link.href.includes("%ED%95%9C%EA%B8%80")));
  assert.ok(links.some(link => link.href.includes("c%20d")));
  assert.ok(links.some(link => link.href.includes("bad%zz")));
  assert.equal(findDocumentFragment(root, "#missing%zz"), null);
  assert.equal(findDocumentFragment(root, "#missing"), null);
  assert.equal(findDocumentFragment(root, "https://example.com/#topics"), null);
});

test("literal IDs win over decoded IDs and heading keys before any decoding", () => {
  const { root, targets } = fragmentRoot('<p id="c d"></p><p id="c%20d"></p><p id="broken%zz"></p><h1 id="namespaced-heading" data-heading-key="c d"></h1>');
  assert.equal(findDocumentFragment(root, "#c%20d"), targets[1]);
  assert.equal(findDocumentFragment(root, "#broken%zz"), targets[2]);
  assert.equal(findDocumentFragment(root, "#namespaced%2Dheading"), targets[3]);
});

test("Vault title appears once while authored formatting and every heading destination survive", () => {
  const source = '# 회사 기준\r\n\r\n# **회사 기준**\r\n\r\n본문\r\n\r\n# 회사 기준\r\n\r\n[처음](#회사-기준) [본문 제목](#회사-기준-1) [나중](#회사-기준-2)\r\n';
  const html = preview(source, "personal/company.md", "vault");
  assert.equal([...html.matchAll(/<h1\b/g)].length, 2, "only the added title is collapsed; the later heading stays");
  assert.match(html, /<h1[^>]*><strong>회사 기준<\/strong><\/h1>/);
  assert.doesNotMatch(html, /company\.md/);
  const { root } = fragmentRoot(html);
  for (const key of ["회사-기준", "회사-기준-1", "회사-기준-2"]) assert.equal(findDocumentFragment(root, `#${key}`)?.getAttribute("data-heading-key"), key);
  assert.match(html, /<div[^>]*class="document-title-anchor"><\/div>/);
  assert.doesNotMatch(render(source, "personal/company.md", "vault"), /document-source|원문 보기/);
  assert.equal(preview(source, "personal/company.md", "vault"), html, "repeated display is stable and does not edit the input");
});

test("only the known added Vault heading is collapsed; distinct or meaningful headings remain", () => {
  assert.equal([...preview('# Title\n\n# Title').matchAll(/<h1\b/g)].length, 2, "Git does not add a title");
  for (const source of [
    '# Title\n\n# title', '# Title\n\n# Title!', '# Title\n\n本文\n\n# Title',
    '# Title\n\n## Title', '# [Title](https://example.com)\n\n# Title',
    '# `Title`\n\n# Title', '# ![Title](https://example.com/image.png)\n\n# Title',
    '# Title [^n]\n\n# Title\n\n[^n]: 근거',
  ]) assert.doesNotMatch(preview(source, "a.md", "vault"), /document-title-anchor/, source);
  const linkedBody = preview('# Title\n\n# [Title](https://example.com)', "a.md", "vault");
  assert.equal([...linkedBody.matchAll(/<h1\b/g)].length, 1);
  assert.equal(anchors(linkedBody)[0].href, "https://example.com");
});

test("Markdown itself owns the leading title, including Setext and front matter", () => {
  for (const source of ['# **제목** ###\n\n본문', '제목\n====\n\n본문', '\uFEFF---\r\ntitle: metadata\r\n---\r\n# 제목\r\n']) {
    const html = preview(source);
    assert.equal([...html.matchAll(/<h1\b/g)].length, 1);
    assert.doesNotMatch(html, /notes\.md|metadata/);
    assert.match(html, /제목/);
  }
});

test("a missing leading title falls back once without stealing a section or code example", () => {
  for (const source of ['문단\n\n## 하위 제목', '```md\n# Example\n```\n\n# Later', '    # Indented code', '---\ntitle: metadata\n---', '', ' \t\r\n']) {
    const html = preview(source);
    assert.equal([...html.matchAll(/<h1>notes\.md<\/h1>/g)].length, 1, source);
    assert.doesNotMatch(render(source), /document-source|원문 보기/);
  }
  for (const path of ["notes.md", "notes.txt"]) {
    for (const source of [null, ""]) assert.equal([...render(source, path).matchAll(new RegExp(`>${path.replace('.', '\\.')}<`, 'g'))].length, 1);
  }
});


test("direct records retain YAML-shaped content, literal source, and their explicit title", () => {
  const html = renderToStaticMarkup(<DocumentPreview kind="record" path="record" title="A 고객 실험" content={'---\n가격: 12000\n---\n\n# 관찰 결과\n\n<script>alert(1)</script>'} />);
  assert.match(html, /A 고객 실험/);
  assert.match(html, /가격: 12000/);
  assert.match(html, /관찰 결과/);
  assert.doesNotMatch(html, /<script>/);
});
test("direct record headings keep authored links and avoid a repeated title", () => {
  const html = renderToStaticMarkup(<DocumentPreview kind="record" path="record" title="실험 결과" content={'# 실험 결과\n\n[결과](#실험-결과)\n\n내용'} />);
  assert.equal((html.match(/<h1\b/g) ?? []).length, 1);
  const { root } = fragmentRoot(html);
  assert.ok(findDocumentFragment(root, "#실험-결과"));
});


test("generated labels never repeat or truncate the authored record heading", () => {
  for (const content of ['# ' + '가'.repeat(81) + '\n\n본문', '# ~~이전~~ 새 결정\n\n본문', '관찰한 내용 그대로']) {
    const html = renderToStaticMarkup(<DocumentPreview kind="record" generatedTitle title="자동 목록 이름" path="record" content={content} />);
    assert.doesNotMatch(html, /자동 목록 이름/);
    assert.equal((html.match(/<h1\b/g) ?? []).length, content.startsWith('# ') ? 1 : 0);
    if (content.includes('가')) assert.ok(html.includes('가'.repeat(81)));
  }
});

test("Context synthetic heading preserves authored formatting and destinations", () => {
  const html = renderToStaticMarkup(<DocumentPreview kind="context" path="personal/note.md" content={'# Title\n\n# **Title**\n\n[body](#title-1)'} />);
  assert.equal((html.match(/<h1\b/g) ?? []).length, 1);
  assert.match(html, /<strong>Title<\/strong>/);
  assert.ok(findDocumentFragment(fragmentRoot(html).root, "#title-1"));
});

test("seeded Memory renders Context, Vault, Git and record provenance through its actual component", async () => {
  const { default: Memory } = await import("./Memory");
  const kinds = ["context", "vault", "git", "record"] as const;
  const item = { id: "m_seed", scope: "personal" as const, revision: 1, status: "proposed", origin: "assistant", subject_name: null, updated_at: "2026-01-01", support: "source-linked", kind: "record" as const, title: "Evidence", body: "Preserved record", subject_id: null, effective_from: null, effective_until: null, evidence: kinds.map((kind, i) => ({ entity_id: `e_${i}`, kind, repository: kind === "context" ? "ontology-context:00000000-0000-0000-0000-000000000001" : kind === "record" ? "분신" : "/original/root", path: kind === "record" ? "A record" : "personal/note.md", source_revision: "a".repeat(64), content_digest: "b".repeat(64), generation: 1, current: true })) };
  const html = renderToStaticMarkup(<Memory initialItem={item} selectedId={item.id} visible scope="personal" csrf="fixture" request={async () => { throw new Error("Static seeded render needs no request"); }} onBusy={() => {}} onChange={() => {}} onNavigate={() => {}} onMetadataChange={() => {}} />);
  for (const label of ["Context", "Vault", "Git", "기록"]) assert.match(html, new RegExp(`<dt>출처<\/dt><dd>${label}<\/dd>`));
  assert.match(html, /ontology-context:/);
});

test("personal Memory shows pending classification, suggestions, and retry without creating a subject", async () => {
  const { default: Memory, draftFromItem } = await import("./Memory");
  const base = { id: "m_seed", scope: "personal" as const, revision: 1, status: "accepted", origin: "user", subject_name: null, updated_at: "2026-01-01", support: "user-recorded", kind: "record" as const, title: "진학 메모", body: "연구실 탐색", subject_id: null, effective_from: null, effective_until: null, evidence: [] };
  const view = (grouping: { mode: "auto"; state: "pending" | "suggested" | "error"; suggestions: { candidate_ids?: string[]; new_subject?: string | null }; reason: string | null }) => renderToStaticMarkup(<Memory initialItem={{ ...base, grouping }} selectedId={base.id} visible scope="personal" csrf="fixture" request={async () => { throw new Error("Static seeded render needs no request"); }} onBusy={() => {}} onChange={() => {}} onNavigate={() => {}} onMetadataChange={() => {}} />);
  assert.match(view({ mode: "auto", state: "pending", suggestions: {}, reason: null }), /묶음 분류 중/);
  assert.match(view({ mode: "auto", state: "suggested", suggestions: { new_subject: "대학원 진학" }, reason: "주된 문제를 확인" }), /새 묶음.*만들고 연결/);
  assert.match(view({ mode: "auto", state: "error", suggestions: {}, reason: "실패" }), /다시 시도/);
  assert.equal(draftFromItem({ ...base, grouping: { mode: "off", state: "off", suggestions: {}, reason: null } }).grouping_preference, "off");
  assert.equal(draftFromItem({ ...base, grouping: { mode: "auto", state: "pending", suggestions: {}, reason: null } }).grouping_preference, "auto");
});
