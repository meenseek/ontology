export type ContextItem = { scope: string; path: string; source_path: string; content_digest: string; byte_len: number };
type ContextRead = { metadata: ContextItem & { material_id: string; revision: number; origin_kind: "imported-file" | "native"; source_digest: string | null }; content: string };
export function contextUrl(kind: "search" | "read" | "download", scope: string, value: string, after: string | null = null): string {
  const params = new URLSearchParams({ scope });
  if (kind === "search") {
    params.set("q", value); params.set("limit", "20");
    if (after !== null) params.set("after", after);
  } else params.set("path", value);
  return `/api/context${kind === "search" ? "" : `/${kind}`}?${params}`;
}
export function originalFilename(path: string): string { return path.slice(path.lastIndexOf("/") + 1); }
export function editorNewlines(content: string): "lf" | "crlf" | null {
  if (/\r(?!\n)/.test(content)) return null;
  const hasCrLf = content.includes("\r\n");
  if (hasCrLf && content.replace(/\r\n/g, "").includes("\n")) return null;
  return hasCrLf ? "crlf" : "lf";
}
export function editorDraft(content: string): string { return content.replace(/\r\n?|\n/g, "\n"); }
export function editorContent(draft: string, newlines: "lf" | "crlf"): string { return newlines === "crlf" ? draft.replace(/\n/g, "\r\n") : draft; }
export function OriginalText({ content }: { content: string }) {
  return <pre className="source-text context-text" aria-label="원본 텍스트">{content}</pre>;
}
export function failure(error: unknown, reading = false): string {
  const status = error && typeof error === "object" && "status" in error ? error.status : null;
  if (status === 403) return "세션을 확인할 수 없습니다. 페이지를 새로고침해 주세요.";
  if (status === 409) return "자료 반영 또는 복구가 진행 중입니다. 완료 후 다시 시도해 주세요.";
  if (status === 404) return "자료가 없거나 이 범위에서 열 수 없습니다.";
  if (status === 413) return reading ? "열람 가능한 크기를 넘었습니다. 원본 다운로드를 이용해 주세요." : "조회 또는 다운로드 한도를 넘었습니다.";
  if (status === 400 && reading) return "UTF-8 텍스트로 열 수 없는 자료입니다. 원본 다운로드를 이용해 주세요.";
  return "요청을 완료하지 못했습니다. 다시 시도해 주세요.";
}

export function ContextProvenance({ origin_kind, source_digest }: { origin_kind: ContextRead["metadata"]["origin_kind"]; source_digest: string | null }) {
  return <><dt>{origin_kind === "native" ? "출처" : "출처 SHA-256"}</dt><dd>{origin_kind === "native" ? "온톨로지에서 작성" : source_digest}</dd></>;
}
