import { defineConfig } from "playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: true,
  timeout: 90_000,
  globalTimeout: 15 * 60_000,
  reporter: [["list"], ["json", { outputFile: "test-results/results.json" }]],
  outputDir: "test-results/artifacts",
  use: {
    browserName: "chromium",
    baseURL: "http://127.0.0.1:47839",
    locale: "ko-KR",
    serviceWorkers: "block",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    video: "off",
    launchOptions: { args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] },
  },
  webServer: {
    command: "pnpm exec vite --host 127.0.0.1 --port 47839 --strictPort",
    url: "http://127.0.0.1:47839/map-preview.html",
    reuseExistingServer: false,
    timeout: 30_000,
    gracefulShutdown: { signal: "SIGTERM", timeout: 5_000 },
    stdout: "pipe",
    stderr: "pipe",
  },
});
