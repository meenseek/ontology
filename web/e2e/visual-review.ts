import { chromium, expect, type Page } from "playwright/test";
import { createServer } from "node:http";
import { readFile, readdir, mkdir, writeFile, lstat } from "node:fs/promises";
import { resolve, join, relative, extname, sep } from "node:path";
import { platform } from "node:os";
import { createMapFixture } from "../src/map-fixture.ts";
import { installGraphProbe, settledFramesMatch } from "./graph-probe.ts";
import { canonical, captureLaunchArgs, changedRegions, compareDirectories, hash, judge, requireValue, scenes, validateGraphics, validateState, type Capture, type Review } from "./visual-diff.ts";

async function files(root: string, directory = root): Promise<Record<string, string>> {
  const result: Record<string, string> = {};
  for (const item of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
    const path = join(directory, item.name);
    requireValue(!item.isSymbolicLink(), "Build/source bindings cannot contain symlinks");
    if (item.isDirectory()) Object.assign(result, await files(root, path));
    else if (item.isFile()) result[relative(root, path)] = hash(await readFile(path));
  }
  return result;
}
async function sourceBinding(source: string) {
  const result = { ...await files(source, join(source, "src")), ...await files(source, join(source, "build")) };
  for (const name of ["index.html", "package.json", "pnpm-lock.yaml", "vite.config.ts"]) result[name] = hash(await readFile(join(source, name)));
  result["../src/api.rs"] = hash(await readFile(resolve(source, "../src/api.rs")));
  return result;
}
async function captureInputs() {
  const fixture = createMapFixture(false);
  return {
    fixture: hash(canonical({ source: hash(await readFile(new URL("../src/map-fixture.ts", import.meta.url))), nodes: fixture.nodes, respond: fixture.respond.toString() })),
    tool: hash(canonical(Object.fromEntries(await Promise.all(["visual-review.ts", "visual-diff.ts", "graph-probe.ts", "png.ts"].map(async name => [name, hash(await readFile(new URL(name, import.meta.url)))]))))),
    observer: hash(installGraphProbe.toString()),
  };
}
async function settled(page: Page) {
  await page.evaluate(() => document.fonts.ready);
  let previous = "", serial = -1, stable = 0;
  await expect.poll(async () => {
    const sample = await page.evaluate(() => window.__graphQA.frame());
    if (!sample || sample.serial === serial || !sample.stars.length) return stable;
    serial = sample.serial;
    const current = JSON.stringify({ ...sample, serial: 0 });
    stable = settledFramesMatch(previous, current) ? stable + 1 : 0; previous = current;
    return stable;
  }, { timeout: 30_000, intervals: [100] }).toBeGreaterThanOrEqual(5);
}
async function capture(source: string, dist: string, output: string, installed?: string) {
  source = resolve(source); dist = resolve(dist);
  const policySource = await readFile(resolve(source, "../src/api.rs"), "utf8");
  const policy = policySource.match(/headers\.insert\(\s*"content-security-policy",\s*HeaderValue::from_static\("([^"]+)"\)\s*\);/)?.[1];
  requireValue(policy, "Missing actual production CSP");
  const build = { source: await sourceBinding(source), assets: await files(dist) };
  requireValue(Object.keys(build.assets).includes("index.html"), "Production build missing");
  const fixture = createMapFixture(false), inputs = await captureInputs();
  await mkdir(output, { recursive: false, mode: 0o700 });
  const server = createServer(async (request, response) => {
    const pathname = new URL(request.url!, "http://127.0.0.1").pathname;
    const filename = resolve(dist, "." + (pathname === "/" ? "/index.html" : pathname));
    if (!filename.startsWith(dist + sep)) { response.writeHead(403).end(); return; }
    try {
      const stat = await lstat(filename); if (!stat.isFile() || stat.isSymbolicLink()) throw new Error("not a file");
      const body = await readFile(filename), type = ({ ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml" } as Record<string, string>)[extname(filename)] ?? "application/octet-stream";
      response.writeHead(200, { "content-type": type, "content-security-policy": policy, "x-content-type-options": "nosniff" }).end(body);
    } catch { response.writeHead(404).end(); }
  });
  let browser: Awaited<ReturnType<typeof chromium.launch>> | undefined;
  try {
    let base = installed;
    if (base) {
      const url = new URL(base);
      requireValue(url.protocol === "http:" && url.hostname === "127.0.0.1" && url.pathname === "/" && !url.search && !url.username && !url.password, "Installed target must be the explicit local app root");
      base = url.origin;
      for (const [name, expected] of Object.entries(build.assets)) {
        const response = await fetch(`${base}/${name}`);
        requireValue(response.ok && hash(Buffer.from(await response.arrayBuffer())) === expected, `Installed asset differs: ${name}`);
      }
    } else {
      await new Promise<void>((done, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", done); });
      const address = server.address(); requireValue(address && typeof address !== "string", "Missing server address");
      base = `http://127.0.0.1:${address.port}`;
    }
    browser = await chromium.launch({ headless: true, args: [...captureLaunchArgs] });
    const browserSession = await browser.newBrowserCDPSession();
    const graphics = (await browserSession.send("SystemInfo.getInfo")).gpu.featureStatus ?? {};
    await browserSession.detach();
    validateGraphics(graphics);
    const runtime: Capture["environment"]["runtime"] = [];
    const manifest: Capture = { build, ...inputs, environment: { browser: browser.version(), platform: platform(), devices: scenes.map(s => [s.id, s.width, s.height, s.dpr]), locale: "ko-KR", reducedMotion: "reduce", launchArgs: [...captureLaunchArgs], graphics, runtime }, scenes: [] };
    for (const scene of scenes) {
      const context = await browser.newContext({ viewport: { width: scene.width, height: scene.height }, deviceScaleFactor: scene.dpr, hasTouch: scene.width === 390, isMobile: scene.width === 390, locale: "ko-KR", timezoneId: "Asia/Seoul", reducedMotion: "reduce", serviceWorkers: "block" });
      const errors: string[] = [], requests: string[] = [];
      try {
        await context.addInitScript(installGraphProbe);
        await context.route("**/*", async route => {
          const url = new URL(route.request().url());
          if (url.origin !== base) { errors.push(`External request: ${url.origin}`); return route.abort("blockedbyclient"); }
          if (!url.pathname.startsWith("/api/")) return route.continue();
          requests.push(url.pathname);
          const response = fixture.respond(url, route.request().postData());
          if (!response.ok) errors.push(`Unexpected synthetic request: ${url.pathname}`);
          return route.fulfill({ status: response.status, contentType: "application/json", body: await response.text() });
        });
        const page = await context.newPage(); page.on("pageerror", error => errors.push(error.message));
        const staticResponses: Promise<void>[] = [], verifiedAssets = new Set<string>();
        page.on("response", response => {
          const url = new URL(response.url());
          if (url.pathname.startsWith("/api/")) return;
          staticResponses.push((async () => {
            const name = url.pathname === "/" ? "index.html" : url.pathname.slice(1);
            requireValue(url.origin === base && Object.hasOwn(build.assets, name), `Unexpected static response: ${url.pathname}`);
            requireValue(response.ok() && hash(await response.body()) === build.assets[name], `Static asset differs: ${url.pathname}`);
            requireValue(response.headers()["content-security-policy"] === policy, `Production CSP differs: ${url.pathname}`);
            verifiedAssets.add(name);
          })().catch(error => { errors.push(String(error)); }));
        });
        let zoomStart: number[] | null = null, zoomClicks = 0;
        const zoomSteps: number[][] = [];
        page.setDefaultTimeout(30_000);
        await page.goto(base!);
        await page.waitForFunction(() => Boolean(window.__graphQA?.frame()?.stars.length), undefined, { timeout: 30_000 });
        const renderedCanvas = page.locator(".graph-viewport canvas[data-engine]");
        await expect(renderedCanvas).toHaveCount(1, { timeout: 30_000 });
        await expect(renderedCanvas).toBeVisible({ timeout: 30_000 }); await settled(page);
        runtime.push(await page.evaluate(() => {
          const canvas = document.querySelector<HTMLCanvasElement>(".graph-viewport canvas[data-engine]")!, gl = canvas.getContext("webgl2")!, ext = gl.getExtension("WEBGL_debug_renderer_info");
          const probe = document.createElement("canvas").getContext("2d")!;
          return { userAgent: navigator.userAgent, renderer: ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
            fonts: ["Arial", "AppleGothic", "system-ui"].map(font => { probe.font = `16px ${font}`; return [font, probe.measureText("가나다 ABC").width]; }) };
        }));
        const group = page.locator('.graph-group-target[data-node-id="preview-purpose-1"]');
        await expect(group).toBeAttached(); await group.focus(); await group.press("Enter");
        await expect(page.locator(".graph-core-title")).toContainText("보안 플랫폼");
        await expect(page.getByRole("button", { name: "묶음 접기", exact: true })).toBeVisible(); await settled(page);
        if (scene.state === "selected") {
          const point = await page.evaluate(() => {
            const canvas = document.querySelector<HTMLCanvasElement>(".graph-viewport canvas[data-engine]")!, rect = canvas.getBoundingClientRect();
            const star = window.__graphQA.frame()!.stars.find(s => s.id === "preview-1-0");
            if (!star || star.opacity < .99) throw new Error("Requested member is not actually painted");
            const x = rect.left + star.x, y = rect.top + star.y;
            return { x, y, onCanvas: document.elementFromPoint(x, y) === canvas };
          });
          requireValue(point.onCanvas, "Requested member is covered; do not bypass actual picking");
          await page.mouse.move(point.x, point.y); await page.mouse.click(point.x, point.y);
          await expect.poll(() => page.evaluate(() => new URLSearchParams(location.search).get("focus")), { timeout: 30_000 }).toBe("preview-1-0");
          await expect(page.locator(".management-panel")).toBeVisible();
        } else if (scene.state === "zoomed") {
            const expand = page.getByRole("button", { name: "미니맵 펼치기", exact: true });
            if (await expand.count()) await expand.click();
            // Its resize reframes the disclosed group. Finish that camera move
            // before recording the first zoom, rather than racing the resize.
            await settled(page);
            zoomStart = await page.evaluate(() => window.__graphQA.frame()!.camera);
            for (let i = 0; i < 2; i++) { await page.getByRole("button", { name: "확대", exact: true }).click(); zoomClicks++; await settled(page); zoomSteps.push(await page.evaluate(() => window.__graphQA.frame()!.camera)); }
        }
        await page.mouse.move(0, 0); await settled(page);
        // Playwright's default caret hiding edits every input's inline style,
        // invalidating paint after the settled-frame observation. These scenes
        // focus map controls, so capture the untouched page instead.
        requireValue(await page.evaluate(() => !document.activeElement?.matches("input,textarea,[contenteditable]")), `${scene.id}: unexpected editing focus`);
        const first = await page.screenshot({ caret: "initial" }); await page.waitForTimeout(120);
        const second = await page.screenshot({ caret: "initial" });
        requireValue(changedRegions(first, second).changed === 0, `${scene.id}: paint is still changing; do not ignore it`);
        const state = await page.evaluate(({ zoomStart, zoomClicks, zoomSteps }) => ({ focus: new URLSearchParams(location.search).get("focus"), expanded: document.querySelector(".graph-core-title")?.textContent ?? null,
          panel: !!document.querySelector(".management-panel:not([hidden])"), zoomStart, zoomClicks, zoomSteps,
          plot: document.querySelector(".graph-viewport")!.getBoundingClientRect().toJSON(), guidance: document.querySelector(".graph-guidance")!.getBoundingClientRect().toJSON(),
          frame: window.__graphQA.frame()!, labels: [...document.querySelectorAll<HTMLElement>(".node-label:not([hidden])")].filter(e => +e.style.opacity > .99).map(e => ({ id: e.dataset.nodeId, rect: e.getBoundingClientRect().toJSON(), text: e.textContent, style: e.getAttribute("style"), className: e.className })) }), { zoomStart, zoomClicks, zoomSteps });
        validateState(scene, state);
        requireValue(state.frame?.stars.length && state.labels.length, `${scene.id}: no actual painted stars or titles`);
        await Promise.all(staticResponses);
        requireValue(verifiedAssets.has("index.html") && requests.includes("/api/graph") && requests.includes("/api/session") && errors.length === 0, `${scene.id}: ${errors.join("; ")}`);
        const png = `${scene.id}.png`; await writeFile(join(output, png), second, { flag: "wx", mode: 0o600 });
        manifest.scenes.push({ id: scene.id, png, sha256: hash(second), state });
        console.error(`Captured ${scene.id}`);
      } catch (error) {
        const page = context.pages()[0];
        if (page && !page.isClosed()) {
          await page.screenshot({ path: join(output, `${scene.id}-failure.png`), caret: "initial" }).catch(() => {});
          const state = await page.evaluate(() => ({ frame: window.__graphQA?.frame(), text: document.body.innerText, viewports: document.querySelectorAll(".graph-viewport").length, canvases: [...document.querySelectorAll("canvas")].map(e => e.parentElement?.outerHTML.slice(0, 400)) })).catch(() => null);
          await writeFile(join(output, `${scene.id}-failure.json`), JSON.stringify({ error: String(error), errors, state }, null, 2) + "\n", { flag: "wx", mode: 0o600 }).catch(() => {});
        }
        throw error;
      } finally { await context.close(); }
    }
    requireValue(canonical(build) === canonical({ source: await sourceBinding(source), assets: await files(dist) }), "Source or build changed during capture");
    requireValue(canonical(inputs) === canonical(await captureInputs()), "Capture tool, fixture or observer changed during capture");
    await writeFile(join(output, "capture.json"), JSON.stringify(manifest, null, 2) + "\n", { flag: "wx", mode: 0o600 });
  } finally {
    await browser?.close();
    if (server.listening) { server.closeAllConnections(); await new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done())); }
  }
}
const [command, ...args] = process.argv.slice(2);
const option = (name: string) => { const at = args.indexOf(`--${name}`); requireValue(at >= 0 && args[at + 1] && !args[at + 1].startsWith("--"), `Missing --${name}`); return args[at + 1]; };
try {
  if (command === "capture") await capture(option("source"), option("dist"), option("output"), args.includes("--url") ? option("url") : undefined);
  else if (command === "diff" || command === "check") {
    const report = await compareDirectories(resolve(option("before")), resolve(option("after")));
    if (command === "diff") await writeFile(option("output"), JSON.stringify(report, null, 2) + "\n", { flag: "wx", mode: 0o600 });
    else {
      const review = JSON.parse(await readFile(option("review"), "utf8")) as Review, result = judge(report, review);
      console.log(JSON.stringify(result)); if (!result.complete) process.exitCode = 2;
    }
  } else throw new Error("Use capture --source WEB --dist DIST --output NEW_DIRECTORY [--url LOCAL_APP], diff --before DIR --after DIR --output NEW_REPORT, or check --before DIR --after DIR --review JSON");
} catch (error) { console.error(error instanceof Error ? error.message : error); process.exitCode = 1; }
