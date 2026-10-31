import test from "node:test";
import assert from "node:assert/strict";
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { canonical, captureLaunchArgs, compare, hash, judge, scenes, type Capture, type Review } from "./visual-diff";

// Real RGB PNG bytes exercise the decoder/diff/check path, without a browser or
// a golden screenshot dependency. All nine required physical sizes are used.
function png(width: number, height: number, mark = false) {
  const crc = (bytes: Buffer) => {
    let value = 0xffffffff;
    for (const byte of bytes) { value ^= byte; for (let i = 0; i < 8; i++) value = (value >>> 1) ^ (value & 1 ? 0xedb88320 : 0); }
    return (value ^ 0xffffffff) >>> 0;
  };
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data]), header = Buffer.alloc(4), tail = Buffer.alloc(4);
    header.writeUInt32BE(data.length); tail.writeUInt32BE(crc(body)); return Buffer.concat([header, body, tail]);
  };
  const header = Buffer.alloc(13); header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 2;
  const raw = Buffer.alloc((width * 3 + 1) * height);
  if (mark) raw[(width * 3 + 1) * 40 + 1 + 40 * 3] = 255;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}
test("two-build completion controls cover unknown changes, incomplete/stale evidence and reviewed transfers", () => {
  const before: Capture = { build: { source: { app: hash("source") }, assets: { app: hash("build") } }, fixture: hash("fixture"), observer: hash("observer"), tool: hash("tool"), environment: { browser: "synthetic", platform: "synthetic", devices: scenes.map(s => [s.id, s.width, s.height, s.dpr]), locale: "ko-KR", reducedMotion: "reduce", launchArgs: [...captureLaunchArgs], graphics: { gpu_compositing: "disabled_software", rasterization: "disabled_software", webgl: "enabled_readback" }, runtime: scenes.map(() => ({ userAgent: "synthetic", renderer: "synthetic", fonts: [["Arial", 20], ["AppleGothic", 24], ["system-ui", 20]] })) }, scenes: [] };
  const images = new Map<string, Buffer>();
  for (const scene of scenes) {
    const image = png(scene.width * scene.dpr, scene.height * scene.dpr), name = `${scene.id}.png`;
    images.set(`before/${name}`, image); images.set(`after/${name}`, image);
    before.scenes.push({ id: scene.id, png: name, sha256: hash(image), state: {
      focus: scene.state === "selected" ? "preview-1-0" : null, panel: scene.state === "selected", expanded: scene.state === "selected" ? null : "보안 플랫폼 · 39개",
      zoomStart: scene.state === "zoomed" ? Array(16).fill(0) : null, zoomClicks: scene.state === "zoomed" ? 2 : 0,
      zoomSteps: scene.state === "zoomed" ? [Array(16).fill(.5), Array(16).fill(1)] : [],
      frame: { camera: Array(16).fill(1), stars: [{ id: "preview-1-0", opacity: 1 }] }, labels: [{ id: "preview-1-0" }] } });
  }
  const after = structuredClone(before), changedImages = new Map(images), changed = structuredClone(after);
  const image = png(1440, 900, true); changed.scenes[0].sha256 = hash(image); changedImages.set(`after/${changed.scenes[0].png}`, image);
  const report = compare(before, changed, changedImages), empty: Review = { report_sha256: hash(canonical(report)), decisions: [] };
  const results: { scenario: string; enforced: boolean }[] = [];
  const observe = (scenario: string, run: () => void) => { run(); results.push({ scenario, enforced: true }); };
  observe("unexplained-pixel", () => {
    assert.equal(report.cases[0].changed, 1); assert.equal(report.cases[0].regions[0].pixels, 1);
    const result = judge(report, empty); assert.equal(result.complete, false); assert.equal(result.unexplained, 1);
  });
  observe("missing-scene", () => {
    assert.throws(() => compare(before, { ...after, scenes: after.scenes.slice(0, 8) }, images), /matrix/);
    assert.throws(() => compare(before, { ...after, scenes: [...after.scenes.slice(0, 8), after.scenes[0]] }, images), /matrix/);
    const wrong = structuredClone(after); wrong.scenes[0].state.focus = null;
    assert.throws(() => compare(before, wrong, images), /wrong selected state/);
    const unzoomed = structuredClone(after); unzoomed.scenes[2].state.zoomStart = [...unzoomed.scenes[2].state.frame.camera];
    assert.throws(() => compare(before, unzoomed, images), /camera change/);
    const lostClick = structuredClone(after); lostClick.scenes[2].state.zoomSteps[0] = [...lostClick.scenes[2].state.zoomStart!];
    assert.throws(() => compare(before, lostClick, images), /no observed camera change/);
  });
  observe("stale-png", () => { assert.throws(() => compare(before, after, changedImages), /Screenshot missing or changed/); });
  observe("stale-review", () => {
    assert.throws(() => judge(report, { ...empty, report_sha256: hash("old report") }), /another report/);
    assert.throws(() => compare(before, { ...after, fixture: hash("different inputs") }, images), /Fixture, tool or observer/);
    assert.throws(() => compare(before, { ...after, tool: hash("different capture tool") }, images), /Fixture, tool or observer/);
    assert.throws(() => compare(before, { ...after, environment: { ...after.environment, browser: "different" } }, images), /environment differs/);
    const missing = structuredClone(after); missing.environment.runtime = [];
    assert.throws(() => compare(before, missing, images), /Incomplete device/);
    const missingFont = structuredClone(after); missingFont.environment.runtime[0].fonts = [];
    assert.throws(() => compare(before, missingFont, images), /Incomplete renderer/);
    const missingRaster = structuredClone(after); missingRaster.environment.launchArgs = [];
    assert.throws(() => compare(before, missingRaster, images), /raster configuration/);
    const missingGraphics = structuredClone(after); missingGraphics.environment.graphics = {};
    assert.throws(() => compare(before, missingGraphics, images), /graphics\/WebGL environment/);
    const wrongRaster = structuredClone(after); wrongRaster.environment.graphics.rasterization = "enabled";
    assert.throws(() => compare(before, wrongRaster, images), /graphics\/WebGL environment/);
    const wrongCompositor = structuredClone(after); wrongCompositor.environment.graphics.gpu_compositing = "enabled";
    assert.throws(() => compare(before, wrongCompositor, images), /graphics\/WebGL environment/);
    const ignoredFlags = structuredClone(after); ignoredFlags.environment.graphics.rasterization = "enabled";
    assert.throws(() => compare(ignoredFlags, ignoredFlags, images), /graphics\/WebGL environment/);
    const missingWebgl = structuredClone(after); missingWebgl.environment.graphics.webgl = "unavailable_software";
    assert.throws(() => compare(before, missingWebgl, images), /graphics\/WebGL environment/);
  });
  observe("identical-nine-scenes", () => {
    const same = compare(before, after, images), result = judge(same, { report_sha256: hash(canonical(same)), decisions: [] });
    assert.equal(result.complete, true); assert.equal(result.scenes, 9); assert.equal(result.changedPixels, 0);
  });
  observe("explained-region", () => {
    const decision = { scene: report.cases[0].id, region: report.cases[0].regions[0].id, verdict: "intentional" as const, reason: "A synthetic red pixel is the deliberately changed control." };
    assert.equal(judge(report, { ...empty, decisions: [decision] }).complete, true);
    assert.equal(judge(report, { ...empty, decisions: [{ ...decision, verdict: "regression" }] }).complete, false);
    assert.throws(() => judge(report, { ...empty, decisions: [{ ...decision, reason: " " }] }), /reason/);
    assert.throws(() => judge(report, { ...empty, decisions: [decision, decision] }), /duplicate/);
    assert.throws(() => judge(report, { ...empty, decisions: [{ ...decision, region: "unknown" }] }), /Unknown/);
  });
  if (process.env.ONTOLOGY_VISUAL_OBSERVATIONS) writeFileSync(process.env.ONTOLOGY_VISUAL_OBSERVATIONS, JSON.stringify(results, null, 2) + "\n", { flag: "wx", mode: 0o600 });
});
