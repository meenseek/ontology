/** Ignore sub-millionth rounding noise, while preserving every frame field and
 * all visible motion. Exact JSON equality can oscillate after OrbitControls pan. */
export function settledFrameKey(frame: string): string {
  return JSON.stringify(JSON.parse(frame), (_key, value) => typeof value === "number" ? Math.round(value * 1e6) / 1e6 : value);
}

// BEGIN rendered graph observer — also exercised against the installed bundle.
export function installGraphProbe() {
  type Frame = { serial: number; camera: number[]; segments: { x1: number; y1: number; x2: number; y2: number }[]; stars: { id: string; x: number; y: number; opacity: number; radius: number; hitRadius: number; sprites: { pixels: number; stroke: number; opacity: number; color: string }[] }[] };
  let frame: Frame | undefined, serial = 0, running = false;
  let report = { frames: 0, movingFrames: 0, minLabels: Infinity, samples: 0, maxError: 0 };
  // Three's public devtools observation exposes the actual scene without replacing
  // its renderer, textures or application modules. This works in built assets too.
  const devtools = new EventTarget();
  (window as Window & { __THREE_DEVTOOLS__?: EventTarget }).__THREE_DEVTOOLS__ = devtools;
  devtools.addEventListener("observe", event => {
    const scene = (event as CustomEvent).detail as import("three").Scene;
    if (!scene.isScene) return;
    const painted = new Set<import("three").Object3D>(), wrapped = new WeakSet<import("three").Object3D>();
    const before = scene.onBeforeRender;
    scene.onBeforeRender = function (this: typeof scene, ...args) {
      before.apply(this, args);
      painted.clear();
      scene.traverseVisible(object => {
        if (!("material" in object) || wrapped.has(object)) return;
        wrapped.add(object);
        const after = object.onAfterRender;
        object.onAfterRender = function (this: typeof object, ...drawArgs) { painted.add(this); after.apply(this, drawArgs); };
      });
    };
    const previous = scene.onAfterRender;
    scene.onAfterRender = function (this: typeof scene, ...args) {
      previous.apply(this, args);
      const [renderer, , camera] = args, rect = renderer.domElement.getBoundingClientRect();
      const stars: Frame["stars"] = [];
      const segments: Frame["segments"] = [];
      scene.traverseVisible(object => {
        const line = object as import("three").Line;
        if (painted.has(object) && line.isLine && !Array.isArray(line.material) && line.material.opacity > .01) {
          const points = line.geometry.getAttribute("position");
          if (points?.count === 2) {
            const a = line.position.clone().fromBufferAttribute(points, 0).applyMatrix4(line.matrixWorld).project(camera);
            const b = line.position.clone().fromBufferAttribute(points, 1).applyMatrix4(line.matrixWorld).project(camera);
            if (Math.abs(a.z) <= 1 && Math.abs(b.z) <= 1) segments.push({ x1: (a.x + 1) * rect.width / 2, y1: (1 - a.y) * rect.height / 2, x2: (b.x + 1) * rect.width / 2, y2: (1 - b.y) * rect.height / 2 });
          }
        }
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
        let radius = 0, hitRadius = 0;
        for (const child of object.children) {
          if (!painted.has(child)) continue;
          const body = child as import("three").Mesh;
          const matrix = child.matrixWorld.elements;
          const pixels = Math.hypot(matrix[0], matrix[1], matrix[2]) * p[5] * rect.height / 2;
          // A deliberately conservative bound of the actual disc/semantic sprite,
          // excluding the independent stellar-light mesh and transparent hit plane.
          if (body.isMesh && body.name !== "stellar-light") radius = Math.max(radius, pixels * .26 + 1);
          const imageBody = child as import("three").Sprite;
          if (imageBody.isSprite && !imageBody.material.map && imageBody.material.opacity === 0) hitRadius = Math.max(hitRadius, pixels / 2);
          if (imageBody.isSprite && imageBody.material.opacity > .01 && imageBody.material.map) radius = Math.max(radius, pixels / 2 + 1);
          const sprite = child as import("three").Sprite, material = sprite.material, image = material?.map?.image;
          if (!sprite.isSprite || !(image instanceof HTMLCanvasElement) || image.width !== 128) continue;
          const alpha = image.getContext("2d")!.getImageData(0, 64, image.width, 1).data;
          let ink = 0;
          for (let i = 3; i < alpha.length; i += 4) ink += alpha[i] / 255;
          sprites.push({ pixels, stroke: ink / 2 * pixels / image.width, opacity: material.opacity, color: material.color.getHexString() });
        }
        stars.push({ id: object.userData.nodeId, x: ((p[0] * vx + p[4] * vy + p[8] * vz + p[12]) / w + 1) * rect.width / 2,
          y: (1 - (p[1] * vx + p[5] * vy + p[9] * vz + p[13]) / w) * rect.height / 2,
          opacity: object.userData.opacity?.() ?? 1, radius, hitRadius, sprites });
      });
      frame = { serial: ++serial, segments, camera: camera.matrixWorld.toArray(), stars };
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
  const api = { frame: () => frame, start, stop: () => { running = false; return report; } };
  window.__graphQA = api;
  return api;
}

declare global { interface Window { __graphQA: ReturnType<typeof installGraphProbe> } }
