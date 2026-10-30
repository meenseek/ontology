import { expect, test } from "playwright/test";
import { luminance, screenshotPixels, edgeLightPixels, ringRidge, sameRegionPixels } from "./png";

import { installGraphProbe } from "./graph-probe";

async function settled(page: import("playwright/test").Page) {
  let previous = "", sequence = "", stable = 0;
  await expect.poll(async () => {
    const value = await page.locator(".fixture-notice").evaluate(element => ({ ...(element as HTMLElement).dataset }));
    if (!value.frame || value.frameSequence === sequence) return stable;
    sequence = value.frameSequence!;
    const motion = JSON.parse(value.motion ?? "[]").at(-1);
    stable = value.frame === previous && motion && !motion.layout && !motion.dragging && !motion.settling ? stable + 1 : 0;
    previous = value.frame;
    return stable;
  }, { timeout: 25_000, intervals: [100] }).toBeGreaterThanOrEqual(3);
}

// Measure WebGL light without DOM names, keys or controls contaminating either
// edge or positive samples. Visibility preserves layout; the exact UI is restored.
async function lightScreenshot(page: import("playwright/test").Page) {
  const before = await page.evaluate(() => ({ focus: document.activeElement?.outerHTML.slice(0, 600), frame: window.__graphQA.frame(),
    styles: [...document.querySelector(".graph-viewport")!.children].map(e => ({ class: e.className, style: e.getAttribute("style") })) }));
  const region = await page.locator(".graph-viewport").evaluate(element => ({ rect: element.getBoundingClientRect().toJSON(), dpr: devicePixelRatio }));
  const original = await page.screenshot(), originalPixels = screenshotPixels(original);
  const layers = await page.locator(".graph-viewport").evaluateHandle(element => {
    const canvas = element.querySelector("canvas");
    const scene = [...element.children].find(child => child.contains(canvas));
    if (!canvas || !scene) throw new Error("Missing actual WebGL wrapper");
    return [...element.children].filter(child => child !== scene).map(child => {
      const element = child as HTMLElement;
      return { element, hadStyle: element.hasAttribute("style"), value: element.style.getPropertyValue("visibility"), priority: element.style.getPropertyPriority("visibility") };
    });
  });
  let isolated: Buffer;
  try {
    expect(await layers.evaluate(list => list.length), "actual DOM overlay layers exist").toBeGreaterThan(0);
    await layers.evaluate(list => list.forEach(({ element }) => element.style.setProperty("visibility", "hidden", "important")));
    isolated = await page.screenshot();
    expect(await layers.evaluate(list => list.every(({ element }) => element.isConnected && getComputedStyle(element).visibility === "hidden")), "only the actual connected DOM overlays are isolated").toBe(true);
  } finally {
    try {
      await layers.evaluate(list => list.forEach(({ element, value, priority, hadStyle }) => {
        if (value) element.style.setProperty("visibility", value, priority);
        else element.style.removeProperty("visibility");
        if (!hadStyle && !element.style.length) element.removeAttribute("style");
      }));
    } finally { await layers.dispose(); }
  }
  expect(sameRegionPixels(screenshotPixels(isolated!), originalPixels, region.rect, region.dpr), "visible DOM ink inside the plot is a real positive control").toBe(false);
  const restored = await page.screenshot();
  const restoredExactly = sameRegionPixels(screenshotPixels(restored), originalPixels, region.rect, region.dpr);
  if (!restoredExactly) {
    const after = await page.evaluate(() => ({ focus: document.activeElement?.outerHTML.slice(0, 600), frame: window.__graphQA.frame(),
      styles: [...document.querySelector(".graph-viewport")!.children].map(e => ({ class: e.className, style: e.getAttribute("style") })) }));
    await test.info().attach("light-normal", { body: original, contentType: "image/png" });
    await test.info().attach("light-isolated", { body: isolated!, contentType: "image/png" });
    await test.info().attach("light-restored", { body: restored, contentType: "image/png" });
    await test.info().attach("light-state", { body: JSON.stringify({ before, after, region }), contentType: "application/json" });
  }
  expect(restoredExactly, "restoring the overlays preserves every decoded pixel in the actual clipped plot").toBe(true);
  return isolated!;
}

