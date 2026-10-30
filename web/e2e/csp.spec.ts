import { expect, test } from "playwright/test";
import { createServer, type Server } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

declare global {
  interface Window {
    __ontologyCsp: { directive: string; blocked: string }[];
    __ontologyInlineExecuted?: boolean;
  }
}

let server: Server, base: string;
// Use the actual production policy and freshly built app, without a store or a
// second policy allow-list that could hide a production-only CSP regression.
test.beforeAll(async () => {
  const source = await readFile(new URL("../../src/api.rs", import.meta.url), "utf8");
  const policy = source.match(/headers\.insert\(\s*"content-security-policy",\s*HeaderValue::from_static\("([^"]+)"\)\s*\);/)?.[1];
  if (!policy) throw new Error("Could not read the actual production CSP binding");
  const dist = resolve(fileURLToPath(new URL("../dist/", import.meta.url)));
  server = createServer(async (request, response) => {
    const pathname = new URL(request.url!, "http://127.0.0.1").pathname;
    const filename = resolve(dist, "." + (pathname === "/" ? "/index.html" : pathname));
    if (!filename.startsWith(dist + sep) && filename !== resolve(dist, "index.html")) {
      response.writeHead(403).end(); return;
    }
    if (pathname === "/favicon.ico") { response.writeHead(204).end(); return; }
    try {
      const body = await readFile(filename);
      const type = ({ ".html": "text/html", ".js": "text/javascript", ".css": "text/css" } as Record<string, string>)[extname(filename)] ?? "application/octet-stream";
      response.writeHead(200, { "content-type": type, "content-security-policy": policy, "x-content-type-options": "nosniff" }).end(body);
    } catch { response.writeHead(404).end(); }
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject); server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Missing owned test server address");
  base = `http://127.0.0.1:${address.port}`;
});

test.afterAll(async () => {
  if (!server) return;
  server.closeAllConnections();
  await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
});

for (const width of [1440, 390]) test(`production CSP ${width}px: static graph CSS and blocked inline code`, async ({ page, context }) => {
  await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
  const unexpected: string[] = [], errors: string[] = [], consoleErrors: string[] = [];
  const nodes = [{ id: "csp-subject", scope: "personal", kind: "subject", label: "합성 목적" }, ...Array.from({ length: 4 }, (_, i) => ({
    id: `csp-doc-${i}`, scope: "personal", kind: "document", title: `합성 자료 ${i}`, label: `synthetic/${i}.md`, subject_id: "csp-subject", subject_name: "합성 목적",
    source_kind: "original", context_scope: "personal", context_path: `synthetic/${i}.md`, status: "ok", present: true, current: true,
  }))];
  const links = nodes.slice(1).map(node => ({ source: node.id, target: "csp-subject", kind: "subject", current: true }));
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin !== base) { unexpected.push(url.href); return route.abort("blockedbyclient"); }
    if (!url.pathname.startsWith("/api/")) return route.continue();
    let data: unknown;
    if (url.pathname === "/api/session") data = { csrf: "synthetic-only", areas: [] };
    else if (url.pathname === "/api/graph") {
      const scope = url.searchParams.get("scope") ?? "personal";
      data = { scope, query: "", focus: { id: null, found: false }, nodes: nodes.map(node => ({ ...node, scope })), links, matched: 5,
        totals: { documents: 4, memories: 0, markers: 1, links: 4 }, returned: { knowledge: 4, markers: 1, links: 4 }, omitted: { nodes: 0, links: 0 },
        eligible: { nodes: 5, links: 4 }, limits: { nodes: 800, links: 2000, response_bytes: 1048576, byte_limited: false }, truncated: false };
    } else if (url.pathname === "/api/sync") data = { enabled: false, running: false, error: null, report: null };
    else { unexpected.push(url.href); return route.abort("blockedbyclient"); }
    return route.fulfill({ contentType: "application/json", body: JSON.stringify(data) });
  });
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") consoleErrors.push(message.text()); });
  await page.addInitScript(() => {
    window.__ontologyCsp = [];
    document.addEventListener("securitypolicyviolation", event => window.__ontologyCsp.push({ directive: event.effectiveDirective, blocked: event.blockedURI }));
  });
  const response = await page.goto(base);
  expect(response?.headers()["content-security-policy"]).toContain("style-src 'self';");
  expect(response?.headers()["content-security-policy"]).not.toMatch(/unsafe-inline|unsafe-eval/);
  const canvas = page.locator(".graph-viewport canvas[data-engine]");
  await expect(canvas).toBeVisible();
  await page.getByRole("button", { name: "목록 보기", exact: true }).click();
  await expect(page.locator(".graph-list")).toBeVisible();
  await page.getByRole("button", { name: "3D 보기", exact: true }).click();
  await expect(canvas).toBeVisible();
  await expect.poll(() => page.evaluate(() => {
    const selectors = [...document.styleSheets].flatMap(sheet => [...sheet.cssRules].map(rule => (rule as CSSStyleRule).selectorText ?? ""));
    return [".float-tooltip-kap", ".scene-nav-info", ".graph-info-msg"].every(name => selectors.some(selector => selector.includes(name)));
  })).toBe(true);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(await page.evaluate(() => window.__ontologyCsp)).toEqual([]);
  expect(await page.locator("style").count()).toBe(0);
  expect(unexpected).toEqual([]); expect(errors).toEqual([]); expect(consoleErrors).toEqual([]);

  // A quiet console alone could mean the policy was weakened. Exercise real
  // browser enforcement with arbitrary CSS and JS after checking normal usage.
  const blocked = await page.evaluate(async () => {
    const element = document.createElement("div"); element.id = "csp-negative-probe"; document.body.append(element);
    const before = getComputedStyle(element).color;
    const style = document.createElement("style"); style.textContent = "#csp-negative-probe{color:rgb(17,34,51)!important}";
    const script = document.createElement("script"); script.textContent = "window.__ontologyInlineExecuted=true";
    document.head.append(style, script);
    await new Promise(resolve => setTimeout(resolve, 50));
    const result = { style: getComputedStyle(element).color === before, script: window.__ontologyInlineExecuted !== true, violations: window.__ontologyCsp };
    element.remove(); style.remove(); script.remove(); return result;
  });
  expect(blocked.style).toBe(true); expect(blocked.script).toBe(true);
  expect(blocked.violations.some(event => event.directive === "style-src-elem" && event.blocked === "inline")).toBe(true);
  expect(blocked.violations.some(event => event.directive === "script-src-elem" && event.blocked === "inline")).toBe(true);
});
