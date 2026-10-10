import { useEffect, useRef } from "react";
import { Vector3 } from "three";
import type { PerspectiveCamera } from "three";
import type { PositionedNode } from "./graph";
import type { CameraPose } from "./camera-motion";
import { starColor } from "./presentation";

type XY = { x: number; y: number };
export function miniMapTransform(points: readonly XY[], width: number, height: number) {
  const xs = points.map(p => p.x), ys = points.map(p => p.y);
  const minX = Math.min(0, ...xs), maxX = Math.max(0, ...xs), minY = Math.min(0, ...ys), maxY = Math.max(0, ...ys);
  const center = { x: (minX + maxX) / 2, y: (minY + maxY) / 2 };
  const scale = Math.min((width - 20) / Math.max(64, maxX - minX), (height - 20) / Math.max(64, maxY - minY));
  return {
    scale,
    toPixel: (p: XY): XY => ({ x: width / 2 + (p.x - center.x) * scale, y: height / 2 - (p.y - center.y) * scale }),
    toWorld: (p: XY): XY => ({ x: center.x + (p.x - width / 2) / scale, y: center.y - (p.y - height / 2) / scale }),
  };
}
type Props = { nodes: PositionedNode[]; getCamera: () => { camera: PerspectiveCamera; pose: CameraPose } | null; onMove: (pose: CameraPose) => void; disabled: boolean; visible: boolean };
export default function MiniMap({ nodes, getCamera, onMove, disabled, visible }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const navigate = useRef<((x: number, y: number, relative?: boolean) => void) | null>(null);
  const pointer = useRef<number | null>(null);
  useEffect(() => {
    const element = canvas.current, context = element?.getContext("2d");
    if (!element || !context || !visible) { navigate.current = null; return; }
    let frame = 0, last = -Infinity;
    const right = new Vector3(), up = new Vector3();
    const draw = (now: number) => {
      frame = requestAnimationFrame(draw);
      if (now - last < 50) return;
      last = now;
      const view = getCamera();
      if (!view) return;
      view.camera.updateMatrixWorld();
      right.setFromMatrixColumn(view.camera.matrixWorld, 0); up.setFromMatrixColumn(view.camera.matrixWorld, 1);
      const project = (point: { x: number; y: number; z: number }) => ({ x: point.x * right.x + point.y * right.y + point.z * right.z, y: point.x * up.x + point.y * up.y + point.z * up.z });
      const width = element.clientWidth, height = element.clientHeight;
      if (!width || !height) return;
      const ratio = Math.min(window.devicePixelRatio, 2);
      if (element.width !== Math.round(width * ratio) || element.height !== Math.round(height * ratio)) {
        element.width = Math.round(width * ratio); element.height = Math.round(height * ratio);
      }
      context.setTransform(ratio, 0, 0, ratio, 0, 0); context.clearRect(0, 0, width, height);
      const projected = nodes.map(project), transform = miniMapTransform(projected, width, height);
      nodes.forEach((node, index) => {
        const p = transform.toPixel(projected[index]);
        context.fillStyle = node.kind === "document" || node.kind === "memory" ? starColor(node) : "#71899f";
        context.globalAlpha = .8; context.beginPath(); context.arc(p.x, p.y, node.kind === "subject" ? 2 : 1.2, 0, Math.PI * 2); context.fill();
      });
      const target = project(view.pose.target), center = transform.toPixel(target);
      const distance = new Vector3().copy(view.camera.position).distanceTo(view.pose.target as Vector3);
      const halfHeight = distance * Math.tan(view.camera.fov * Math.PI / 360) * transform.scale;
      const halfWidth = halfHeight * view.camera.aspect;
      context.globalAlpha = 1; context.fillStyle = "rgba(166,197,219,.06)"; context.strokeStyle = "rgba(183,211,231,.65)"; context.lineWidth = 1;
      const left = Math.max(1, center.x - halfWidth), top = Math.max(1, center.y - halfHeight);
      const rightEdge = Math.min(width - 1, center.x + halfWidth), bottom = Math.min(height - 1, center.y + halfHeight);
      if (rightEdge > left && bottom > top) {
        context.fillRect(left, top, rightEdge - left, bottom - top);
        context.strokeRect(left, top, rightEdge - left, bottom - top);
      }
      context.fillStyle = "#d2e2ed"; context.fillRect(center.x - 1.5, center.y - 1.5, 3, 3);
      // Re-read the pose when input arrives, rather than overwriting a newer pan/zoom.
      const basisRight = right.clone(), basisUp = up.clone();
      navigate.current = (x, y, relative = false) => {
        const current = getCamera();
        if (!current) return;
        const at = project(current.pose.target);
        const next = relative ? { x: at.x + x / transform.scale, y: at.y - y / transform.scale } : transform.toWorld({ x, y });
        const delta = basisRight.clone().multiplyScalar(next.x - at.x).addScaledVector(basisUp, next.y - at.y);
        onMove({ position: new Vector3().copy(current.camera.position).add(delta), target: new Vector3().copy(current.pose.target as Vector3).add(delta) });
      };
    };
    frame = requestAnimationFrame(draw);
    return () => { cancelAnimationFrame(frame); navigate.current = null; pointer.current = null; };
  }, [nodes, getCamera, onMove, visible]);
  const move = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    navigate.current?.(event.clientX - rect.left, event.clientY - rect.top);
  };
  return <div className="graph-minimap">
    <span aria-hidden="true">전체 지도</span>
    <canvas ref={canvas} tabIndex={disabled ? -1 : 0} role="application" aria-label="미니맵. 클릭하거나 끌어서 지도 이동. 방향키로 이동, Home으로 가운데 보기." aria-disabled={disabled}
      onPointerDown={event => {
        if (disabled || event.button !== 0) return;
        event.preventDefault(); event.stopPropagation(); event.currentTarget.focus();
        pointer.current = event.pointerId; event.currentTarget.setPointerCapture(event.pointerId); move(event);
      }}
      onPointerMove={event => { if (!disabled && pointer.current === event.pointerId) move(event); }}
      onPointerUp={event => { if (pointer.current === event.pointerId) { pointer.current = null; event.currentTarget.releasePointerCapture(event.pointerId); } }}
      onPointerCancel={() => { pointer.current = null; }} onLostPointerCapture={() => { pointer.current = null; }}
      onKeyDown={event => {
        if (disabled) return;
        const delta: Record<string, XY> = { ArrowLeft: { x: -10, y: 0 }, ArrowRight: { x: 10, y: 0 }, ArrowUp: { x: 0, y: -10 }, ArrowDown: { x: 0, y: 10 } };
        if (delta[event.key]) { event.preventDefault(); navigate.current?.(delta[event.key].x, delta[event.key].y, true); }
        else if (event.key === "Home") { event.preventDefault(); navigate.current?.(event.currentTarget.clientWidth / 2, event.currentTarget.clientHeight / 2); }
      }} />
  </div>;
}