async function relationships(page: import("playwright/test").Page) {
  await page.goto("/map-preview.html");
  await page.locator(".fixture-notice").evaluate(e => (e as HTMLElement).hidden = true);
  await settled(page);
  await page.getByText("보기 설정", { exact: true }).click();
  await page.getByRole("combobox", { name: "묶음 기준", exact: true }).selectOption("relationships");
  await page.getByText("보기 설정", { exact: true }).click();
  await settled(page);
}

async function panNode(page: import("playwright/test").Page, id: string, edge: "top" | "bottom", margin: number) {
  for (let i = 0; i < 4; i++) {
    const move = await page.evaluate(({ id, edge, margin }) => {
      const c = document.querySelector(".graph-viewport canvas")!.getBoundingClientRect(), f = window.__graphQA.frame()!;
      const star = f.stars.find(s => s.id === id)!;
      const dx = c.width * .35 - star.x, dy = (edge === "top" ? margin : c.height - margin) - star.y;
      if (Math.hypot(dx, dy) < .7) return null;
      const starts = [.03, .18, .35, .5, .65, .82, .97].flatMap(x => [.05, .15, .3, .5, .7, .85, .95].map(y => ({ x: c.left + c.width * x, y: c.top + c.height * y })));
      const start = starts.find(p => document.elementFromPoint(p.x, p.y)?.tagName === "CANVAS" && f.stars.every(s => Math.hypot(p.x - c.left - s.x, p.y - c.top - s.y) > Math.max(24, s.hitRadius * Math.SQRT2 + 4)));
      if (!start) throw new Error("No empty canvas point for actual pan");
      return { ...start, dx, dy };
    }, { id, edge, margin });
    if (!move) return;
    await page.mouse.move(move.x, move.y); await page.mouse.down();
    await page.mouse.move(move.x + move.dx, move.y + move.dy, { steps: 12 }); await page.mouse.up();
    await page.mouse.move(0, 0); await settled(page);
  }
  const error = await page.evaluate(({ id, edge, margin }) => {
    const c = document.querySelector(".graph-viewport canvas")!.getBoundingClientRect(), s = window.__graphQA.frame()!.stars.find(s => s.id === id)!;
    return Math.hypot(s.x - c.width * .35, s.y - (edge === "top" ? margin : c.height - margin));
  }, { id, edge, margin });
  expect(error, "actual pan reached the edge scene").toBeLessThan(1);
}

async function nodePoint(page: import("playwright/test").Page, id: string) {
  return page.evaluate(id => {
    const canvas = document.querySelector(".graph-viewport canvas")!.getBoundingClientRect(), star = window.__graphQA.frame()!.stars.find(s => s.id === id)!;
    return { x: canvas.x + star.x, y: canvas.y + star.y, star, canvas: canvas.toJSON() as { left: number; top: number; bottom: number; width: number; height: number } };
  }, id);
}

async function clickDocument(page: import("playwright/test").Page, id: string) {
  const p = await nodePoint(page, id);
  await page.evaluate(() => {
    const calls: string[] = [], previous = history.pushState.bind(history);
    history.pushState = (...args) => { calls.push(String(args[2])); return previous(...args); };
    (window as Window & { __testActivations?: string[] }).__testActivations = calls;
  });
  await page.mouse.click(p.x, p.y);
  await expect(page.locator(".management-panel")).toBeVisible();
  expect(await page.evaluate(() => new URLSearchParams(location.search).get("focus"))).toBe(id);
  expect(await page.evaluate(() => (window as Window & { __testActivations?: string[] }).__testActivations!.length)).toBe(1);
  await settled(page);
}

test("GPU light edges preserve actual star cores across DPR and resize", async ({ page }) => {
  await page.goto("/star-check.html");
  await expect(page.locator("#results")).toHaveAttribute("data-passed", "true", { timeout: 80_000 });
});

test.beforeEach(async ({ context }) => {
  await context.addInitScript(installGraphProbe);
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    return url.origin === "http://127.0.0.1:47839" && !url.pathname.startsWith("/api/")
      ? route.continue() : route.abort("blockedbyclient");
  });
});

