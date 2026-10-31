import { createHash } from "node:crypto";
import { readFile, lstat } from "node:fs/promises";
import { join } from "node:path";
import { screenshotPixels } from "./png.ts";

export const scenes = [1440, 920, 390].flatMap(width => ["selected", "expanded", "zoomed"].map(state => ({
  id: `${width}-${state}`, width, height: width === 390 ? 844 : 900, dpr: width === 390 ? 2 : 1, state,
})));
// Keep CSS raster and composition on the CPU; SwiftShader supplies actual WebGL.
// Validate actual graphics status at capture and comparison; flags alone are insufficient.
// Keep automatic frames: this Playwright shell does not complete CDP beginFrame.
// These options do not replace an exact PNG comparison.
// https://chromium.googlesource.com/chromium/src/+/HEAD/headless/lib/browser/command_line_handler.cc
export const captureLaunchArgs = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--run-all-compositor-stages-before-draw", "--disable-new-content-rendering-timeout", "--disable-image-animation-resync", "--disable-threaded-animation", "--disable-checker-imaging", "--force-color-profile=srgb", "--disable-gpu-compositing", "--disable-gpu-rasterization"] as const;
export const hash = (value: Buffer | string) => createHash("sha256").update(value).digest("hex");
export const canonical = (value: unknown): string => {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${canonical((value as Record<string, unknown>)[k])}`).join(",")}}`;
  return JSON.stringify(value);
};
export const requireValue: (value: unknown, message: string) => asserts value = (value, message) => { if (!value) throw new Error(message); };
export function validateGraphics(graphics: Record<string, string> | undefined) {
  requireValue(graphics?.gpu_compositing === "disabled_software" && graphics.rasterization === "disabled_software" && graphics.webgl === "enabled_readback", "Actual software graphics/WebGL environment is unavailable");
}
export type Capture = {
  build: { source: Record<string, string>; assets: Record<string, string> };
  fixture: string; observer: string; tool: string; environment: {
    browser: string; platform: string; devices: (string | number)[][]; locale: string; reducedMotion: string; launchArgs: string[]; graphics: Record<string, string>;
    runtime: { userAgent: string; renderer: string; fonts: (string | number)[][] }[];
  };
  scenes: { id: string; png: string; sha256: string; state: SceneState }[];
};
export type SceneState = {
  focus: string | null; expanded: string | null; panel: boolean; zoomStart: number[] | null; zoomClicks: number; zoomSteps: number[][];
  frame: { camera: number[]; stars: { id: string; opacity: number }[] };
  labels: { id?: string }[];
};
export function validateState(scene: typeof scenes[number], state: SceneState) {
  const camera = state?.frame?.camera;
  requireValue(camera?.length === 16 && camera.every(Number.isFinite) && state.frame.stars.some(s => s.id.startsWith("preview-1-") && s.opacity > .01), `${scene.id}: missing actual frame/member`);
  requireValue(state.labels.some(s => scene.state === "selected" ? s.id === "preview-1-0" : s.id?.startsWith("preview-1-")), `${scene.id}: required member title missing`);
  if (scene.state === "selected") requireValue(state.focus === "preview-1-0" && state.panel === true && state.zoomClicks === 0 && state.zoomStart === null && state.zoomSteps?.length === 0, `${scene.id}: wrong selected state`);
  else {
    requireValue(state.focus === null && state.panel === false && state.expanded?.includes("보안 플랫폼") && state.expanded.includes("39개"), `${scene.id}: wrong expanded purpose`);
    if (scene.state === "zoomed") {
      requireValue(state.zoomClicks === 2 && state.zoomStart?.length === 16 && state.zoomStart.every(Number.isFinite) && state.zoomSteps?.length === 2 && camera.some((value, i) => Math.abs(value - state.zoomStart![i]) > 1e-6), `${scene.id}: no actual two-click camera change`);
      let previous = state.zoomStart;
      for (const step of state.zoomSteps) {
        requireValue(step.length === 16 && step.every(Number.isFinite) && step.some((value, i) => Math.abs(value - previous![i]) > 1e-6), `${scene.id}: a zoom click had no observed camera change`);
        previous = step;
      }
      requireValue(camera.every((value, i) => Math.abs(value - previous![i]) <= 1e-6), `${scene.id}: camera moved after final zoom observation`);
    } else requireValue(state.zoomClicks === 0 && state.zoomStart === null && state.zoomSteps?.length === 0, `${scene.id}: unexpected zoom`);
  }
}
export type Region = { id: string; left: number; top: number; right: number; bottom: number; pixels: number };
export type Review = { report_sha256: string; decisions: { scene: string; region: string; verdict: "intentional" | "regression"; reason: string }[] };

