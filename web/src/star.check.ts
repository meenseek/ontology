import { Color, Mesh, PerspectiveCamera, PlaneGeometry, Scene, WebGLRenderer } from "three";
import { starMaterial } from "./Graph.tsx";
import { advanceStarClock, spriteScale, starMotion, starPhase, starShape } from "./presentation";

// Development-only GPU regression: import the shipped shader, never a CPU copy of its math.
const output = document.querySelector<HTMLPreElement>("#results")!;
const results: { name: string; passed: boolean; measured: number; expected: string }[] = [];
const record = (name: string, passed: boolean, measured: number, expected: string) => results.push({ name, passed, measured, expected });
const width = 160;
const colors = ["#bad3ee", "#efd8ac"];
const sizes = [26, 56, 140];
const variants = ["stellar-2", "stellar-16", "stellar-7"].map(id => ({ id, phase: starPhase(id), shape: starShape(id) }));
try {
  const renderer = new WebGLRenderer({ antialias: false, preserveDrawingBuffer: true });
  renderer.setClearColor("#0b1420", 1);
  const scene = new Scene(), camera = new PerspectiveCamera(45, 1, .1, 100);
  camera.position.z = 10;
  const geometry = new PlaneGeometry(1, 1), material = starMaterial(), mesh = new Mesh(geometry, material);
  mesh.frustumCulled = false;
  scene.add(mesh);
  let dpr = 1;
  function draw(size: number, color: string, time: number, phase = 0, opacity = 1, shape = 0) {
    const motion = starMotion(size, phase, time);
    mesh.scale.setScalar(spriteScale(size, width, camera.projectionMatrix.elements[5]));
    material.uniforms.uColor.value.copy(new Color(color));
    material.uniforms.uOpacity.value = opacity;
    material.uniforms.uPhase.value = phase;
    material.uniforms.uRotation.value = motion.rotation;
    material.uniforms.uTilt.value = motion.tilt;
    material.uniforms.uDetail.value = motion.detail;
    material.uniforms.uShimmer.value = motion.shimmer;
    material.uniforms.uPixels.value = size;
    material.uniforms.uShape.value = shape;
    renderer.render(scene, camera);
  }
  function pixels() {
    const gl = renderer.getContext(), data = new Uint8Array(width * width * dpr * dpr * 4);
    gl.readPixels(0, 0, width * dpr, width * dpr, gl.RGBA, gl.UNSIGNED_BYTE, data);
    return data;
  }
  // Read output RGB, including the actual background/compositing and output color space.
  function region(data: Uint8Array, inner: number, outer: number, includes = (_x: number, _y: number) => true) {
    let sum = 0, count = 0, peak = 0;
    const n = width * dpr;
    for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
      const dx = (x + .5) / dpr - width / 2, dy = (y + .5) / dpr - width / 2;
      const r = Math.hypot(dx, dy);
      if (r < inner || r >= outer || !includes(dx, dy)) continue;
      const at = (y * n + x) * 4;
      const value = .2126 * data[at] + .7152 * data[at + 1] + .0722 * data[at + 2];
      sum += value; count++; peak = Math.max(peak, value);
    }
    if (!count) throw new Error("Empty pixel sample region");
    return { mean: sum / count, peak };
  }
  const primaryAxis = (x: number, y: number) => Math.min(Math.abs(x), Math.abs(y)) <= .75;
  const secondaryAxis = (x: number, y: number) => Math.abs(Math.abs(x) - Math.abs(y)) <= .75;
  const betweenGlints = (x: number, y: number) => Math.min(Math.abs(x), Math.abs(y)) > 1.5 && Math.abs(Math.abs(x) - Math.abs(y)) > 1.5;
  const swing = (values: number[]) => Math.max(...values) - Math.min(...values);
  const coreRatio = (values: number[]) => Math.min(...values) / Math.max(...values);
  const difference = (a: Uint8Array, b: Uint8Array) => a.reduce((sum, v, i) => sum + Math.abs(v - b[i]), 0) / a.length;
  function surfaceContrast(a: Uint8Array, b: Uint8Array, size: number) {
    const n = width * dpr, samples: { ring: number; a: number; b: number }[] = [];
    const rings = new Map<number, { a: number; b: number; count: number }>();
    const luminance = (data: Uint8Array, at: number) => .2126 * data[at] + .7152 * data[at + 1] + .0722 * data[at + 2];
    for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
      const radius = Math.hypot((x + .5) / dpr - width / 2, (y + .5) / dpr - width / 2);
      // Stay inside the resolved disc, clear of its limb and the external glow.
      if (radius >= size * .27 * .85) continue;
      const ring = Math.floor(radius / 2), at = (y * n + x) * 4;
      const sample = { ring, a: luminance(a, at), b: luminance(b, at) };
      samples.push(sample);
      const total = rings.get(ring) ?? { a: 0, b: 0, count: 0 };
      total.a += sample.a; total.b += sample.b; total.count++;
      rings.set(ring, total);
    }
    let varianceA = 0, varianceB = 0, movement = 0, changed = 0;
    for (const sample of samples) {
      const total = rings.get(sample.ring)!;
      // Subtract each radial mean so a smooth bright core cannot pass as texture.
      varianceA += (sample.a - total.a / total.count) ** 2;
      varianceB += (sample.b - total.b / total.count) ** 2;
      const delta = Math.abs(sample.a - sample.b);
      movement += delta;
      if (delta >= 4) changed++;
    }
    if (!samples.length) throw new Error("Empty stellar surface sample");
    return { spatial: Math.sqrt(Math.min(varianceA, varianceB) / samples.length), temporal: movement / samples.length, changed: changed / samples.length };
  }
  for (dpr of [1, 2]) {
    renderer.setPixelRatio(dpr); renderer.setSize(width, width);
    for (const color of colors) for (const size of sizes) {
      const label = `${color} / ${size}px / DPR ${dpr}`;
      const core: number[] = [], primary: number[] = [], secondary: number[] = [], corona: number[] = [];
      for (let sample = 0; sample <= 28; sample++) {
        draw(size, color, sample / 4);
        const data = pixels();
        core.push(region(data, 0, 1.6).mean);
        primary.push(region(data, size * .29, size * .43, primaryAxis).peak);
        secondary.push(region(data, size * .29, size * .43, secondaryAxis).peak);
        corona.push(region(data, size * .22, size * .28, betweenGlints).mean);
      }
      if (size < 100) {
        // The shader's 0.86 linear core floor becomes a >0.92 output RGB ratio.
        record(`${label}: core remains bright`, coreRatio(core) >= .92, coreRatio(core), ">=0.92 of brightest core");
        record(`${label}: restrained core changes`, swing(core) >= 10 && swing(core) <= 22, swing(core), "10–22 RGB luminance levels");
        record(`${label}: visible primary glint changes`, swing(primary) >= 12, swing(primary), ">=12 RGB luminance levels");
        record(`${label}: visible secondary glint changes`, swing(secondary) >= 8, swing(secondary), ">=8 RGB luminance levels");
        record(`${label}: corona changes between glints`, swing(corona) >= 2, swing(corona), ">=2 RGB luminance levels");
      } else {
        for (const { id, phase } of variants) for (const time of [0, 7, 19]) {
          const sample = `${label} / ${id} / ${time}–${time + 2}s`;
          draw(size, color, time, phase); const before = pixels();
          draw(size, color, time + 2, phase); const after = pixels();
          const contrast = surfaceContrast(before, after, size);
          record(`${sample}: surface spatial contrast`, contrast.spatial >= 3.5, contrast.spatial, ">=3.5 RGB luminance standard deviation after radial mean removal");
          record(`${sample}: surface temporal contrast`, contrast.temporal >= 2, contrast.temporal, ">=2 RGB luminance mean difference inside disc over 2s");
          record(`${sample}: visible area moves`, contrast.changed >= .25, contrast.changed, ">=25% of sampled disc changes by at least 4 RGB luminance levels");
          const brightness = Math.min(region(before, 0, 1.6).mean, region(after, 0, 1.6).mean);
          record(`${sample}: luminous core`, brightness >= 210, brightness, ">=210 RGB luminance levels");
          draw(size, color, time + 2, phase, .35);
          const inactive = region(pixels(), 0, 1.6).mean;
          record(`${sample}: inactive stays dimmer`, inactive < brightness * .6, inactive / brightness, "<0.6 of sampled active core");
        }
      }
      draw(size, color, 1.75, 0, .35);
      const inactive = region(pixels(), 0, 1.6).mean;
      record(`${label}: inactive stays dimmer`, inactive < Math.min(...core) * .6, inactive / Math.min(...core), "<0.6 of dimmest active core");
      const clock = { seconds: 1.1, lastTime: 0 as number | null };
      draw(size, color, clock.seconds); const frozen = pixels();
      for (const now of [50, 100, 200, 20000]) advanceStarClock(clock, now, true);
      draw(size, color, clock.seconds);
      const drift = difference(frozen, pixels());
      record(`${label}: reduced motion is frozen`, drift === 0, drift, "identical rendered bytes");
      const data = pixels(), outside = region(data, size / 2 + 1, size / 2 + 3);
      const background = .2126 * data[0] + .7152 * data[1] + .0722 * data[2];
      const spill = Math.max(Math.abs(outside.mean - background), Math.abs(outside.peak - background));
      record(`${label}: fixed footprint`, spill < .01, spill, "unchanged background outside quad");
    }
    const outlines = [0, 1, 2, 3].map(shape => { draw(140, colors[0], 1, 0, 1, shape); return pixels(); });
    for (let shape = 1; shape < outlines.length; shape++) {
      const delta = difference(outlines[0], outlines[shape]);
      record(`shape ${shape} / DPR ${dpr}: distinct outline`, delta > .1, delta, ">0.1 mean RGB difference from round star");
    }
  }
  output.dataset.passed = String(results.every(r => r.passed));
  output.textContent = `${results.every(r => r.passed) ? "PASS" : "FAIL"} · ${results.filter(r => r.passed).length}/${results.length}\n${JSON.stringify(results, null, 2)}`;
  // Same renderer/material also drives the live gallery. No app data or network access.
  dpr = Math.min(devicePixelRatio, 2);
  renderer.setPixelRatio(dpr); renderer.setSize(width * 3, width * 3);
  renderer.setScissorTest(true);
  document.querySelector("#gallery")!.append(renderer.domElement);
  document.querySelector("#variants")!.textContent = variants.map(({ id, phase, shape }) => {
    const { period, tilt } = starMotion(140, phase, 0);
    return `${id}: 모양 ${shape + 1}, 위상 ${phase.toFixed(3)}, 주기 ${period.toFixed(1)}초, 기울기 ${(tilt * 180 / Math.PI).toFixed(1)}°`;
  }).join("\n");
  const media = matchMedia("(prefers-reduced-motion: reduce)");
  const clock = { seconds: 0, lastTime: null as number | null };
  const pause = document.querySelector<HTMLButtonElement>("#pause")!;
  let stopped = false;
  const label = () => { pause.disabled = media.matches; pause.textContent = media.matches ? "동작 줄이기 적용" : stopped ? "동작 재개" : "동작 멈춤"; };
  pause.addEventListener("click", () => { stopped = !stopped; label(); });
  media.addEventListener("change", label); label();
  let frame = 0;
  const animate = (now: number) => {
    const time = advanceStarClock(clock, now, stopped || media.matches || document.hidden);
    if (!document.hidden) {
      for (const [row, color] of colors.entries()) for (const [col, size] of sizes.entries()) {
        renderer.setViewport(col * width, (2 - row) * width, width, width);
        renderer.setScissor(col * width, (2 - row) * width, width, width);
        draw(size, color, time, starPhase(`${row}-${col}`));
      }
      for (const [col, { phase, shape }] of variants.entries()) {
        renderer.setViewport(col * width, 0, width, width);
        renderer.setScissor(col * width, 0, width, width);
        draw(140, colors[0], time, phase, 1, shape);
      }
    }
    frame = requestAnimationFrame(animate);
  };
  frame = requestAnimationFrame(animate);
  window.addEventListener("pagehide", () => {
    cancelAnimationFrame(frame); media.removeEventListener("change", label);
    material.dispose(); geometry.dispose(); renderer.dispose(); renderer.forceContextLoss();
  }, { once: true });
} catch (error) {
  output.dataset.passed = "false";
  output.textContent = `FAIL · ${error instanceof Error ? error.message : String(error)}\n${JSON.stringify(results, null, 2)}`;
}
