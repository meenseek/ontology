import { expect, test } from "playwright/test";
import { luminance, screenshotPixels } from "./png";

// BEGIN rendered graph observer — also exercised against the installed bundle.
function installGraphProbe() {
  type Frame = { serial: number; camera: number[]; stars: { id: string; x: number; y: number; opacity: number; sprites: { pixels: number; stroke: number; opacity: number; color: string }[] }[] };
  let frame: Frame | undefined, serial = 0, running = false;
  let report = { frames: 0, movingFrames: 0, minLabels: Infinity, samples: 0, maxError: 0 };
  // Three's public devtools observation exposes the actual scene without replacing
  // its renderer, textures or application modules. This works in built assets too.
  const devtools = new EventTarget();
  (window as Window & { __THREE_DEVTOOLS__?: EventTarget }).__THREE_DEVTOOLS__ = devtools;
  devtools.addEventListener("observe", event => {
    const scene = (event as CustomEvent).detail as import("three").Scene;
    if (!scene.isScene) return;
    const previous = scene.onAfterRender;
    scene.onAfterRender = function (...args) {
      previous.apply(this, args);
      const [renderer, , camera] = args, rect = renderer.domElement.getBoundingClientRect();
      const stars: Frame["stars"] = [];
      scene.traverse(object => {
        if (!object.userData.nodeId || !object.visible) return;
        // Read the matrices just rendered; getWorldPosition would update them.
        const m = object.matrixWorld.elements, view = camera.matrixWorldInverse.elements, p = camera.projectionMatrix.elements;
        const x = m[12], y = m[13], z = m[14];
        const vx = view[0] * x + view[4] * y + view[8] * z + view[12];
        const vy = view[1] * x + view[5] * y + view[9] * z + view[13];
        const vz = view[2] * x + view[6] * y + view[10] * z + view[14];
        const w = p[3] * vx + p[7] * vy + p[11] * vz + p[15];
        if (w <= 0) return;
        const sprites: Frame["stars"][number]["sprites"] = [];
        for (const child of object.children) {
          const sprite = child as import("three").Sprite, material = sprite.material, image = material?.map?.image;
          if (!sprite.isSprite || !(image instanceof HTMLCanvasElement) || image.width !== 128) continue;
          const alpha = image.getContext("2d")!.getImageData(0, 64, image.width, 1).data;
          let ink = 0;
          for (let i = 3; i < alpha.length; i += 4) ink += alpha[i] / 255;
          const pixels = sprite.scale.x * p[5] * rect.height / 2;
          sprites.push({ pixels, stroke: ink / 2 * pixels / image.width, opacity: material.opacity, color: material.color.getHexString() });
        }
        stars.push({ id: object.userData.nodeId, x: ((p[0] * vx + p[4] * vy + p[8] * vz + p[12]) / w + 1) * rect.width / 2,
          y: (1 - (p[1] * vx + p[5] * vy + p[9] * vz + p[13]) / w) * rect.height / 2,
          opacity: object.userData.opacity?.() ?? 1, sprites });
      });
      frame = { serial: ++serial, camera: camera.matrixWorld.toArray(), stars };
    };
  });
  const start = () => {
    running = true;
    report = { frames: 0, movingFrames: 0, minLabels: Infinity, samples: 0, maxError: 0 };
    let lastSerial = -1, previousCamera: number[] | undefined;
    const sample = () => {
      if (!running) return;
      const canvas = document.querySelector(".graph-viewport canvas");
      if (frame && canvas && frame.serial !== lastSerial) {
        lastSerial = frame.serial;
        report.frames++;
        if (previousCamera && frame.camera.some((n, i) => Math.abs(n - previousCamera![i]) > .00001)) report.movingFrames++;
        previousCamera = frame.camera;
        const rect = canvas.getBoundingClientRect(), stars = new Map(frame.stars.map(star => [star.id, star]));
        const labels = [...document.querySelectorAll<HTMLElement>(".node-label:not([hidden])")].filter(label => +label.style.opacity > .99);
        report.minLabels = Math.min(report.minLabels, labels.length);
        for (const label of labels) {
          const star = stars.get(label.dataset.nodeId!);
          if (!star || star.opacity < .99 || star.x < 0 || star.y < 0 || star.x > rect.width || star.y > rect.height ||
            parseFloat(label.style.getPropertyValue("--leader-length")) > 0 || label.classList.contains("routed-label")) continue;
          const box = label.getBoundingClientRect();
          const x = box.left - rect.left + parseFloat(label.style.getPropertyValue("--leader-x"));
          const y = box.top - rect.top + parseFloat(label.style.getPropertyValue("--leader-y"));
          report.samples++;
          report.maxError = Math.max(report.maxError, Math.hypot(x - star.x, y - star.y));
        }
      }
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  };
  window.__graphQA = { frame: () => frame, start, stop: () => { running = false; return report; } };
}
declare global { interface Window { __graphQA: { frame: () => { serial: number; camera: number[]; stars: { id: string; x: number; y: number; opacity: number; sprites: { pixels: number; stroke: number; opacity: number; color: string }[] }[] } | undefined; start: () => void; stop: () => { frames: number; movingFrames: number; minLabels: number; samples: number; maxError: number } } } }
// END rendered graph observer

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

    test("spatial member rings stay thin without weakening taxonomy markers", async ({ page }) => {
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
      expect(members.length).toBeGreaterThanOrEqual(6);
      expect(markers.length).toBeGreaterThan(0);
      for (const member of members) {
        expect(member.stroke).toBeGreaterThanOrEqual(.4);
        expect(member.stroke).toBeLessThanOrEqual(.75);
        expect(member.opacity).toBeGreaterThan(.5);
      }
      expect(new Set(members.map(member => member.color)).size).toBe(1);
      for (const marker of markers) expect(marker.stroke).toBeGreaterThanOrEqual(1.2);
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