// Group adjacent changed 32px tiles, preserving every changed pixel. A threshold
// or a broad ignore mask would silently approve small or unexpected changes.
export function changedRegions(before: Buffer, after: Buffer): { changed: number; regions: Region[] } {
  const a = screenshotPixels(before), b = screenshotPixels(after);
  requireValue(a.width === b.width && a.height === b.height, "Screenshot dimensions differ");
  if (a.channels === b.channels && a.pixels.equals(b.pixels)) return { changed: 0, regions: [] };
  const cols = Math.ceil(a.width / 32), rows = Math.ceil(a.height / 32);
  const tiles = new Map<number, { count: number; left: number; top: number; right: number; bottom: number }>();
  let changed = 0;
  for (let y = 0; y < a.height; y++) for (let x = 0; x < a.width; x++) {
    const ai = (y * a.width + x) * a.channels, bi = (y * b.width + x) * b.channels;
    if ([0, 1, 2, 3].every(c => (c === 3 && a.channels === 3 ? 255 : a.pixels[ai + c]) === (c === 3 && b.channels === 3 ? 255 : b.pixels[bi + c]))) continue;
    changed++;
    const key = Math.floor(y / 32) * cols + Math.floor(x / 32), tile = tiles.get(key) ?? { count: 0, left: x, top: y, right: x + 1, bottom: y + 1 };
    tile.count++; tile.left = Math.min(tile.left, x); tile.top = Math.min(tile.top, y); tile.right = Math.max(tile.right, x + 1); tile.bottom = Math.max(tile.bottom, y + 1); tiles.set(key, tile);
  }
  const regions: Region[] = [];
  while (tiles.size) {
    const stack = [tiles.keys().next().value!], members: number[] = [];
    let pixels = 0, left = a.width, top = a.height, right = 0, bottom = 0;
    while (stack.length) {
      const key = stack.pop()!, tile = tiles.get(key); if (!tile) continue;
      tiles.delete(key); members.push(key); pixels += tile.count;
      left = Math.min(left, tile.left); top = Math.min(top, tile.top); right = Math.max(right, tile.right); bottom = Math.max(bottom, tile.bottom);
      const x = key % cols, y = Math.floor(key / cols);
      for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) if (x + dx >= 0 && x + dx < cols && y + dy >= 0 && y + dy < rows) {
        const next = (y + dy) * cols + x + dx; if (tiles.has(next)) stack.push(next);
      }
    }
    regions.push({ id: hash(canonical({ before: hash(before), after: hash(after), tiles: members.sort((a, b) => a - b) })), left, top, right, bottom, pixels });
  }
  requireValue(regions.reduce((sum, r) => sum + r.pixels, 0) === changed, "Changed pixels were lost");
  return { changed, regions };
}

