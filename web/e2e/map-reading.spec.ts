import { expect, test, type Page } from "playwright/test";
import { installGraphProbe, settledFramesMatch } from "./graph-probe";

async function settled(page: Page) {
  await page.evaluate(() => document.fonts.ready);
  let previous = "", sequence = "", stable = 0;
  await expect.poll(async () => {
    const value = await page.locator(".fixture-notice").evaluate(e => ({ ...(e as HTMLElement).dataset }));
    if (!value.frame || sequence === value.frameSequence) return stable;
    sequence = value.frameSequence!;
    const motion = JSON.parse(value.motion ?? "[]").at(-1);
    stable = settledFramesMatch(previous, value.frame) && motion && !motion.layout && !motion.dragging && !motion.settling ? stable + 1 : 0;
    previous = value.frame;
    return stable;
  }, { timeout: 25_000, intervals: [100] }).toBeGreaterThanOrEqual(3);
}

// Measure real word fragments, rather than inferring wrapping from element
// height or the presence of a CSS declaration. This also catches clipped ink.
async function reading(page: Page) {
  return page.evaluate(() => {
    const q = (s: string) => document.querySelector<HTMLElement>(s)!;
    const help = q(".graph-quick-help"), guidance = q(".graph-guidance"), plot = q(".graph-viewport");
    const active = [...help.children].find(e => getComputedStyle(e).display !== "none")!;
    const lines: { top: number; words: string[]; left: number; right: number; bottom: number }[] = [];
    const walker = document.createTreeWalker(active, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      for (const match of node.textContent!.matchAll(/\S+/g)) {
        const range = document.createRange(); range.setStart(node, match.index!); range.setEnd(node, match.index! + match[0].length);
        for (const rect of range.getClientRects()) {
          let line = lines.find(l => Math.abs(l.top - rect.top) < 2);
          if (!line) { line = { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right, words: [] }; lines.push(line); }
          line.words.push(match[0]); line.left = Math.min(line.left, rect.left); line.right = Math.max(line.right, rect.right); line.bottom = Math.max(line.bottom, rect.bottom);
        }
      }
    }
    const inset = (e: HTMLElement) => e.getBoundingClientRect().left + parseFloat(getComputedStyle(e).paddingLeft);
    const plotBox = plot.getBoundingClientRect(), helpBox = help.getBoundingClientRect(), box = guidance.getBoundingClientRect();
    const side = document.querySelector(".explorer-open .map-sidebar")?.getBoundingClientRect();
    const offset = side && innerWidth > 900 ? side.right - plotBox.left : 0;
    // Pinned purpose cards sit directly below the filter row, so they join the
    // shared column whenever they are painted.
    const strip = document.querySelector<HTMLElement>(".graph-group-navigation.with-keys"), card = strip?.querySelector(".graph-group-target.core-name")?.getBoundingClientRect();
    const starts = [inset(q(".map-toolbar")), inset(q(".map-status")), helpBox.left - offset, inset(q(".map-legend")), ...(card ? [card.left + strip!.scrollLeft - offset] : [])];
    const css = getComputedStyle(guidance), lineHeight = parseFloat(getComputedStyle(active).lineHeight);
    return { text: active.textContent!, lines, lineHeight, fontSize: parseFloat(getComputedStyle(active).fontSize),
      guideHeight: box.height, lineBudget: lineHeight * lines.length + parseFloat(css.paddingTop) + parseFloat(css.paddingBottom),
      starts, insetError: Math.max(...starts) - Math.min(...starts), plotHeight: plotBox.height,
      cardStrip: !!card,
      outside: box.top >= plotBox.bottom - .5, remainingSpaceError: Math.abs(plotBox.bottom - box.top),
      visible: lines.every(l => l.left >= helpBox.left - 1 && l.right <= helpBox.right + 1 && l.top >= box.top && l.bottom <= box.bottom),
      canvasError: Math.max(...(["left", "top", "width", "height"] as const).map(k => Math.abs(q(".graph-viewport canvas").getBoundingClientRect()[k] - plotBox[k]))) };
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(installGraphProbe);
  await page.route("**/*", route => new URL(route.request().url()).origin === "http://127.0.0.1:47839" ? route.continue() : route.abort("blockedbyclient"));
  await page.emulateMedia({ reducedMotion: "reduce" });
});

test.describe("touch guidance reading budget", () => {
  test.use({ hasTouch: true, isMobile: true, deviceScaleFactor: 2, viewport: { width: 390, height: 844 } });
  test("complete gestures keep one normal row across phone widths and wrap fully with enlarged text", async ({ page }) => {
    await page.goto("/map-preview.html?sample=sparse");
    await page.locator(".fixture-notice").evaluate(e => (e as HTMLElement).hidden = true);
    for (const width of [390, 375, 360, 414]) {
      await page.setViewportSize({ width, height: 844 }); await settled(page);
      const value = await reading(page);
      expect(value.text).toMatch(/빈 (곳|공간).*끌어 이동.*두 손가락.*확대·축소.*묶음.*(누르면|눌러).*펼/);
      expect(value.lines, `${width}px: a final orphan must not steal a map row`).toHaveLength(1);
      expect(value.fontSize).toBe(11); expect(value.visible).toBe(true);
      expect(value.guideHeight).toBeCloseTo(value.lineBudget, 0);
      expect(value.insetError).toBeLessThanOrEqual(1);
      expect(value.outside).toBe(true); expect(value.remainingSpaceError).toBeLessThanOrEqual(1); expect(value.canvasError).toBeLessThanOrEqual(1);
      expect(value.plotHeight).toBeGreaterThanOrEqual(200);
    }
    // A font with wider Korean advances must not move the original orphan to
    // a different device. Exercise the actual DOM, without shrinking the type.
    await page.setViewportSize({ width: 360, height: 844 });
    await page.locator(".graph-quick-help").evaluate(e => (e as HTMLElement).style.fontFamily = "AppleGothic, sans-serif");
    await settled(page);
    const alternateFont = await reading(page);
    expect(alternateFont.lines, "wider Korean glyphs keep the normal mobile map row").toHaveLength(1);
    expect(alternateFont.fontSize).toBe(11); expect(alternateFont.visible).toBe(true);
    expect(alternateFont.guideHeight).toBeCloseTo(alternateFont.lineBudget, 0);
    await test.info().attach("font-transfer-guidance", { body: await page.screenshot(), contentType: "image/png" });
    await page.locator(".graph-quick-help").evaluate(e => (e as HTMLElement).style.removeProperty("font-family"));
    await page.setViewportSize({ width: 360, height: 844 });
    await page.locator(".graph-quick-help").evaluate(e => (e as HTMLElement).style.fontSize = "16.5px");
    await settled(page);
    const enlarged = await reading(page);
    expect(enlarged.fontSize).toBe(16.5); expect(enlarged.lines.length).toBeGreaterThan(1);
    expect(enlarged.lines.at(-1)!.words.length).toBeGreaterThan(1);
    expect(enlarged.visible).toBe(true); expect(enlarged.guideHeight).toBeCloseTo(enlarged.lineBudget, 0);
    expect(enlarged.canvasError).toBeLessThanOrEqual(1); expect(enlarged.plotHeight).toBeGreaterThanOrEqual(200);
    await test.info().attach("enlarged-touch-guidance", { body: await page.screenshot(), contentType: "image/png" });
  });
});

test("map chrome shares its inset at responsive boundaries, including the open explorer", async ({ page }) => {
  await page.goto("/map-preview.html?sample=sparse");
  await page.locator(".fixture-notice").evaluate(e => (e as HTMLElement).hidden = true);
  for (const width of [701, 900, 920, 1180, 1200, 1201, 1440]) {
    await page.setViewportSize({ width, height: 900 }); await settled(page);
    const value = await reading(page);
    expect(value.insetError, `${width}px: toolbar/status/guidance/legend start together`).toBeLessThanOrEqual(1);
    expect(value.visible).toBe(true); expect(value.canvasError).toBeLessThanOrEqual(1);
  }
  for (const width of [1180, 920]) {
    await page.setViewportSize({ width, height: 900 }); await settled(page);
    await page.getByRole("button", { name: "필터·묶음", exact: true }).click(); await settled(page);
    const value = await reading(page);
    expect(value.insetError, `${width}px: explorer offset uses the same inset`).toBeLessThanOrEqual(1);
    expect(value.visible).toBe(true);
    await page.locator(".map-sidebar .sidebar-heading button").click(); await settled(page);
  }
});

// Pinned purpose cards appear whenever the map column is at most 700px wide.
// The row scrolls, so it runs to the map edge. Scrolling to the end stops the
// last card on the shared inset, and a focused card cut by that inset is moved
// inside it; the browser may align it to the inset or centre it. The previous
// scroll position is restored for later measurements.
async function cardRow(page: Page) {
  return page.evaluate(() => {
    const strip = document.querySelector<HTMLElement>(".graph-group-navigation.with-keys"), plot = document.querySelector(".graph-viewport")!.getBoundingClientRect();
    const toolbar = document.querySelector<HTMLElement>(".map-toolbar")!, inner = toolbar.getBoundingClientRect().right - parseFloat(getComputedStyle(toolbar).paddingRight);
    if (!strip) return null;
    const cards = [...strip.querySelectorAll<HTMLElement>(".graph-group-target.core-name")], previous = strip.scrollLeft;
    strip.scrollLeft = 0;
    const cut = cards.find(card => card.getBoundingClientRect().right > inner + 1);
    cut?.focus();
    const focused = cut?.getBoundingClientRect(), focusOverflow = focused ? Math.max(0, focused.right - inner, strip.getBoundingClientRect().left - focused.left) : null;
    cut?.blur();
    strip.scrollLeft = strip.scrollWidth;
    const observed = { overflows: strip.scrollWidth > strip.clientWidth, edgeError: Math.abs(strip.getBoundingClientRect().right - plot.right),
      lastEndError: Math.abs(cards.at(-1)!.getBoundingClientRect().right - inner), focusOverflow };
    strip.scrollLeft = previous;
    return observed;
  });
}

// Each case is its own test so a failing boundary is named in the report.
async function expectCardRow(page: Page, label: string) {
  const value = await reading(page), edge = await cardRow(page);
  await test.info().attach("card-inset", { body: JSON.stringify({ label, cardStrip: value.cardStrip, starts: value.starts, edge }), contentType: "application/json" });
  expect(value.cardStrip, `${label}: pinned purpose cards are shown`).toBe(true);
  expect(value.insetError, `${label}: cards start with toolbar/status/guidance/legend`).toBeLessThanOrEqual(1);
  expect(edge!.overflows, `${label}: the case exercises a scrolling card row`).toBe(true);
  expect(edge!.edgeError, `${label}: the scrolling row reaches the map edge`).toBeLessThanOrEqual(1);
  expect(edge!.lastEndError, `${label}: the last card stops on the shared inset`).toBeLessThanOrEqual(1);
  expect(edge!.focusOverflow, `${label}: a focused cut card moves inside the shared inset`).toBeLessThanOrEqual(1);
}

test.describe("pinned purpose cards", () => {
  test.use({ hasTouch: true, isMobile: true, deviceScaleFactor: 2 });
  for (const width of [390, 360, 414, 700]) test(`${width}px cards start on the shared map inset and scroll to it`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.goto("/map-preview.html");
    await page.locator(".fixture-notice").evaluate(e => (e as HTMLElement).hidden = true);
    await settled(page);
    await expectCardRow(page, `${width}px`);
  });
});

