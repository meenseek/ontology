import { test, expect, type Page } from "playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { createMapFixture } from "../src/map-fixture";
import { installGraphProbe, settledFramesMatch } from "./graph-probe";

const observations: Record<string, unknown> = {};
const originalMobileMembers = ["14", "12", "10", "13", "11", "0", "17", "18", "15", "1", "19"].map(id => `preview-1-${id}`);

async function settled(page: Page) {
  await page.evaluate(() => document.fonts.ready);
  let previous = "", serial = -1, stable = 0;
  await expect.poll(async () => {
    const frame = await page.evaluate(() => window.__graphQA?.frame());
    if (!frame?.stars.length || frame.serial === serial) return stable;
    serial = frame.serial;
    const current = JSON.stringify({ ...frame, serial: 0 });
    stable = settledFramesMatch(previous, current) ? stable + 1 : 0;
    previous = current;
    return stable;
  }, { timeout: 30_000, intervals: [100] }).toBeGreaterThanOrEqual(5);
}

async function openMap(page: Page, longTitles = false, fontFamily?: string) {
  const fixture = createMapFixture(false), errors: string[] = [];
  if (longTitles) {
    fixture.nodes.find(node => node.id === "preview-1-0")!.title = "사용자의 맥락을 이해하는 방법 — 첫 번째 기록";
    fixture.nodes.find(node => node.id === "preview-1-12")!.title = "사용자의 맥락을 이해하는 방법 — 두 번째 기록";
  }
  await page.addInitScript(installGraphProbe);
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    if (url.origin !== "http://127.0.0.1:47839") {
      errors.push(`Unexpected external request: ${url.origin}`);
      return route.abort("blockedbyclient");
    }
    if (!url.pathname.startsWith("/api/")) return route.continue();
    const response = fixture.respond(url, route.request().postData());
    if (!response.ok) errors.push(`Unexpected synthetic request: ${url.pathname}`);
    return route.fulfill({ status: response.status, contentType: "application/json", body: await response.text() });
  });
  if (fontFamily) await page.addInitScript(font => {
    document.addEventListener("DOMContentLoaded", () => { document.documentElement.style.fontFamily = font; }, { once: true });
  }, fontFamily);
  await page.goto("/");
  await expect(page.locator(".graph-viewport canvas[data-engine]")).toBeVisible();
  await settled(page);
  return errors;
}

async function expand(page: Page) {
  const group = page.locator('.graph-group-target[data-node-id="preview-purpose-1"]');
  await group.focus();
  await group.press("Enter");
  await expect(page.getByRole("button", { name: "묶음 접기", exact: true })).toBeVisible();
  await settled(page);
}

async function titles(page: Page, ids: string[]) {
  return page.evaluate(ids => ids.map(id => {
    const label = document.querySelector<HTMLElement>(`.node-label[data-node-id="${id}"]:not([hidden])`);
    const strong = label?.querySelector<HTMLElement>("strong");
    if (!label || !strong || +label.style.opacity < .99) return { id, visible: false, unclipped: false };
    return { id, visible: true, unclipped: strong.scrollWidth <= strong.clientWidth + 1 && strong.scrollHeight <= strong.clientHeight + 1,
      text: strong.textContent, width: label.getBoundingClientRect().width, height: label.getBoundingClientRect().height };
  }), ids);
}

test.use({ reducedMotion: "reduce" });
test.afterAll(({}, info) => {
  const directory = process.env.ONTOLOGY_POLISH_OBSERVATIONS;
  if (!directory) return;
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  writeFileSync(join(directory, `worker-${info.workerIndex}.json`), JSON.stringify(observations, null, 2) + "\n", { flag: "wx", mode: 0o600 });
});

