import type { Point } from "./positions";
import { REVEAL_DURATION } from "./reveal-transition";

export type CameraPose = { position: Point; target: Point };
/** Dolly about the current target, preserving the 3D viewing direction and pan. */
export function zoomCameraPose(pose: CameraPose, factor: number, min = 1, max = Infinity): CameraPose {
  const offset = { x: pose.position.x - pose.target.x, y: pose.position.y - pose.target.y, z: pose.position.z - pose.target.z };
  const distance = Math.hypot(offset.x, offset.y, offset.z);
  if (!distance || !Number.isFinite(factor) || factor <= 0) return copy(pose);
  const scale = Math.max(min, Math.min(max, distance / factor)) / distance;
  return { position: { x: pose.target.x + offset.x * scale, y: pose.target.y + offset.y * scale, z: pose.target.z + offset.z * scale }, target: { ...pose.target } };
}
const copy = (pose: CameraPose): CameraPose => ({ position: { ...pose.position }, target: { ...pose.target } });
const mix = (a: Point, b: Point, t: number): Point => ({ x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t, z: a.z + (b.z - a.z) * t });
/** Retarget from the observed pose. Never finish an interrupted tween at its old destination. */
export class CameraMotion {
  private flight: { from: CameraPose; to: CameraPose; start: number; duration: number } | null = null;
  get moving() { return this.flight !== null; }
  stop() { this.flight = null; }
  finish(): CameraPose | null {
    const pose = this.flight ? copy(this.flight.to) : null;
    this.flight = null;
    return pose;
  }
  move(from: CameraPose, to: CameraPose, now: number, duration = REVEAL_DURATION): CameraPose {
    this.flight = duration > 0 ? { from: copy(from), to: copy(to), start: now, duration } : null;
    return copy(duration > 0 ? from : to);
  }
  advance(now: number): CameraPose | null {
    const flight = this.flight;
    if (!flight) return null;
    const t = Math.max(0, Math.min(1, (now - flight.start) / flight.duration));
    const eased = t * t * (3 - 2 * t);
    if (t === 1) this.flight = null;
    return { position: mix(flight.from.position, flight.to.position, eased), target: mix(flight.from.target, flight.to.target, eased) };
  }
}
