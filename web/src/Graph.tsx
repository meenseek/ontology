import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
import type { ForceGraphMethods } from "react-force-graph-3d";
import { CanvasTexture, Color, Group, Mesh, PlaneGeometry, ShaderMaterial, Sprite, SpriteMaterial, Vector2, Vector3 } from "three";
import type { Camera, PerspectiveCamera } from "three";
import type { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { active, kindName, linkColor, linkName, stateName } from "./graph";
import { fixPosition } from "./positions";
import type { Positions } from "./positions";
import type { GraphLink, Model, PositionedNode } from "./graph";
import { MAX_VISIBLE_LABELS, advanceStarClock, nodePresentation, nodeScreenMetrics, nodeScreenSize, spriteScale, screenPickDistance, starColor, starMotion, starPhase, starShape, visibleLabels, type StarClock } from "./presentation";

type RenderLink = Omit<GraphLink, "source" | "target">;
type Props = { positions: Positions; snapshot: Model; nodes: PositionedNode[]; links: GraphLink[]; selected: string | null; rotate: boolean; reduced: boolean; visible: boolean; fit: number; disabled: boolean; onSelect: (id: string) => void; onFailure: () => void };
function texture(kind: "ring" | "selection" | "change") {
  const canvas = document.createElement("canvas"); canvas.width = canvas.height = 128;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("별 모양을 그릴 수 없습니다.");
  context.strokeStyle = "white"; context.lineWidth = kind === "selection" ? 1 : kind === "ring" ? 4 : 3;
  if (kind === "change") context.setLineDash([12, 10]);
  context.beginPath(); context.arc(64, 64, 56, 0, Math.PI * 2); context.stroke();
  return new CanvasTexture(canvas);
}
/** One program and quad serve every star; only its spherical sampling field rotates. */
// After shader/motion edits, run the actual GPU checks in web/star-check.html.
export function starMaterial() {
  return new ShaderMaterial({
    transparent: true, depthWrite: false, depthTest: false, toneMapped: false,
    uniforms: {
      uColor: { value: new Color() }, uOpacity: { value: 1 }, uPhase: { value: 0 },
      uRotation: { value: 0 }, uTilt: { value: 0 }, uDetail: { value: 0 }, uShimmer: { value: 1 }, uPixels: { value: 26 }, uShape: { value: 0 },
      uNear: { value: 0 }, uWobble: { value: new Vector2() },
    },
    vertexShader: `
      varying vec2 vUv;
      void main() {
        vUv = uv;
        vec4 center = modelViewMatrix * vec4(0.0, 0.0, 0.0, 1.0);
        vec2 size = vec2(length(modelMatrix[0].xyz), length(modelMatrix[1].xyz));
        center.xy += position.xy * size * -center.z;
        gl_Position = projectionMatrix * center;
      }
    `,
    fragmentShader: `
      varying vec2 vUv;
      uniform vec3 uColor;
      uniform float uOpacity, uPhase, uRotation, uTilt, uDetail, uShimmer, uPixels, uNear, uShape;
      uniform vec2 uWobble;
      float hash(vec3 p) {
        p = fract(p * 0.1031);
        p += dot(p, p.yzx + 33.33);
        return fract((p.x + p.y) * p.z);
      }
      float noise(vec3 p) {
        vec3 i = floor(p), f = fract(p);
        f = f * f * (3.0 - 2.0 * f);
        return mix(mix(mix(hash(i), hash(i + vec3(1,0,0)), f.x),
                       mix(hash(i + vec3(0,1,0)), hash(i + vec3(1,1,0)), f.x), f.y),
                   mix(mix(hash(i + vec3(0,0,1)), hash(i + vec3(1,0,1)), f.x),
                       mix(hash(i + vec3(0,1,1)), hash(i + vec3(1,1,1)), f.x), f.y), f.z);
      }
      void main() {
        vec2 p = vUv * 2.0 - 1.0 - uWobble;
        float distance = length(p);
        float spokes = uShape < 1.5 ? 4.0 : uShape < 2.5 ? 5.0 : 6.0;
        float silhouette = uShape < 0.5 ? 1.0 : 1.0 + 0.12 * cos(spokes * atan(p.y, p.x) + uPhase);
        float radius = mix(0.30, 0.54, uDetail) * silhouette;
        float r = distance / radius;
        if (distance >= 1.0) discard;
        float edge = fwidth(r);
        float disc = 1.0 - smoothstep(1.0 - edge, 1.0 + edge, r);
        vec3 normal = vec3(p / radius, sqrt(max(0.0, 1.0 - r * r)));
        float grain = 0.0;
        if (uDetail > 0.0 && r < 1.0) {
          // Tilt the sampling axis in the view plane; the spherical outline stays still.
          float ct = cos(uTilt), st = sin(uTilt);
          vec3 q = vec3(ct * normal.x - st * normal.y, st * normal.x + ct * normal.y, normal.z);
          float c = cos(uRotation), s = sin(uRotation);
          q = vec3(c * q.x + s * q.z, q.y, -s * q.x + c * q.z);
          q += vec3(uPhase, 0.0, uPhase * 0.37);
          float broad = noise(q * 3.4) * 0.75 + noise(q * 7.8) * 0.25;
          grain = ((broad - 0.5) * 0.8 + (noise(q * 20.0) - 0.5) * 0.10) * uDetail;
        }
        // Broad emissive structures cross the hot center without a dark hemisphere.
        float pulse = clamp((uShimmer - 0.7) / 0.3, 0.0, 1.0);
        float coreShimmer = mix(0.86, 1.0, pulse);
        float heat = exp(-r * r * 4.5);
        vec3 surface = mix(uColor, vec3(1.0), 0.35 + 0.55 * heat)
          * (0.90 + 0.10 * normal.z + grain) * coreShimmer * (1.0 + 0.12 * uNear);
        float fade = 1.0 - smoothstep(0.72, 1.0, distance);
        float halo = mix(0.24, 0.28, uDetail) * exp(-mix(10.0, 8.0, uDetail)
          * max(0.0, distance - mix(radius * 0.7, 0.40, uDetail)));
        // Keep the glint about one CSS pixel wide, including at overview size.
        vec2 pixel = p * uPixels * 0.5;
        float primary = (exp(-pow(abs(pixel.x) / 0.55, 1.4)) * exp(-p.y * p.y * 2.0)
          + exp(-pow(abs(pixel.y) / 0.55, 1.4)) * exp(-p.x * p.x * 2.0))
          * (0.08 + 0.82 * pulse + 0.65 * uNear);
        vec2 diagonal = vec2(pixel.x + pixel.y, pixel.x - pixel.y) * 0.70710678;
        float secondary = (exp(-pow(abs(diagonal.x) / 0.42, 1.4))
          + exp(-pow(abs(diagonal.y) / 0.42, 1.4))) * exp(-distance * distance * 3.5)
          * (0.02 + 0.22 * pulse);
        // A brighter crest stays inside the same footprint and fades out at close range.
        halo *= 1.0 + 0.70 * pulse * (1.0 - uDetail) + 1.1 * uNear;
        halo = (halo + (primary + secondary) * (1.0 - uDetail)) * fade * uShimmer;
        float alpha = clamp(disc + halo * (1.0 - disc), 0.0, 1.0);
        vec3 color = mix(uColor, surface, disc / max(alpha, 0.0001));
        gl_FragColor = vec4(color, alpha * uOpacity);
        #include <colorspace_fragment>
      }
    `,
  });
}
function screenStar(geometry: PlaneGeometry, material: ShaderMaterial, node: PositionedNode, clock: StarClock, isReduced: () => boolean, cursor: { current: { x: number; y: number } | null }, dragged: { current: string | null }) {
  const mesh = new Mesh(geometry, material), viewport = new Vector2(), position = new Vector3(), projected = new Vector3();
  const color = new Color(starColor(node));
  const phase = starPhase(node.id), opacity = active(node) ? 1 : .35;
  mesh.renderOrder = 1;
  // Its screen-sized quad is not a world-space culling or picking boundary.
  mesh.frustumCulled = false;
  mesh.raycast = () => {};
  mesh.onBeforeRender = (renderer, _scene, camera) => {
    renderer.getSize(viewport);
    mesh.getWorldPosition(position).applyMatrix4(camera.matrixWorldInverse);
    const pixels = nodeScreenSize(node.kind, -position.z, viewport.y, camera.projectionMatrix.elements[5]);
    mesh.scale.setScalar(spriteScale(pixels, viewport.y, camera.projectionMatrix.elements[5]));
    mesh.updateMatrixWorld();
    const motion = starMotion(pixels, phase, advanceStarClock(clock, performance.now(), isReduced()));
    let near = 0;
    if (cursor.current && position.z < 0) {
      projected.copy(position).applyMatrix4(camera.projectionMatrix);
      const distance = Math.hypot((projected.x + 1) * viewport.x / 2 - cursor.current.x, (1 - projected.y) * viewport.y / 2 - cursor.current.y);
      const reach = Math.max(0, 1 - distance / 52);
      near = reach * reach * (3 - 2 * reach);
    }
    // A shared material must upload every node's values, even between consecutive star draws.
    material.uniforms.uColor.value.copy(color);
    material.uniforms.uOpacity.value = opacity;
    material.uniforms.uPhase.value = phase;
    material.uniforms.uRotation.value = motion.rotation;
    material.uniforms.uTilt.value = motion.tilt;
    material.uniforms.uDetail.value = motion.detail;
    material.uniforms.uShimmer.value = motion.shimmer;
    material.uniforms.uPixels.value = pixels;
    material.uniforms.uShape.value = starShape(node.id);
    near = Math.max(near, dragged.current === node.id ? .8 : 0);
    material.uniforms.uNear.value = near;
    const wobble = isReduced() ? 0 : near * 0.13;
    material.uniforms.uWobble.value.set(wobble * Math.sin(clock.seconds * 9 + phase), wobble * Math.cos(clock.seconds * 7 + phase));
    material.uniformsNeedUpdate = true;
  };
  return mesh;
}
function screenSprite(material: SpriteMaterial, node: PositionedNode, part: "body" | "selection" | "change" | "hit", selected: boolean) {
  const sprite = new Sprite(material), viewport = new Vector2(), position = new Vector3(), cursor = new Vector3();
  const resize = (camera: Camera) => {
    sprite.getWorldPosition(position).applyMatrix4(camera.matrixWorldInverse);
    const pixels = nodeScreenSize(node.kind, -position.z, viewport.y, camera.projectionMatrix.elements[5]);
    sprite.scale.setScalar(spriteScale(nodeScreenMetrics(pixels, selected, node.changed)[part], viewport.y, camera.projectionMatrix.elements[5]));
    sprite.updateMatrixWorld();
  };
  sprite.onBeforeRender = (renderer, _scene, camera) => { renderer.getSize(viewport); resize(camera); };
  // Draw luminous bodies over native relation lines while retaining their positions.
  sprite.renderOrder = part === "hit" ? 0 : 1;
  const raycast = sprite.raycast;
  sprite.raycast = part === "hit" ? (raycaster, intersections) => {
    // Picking can precede a render after the camera moves, including behind a node.
    if (!raycaster.camera) return;
    raycaster.camera.updateMatrixWorld();
    resize(raycaster.camera);
    if (sprite.scale.x <= 0) return;
    const first = intersections.length;
    raycast.call(sprite, raycaster, intersections);
    if (intersections.length === first) return;
    sprite.getWorldPosition(position).project(raycaster.camera);
    raycaster.ray.at(1, cursor).project(raycaster.camera);
    const score = screenPickDistance(position, cursor, viewport.x, viewport.y);
    if (!Number.isFinite(score)) { intersections.splice(first); return; }
    // Both forcegraph hover/click and DragControls sort these hits by distance.
    // Preserve native hit eligibility, point and object; rank overlapping nodes by
    // screen-center proximity instead of camera depth. Native drag uses the object
    // world position for its plane, so this sort key does not alter dragging.
    for (let i = first; i < intersections.length; i++) intersections[i].distance = score;
  } : () => {};
  return sprite;
}
const endpoint = (value: string | number | { id?: string | number } | undefined) => typeof value === "object" ? value.id : value;
export default function Graph({ positions, snapshot, nodes, links, selected, rotate, reduced, visible, fit, disabled, onSelect, onFailure }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const graph = useRef<ForceGraphMethods<PositionedNode, RenderLink> | undefined>(undefined);
  const labelLayer = useRef<HTMLDivElement>(null);
  const hoveredId = useRef<string | null>(null);
  const dragged = useRef<string | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const allowDrag = useRef(true), suppressClickUntil = useRef(0);
  const pointer = useRef<{ pointerId: number; pointerType: string } | null>(null);
  const cursor = useRef<{ x: number; y: number } | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [hover, setHover] = useState<{ title: string; detail: string } | null>(null);
  // Renderer endpoint mutation stays out of the reconciled model.
  const data = useMemo(() => ({ nodes: nodes.map(n => ({ ...n })), links: links.map(l => ({ ...l })) }), [nodes, links]);
  const motionClock = useRef<StarClock>({ seconds: 0, lastTime: null });
  const motionReduced = useRef(reduced); motionReduced.current = reduced;
  const resources = useMemo(() => ({ geometry: new PlaneGeometry(1, 1), star: starMaterial(), ring: texture("ring"), selection: texture("selection"), change: texture("change"), materials: new Map<string, SpriteMaterial>() }), []);
  useEffect(() => {
    const element = container.current;
    if (!element) return;
    const observer = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      if (rect) setSize({ width: Math.max(1, Math.floor(rect.width)), height: Math.max(1, Math.floor(rect.height)) });
    });
    observer.observe(element); return () => observer.disconnect();
  }, []);
  useEffect(() => () => {
    resources.geometry.dispose(); resources.star.dispose(); resources.ring.dispose(); resources.selection.dispose(); resources.change.dispose();
    for (const material of resources.materials.values()) material.dispose();
    resources.materials.clear();
  }, [resources]);
  const ready = size.width > 0 && size.height > 0;
  useEffect(() => {
    const instance = graph.current;
    if (!ready || !instance) return;
    const renderer = instance.renderer(); renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    const canvas = renderer.domElement;
    const lost = (event: Event) => { event.preventDefault(); instance.pauseAnimation(); onFailure(); };
    canvas.addEventListener("webglcontextlost", lost);
    return () => { canvas.removeEventListener("webglcontextlost", lost); };
  }, [ready, onFailure]);
  useEffect(() => {
    const instance = graph.current;
    if (!ready || !instance) return;
    // Keep the camera and elapsed surface time; only discard the suspended interval.
    motionClock.current.lastTime = null;
    if (!visible || disabled) cursor.current = null;
    if (visible) instance.resumeAnimation(); else instance.pauseAnimation();
    return () => { motionClock.current.lastTime = null; instance.pauseAnimation(); };
  }, [ready, visible, disabled]);
  useEffect(() => {
    const controls = graph.current?.controls() as OrbitControls | undefined;
    if (controls) { controls.zoomToCursor = true; controls.autoRotate = rotate; controls.autoRotateSpeed = .2; controls.enableDamping = !reduced; }
  }, [ready, rotate, reduced]);
  const viewKey = nodes.map(n => n.id).sort().join("|");
  const cameraKey = `${viewKey}:${selected}:${fit}:${size.width}:${size.height}`;
  const appliedCamera = useRef("");
  const positionCamera = useCallback(() => {
    const instance = graph.current;
    if (!ready || !instance || !nodes.length || positions.dragging || appliedCamera.current === cameraKey) return;
    appliedCamera.current = cameraKey;
    const target = nodes.find(n => n.id === selected);
    if (target) {
      instance.cameraPosition({ x: target.x + 80, y: target.y + 45, z: target.z + 130 }, { x: target.x, y: target.y, z: target.z }, reduced ? 0 : 650);
      return;
    }
    const bounds = nodes.reduce((b, n) => ({ minX: Math.min(b.minX, n.x), maxX: Math.max(b.maxX, n.x), minY: Math.min(b.minY, n.y), maxY: Math.max(b.maxY, n.y), minZ: Math.min(b.minZ, n.z), maxZ: Math.max(b.maxZ, n.z) }), { minX: Infinity, maxX: -Infinity, minY: Infinity, maxY: -Infinity, minZ: Infinity, maxZ: -Infinity });
    const center = { x: (bounds.minX + bounds.maxX) / 2, y: (bounds.minY + bounds.maxY) / 2, z: (bounds.minZ + bounds.maxZ) / 2 };
    const radius = Math.max(18, ...nodes.map(n => Math.hypot(n.x - center.x, n.y - center.y, n.z - center.z) + 6));
    const camera = instance.camera() as PerspectiveCamera;
    const vertical = camera.fov * Math.PI / 180;
    const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * size.width / size.height);
    const fitDistance = radius * 1.15 / Math.sin(Math.min(vertical, horizontal) / 2);
    // Keep the pixel-sized lattice from being magnified when it fits the viewport.
    // Larger graphs still fit in the initial overview.
    const firstOverview = positions.hasCompactInitialLayout && fit === 0 && nodes.length === snapshot.nodes.length;
    const distance = firstOverview ? Math.max(fitDistance, size.height * camera.projectionMatrix.elements[5] / 2) : fitDistance;
    instance.cameraPosition({ x: center.x, y: center.y, z: center.z + distance }, center, reduced ? 0 : 650);
  }, [ready, cameraKey, nodes, selected, reduced, size, positions, fit, snapshot.nodes.length]);
  useEffect(() => {
    let second = 0;
    const first = requestAnimationFrame(() => { second = requestAnimationFrame(positionCamera); });
    return () => { cancelAnimationFrame(first); cancelAnimationFrame(second); };
  }, [positionCamera]);
  useEffect(() => {
    const canvas = graph.current?.renderer().domElement;
    const owner = canvas?.ownerDocument;
    // 3d-force-graph 1.80 emits a document-only touch pointerup without a pointerId.
    // OrbitControls r185 mistakes it for one finger leaving a multitouch gesture and
    // dereferences an absent mouse position. The real pointerup still cleans up normally.
    const ignoreLegacyRelease = (event: PointerEvent) => {
      if (!event.isTrusted && event.target === owner && event.pointerType === "touch" && event.pointerId === 0) event.stopImmediatePropagation();
    };
    owner?.addEventListener("pointerup", ignoreLegacyRelease, true);
    const cancel = () => {
      allowDrag.current = false;
      if (positions.dragging) suppressClickUntil.current = performance.now() + 350;
      positions.cancel();
      dragged.current = null; setDraggingId(null);
      // DragControls handles pointerup/leave but has no pointercancel listener.
      if (pointer.current) canvas?.dispatchEvent(new PointerEvent("pointerup", { ...pointer.current, bubbles: true }));
      pointer.current = null;
    };
    window.addEventListener("blur", cancel);
    canvas?.addEventListener("pointercancel", cancel);
    if (!visible || disabled) cancel();
    return () => { window.removeEventListener("blur", cancel); canvas?.removeEventListener("pointercancel", cancel); cancel(); owner?.removeEventListener("pointerup", ignoreLegacyRelease, true); };
  }, [ready, positions, snapshot, nodes, visible, disabled, fit]);
  const dragNode = (node: PositionedNode) => {
    const instance = graph.current;
    if (!allowDrag.current || disabled || !instance) return;
    if (!positions.dragging) {
      const camera = instance.camera(), controls = instance.controls() as OrbitControls;
      const origin = nodes.find(n => n.id === node.id)!;
      const depth = -new Vector3(origin.x, origin.y, origin.z).applyMatrix4(camera.matrixWorldInverse).z;
      const projectionY = camera.projectionMatrix.elements[5];
      const dragProjection = new Vector3();
      positions.begin(node.id, 2 * Math.max(.001, depth) / (size.height * projectionY), {
        right: new Vector3().setFromMatrixColumn(camera.matrixWorld, 0),
        up: new Vector3().setFromMatrixColumn(camera.matrixWorld, 1), spacingPixels: 24,
        visible: nodes.map(value => value.id),
        worldPerPixel: atDepth => 2 * atDepth / (size.height * projectionY),
        isVisible: (at, radius) => Math.abs(at.x) <= size.width / 2 + radius && Math.abs(at.y) <= size.height / 2 + radius,
        radius: (value, atDepth) => {
          const pixels = nodeScreenSize(value.kind, atDepth, size.height, projectionY);
          const metrics = nodeScreenMetrics(pixels, value.id === selected, value.changed);
          return metrics.radius;
        },
        project: value => {
          camera.updateMatrixWorld();
          dragProjection.set(value.x, value.y, value.z).applyMatrix4(camera.matrixWorldInverse);
          const depth = camera.projectionMatrix.elements[11] === -1 ? -dragProjection.z : 1;
          dragProjection.applyMatrix4(camera.projectionMatrix);
          return { x: dragProjection.x * size.width / 2, y: dragProjection.y * size.height / 2, depth };
        },
      });
      if (positions.dragging) { dragged.current = node.id; setDraggingId(node.id); }
      instance.cameraPosition({ ...camera.position }, { ...controls.target }, 0);
      controls.autoRotate = false;
      hoveredId.current = null; setHover(null);
    }
    // The callback already contains the new 3D position; its drag-end delta has the opposite sign.
    positions.move(node.id, node);
    const current = new Map(nodes.map(value => [value.id, value]));
    for (const rendered of data.nodes) fixPosition(rendered, current.get(rendered.id)!);
    instance.d3ReheatSimulation();
    suppressClickUntil.current = performance.now() + 350;
  };
  useEffect(() => {
    const layer = labelLayer.current;
    if (!ready || !layer || !visible) return;
    const elements = [...layer.children] as HTMLElement[];
    for (const element of elements) { element.hidden = true; delete element.dataset.nodeId; }
    const measuring = elements[0];
    if (!measuring) return;
    const labels = new Map(nodes.map(node => [node.id, {
      node, ...nodePresentation(node, true),
      status: `${kindName[node.kind]}${!active(node) ? ` · ${stateName(node)}` : ""}${node.changed ? " · 변경" : ""}`,
    }]));
    const fill = (element: HTMLElement, id: string) => {
      const label = labels.get(id)!;
      element.dataset.nodeId = id;
      element.className = `node-label ${active(label.node) ? "" : "inactive"}`;
      element.children[0].textContent = label.title;
      const subtitle = element.children[1] as HTMLElement;
      subtitle.textContent = label.subtitle; subtitle.hidden = !label.subtitle;
      element.children[2].textContent = label.status;
    };
    // Measure each literal label once per model/viewport change using an existing slot.
    // Animation frames only project points and reposition the same bounded DOM pool.
    const dimensions = new Map<string, { width: number; height: number }>();
    measuring.hidden = false; measuring.style.visibility = "hidden";
    for (const node of nodes) {
      fill(measuring, node.id);
      dimensions.set(node.id, { width: measuring.offsetWidth, height: measuring.offsetHeight });
    }
    measuring.hidden = true; measuring.style.visibility = "";
    let frame = 0, lastProjection = "", lastPositions = -1;
    const projected = new Vector3();
    const draw = () => {
      const instance = graph.current;
      instance?.camera().updateMatrixWorld();
      positions.advance(performance.now(), reduced);
      if (instance && lastPositions !== positions.revision) {
        lastPositions = positions.revision;
        const byId = new Map(nodes.map(node => [node.id, node]));
        for (const node of data.nodes) fixPosition(node, byId.get(node.id)!);
        // With cooldownTicks=0 this refreshes objects/edges without a simulation tick.
        instance.d3ReheatSimulation();
      }
      const controls = instance?.controls() as OrbitControls | undefined;
      if (controls) controls.autoRotate = rotate && !positions.dragging && !positions.settling && controls.enabled;
      const camera = instance?.camera();
      if (camera) {
        camera.updateMatrixWorld();
        const key = `${camera.matrixWorld.elements.join(",")}|${camera.projectionMatrix.elements.join(",")}|${hoveredId.current}|${dragged.current}|${positions.revision}`;
        if (key !== lastProjection) {
          lastProjection = key;
          const candidates = nodes.map(node => {
            projected.set(node.x, node.y, node.z).applyMatrix4(camera.matrixWorldInverse);
            const pixels = nodeScreenSize(node.kind, -projected.z, size.height, camera.projectionMatrix.elements[5]);
            const { radius } = nodeScreenMetrics(pixels, node.id === selected, node.changed);
            projected.applyMatrix4(camera.projectionMatrix);
            return { id: node.id, kind: node.kind, active: active(node), x: (projected.x + 1) * size.width / 2, y: (1 - projected.y) * size.height / 2, depth: projected.z, radius, ...dimensions.get(node.id)! };
          });
          const visible = visibleLabels(candidates, size.width, size.height, dragged.current ?? selected, hoveredId.current);
          for (const [index, element] of elements.entries()) {
            const box = visible[index];
            element.hidden = !box;
            if (box && element.dataset.nodeId !== box.id) fill(element, box.id);
            element.classList.toggle("hovered", !!box && box.id === hoveredId.current);
            if (!box) continue;
            element.style.left = `${box.left}px`; element.style.top = `${box.top}px`;
          }
        }
      }
      frame = requestAnimationFrame(draw);
    };
    draw(); return () => cancelAnimationFrame(frame);
  }, [ready, nodes, selected, size, visible, positions, data, reduced, rotate]);
  const object = useCallback((node: PositionedNode) => {
    const group = new Group();
    const material = (kind: "ring" | "selection" | "change" | "hit", color: string, opacity: number) => {
      const key = `${kind}:${color}:${opacity}`;
      let value = resources.materials.get(key);
      if (!value) {
        value = new SpriteMaterial({ map: kind === "hit" ? null : resources[kind], color, transparent: true, opacity, depthWrite: false, depthTest: true, sizeAttenuation: false });
        resources.materials.set(key, value);
      }
      return value;
    };
    const isKnowledge = node.kind === "document" || node.kind === "memory";
    const color = node.kind === "document" || node.kind === "memory" ? starColor(node) : node.taxonomyColor ?? node.color;
    const isSelected = node.id === selected;
    group.add(isKnowledge
      ? screenStar(resources.geometry, resources.star, node, motionClock.current, () => motionReduced.current, cursor, dragged)
      : screenSprite(material("ring", color, active(node) ? 1 : .35), node, "body", isSelected));
    if (isSelected) group.add(screenSprite(material("selection", "#dce8f6", .52), node, "selection", isSelected));
    if (node.changed) group.add(screenSprite(material("change", "#edb66b", .9), node, "change", isSelected));
    // The invisible plane follows the star and status rings, with a 36px minimum.
    group.add(screenSprite(material("hit", "#ffffff", 0), node, "hit", isSelected));
    return group;
  }, [resources, selected]);
  return <div className="graph-canvas" ref={container}
    onPointerMoveCapture={event => {
      if (disabled || positions.dragging || (event.pointerType !== "mouse" && event.pointerType !== "pen")) { cursor.current = null; return; }
      const rect = event.currentTarget.getBoundingClientRect();
      cursor.current = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    }}
    onPointerLeave={() => { cursor.current = null; }}
    onPointerDownCapture={event => { cursor.current = null; allowDrag.current = true; pointer.current = { pointerId: event.pointerId, pointerType: event.pointerType }; }}
    aria-label="3D 지식 지도. 점을 끌어 배치하고 빈 공간을 드래그해 회전합니다. 스크롤로 커서 위치를 중심으로 확대·축소합니다. 키보드는 목록 보기를 이용하세요.">
    {ready && <ForceGraph3D<PositionedNode, RenderLink>
      ref={graph} width={size.width} height={size.height} graphData={data}
      backgroundColor="rgba(0,0,0,0)" controlType="orbit" showNavInfo={false}
      nodeLabel={() => ""} linkLabel={() => ""} nodeThreeObject={object}
      linkColor={link => {
        const focus = draggingId ?? selected;
        return focus && endpoint(link.source) !== focus && endpoint(link.target) !== focus ? "#35404b" : link.current ? linkColor[link.kind] : "#947867";
      }}
      linkWidth={0}
      linkOpacity={.65} linkDirectionalArrowLength={link => link.kind === "evidence" ? 2 : 0} linkDirectionalArrowRelPos={.8}
      enableNodeDrag={!disabled}
      onNodeDrag={dragNode}
      onNodeDragEnd={() => {
        if (positions.dragging) suppressClickUntil.current = performance.now() + 350;
        if (allowDrag.current) positions.release(performance.now(), reduced);
        const current = new Map(nodes.map(value => [value.id, value]));
        for (const rendered of data.nodes) fixPosition(rendered, current.get(rendered.id)!);
        graph.current?.d3ReheatSimulation();
        dragged.current = null; setDraggingId(null);
      }} enablePointerInteraction={!disabled}
      cooldownTicks={0} warmupTicks={0} onEngineStop={positionCamera}
      onNodeClick={node => { hoveredId.current = null; setHover(null); if (!disabled && performance.now() >= suppressClickUntil.current) onSelect(node.id); }}
      onNodeHover={node => { hoveredId.current = node?.id ?? null; setHover(node ? { title: nodePresentation(node).title, detail: `${nodePresentation(node).subtitle ? `${nodePresentation(node).subtitle} · ` : ""}${kindName[node.kind]} · ${stateName(node)}` } : null); }}
      onLinkHover={link => { if (link) hoveredId.current = null; setHover(link ? { title: linkName[link.kind], detail: link.current ? "등록된 관계" : "과거 출처 근거 · 군집 계산에서 제외" } : null); }}
    />}
    <div className="node-labels" ref={labelLayer} aria-hidden="true">{Array.from({ length: MAX_VISIBLE_LABELS }, (_, index) => <div className="node-label" hidden key={index}><strong /><span /><small /></div>)}</div>
    {hover && <div className="graph-tooltip" role="status"><strong>{hover.title}</strong><span>{hover.detail}</span></div>}
    <div className="graph-instructions" aria-hidden="true">점 끌어 놓으면 성단 정렬 · 빈 공간 회전 · 스크롤 확대·축소</div>
  </div>;
}
