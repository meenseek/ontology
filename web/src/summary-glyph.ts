import { CanvasTexture, SRGBColorSpace } from "three";
import { starColor, starPhase, starShape } from "./presentation";

type Member = { id: string; kind: string; taxonomyColor?: string; opacity?: number };
export type GlyphStar = { x: number; y: number; depth: number; color: string; opacity: number; shape: number; glint: boolean };

/** A small, stable projection of real members. The glyph is a summary, not new graph data. */
export function summaryGlyphStars(groupId: string, members: readonly Member[]): GlyphStar[] {
  const chosen = [...members]
    .sort((a, b) => starPhase(`${a.id}|glyph`) - starPhase(`${b.id}|glyph`) || a.id.localeCompare(b.id))
    .slice(0, 14);
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
      color: starColor(member),
      opacity: member.opacity ?? 1,
      shape: starShape(member.id),
      glint: starPhase(`${member.id}|glint`) < turn * .24,
    };
  }).sort((a, b) => a.depth - b.depth);
}

/** One texture and sprite per summary keeps even large groups cheap to draw. */
export function summaryGlyphTexture(groupId: string, members: readonly Member[]): CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 128;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("별 무리를 그릴 수 없습니다.");
  for (const star of summaryGlyphStars(groupId, members)) {
    const x = 64 + star.x * 49, y = 64 + star.y * 49;
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
  const texture = new CanvasTexture(canvas);
  texture.colorSpace = SRGBColorSpace;
  return texture;
}
