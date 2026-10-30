import { expect, test } from "playwright/test";
import { luminance, screenshotPixels } from "./png";

test.beforeEach(async ({ context }) => {
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
        const frame = JSON.parse(raw), canvas = document.querySelector(".graph-canvas")!.getBoundingClientRect();
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
        const canvas = document.querySelector(".graph-canvas")!.getBoundingClientRect();
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