for (const width of [1440, 920, 390]) test(`shared chrome inset ${width}px survives disclosure and filters`, async ({ page }) => {
  await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
  const errors = await openMap(page);
  await expand(page);
  const bounds = await page.evaluate(() => {
    const box = (selector: string) => document.querySelector(selector)!.getBoundingClientRect().toJSON();
    return { plot: box(".graph-viewport"), actions: box(".graph-core-actions"), minimap: box(".graph-minimap"),
      toolbar: box(".map-toolbar > :last-child"), gutter: parseFloat(getComputedStyle(document.querySelector(".map-workspace")!).getPropertyValue("--map-gutter")) };
  });
  const right = bounds.plot.right - bounds.gutter;
  expect(Math.abs(bounds.actions.right - right)).toBeLessThanOrEqual(.01);
  expect(Math.abs(bounds.minimap.right - right)).toBeLessThanOrEqual(.01);
  if (width > 700) expect(Math.abs(bounds.toolbar.right - right)).toBeLessThanOrEqual(.01);
  observations[`alignment-${width}`] = { ...bounds, right_edge_match: Math.abs(bounds.minimap.right - right) <= .01
    && Math.abs(bounds.actions.right - right) <= .01 && (width <= 700 || Math.abs(bounds.toolbar.right - right) <= .01) };
  await page.getByRole("button", { name: "필터·묶음", exact: true }).click();
  await expect(page.locator(".map-sidebar")).toBeVisible();
  if (width > 900) {
    const free = await page.evaluate(() => ({ sidebar: document.querySelector(".map-sidebar")!.getBoundingClientRect().right,
      actions: document.querySelector(".graph-core-actions")!.getBoundingClientRect().left }));
    expect(free.actions).toBeGreaterThanOrEqual(free.sidebar + bounds.gutter - .01);
  }
  await page.getByRole("button", { name: "탐색 조건 닫기", exact: true }).click();
  await expect(page.getByRole("button", { name: "필터·묶음", exact: true })).toBeFocused();
  await settled(page);
  expect(errors).toEqual([]);
});

for (const width of [360, 390, 414]) test(`mobile hierarchy ${width}px keeps search, scope, record and full member titles`, async ({ page }) => {
  await page.setViewportSize({ width, height: 844 });
  const errors = await openMap(page);
  const header = await page.locator(".app-header").boundingBox();
  expect(header!.height).toBeLessThanOrEqual(112);
  for (const button of await page.locator(".app-header button").all()) {
    await expect(button).toBeVisible();
    const rect = await button.boundingBox();
    expect(rect!.height).toBeGreaterThanOrEqual(44);
    expect(rect!.x).toBeGreaterThanOrEqual(0);
    expect(rect!.x + rect!.width).toBeLessThanOrEqual(width);
  }
  const input = page.getByRole("textbox", { name: "현재 범위의 지식 검색" });
  await input.fill("맥락");
  await input.press("Enter");
  await expect(page.locator(".map-title h1")).toContainText("맥락");
  await page.getByRole("button", { name: "검색 해제", exact: true }).click();
  await page.getByRole("button", { name: "meenseek 사업", exact: true }).click();
  await expect(page.getByRole("button", { name: "meenseek 사업", exact: true })).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "내 지식", exact: true }).click();
  await settled(page);
  await expand(page);
  const actions = await page.locator(".graph-core-actions").boundingBox();
  expect(actions!.height).toBeLessThanOrEqual(96);
  const observed = await titles(page, originalMobileMembers);
  if (width === 390) {
    expect(observed.every(title => title.visible && title.unclipped)).toBe(true);
    observations["header-390"] = { compact_header: header!.height <= 112, height_css_px: header!.height };
    observations["titles-390"] = { all_fixture_titles_visible: observed.every(title => title.visible && title.unclipped), titles: observed };
  }
  observations[`mobile-${width}`] = { header, actions, titles: observed };
  for (const button of await page.locator(".graph-core-actions button").all()) {
    const rect = await button.boundingBox();
    expect(rect!.height).toBeGreaterThanOrEqual(44);
    expect(rect!.x).toBeGreaterThanOrEqual(0);
    expect(rect!.x + rect!.width).toBeLessThanOrEqual(width);
  }
  await page.getByRole("button", { name: "다음", exact: true }).click();
  await expect(page.locator(".graph-core-page")).toHaveText("2/4");
  await page.getByRole("button", { name: "이전", exact: true }).click();
  await expect(page.locator(".graph-core-page")).toHaveText("1/4");
  await page.getByRole("button", { name: "묶음 접기", exact: true }).click();
  await expect(page.locator(".graph-core-actions")).toHaveCount(0);
  await page.getByRole("button", { name: "기록 남기기", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "기록 남기기" })).toBeVisible();
  await page.getByRole("button", { name: "관리 패널 닫기", exact: true }).click();
  await expect(page.getByRole("button", { name: "기록 남기기", exact: true })).toBeFocused();
  expect(errors).toEqual([]);
});