for (const viewport of [{ width: 1440, height: 900, dpr: 1 }, { width: 920, height: 900, dpr: 1 }, { width: 390, height: 844, dpr: 2 }]) {
  test.describe(`visual ${viewport.width}px DPR ${viewport.dpr}`, () => {
    test.use({ viewport: { width: viewport.width, height: viewport.height }, deviceScaleFactor: viewport.dpr, contextOptions: { reducedMotion: "reduce" } });

    test("small taxonomy rings retain color and the map background is continuous", async ({ page }) => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.goto("/map-preview.html?sample=sparse");
      await page.locator(".fixture-notice").evaluate(element => element.setAttribute("hidden", ""));
      await page.waitForFunction(() => {
        const notice = document.querySelector<HTMLElement>(".fixture-notice")!, raw = notice.dataset.frame;
        if (!raw) return false;
        const frame = JSON.parse(raw), canvas = document.querySelector(".graph-viewport")!.getBoundingClientRect();
        const motion = JSON.parse(notice.dataset.motion ?? "[]").at(-1);
        return frame.width === Math.floor(canvas.width) && frame.height === Math.floor(canvas.height) &&
          frame.stars.filter((star: { id: string }) => star.id.startsWith("preview-purpose-")).length === 6 &&
          motion && !motion.layout && !motion.dragging && !motion.settling;
      });
      // The initial packing starts after the camera has rested for 150ms.
      // Require distinct, identical rendered frames, as the fixture's own checks do.
      let previous = "", sequence = "", stable = 0;
      await expect.poll(async () => {
        const sample = await page.locator(".fixture-notice").evaluate(element => ({
          sequence: (element as HTMLElement).dataset.frameSequence!,
          frame: (element as HTMLElement).dataset.frame!,
        }));
        if (sample.sequence !== sequence) {
          sequence = sample.sequence;
          stable = sample.frame === previous ? stable + 1 : 0;
          previous = sample.frame;
        }
        return stable;
      }, { timeout: 20_000, intervals: [100] }).toBeGreaterThanOrEqual(3);
      const geometry = await page.evaluate(() => {
        const canvas = document.querySelector(".graph-viewport")!.getBoundingClientRect();
        // Titles may be withheld to avoid collisions. Measure every actual body
        // through the existing fixture's rendered projection, independent of labels.
        const frame = JSON.parse(document.querySelector<HTMLElement>(".fixture-notice")!.dataset.frame!);
        const markers = frame.stars.filter((star: { id: string }) => star.id.startsWith("preview-purpose-"))
          .map((star: { id: string; px: number; py: number }) => ({ id: star.id, x: canvas.x + star.px, y: canvas.y + star.py }));
        return { top: canvas.top, markers };
      });
      const shot = screenshotPixels(await page.screenshot()), dpr = viewport.dpr;
      for (const marker of geometry.markers) {
        const cx = marker.x * dpr, cy = marker.y * dpr, background = luminance(shot.rgb(cx + 8 * dpr, cy));
        let peak = 0, visible = 0;
        for (let y = Math.floor(cy - 6 * dpr); y <= cy + 6 * dpr; y++) for (let x = Math.floor(cx - 6 * dpr); x <= cx + 6 * dpr; x++) {
          const radius = Math.hypot((x + .5 - cx) / dpr, (y + .5 - cy) / dpr);
          if (radius < 3 || radius > 5.5) continue;
          const contrast = (luminance(shot.rgb(x, y)) + .05) / (background + .05);
          peak = Math.max(peak, contrast);
          if (contrast >= 3) visible++;
        }
        expect(peak, `${marker.id}: rendered ring contrast`).toBeGreaterThanOrEqual(3);
        expect(visible, `${marker.id}: visible ring pixels`).toBeGreaterThanOrEqual(8 * dpr ** 2);
      }
      const y = Math.round(geometry.top * dpr), above = shot.rgb(5 * dpr, y - 1), below = shot.rgb(5 * dpr, y);
      expect(Math.max(...above.map((value, i) => Math.abs(value - below[i])))).toBeLessThanOrEqual(1);
      await page.getByText("관계·상태 읽기", { exact: true }).click();
      const help = await page.locator(".legend-guide").innerText();
      expect(help.match(/분류 소속선은/g)).toHaveLength(1);
      expect(help).toContain("켜 둔 폴더 연결은 계속 표시합니다");
      const html = await (await page.request.get("/index.html")).text();
      const theme = html.match(/name="theme-color" content="([^"]+)"/)?.[1];
      expect(theme).toBe(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--surface-page").trim()));
      expect(errors).toEqual([]);
    });

    test("long document context wraps by meaning and keeps its disclosure", async ({ page }) => {
      await page.goto("/map-preview.html?sample=sparse");
      await page.locator(".fixture-notice").evaluate(element => element.setAttribute("hidden", ""));
      await page.getByRole("button", { name: "목록 보기", exact: true }).click();
      const path = "synthetic/보안 플랫폼 개발·검증과 사용자 경험을 함께 개선하기/0.md";
      await page.locator(".graph-list > button").filter({ has: page.getByText(path, { exact: true }) }).click();
      const summary = page.locator(".document-context > summary");
      await expect(summary).toContainText("목적 소속: 보안 플랫폼");
      const count = summary.locator(".summary-count");
      await expect(count).toHaveText(/^원문 링크 \d+개$/);
      expect(await count.evaluate(element => {
        const range = document.createRange(); range.selectNodeContents(element);
        const lines = [...range.getClientRects()], panel = element.closest(".management-panel")!.getBoundingClientRect();
        // Korean text, the numeric count and its suffix are separate text runs.
        // One visual line has overlapping vertical bounds, even with several runs.
        return lines.length > 0 && lines.every(line => line.top < lines[0].bottom && line.bottom > lines[0].top &&
          line.left >= panel.left && line.right <= panel.right);
      })).toBe(true);
      await summary.click();
      await expect(page.locator(".document-context")).toHaveAttribute("open", "");
      await summary.press("Enter");
      await expect(page.locator(".document-context")).not.toHaveAttribute("open", "");
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    });
  });
}

