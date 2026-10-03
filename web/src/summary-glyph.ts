import { CanvasTexture, SRGBColorSpace } from "three";
import { starColor, starPhase, starShape, summaryAppearance } from "./presentation";

type Member = { id: string; kind: string; taxonomyColor?: string; opacity?: number };
export type GlyphStar = { x: number; y: number; depth: number; phase: number; color: string; opacity: number; shape: number; glint: boolean };

/** A small, stable projection of real members. The glyph is a summary, not new graph data. */
export function summaryGlyphStars(groupId: string, members: readonly Member[]): GlyphStar[] {
  const chosen = [...members]
    .sort((a, b) => starPhase(`${a.id}|glyph`) - starPhase(`${b.id}|glyph`) || a.id.localeCompare(b.id))
    .slice(0, summaryAppearance(members.length).samples);
  const turn = Math.PI * 2;
  const rotation = starPhase(`${groupId}|cloud`);
  return chosen.map((member, index) => {
    // Stratify the projected disc so small groups have a bright center rather
    // than a hollow ring; jitter and depth keep its silhouette organic.
    const longitude = rotation + index * 2.399963229728653 + (starPhase(`${member.id}|jitter`) / turn - .5) * .38;
    const spread = Math.sqrt((index + .18) / chosen.length) * (.72 + starPhase(`${member.id}|spread`) / turn * .25);
    return {
      x: Math.cos(longitude) * spread,
      y: Math.sin(longitude) * spread,
      depth: starPhase(`${member.id}|depth`) / Math.PI - 1,
      phase: starPhase(`${member.id}|drift`),
      color: starColor(member),
      opacity: member.opacity ?? 1,
      shape: starShape(member.id),
      glint: starPhase(`${member.id}|glint`) < turn * .24,
    };
  }).sort((a, b) => a.depth - b.depth);
}

/** Independent smooth drift stays inside the existing summary disc. */
export function summaryGlyphPosition(star: GlyphStar, seconds: number, amount: number): { x: number; y: number } {
  if (!Number.isFinite(seconds) || !Number.isFinite(amount) || amount <= 0) return { x: star.x, y: star.y };
  const phase = star.phase, time = seconds * (.6 + phase / (Math.PI * 2) * .9);
  const reach = (.045 + (star.depth + 1) * .0125) * Math.min(1, amount);
  const x = star.x + reach * (.62 * Math.sin(time + phase) + .38 * Math.sin(time * .73 + phase * 2.17));
  const y = star.y + reach * (.55 * Math.cos(time * 1.13 + phase * 1.41) + .45 * Math.sin(time * .47 + phase * 3.1));
  const limit = Math.max(1, Math.hypot(x, y) / .97);
  return { x: x / limit, y: y / limit };
}

/** One retained canvas per summary; only a hovered/fading glyph uploads, at most 30fps. */
export function summaryGlyphTexture(groupId: string, members: readonly Member[]) {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 128;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("별 무리를 그릴 수 없습니다.");
  const stars = summaryGlyphStars(groupId, members);
  const paint = (amount: number, seconds: number) => {
    context.clearRect(0, 0, 128, 128);
    for (const star of stars) {
      const at = summaryGlyphPosition(star, seconds, amount);
      const x = 64 + at.x * 49, y = 64 + at.y * 49;
      const near = (star.depth + 1) / 2;
      const radius = 3.3 + near * 2;
      const glow = context.createRadialGradient(x, y, 0, x, y, radius * 3.2);
      glow.addColorStop(0, star.color + "a6");
      glow.addColorStop(.3, star.color + "58");
      glow.addColorStop(1, star.color + "00");
      context.fillStyle = glow;
      context.globalAlpha = star.opacity;
      context.beginPath(); context.arc(x, y, radius * 3.2, 0, Math.PI * 2); context.fill();
      context.fillStyle = star.color;
      context.globalAlpha = (.7 + near * .3) * star.opacity;
      context.beginPath();
      if (star.shape === 0) context.arc(x, y, radius, 0, Math.PI * 2);
      else {
        const points = star.shape + 3;
        for (let point = 0; point < points * 2; point++) {
          const angle = point * Math.PI / points - Math.PI / 2;
          const distance = radius * (point % 2 ? .63 : 1.18);
          const px = x + Math.cos(angle) * distance, py = y + Math.sin(angle) * distance;
          if (point === 0) context.moveTo(px, py); else context.lineTo(px, py);
        }
        context.closePath();
      }
      context.fill();
      context.fillStyle = "#ffffff";
      context.globalAlpha = (.32 + near * .2) * star.opacity;
      context.beginPath(); context.arc(x, y, Math.max(1, radius * .38), 0, Math.PI * 2); context.fill();
      if (star.glint) {
        context.strokeStyle = star.color;
        context.globalAlpha = (.34 + near * .22) * star.opacity;
        context.lineWidth = 1;
        context.beginPath();
        context.moveTo(x - radius * 2.1, y); context.lineTo(x + radius * 2.1, y);
        context.moveTo(x, y - radius * 2.1); context.lineTo(x, y + radius * 2.1);
        context.stroke();
      }
      context.globalAlpha = 1;
    }
  };
  paint(0, 0);
  const texture = new CanvasTexture(canvas);
  texture.colorSpace = SRGBColorSpace;
  let amount = 0, lastTime: number | null = null, lastPaint = -Infinity;
  const updateMotion = (hovered: boolean, seconds: number, reduced: boolean) => {
    if (!Number.isFinite(seconds)) return;
    const dt = lastTime === null ? 0 : Math.max(0, Math.min(.05, seconds - lastTime));
    lastTime = seconds;
    const previous = amount, target = hovered && !reduced ? 1 : 0;
    amount = reduced ? 0 : target + (amount - target) * Math.exp(-dt / .12);
    if (!target && amount < .001) amount = 0;
    if ((!amount && !previous) || (amount && seconds - lastPaint < 1 / 30)) return;
    paint(amount, seconds); lastPaint = seconds; texture.needsUpdate = true;
  };
  return { texture, updateMotion };
}
