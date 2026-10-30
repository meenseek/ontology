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