// The record panel narrows a desktop map below 700px, where the inset is 26px
// or 20px rather than the phone value. At 1440px the explorer still docks
// beside the narrowed map, so the cards follow its offset.
for (const [width, explorer] of [[1440, false], [1000, false], [1440, true]] as const) test(`${width}px record panel${explorer ? " and explorer" : ""} keeps pinned cards on the shared map inset`, async ({ page }) => {
  await page.setViewportSize({ width, height: 900 });
  await page.goto("/map-preview.html");
  await page.locator(".fixture-notice").evaluate(e => (e as HTMLElement).hidden = true);
  await settled(page);
  await page.getByRole("button", { name: "기록 남기기", exact: true }).click(); await settled(page);
  if (explorer) {
    await page.getByRole("button", { name: "필터·묶음", exact: true }).click(); await settled(page);
    await expect(page.locator(".map-workspace.explorer-open"), "the explorer docks beside the map rather than covering it").toHaveCount(1);
  }
  await expectCardRow(page, `${width}px record panel${explorer ? " and explorer" : ""}`);
});

test("selected closeup retains both opposed titles with clear ownership and real picking", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/map-preview.html");
  await page.locator(".fixture-notice").evaluate(e => (e as HTMLElement).hidden = true);
  await settled(page);
  await page.getByRole("button", { name: "목록 보기", exact: true }).click();
  await page.locator(".graph-list > button").filter({ has: page.getByText("synthetic/보안 플랫폼 개발·검증과 사용자 경험을 함께 개선하기/0.md", { exact: true }) }).click();
  await page.getByRole("button", { name: "3D 보기", exact: true }).click(); await settled(page);
  const observed = await page.evaluate(() => {
    const frame = window.__graphQA.frame()!, plot = document.querySelector(".graph-viewport")!.getBoundingClientRect();
    const labels = [...document.querySelectorAll<HTMLElement>(".node-label:not([hidden])")].filter(e => +e.style.opacity > .99).map(e => {
      const rect = e.getBoundingClientRect(), star = frame.stars.find(s => s.id === e.dataset.nodeId)!;
      return { id: e.dataset.nodeId!, left: rect.left - plot.left, right: rect.right - plot.left, top: rect.top - plot.top, bottom: rect.bottom - plot.top,
        star, guided: e.classList.contains("offset-label") || e.classList.contains("routed-label") };
    });
    const pairs: string[][] = [];
    for (const [i, a] of labels.entries()) for (const b of labels.slice(i + 1)) {
      const overlap = Math.min(a.right, b.right) - Math.max(a.left, b.left), gap = Math.max(a.top, b.top) - Math.min(a.bottom, b.bottom);
      const inward = (a.star.x < a.left && b.star.x > b.right) || (b.star.x < b.left && a.star.x > a.right);
      if (!a.guided && !b.guided && inward && overlap >= Math.min(a.right - a.left, b.right - b.left) / 2 && gap >= 0 && gap <= Math.max(a.bottom - a.top, b.bottom - b.top)) pairs.push([a.id, b.id]);
    }
    return { labels, pairs };
  });
  expect(observed.labels.map(l => l.id)).toEqual(expect.arrayContaining(["preview-5-0", "preview-1-12"]));
  expect(observed.pairs, "individually nearest titles still need a legible pairwise owner cue").toEqual([]);
  await test.info().attach("selected-title-ownership", { body: await page.screenshot(), contentType: "image/png" });
  for (const id of ["preview-5-0", "preview-1-12"]) {
    // Picking another purpose changes the disclosed members. Each target is
    // exercised from the reported scene, rather than relying on a hidden star.
    await page.getByRole("button", { name: "목록 보기", exact: true }).click();
    await page.locator(".graph-list > button").filter({ has: page.getByText("synthetic/보안 플랫폼 개발·검증과 사용자 경험을 함께 개선하기/0.md", { exact: true }) }).click();
    await page.getByRole("button", { name: "3D 보기", exact: true }).click(); await settled(page);
    const p = await page.evaluate(id => { const star = window.__graphQA.frame()!.stars.find(s => s.id === id)!, plot = document.querySelector(".graph-viewport")!.getBoundingClientRect(); return { x: plot.left + star.x, y: plot.top + star.y }; }, id);
    await page.mouse.move(p.x, p.y);
    await expect(page.locator(`.graph-tooltip[data-node-id="${id}"]`)).toBeVisible();
    await page.mouse.click(p.x, p.y);
    await expect.poll(() => page.evaluate(() => new URLSearchParams(location.search).get("focus"))).toBe(id);
    await expect(page.locator(`.node-label[data-node-id="${id}"]:not([hidden]):not(.compact)`)).toBeVisible();
    await settled(page);
  }
});