for (const fontFamily of [undefined, 'Arial, "AppleGothic", sans-serif']) test(`mobile shared-prefix titles retain their different endings${fontFamily ? " with wider fallback text" : ""}`, async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const errors = await openMap(page, true, fontFamily);
  await expand(page);
  const observed = await titles(page, ["preview-1-0", "preview-1-12"]);
  expect(observed.every(title => title.visible && title.unclipped)).toBe(true);
  expect(observed[0].text).toContain("첫 번째 기록");
  expect(observed[1].text).toContain("두 번째 기록");
  observations[fontFamily ? "shared-prefix-wide-font" : "shared-prefix"] = observed;
  expect(errors).toEqual([]);
});

test("list dates stay beside their item metadata through sort changes", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const errors = await openMap(page);
  await page.getByRole("button", { name: "목록 보기", exact: true }).click();
  const date = page.locator(".graph-list .node-date").first();
  await expect(date).toBeVisible();
  const separation = await date.evaluate(element => {
    const metadata = element.closest(".node-meta"), box = element.getBoundingClientRect();
    if (!metadata) return Infinity;
    const preceding = document.createRange();
    preceding.setStart(metadata, 0); preceding.setEndBefore(element);
    return Math.hypot(box.left - preceding.getBoundingClientRect().right, box.top - preceding.getBoundingClientRect().top);
  });
  expect(separation).toBeLessThanOrEqual(16);
  const sort = page.getByRole("combobox", { name: "표시 항목 정렬" });
  await sort.selectOption("created");
  await expect(date).toHaveText("추가일 미확인");
  await sort.selectOption("name");
  await expect(page.locator(".graph-list .node-date")).toHaveCount(0);
  observations["list"] = { date_separation_css_px: separation };
  expect(errors).toEqual([]);
});

test("150 percent mobile text reflows without losing header or disclosure controls", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const errors = await openMap(page);
  await expand(page);
  await page.evaluate(() => {
    const elements = [...document.querySelectorAll<HTMLElement>(".app-header *, .graph-core-actions *")];
    const sizes = elements.map(element => parseFloat(getComputedStyle(element).fontSize));
    elements.forEach((element, index) => { element.style.fontSize = `${sizes[index] * 1.5}px`; });
  });
  await settled(page);
  for (const selector of [".app-header", ".graph-core-actions"]) {
    const overflow = await page.locator(selector).evaluate(element => element.scrollWidth > element.clientWidth + 1);
    expect(overflow).toBe(false);
  }
  for (const button of await page.locator(".app-header button, .graph-core-actions button").all()) {
    await expect(button).toBeVisible();
    const rect = await button.boundingBox();
    expect(rect!.x).toBeGreaterThanOrEqual(0);
    expect(rect!.x + rect!.width).toBeLessThanOrEqual(390);
  }
  await page.getByRole("button", { name: "다음", exact: true }).click();
  await expect(page.locator(".graph-core-page")).toHaveText("2/4");
  expect(errors).toEqual([]);
});
