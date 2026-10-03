import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { editorContent, editorDraft, editorNewlines, failure, OriginalText, contextUrl, originalFilename } from "./Original";

test("browser editing retains original newline convention", () => {
  const crlf = "title\r\nbody\r\n";
  assert.equal(editorNewlines(crlf), "crlf");
  assert.equal(editorContent(editorDraft(crlf).replace("body", "changed"), "crlf"), "title\r\nchanged\r\n");
  assert.equal(editorNewlines("title\nbody\n"), "lf");
  assert.equal(editorNewlines("title\r\nbody\n"), null);
  assert.equal(editorNewlines("title\rbody"), null);
});

test("context URLs retain exact scope, Unicode and query delimiters without changing endpoint", () => {
  const scope = "work/alpha", path = "attachments/한글 &?#%=+\".html";
  for (const kind of ["read", "download"] as const) {
    const url = new URL(contextUrl(kind, scope, path), "http://127.0.0.1:47831");
    assert.equal(url.origin, "http://127.0.0.1:47831");
    assert.equal(url.pathname, `/api/context/${kind}`);
    assert.equal(url.searchParams.get("scope"), scope);
    assert.equal(url.searchParams.get("path"), path);
    assert.equal(url.searchParams.size, 2);
    assert.equal(url.hash, "");
  }
  const url = new URL(contextUrl("search", scope, "한글 & 원문?", path), "http://local.invalid");
  assert.equal(url.searchParams.get("q"), "한글 & 원문?");
  assert.equal(url.searchParams.get("after"), path);
  assert.equal(url.searchParams.get("limit"), "20");
  assert.equal(new URL(contextUrl("search", "personal", ""), url).searchParams.has("after"), false);
  assert.equal(originalFilename(path), "한글 &?#%=+\".html");
});

test("original text is escaped, never executed or normalized into markup", () => {
  const content = "\uFEFF---\r\ntitle: 원문\r\n---\r\n<script>alert(1)</script><img src=x onerror=alert(1)> & \" ' \n";
  const output = renderToStaticMarkup(<OriginalText content={content} />);
  assert(!output.includes("<script>"));
  assert(!output.includes("<img"));
  assert(output.includes("&lt;script&gt;alert(1)&lt;/script&gt;"));
  assert(output.includes("\uFEFF---\r\ntitle: 원문\r\n---\r\n"));
  assert(output.includes("&amp; &quot; &#x27;"));
  assert.equal(Buffer.from(JSON.parse(JSON.stringify({ content })).content, "utf8").compare(Buffer.from(content, "utf8")), 0);
  assert(renderToStaticMarkup(<OriginalText content="" />).includes('aria-label="원본 텍스트"'));
});

test("all context read helpers display the pending message for HTTP 409", () => {
  const pending = "자료 반영 또는 복구가 진행 중입니다. 완료 후 다시 시도해 주세요.";
  for (const reading of [false, true]) assert.equal(failure({ status: 409, message: "private path" }, reading), pending);
  assert.equal(failure({ status: 404 }), "자료가 없거나 이 범위에서 열 수 없습니다.");
  assert.equal(failure({ status: 400 }, true), "UTF-8 텍스트로 열 수 없는 자료입니다. 원본 다운로드를 이용해 주세요.");
  assert(!failure({ status: 500 }).includes("private path"));
});