for (const viewport of [{ width: 1440, height: 900, dpr: 1 }, { width: 920, height: 900, dpr: 1 }, { width: 390, height: 844, dpr: 2 }]) {
  test.describe(`rendered contracts ${viewport.width}px`, () => {
    test.use({ viewport: { width: viewport.width, height: viewport.height }, deviceScaleFactor: viewport.dpr });

    test("spatial members have no duplicate ring and retain titles, picking and state rings", async ({ page }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto("/map-preview.html");
      await page.locator(".fixture-notice").evaluate(element => element.setAttribute("hidden", ""));
      await settled(page);
      await page.getByText("보기 설정", { exact: true }).click();
      await page.getByRole("combobox", { name: "묶음 기준", exact: true }).selectOption("relationships");
      await page.getByText("보기 설정", { exact: true }).click();
      await settled(page);
      const group = page.locator(".graph-group-target").filter({ hasText: "근접 묶음" });
      if (!await group.count()) {
        const expand = page.getByRole("button", { name: "미니맵 펼치기", exact: true });
        if (await expand.count()) await expand.click();
        for (let i = 0; i < 5 && !await group.count(); i++) {
          await page.getByRole("button", { name: "축소", exact: true }).click();
          await settled(page);
        }
      }
      await expect(group.first()).toBeAttached();
      await group.first().focus(); await group.first().press("Enter"); await settled(page);
      const sprites = await page.evaluate(() => window.__graphQA.frame()!.stars.flatMap(star => star.sprites));
      const members = sprites.filter(sprite => Math.abs(sprite.pixels - 18) < .05);
      const markers = sprites.filter(sprite => Math.abs(sprite.pixels - 10) < .05 && sprite.opacity > .5);
      expect(members).toHaveLength(0);
      expect(markers.length).toBeGreaterThan(0);
      for (const marker of markers) expect(marker.stroke).toBeGreaterThanOrEqual(1.2);
      await expect(page.locator(".graph-core-close")).toHaveText(/근접 묶음 \d+개 접기/);
      const id = await page.evaluate(() => {
        const f = window.__graphQA.frame()!, c = document.querySelector(".graph-viewport canvas")!.getBoundingClientRect();
        return [...document.querySelectorAll<HTMLElement>(".node-label:not([hidden])")].map(e => e.dataset.nodeId!).find(id => {
          const s = f.stars.find(s => s.id === id);
          return id.startsWith("preview-single-") && s && s.hitRadius >= 18 && document.elementFromPoint(c.x + s.x, c.y + s.y)?.tagName === "CANVAS";
        });
      });
      expect(id).toBeTruthy();
      await page.getByText("관계·상태 읽기", { exact: true }).click();
      await expect(page.locator(".legend-guide")).not.toContainText("작은 원이 구성원을");
      await expect(page.locator(".legend-guide")).toContainText("가는 원은 선택");
      await page.getByText("관계·상태 읽기", { exact: true }).press("Escape");
      await clickDocument(page, id!);
      if (viewport.width > 900) {
        await page.evaluate(id => {
          const previous = window.fetch;
          window.fetch = async (...args) => {
            const response = await previous(...args), url = new URL(String(args[0]), location.href);
            if (url.pathname !== "/api/graph") return response;
            const data = await response.json();
            data.nodes = data.nodes.map((n: { id: string }) => n.id === id ? { ...n, revision: "edge-check-r2" } : n);
            return new Response(JSON.stringify(data), { headers: { "content-type": "application/json" } });
          };
        }, id!);
        await page.getByText("보기 설정", { exact: true }).click();
        await page.getByRole("button", { name: "새로고침", exact: true }).click();
        await page.getByText("보기 설정", { exact: true }).click(); await settled(page);
        const p = await nodePoint(page, id!), shot = screenshotPixels(await page.screenshot());
        const rings = p.star.sprites.filter(s => s.pixels > 20 && s.opacity > .4);
        expect(rings).toHaveLength(2);
        for (const ring of rings) {
          const ridge = ringRidge(shot, p.x, p.y, ring.pixels, viewport.dpr);
          expect(ridge.peak).toBeGreaterThanOrEqual(8);
          expect(ridge.visible).toBeGreaterThanOrEqual(8);
        }
      }
    });

    test("decorative light meets both edges while visible and partial stars remain named and clickable", async ({ page }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
      await relationships(page);
      for (const edge of ["bottom", "top"] as const) {
        await panNode(page, "preview-purpose-0", edge, -60);
        const geometry = await page.evaluate(() => ({ canvas: document.querySelector(".graph-viewport canvas")!.getBoundingClientRect().toJSON(), frame: window.__graphQA.frame()! }));
        const measured = edgeLightPixels(screenshotPixels(await lightScreenshot(page)), viewport.dpr, geometry.canvas, geometry.frame, edge);
        console.info("WebGL light sample", JSON.stringify({ viewport, edge, ...measured }));
        expect(measured.samples, `${edge}: non-semantic edge samples`).toBeGreaterThanOrEqual(100);
        expect(measured.innerPeak, `${edge}: light remains visible inside`).toBeGreaterThanOrEqual(8);
        expect(measured.max, `${edge}: actual decorative cutoff`).toBeLessThanOrEqual(3);
      }
      for (const edge of ["bottom", "top"] as const) {
        await relationships(page);
        await panNode(page, "preview-0-0", edge, edge === "top" ? 2 : 12);
        const p = await nodePoint(page, "preview-0-0");
        expect(p.star.hitRadius * 2).toBeGreaterThanOrEqual(36);
        await page.mouse.move(p.x, p.y);
        await expect(page.locator('.graph-tooltip[data-node-id="preview-0-0"]')).toContainText("사용자의 맥락을 이해하는 방법");
        const shot = screenshotPixels(await page.screenshot());
        expect(Math.min(...shot.rgb(p.x * viewport.dpr, p.y * viewport.dpr)), `${edge}: visible star core`).toBeGreaterThan(100);
        await clickDocument(page, "preview-0-0");
        if (edge === "bottom" && viewport.width > 900) {
          const current = await nodePoint(page, "preview-0-0");
          await panNode(page, "preview-0-0", "bottom", Math.max(12, current.star.radius + 2));
          await expect(page.locator('.node-label[data-node-id="preview-0-0"]:not([hidden])')).toContainText("사용자의 맥락을 이해하는 방법");
        }
      }
      expect(errors).toEqual([]);
    });

    test("guidance stays outside the renderer after selection, pan and minimap expansion", async ({ page }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await page.goto("/map-preview.html?sample=sparse");
      await page.locator(".fixture-notice").evaluate(element => element.setAttribute("hidden", ""));
      await settled(page);
      if (viewport.width > 900) {
        await page.getByRole("button", { name: "목록 보기", exact: true }).click();
        await page.locator(".graph-list > button").filter({ has: page.getByText("synthetic/제품 전략과 의사결정/0.md", { exact: true }) }).click();
        await page.getByRole("button", { name: "3D 보기", exact: true }).click();
        await expect(page.locator(".management-panel")).toBeVisible();
        await settled(page);
        const move = await page.evaluate(() => {
          const star = window.__graphQA.frame()!.stars.find(star => star.id === "preview-0-0")!;
          const canvas = document.querySelector(".graph-viewport canvas")!.getBoundingClientRect();
          const help = document.querySelector(".graph-quick-help")!.getBoundingClientRect();
          return { x: canvas.x + 12, y: canvas.y + 12, dx: help.x + help.width / 2 - canvas.x - star.x, dy: help.y + help.height / 2 - canvas.y - star.y };
        });
        await page.mouse.move(move.x, move.y); await page.mouse.down();
        await page.mouse.move(move.x + move.dx, move.y + move.dy, { steps: 12 }); await page.mouse.up();
        await settled(page);
      }
      const expand = page.getByRole("button", { name: "미니맵 펼치기", exact: true });
      if (await expand.count()) await expand.click();
      const geometry = await page.evaluate(() => {
        const canvas = document.querySelector(".graph-viewport canvas")!.getBoundingClientRect();
        const plot = document.querySelector(".graph-viewport")!.getBoundingClientRect();
        const help = document.querySelector(".graph-quick-help")!.getBoundingClientRect();
        return { outside: help.top >= canvas.bottom - .5,
          rectError: Math.max(...(["left", "top", "width", "height"] as const).map(key => Math.abs(canvas[key] - plot[key]))),
          width: plot.width, height: plot.height,
          bodies: window.__graphQA.frame()!.stars.filter(star => star.opacity > .01 && star.x > 0 && star.y > 0 && star.x < canvas.width && star.y < canvas.height).length };
      });
      expect(geometry.outside).toBe(true);
      expect(geometry.rectError).toBeLessThanOrEqual(1);
      expect(Math.min(geometry.width, geometry.height)).toBeGreaterThanOrEqual(200);
      expect(geometry.bodies).toBeGreaterThan(0);
    });

    test("animated titles share the camera and object positions of the rendered frame", async ({ page }) => {
      await page.emulateMedia({ reducedMotion: "no-preference" });
      const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
      await page.goto("/map-preview.html");
      await page.locator(".fixture-notice").evaluate(element => element.setAttribute("hidden", ""));
      await settled(page);
      const purpose = page.locator('.graph-group-target[data-node-id="preview-purpose-1"]');
      await purpose.focus(); await purpose.press("Enter"); await settled(page);
      const expand = page.getByRole("button", { name: "미니맵 펼치기", exact: true });
      if (await expand.count()) await expand.click();
      await page.evaluate(() => window.__graphQA.start());
      for (const name of ["확대", "확대", "축소", "축소"]) {
        await page.getByRole("button", { name, exact: true }).click();
        await page.waitForTimeout(300);
      }
      const result = await page.evaluate(() => window.__graphQA.stop());
      expect(result.frames).toBeGreaterThanOrEqual(8);
      expect(result.movingFrames).toBeGreaterThanOrEqual(2);
      expect(result.minLabels).toBeGreaterThanOrEqual(1);
      expect(result.samples).toBeGreaterThanOrEqual(10);
      expect(result.maxError).toBeLessThanOrEqual(1);
      await settled(page);
      expect(errors).toEqual([]);
    });
  });
}
