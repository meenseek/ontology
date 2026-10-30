import { inflateSync } from "node:zlib";

// Playwright screenshots are 8-bit RGB/RGBA PNGs. Read their actual composited
// pixels without introducing an image dependency or accepting palette formats.
export function screenshotPixels(bytes: Buffer) {
  if (!bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) throw new Error("Expected PNG screenshot");
  let width = 0, height = 0, channels = 0;
  const parts: Buffer[] = [];
  for (let offset = 8; offset < bytes.length;) {
    const length = bytes.readUInt32BE(offset), tag = bytes.toString("ascii", offset + 4, offset + 8);
    const data = bytes.subarray(offset + 8, offset + 8 + length);
    if (tag === "IHDR") {
      width = data.readUInt32BE(0); height = data.readUInt32BE(4);
      if (data[8] !== 8 || ![2, 6].includes(data[9]) || data[12] !== 0) throw new Error("Unsupported screenshot format");
      channels = data[9] === 6 ? 4 : 3;
    }
    if (tag === "IDAT") parts.push(data);
    offset += length + 12;
  }
  const stride = width * channels, raw = inflateSync(Buffer.concat(parts)), pixels = Buffer.alloc(stride * height);
  if (!width || !height || raw.length !== (stride + 1) * height) throw new Error("Invalid screenshot dimensions");
  const paeth = (a: number, b: number, c: number) => {
    const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  };
  let at = 0;
  for (let y = 0; y < height; y++) {
    const filter = raw[at++];
    if (filter > 4) throw new Error("Unsupported PNG filter");
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? pixels[y * stride + x - channels] : 0;
      const b = y ? pixels[(y - 1) * stride + x] : 0;
      const c = y && x >= channels ? pixels[(y - 1) * stride + x - channels] : 0;
      pixels[y * stride + x] = (raw[at++] + [0, a, b, Math.floor((a + b) / 2), paeth(a, b, c)][filter]) & 255;
    }
  }
  return {
    width, height, channels, pixels,
    rgb(x: number, y: number) {
      x = Math.floor(x); y = Math.floor(y);
      if (x < 0 || y < 0 || x >= width || y >= height) throw new Error("Pixel outside screenshot");
      const start = (y * width + x) * channels;
      return [...pixels.subarray(start, start + 3)];
    },
  };
}

export function luminance(rgb: number[]) {
  return rgb.map(value => {
    const v = value / 255;
    return v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4;
  }).reduce((sum, value, i) => sum + value * [.2126, .7152, .0722][i], 0);
}

export function edgeLightPixels(shot: ReturnType<typeof screenshotPixels>, dpr: number,
  canvas: { left: number; top: number; bottom: number; width: number },
  frame: { stars: { x: number; y: number; radius: number; opacity: number }[]; segments: { x1: number; y1: number; x2: number; y2: number }[] }, edge: "top" | "bottom") {
  const border = Math.round((edge === "top" ? canvas.top : canvas.bottom) * dpr);
  const inside = edge === "top" ? border : border - 1, outside = edge === "top" ? border - 1 : border;
  const distance = (x: number, y: number, s: typeof frame.segments[number]) => {
    const dx = s.x2 - s.x1, dy = s.y2 - s.y1;
    const t = Math.max(0, Math.min(1, ((x - s.x1) * dx + (y - s.y1) * dy) / Math.max(.001, dx * dx + dy * dy)));
    return Math.hypot(x - s.x1 - t * dx, y - s.y1 - t * dy);
  };
  const semantic = (x: number, y: number) => frame.stars.some(s => s.opacity > .01 && Math.hypot(x - s.x, y - s.y) <= s.radius + 1)
    || frame.segments.some(s => distance(x, y, s) <= 4);
  const samples: number[] = [], inner: number[] = [];
  for (let x = Math.ceil((canvas.left + canvas.width * .15) * dpr); x < (canvas.left + canvas.width * .55) * dpr; x++) {
    const px = (x + .5) / dpr - canvas.left, py = (inside + .5) / dpr - canvas.top, bg = shot.rgb(x, outside);
    if (!semantic(px, py)) samples.push(Math.max(...shot.rgb(x, inside).map((v, i) => Math.abs(v - bg[i]))));
    for (const offset of [24, 32, 40, 48]) {
      const y = inside + (edge === "top" ? 1 : -1) * Math.round(offset * dpr);
      if (!semantic(px, (y + .5) / dpr - canvas.top)) inner.push(Math.max(...shot.rgb(x, y).map((v, i) => v - bg[i])));
    }
  }
  return { samples: samples.length, max: Math.max(...samples), innerPeak: Math.max(...inner) };
}

export function ringRidge(shot: ReturnType<typeof screenshotPixels>, x: number, y: number, pixels: number, dpr: number) {
  const radius = pixels * 56 / 128, ridges: number[] = [];
  for (let i = 0; i < 96; i++) {
    const angle = i * Math.PI * 2 / 96, co = Math.cos(angle), si = Math.sin(angle);
    if (Math.min(Math.abs(co), Math.abs(si)) < .15 || Math.abs(Math.abs(co) - Math.abs(si)) < .1) continue;
    const sample = (r: number) => shot.rgb((x + co * r) * dpr, (y + si * r) * dpr);
    const a = sample(radius - 2), b = sample(radius + 2);
    ridges.push(Math.max(...[-.75, -.25, .25, .75].flatMap(dr => sample(radius + dr).map((v, j) => v - (a[j] + b[j]) / 2))));
  }
  return { peak: Math.max(...ridges), visible: ridges.filter(v => v >= 2).length };
}

// Compare visible pixels in the actual clipped paint region, independent of PNG
// compression and unrelated native controls outside the modified viewport.
export function sameRegionPixels(a: ReturnType<typeof screenshotPixels>, b: ReturnType<typeof screenshotPixels>,
  rect: { left: number; top: number; right: number; bottom: number }, dpr: number) {
  if (a.width !== b.width || a.height !== b.height) return false;
  const left = Math.max(0, Math.floor(rect.left * dpr)), right = Math.min(a.width, Math.ceil(rect.right * dpr));
  const top = Math.max(0, Math.floor(rect.top * dpr)), bottom = Math.min(a.height, Math.ceil(rect.bottom * dpr));
  if (right <= left || bottom <= top) throw new Error("Empty paint comparison region");
  for (let y = top; y < bottom; y++) {
    if (a.channels === b.channels) {
      const start = (y * a.width + left) * a.channels, end = (y * a.width + right) * a.channels;
      if (!a.pixels.subarray(start, end).equals(b.pixels.subarray(start, end))) return false;
    } else for (let x = left; x < right; x++) {
      const at = (y * a.width + x) * a.channels, bt = (y * b.width + x) * b.channels;
      for (let c = 0; c < 4; c++) if ((c === 3 && a.channels === 3 ? 255 : a.pixels[at + c]) !==
        (c === 3 && b.channels === 3 ? 255 : b.pixels[bt + c])) return false;
    }
  }
  return true;
}
