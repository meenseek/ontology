import { useId, useRef } from "react";
import type { MouseEvent } from "react";
import Markdown from "react-markdown";
import type { Components } from "react-markdown";
import remarkFrontmatter from "remark-frontmatter";
import remarkGfm from "remark-gfm";
import { fileName } from "./presentation";

type Props = { path: string; content: string | null; kind: "vault" | "context" | "git" | "record"; title?: string; generatedTitle?: boolean };
type Tree = { type: string; tagName?: string; value?: string; properties?: Record<string, unknown>; children?: Tree[] };
const text = (node: Tree): string => node.value ?? String(node.properties?.alt ?? (node.children ?? []).map(text).join(""));

function headingIds({ namespace }: { namespace: string }) {
  return (tree: Tree) => {
    const used = new Set<string>();
    const visit = (node: Tree) => {
      if (/^h[1-6]$/.test(node.tagName ?? "") && !node.properties?.id) {
        const base = text(node).toLowerCase().replace(/[^\p{L}\p{N}\p{M}\s_-]/gu, "").trim().replace(/\s+/g, "-") || "section";
        let key = base, suffix = 0;
        while (used.has(key)) key = `${base}-${++suffix}`;
        used.add(key);
        node.properties = { ...node.properties, id: `${namespace}-heading-${key}`, "data-heading-key": key };
      }
      node.children?.forEach(visit);
    };
    visit(tree);
  };
}

/** Only Context and Vault importers adds a title before the body. Preserve the body and all fragment IDs. */
function documentHeading({ fallback, kind, generatedTitle }: { fallback: string; kind: Props["kind"]; generatedTitle?: boolean }) {
  return (tree: Tree) => {
    if (kind === "record" && generatedTitle) return;
    const children = tree.children ?? [];
    const [first, second] = children.filter(node => node.type !== "text" || node.value?.trim());
    if (first?.tagName !== "h1" || !text(first).trim() || (kind === "record" && text(first).trim() !== fallback.trim())) {
      children.unshift({ type: "element", tagName: "h1", properties: {}, children: [{ type: "text", value: fallback }] });
    } else if ((kind === "vault" || kind === "context") && second?.tagName === "h1"
      && first.children?.every(node => node.type === "text")
      && text(first).trim() === text(second).trim()) {
      // Keep the authored heading (including formatting/links). The synthetic title remains an anchor.
      first.tagName = "div";
      first.properties = { ...first.properties, className: ["document-title-anchor"] };
      first.children = [];
    }
    tree.children = children;
  };
}

/** Literal renderer IDs take precedence: footnotes can contain percent signs. */
export function findDocumentFragment(root: ParentNode, href: string): HTMLElement | null {
  if (!href.startsWith("#")) return null;
  const literal = href.slice(1);
  const targets = [...root.querySelectorAll<HTMLElement>("[id]")];
  const exact = targets.find(target => target.id === literal);
  if (exact) return exact;
  let decoded: string;
  try { decoded = decodeURIComponent(literal); } catch { return null; }
  return targets.find(target => target.id === decoded || target.getAttribute("data-heading-key") === decoded) ?? null;
}

function externalHref(value: string): string | undefined {
  if (!/^(https?:\/\/|mailto:)/i.test(value) || /[\u0000-\u0020\u007f\\]/.test(value)) return;
  try {
    const url = new URL(value);
    if (url.protocol === "mailto:" || ((url.protocol === "https:" || url.protocol === "http:") && url.hostname)) return value;
  } catch { /* Keep invalid destinations as literal references. */ }
}

function hasInteractiveDescendant(node: Tree): boolean {
  return (node.children ?? []).some(child => child.tagName === "a" || child.tagName === "img" || hasInteractiveDescendant(child));
}

function Reference({ value }: { value: string }) {
  const relative = value && !/^[a-z][a-z\d+.-]*:/i.test(value) && !/^[\/\\]{2}/.test(value);
  return <span className="document-reference"> ({value || "주소 없음"} · {relative ? "이 원문 경로는 여기서 열 수 없습니다." : "열 수 없는 주소입니다."})</span>;
}

export default function DocumentPreview({ path, content, kind, title, generatedTitle }: Props) {
  const namespace = `document-${useId()}`;
  const fallback = title ?? fileName(path);
  const preview = useRef<HTMLDivElement>(null);
  const followFragment = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault();
    const href = event.currentTarget.getAttribute("href");
    if (preview.current && href) findDocumentFragment(preview.current, href)?.scrollIntoView({ block: "nearest" });
  };
  const components: Components = {
    a({ node, href = "", children, ...props }) {
      const external = externalHref(href), fragment = href.startsWith("#");
      if (!external && !fragment) return <span>{children}<Reference value={href} /></span>;
      // Formatted images and footnotes produce their own controls at any depth.
      // Keep the parent's destination beside those controls, never around them.
      const separate = node && hasInteractiveDescendant(node);
      const link = <a {...props} href={href} rel={external ? "noreferrer" : undefined} target={external ? "_blank" : undefined} onClick={fragment ? followFragment : undefined}>{separate ? "연결된 문서 열기" : children}</a>;
      return separate ? <span>{children} · {link}</span> : link;
    },
    img({ src, alt, title: imageTitle }) {
      const value = typeof src === "string" ? src : "";
      const external = externalHref(value);
      return <span className="document-image-reference" title={imageTitle}>이미지: {alt || "설명 없음"}{external ? <> · <a href={external} target="_blank" rel="noreferrer">이미지 열기</a></> : <Reference value={value} />}</span>;
    },
    table({ node: _node, ...props }) {
      return <div className="document-table" role="region" aria-label="문서 표" tabIndex={0}><table {...props} /></div>;
    },
  };
  if (content === null) return <section className="document-content"><h1 className="document-title">{fallback}</h1><p className="hint">이 경로에서 성공적으로 읽은 원문이 없습니다.</p></section>;
  if (kind !== "record" && !/\.(md|markdown)$/i.test(path)) return <section className="document-content" aria-label="원문"><h1 className="document-title">{fallback}</h1>{content.trim() ? <pre className="source-text">{content}</pre> : <p className="hint">원문이 비어 있습니다.</p>}</section>;
  return <section className="document-content">
    <div className="document-preview" ref={preview} aria-label="문서 미리보기">
      {content.trim() ? <Markdown remarkPlugins={kind === "record" ? [remarkGfm] : [remarkFrontmatter, remarkGfm]} rehypePlugins={[[headingIds, { namespace }], [documentHeading, { fallback, kind, generatedTitle }]]} remarkRehypeOptions={{ footnoteLabel: "각주", footnoteBackLabel: (index, rereference) => `본문 ${index + 1}번 각주로 돌아가기${rereference > 1 ? ` (${rereference})` : ""}` }} urlTransform={value => value} components={components}>{content}</Markdown> : <><h1>{fallback}</h1><p className="hint">원문이 비어 있습니다.</p></>}
    </div>
  </section>;
}
