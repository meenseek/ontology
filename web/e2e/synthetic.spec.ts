import { expect, test, type Page } from "playwright/test";

async function passed(page: Page) {
  const result = page.locator("#result");
  await expect(result).toHaveText(/^(PASS|FAIL)\b[\s\S]*/, { timeout: 80_000 });
  await expect(result).toHaveText(/^PASS\b[\s\S]*/);
}

// The fixture pages own synthetic API responses. Any request escaping those
// fixtures must fail here, including EventSource and requests to the owner's app.
test.beforeEach(async ({ context }) => {
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin === "http://127.0.0.1:47839" && !url.pathname.startsWith("/api/")) {
      return route.continue();
    }
    return route.abort("blockedbyclient");
  });
});

const viewports = [
  { width: 1440, height: 900 },
  { width: 1180, height: 900 },
  { width: 920, height: 900 },
  { width: 390, height: 844 },
];

for (const viewport of viewports) {
  test.describe(`${viewport.width}px`, () => {
    test.use({ viewport });

    test("expanded relationship titles and actual help bounds", async ({ page }) => {
      await page.goto("/app-reading-check.html?scenario=group-labels-help");
      await passed(page);
    });

    test("purpose detail, paging and available map boundaries", async ({ page }) => {
      await page.goto("/app-reading-check.html?scenario=residual");
      // Some scenarios hide their diagnostic output to preserve the app viewport.
      await passed(page);
    });

    if (![1440, 390].includes(viewport.width)) return;

    for (const scenario of ["records", "documents", "related-materials"]) {
      test(`App ${scenario}`, async ({ page }) => {
        await page.goto(`/app-reading-check.html?scenario=${scenario}`);
        await passed(page);
      });
    }

    test("reading components and synthetic request bounds", async ({ page }) => {
      await page.goto("/reading-check.html");
      await passed(page);
    });

    for (const check of ["activation", "activation-extra", "interface"]) {
      test(`map ${check}`, async ({ page }) => {
        await page.goto(`/map-preview.html?check=${check}`);
        const result = page.locator(".fixture-notice");
        await expect(result).toHaveAttribute("data-result", /"passed":(true|false)/, { timeout: 80_000 });
        expect(JSON.parse((await result.getAttribute("data-result"))!)).toMatchObject({ passed: true });
      });
    }

    test("trusted pointer, panel and filter keyboard flow", async ({ page }) => {
      await page.goto("/app-reading-check.html?scenario=residual-mouse");
      const result = page.locator("#result");
      await expect(result).toHaveText(/^AWAITING_MOUSE\n[\s\S]*/, { timeout: 80_000 });
      const point = JSON.parse((await result.textContent())!.split("\n")[1]);
      await page.mouse.click(point.x, point.y);
      await passed(page);
      await expect(page.locator(".graph-core-actions")).toBeVisible();
      await expect(page.locator(".management-panel")).toHaveCount(0);

      await page.goto("/map-preview.html");
      const filters = page.getByRole("button", { name: "필터·묶음", exact: true });
      await filters.click();
      await expect(page.locator(".map-sidebar")).toBeVisible();
      if (viewport.width <= 900) {
        await page.keyboard.press("Escape");
        await expect(filters).toBeFocused();
      } else {
        await page.getByRole("button", { name: "탐색 조건 닫기", exact: true }).click();
      }
      await expect(page.locator(".map-sidebar")).toHaveCount(0);
      await page.getByRole("button", { name: "목록 보기", exact: true }).click();
      await page.locator(".graph-list > button").filter({
        has: page.getByText("제품 전략과 의사결정", { exact: true }),
      }).click();
      await expect(page.locator(".management-panel h2")).toHaveText("제품 전략과 의사결정");
      await page.getByRole("button", { name: "관리 패널 닫기", exact: true }).click();
      await expect(page.locator(".management-panel")).toHaveCount(0);
    });
  });
}

// The narrowest supported screen must protect the name strip as well.
test("360px expanded relationship titles and actual help bounds", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto("/app-reading-check.html?scenario=group-labels-help");
  await passed(page);
});