export function compare(before: Capture, after: Capture, images: Map<string, Buffer>) {
  for (const capture of [before, after]) {
    const env = capture.environment;
    requireValue(env && [env.browser, env.platform].every(v => typeof v === "string" && v.trim()) && env.locale === "ko-KR" && env.reducedMotion === "reduce", "Missing capture environment");
    requireValue(canonical(env.launchArgs) === canonical(captureLaunchArgs), "Missing or different browser raster configuration");
    validateGraphics(env.graphics);
    requireValue(canonical(env.devices) === canonical(scenes.map(s => [s.id, s.width, s.height, s.dpr])) && env.runtime?.length === scenes.length, "Incomplete device/runtime environment");
    requireValue(env.runtime.every(r => [r.userAgent, r.renderer].every(v => typeof v === "string" && v.trim()) && r.fonts?.length === 3 && r.fonts.every((f, i) => f[0] === ["Arial", "AppleGothic", "system-ui"][i] && typeof f[1] === "number" && Number.isFinite(f[1]) && f[1] > 0)), "Incomplete renderer/font environment");
    requireValue([capture.fixture, capture.observer, capture.tool, ...Object.values(capture.build.source), ...Object.values(capture.build.assets)].every(v => typeof v === "string" && /^[a-f0-9]{64}$/.test(v)), "Invalid input/source/build digest");
  }
  requireValue(canonical(before.environment) === canonical(after.environment), "Browser, graphics, fonts or device environment differs");
  requireValue(before.fixture === after.fixture && before.observer === after.observer && before.tool === after.tool, "Fixture, tool or observer changed between captures");
  for (const capture of [before, after]) {
    requireValue(Object.keys(capture.build.source).length > 0 && Object.keys(capture.build.assets).length > 0, "Missing source/build binding");
    requireValue(capture.scenes.length === scenes.length && new Set(capture.scenes.map(s => s.id)).size === scenes.length, "Incomplete or duplicate scene matrix");
    requireValue(capture.scenes.every(s => scenes.some(expected => expected.id === s.id)), "Unexpected scene");
  }
  const cases = scenes.map(scene => {
    const a = before.scenes.find(s => s.id === scene.id)!, b = after.scenes.find(s => s.id === scene.id)!;
    validateState(scene, a.state); validateState(scene, b.state);
    requireValue(a.png === `${scene.id}.png` && b.png === a.png, "Unexpected screenshot path");
    const first = images.get(`before/${a.png}`), second = images.get(`after/${b.png}`);
    requireValue(first && second && hash(first) === a.sha256 && hash(second) === b.sha256, "Screenshot missing or changed since capture");
    for (const image of [first, second]) {
      const png = screenshotPixels(image);
      requireValue(png.width === scene.width * scene.dpr && png.height === scene.height * scene.dpr, "Scene dimensions/DPR differ");
    }
    return { id: scene.id, before: a.sha256, after: b.sha256, ...changedRegions(first, second) };
  });
  return { before: hash(canonical(before)), after: hash(canonical(after)), cases };
}
export type Report = ReturnType<typeof compare>;
export function judge(report: Report, review: Review) {
  requireValue(review.report_sha256 === hash(canonical(report)), "Review belongs to another report");
  requireValue(Array.isArray(review.decisions), "Missing region decisions");
  const needed = new Set(report.cases.flatMap(c => c.regions.map(r => `${c.id}/${r.id}`))), seen = new Set<string>();
  let regressions = 0;
  for (const decision of review.decisions) {
    const key = `${decision.scene}/${decision.region}`;
    requireValue(needed.has(key) && !seen.has(key), "Unknown or duplicate region decision");
    requireValue(typeof decision.reason === "string" && decision.reason.trim(), "Every changed region needs its reason");
    requireValue(["intentional", "regression"].includes(decision.verdict), "Unknown verdict");
    if (decision.verdict === "regression") regressions++;
    seen.add(key);
  }
  const unexplained = needed.size - seen.size;
  return { complete: unexplained === 0 && regressions === 0, unexplained, regressions, scenes: report.cases.length, changedPixels: report.cases.reduce((sum, c) => sum + c.changed, 0), report_sha256: review.report_sha256 };
}
export async function readCapture(directory: string) {
  const manifest = join(directory, "capture.json"), info = await lstat(manifest);
  requireValue(info.isFile() && info.nlink === 1 && info.size <= 1048576, "Expected bounded regular capture manifest");
  const capture = JSON.parse(await readFile(manifest, "utf8")) as Capture, images = new Map<string, Buffer>();
  for (const scene of scenes) {
    const path = join(directory, `${scene.id}.png`), stat = await lstat(path);
    requireValue(stat.isFile() && stat.nlink === 1 && stat.size <= 16 * 1048576, "Expected bounded regular screenshot");
    images.set(`${scene.id}.png`, await readFile(path));
  }
  return { capture, images };
}
export async function compareDirectories(before: string, after: string) {
  requireValue(before !== after, "Two separate captures are required");
  const a = await readCapture(before), b = await readCapture(after);
  const images = new Map([...a.images].map(([key, value]) => [`before/${key}`, value]));
  for (const [key, value] of b.images) images.set(`after/${key}`, value);
  return compare(a.capture, b.capture, images);
}
