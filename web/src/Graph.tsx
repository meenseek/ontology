import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
import type { ForceGraphMethods } from "react-force-graph-3d";
import { CanvasTexture, Color, Group, Mesh, PlaneGeometry, Raycaster, ShaderMaterial, Sprite, SpriteMaterial, Vector2, Vector3 } from "three";
import type { Camera, PerspectiveCamera } from "three";
import type { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { active, constellationView, expandedCoreCameraFrame, kindName, knowledge, linkColor, linkName, stateName } from "./graph";
import { nucleusLabelIds, nucleusLevel, nucleusView } from "./nuclei";
import type { NucleusView } from "./nuclei";
import { fixPosition } from "./positions";
import type { Positions } from "./positions";
import type { GraphLink, Model, PositionedNode } from "./graph";
import { MAX_VISIBLE_LABELS, advanceStarClock, focusedCameraDistance, nodePresentation, nodeScreenMetrics, nodeScreenSize, spriteScale, screenPickDistance, starColor, starMotion, starPhase, starShape, summaryHaloScale, visibleLabels, type StarClock, type SummaryHaloMotion } from "./presentation";
import { summaryGlyphTexture } from "./summary-glyph";

type RenderLink = Omit<GraphLink, "source" | "target">;
type Props = { positions: Positions; snapshot: Model; nodes: PositionedNode[]; links: GraphLink[]; selected: string | null; rotate: boolean; reduced: boolean; visible: boolean; fit: number; disabled: boolean; onSelect: (id: string) => void; onClearSelection: () => boolean; onFailure: () => void };
type SpatialReveal = { members: ReadonlySet<string>; level: number; camera: { x: number; y: number; z: number }; target: { x: number; y: number; z: number } };
function spatialCameraFrame(camera: PerspectiveCamera, controls: OrbitControls, group: { center: { x: number; y: number; z: number }; radius: number }, size: { width: number; height: number }) {
  const vertical = camera.fov * Math.PI / 180;
  const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * size.width / size.height);
  const fitDistance = group.radius * 1.15 / Math.sin(Math.min(vertical, horizontal) / 2);
  const readableDistance = 31 * size.height * camera.projectionMatrix.elements[5] / (2 * 28);
  const offset = camera.position.clone().sub(controls.target);
  if (offset.lengthSq() < 1e-9) offset.set(0, 0, 1);
  offset.normalize().multiplyScalar(Math.max(50, Math.min(fitDistance, readableDistance)));
  return { position: { x: group.center.x + offset.x, y: group.center.y + offset.y, z: group.center.z + offset.z }, target: group.center };
}
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
function screenStar(geometry: PlaneGeometry, material: ShaderMaterial, node: PositionedNode, clock: StarClock, isReduced: () => boolean, cursor: { current: { x: number; y: number } | null }, dragged: { current: string | null }, minimumPixels = 0, haloMotion?: () => number, contactMotion?: () => { x: number; y: number; glow: number } | null) {
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
    const pixels = Math.max(minimumPixels, nodeScreenSize(node.kind, -position.z, viewport.y, camera.projectionMatrix.elements[5]));
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
    const contact = contactMotion?.();
    near = Math.max(0, near + ((haloMotion?.() ?? 1) - 1) * 3 + (contact?.glow ?? 0) * .65);
    material.uniforms.uNear.value = near;
    const wobble = isReduced() ? 0 : near * 0.13;
    // Recoil moves the light inside its quad; the collision footprint and pick target stay fixed.
    material.uniforms.uWobble.value.set(wobble * Math.sin(clock.seconds * 9 + phase) + (contact?.x ?? 0) * .06,
      wobble * Math.cos(clock.seconds * 7 + phase) + (contact?.y ?? 0) * .06);
    material.uniformsNeedUpdate = true;
  };
  return mesh;
}
function screenSprite(material: SpriteMaterial, node: PositionedNode, part: "body" | "selection" | "change" | "hit", selected: boolean, minimumPixels = 0, fixedPixels?: number, scaleMotion?: () => number, rotationMotion?: () => number) {
  const sprite = new Sprite(material), viewport = new Vector2(), position = new Vector3(), cursor = new Vector3();
  const resize = (camera: Camera) => {
    sprite.getWorldPosition(position).applyMatrix4(camera.matrixWorldInverse);
    const pixels = Math.max(minimumPixels, nodeScreenSize(node.kind, -position.z, viewport.y, camera.projectionMatrix.elements[5]));
    sprite.scale.setScalar(spriteScale((fixedPixels ?? nodeScreenMetrics(pixels, selected, node.changed)[part]) * (scaleMotion?.() ?? 1), viewport.y, camera.projectionMatrix.elements[5]));
    sprite.updateMatrixWorld();
  };
  sprite.onBeforeRender = (renderer, _scene, camera) => {
    renderer.getSize(viewport); resize(camera);
    if (rotationMotion) material.rotation = rotationMotion();
  };
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
export default function Graph({ positions, snapshot, nodes, links, selected, rotate, reduced, visible, fit, disabled, onSelect, onClearSelection, onFailure }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const graph = useRef<ForceGraphMethods<PositionedNode, RenderLink> | undefined>(undefined);
  const labelLayer = useRef<HTMLDivElement>(null);
  const coreCloseButton = useRef<HTMLButtonElement>(null);
  const spatialCloseButton = useRef<HTMLButtonElement>(null);
  const hoveredId = useRef<string | null>(null);
  const dragged = useRef<string | null>(null);
  const draggedSummary = useRef<{ id: string; members: readonly string[]; level: number; epoch: number } | null>(null);
  const summaryHalo = useRef<SummaryHaloMotion | null>(null);
  const frozenSpatial = useRef<NucleusView | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [expandedCore, setExpandedCore] = useState<string | null>(null);
  const [spatialReveal, setSpatialReveal] = useState<SpatialReveal | null>(null);
  const [movedGroups, setMovedGroups] = useState<Map<string, { members: readonly string[]; level: number; epoch: number }>>(new Map());
  const [lodLevel, setLodLevel] = useState(0);
  const [settledRevision, setSettledRevision] = useState(0);
  const lodLevelRef = useRef(0), movingRef = useRef(false), settleMovingRef = useRef(false);
  const spatialCloseTimer = useRef<number | null>(null);
  const allowDrag = useRef(true), suppressClickUntil = useRef(0);
  const pointer = useRef<{ pointerId: number; pointerType: string } | null>(null);
  const pressedNode = useRef<{ id: string; pointerId: number; x: number; y: number } | null>(null);
  const cursor = useRef<{ x: number; y: number } | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [hover, setHover] = useState<{ title: string; detail: string } | null>(null);
  // Renderer endpoint mutation stays out of the reconciled model.
  const presentation = useMemo(() => constellationView(nodes, links, selected, expandedCore), [nodes, links, selected, expandedCore]);
  const { nodes: semanticNodes, links: displayLinks, counts: collapsedCounts, cores } = presentation;
  const spatialLevel = Math.max(lodLevel, spatialReveal?.level ?? 0);
  const spatialLocks = useMemo(() => new Map([...movedGroups].filter(([, group]) => group.level === spatialLevel && group.epoch === positions.groupEpoch).map(([id, group]) => [id, group.members])), [movedGroups, spatialLevel, positions.groupEpoch]);
  const spatial = useMemo(() => frozenSpatial.current ?? nucleusView(semanticNodes, snapshot.links,
    spatialLevel, selected, spatialReveal?.members, spatialLocks), [semanticNodes, snapshot.links, spatialLevel, selected, settledRevision, spatialReveal, spatialLocks]);
  const { nodes: displayNodes, counts: spatialCounts, groups: spatialGroups } = spatial;
  const glyphMembers = useMemo(() => {
    const byId = new Map(nodes.map(node => [node.id, node]));
    const members = new Map<string, PositionedNode[]>();
    for (const id of collapsedCounts.keys()) {
      const core = cores.find(candidate => candidate.hub === id);
      if (core) members.set(id, [...core.members].map(memberId => byId.get(memberId)).filter((node): node is PositionedNode => Boolean(node)));
    }
    for (const [id, group] of spatialGroups) members.set(id, group.members.map(memberId => byId.get(memberId)).filter((node): node is PositionedNode => Boolean(node)));
    return members;
  }, [nodes, collapsedCounts, cores, spatialGroups]);
  const collapsibleCore = useMemo(() => !selected && expandedCore && constellationView(nodes, links, null, null).counts.has(expandedCore), [nodes, links, selected, expandedCore]);
  const activeExpandedCore = collapsibleCore ? expandedCore : null;
  // A filter can remove a hub and expose members hidden in the full overview.
  // Give those visible nodes their own positions instead of stacking them at the missing hub.
  const exposesHiddenMembers = useMemo(() => {
    if (nodes.length === snapshot.nodes.length) return false;
    const overview = constellationView(snapshot.nodes, snapshot.links, null, null);
    const visible = new Set(overview.nodes.map(node => node.id));
    return displayNodes.some(node => !visible.has(node.id));
  }, [nodes, snapshot, displayNodes]);
  const layoutCore = exposesHiddenMembers ? "*" : activeExpandedCore ?? cores.find(core => selected && core.members.has(selected))?.hub ?? null;
  useEffect(() => { if (expandedCore && !activeExpandedCore) setExpandedCore(null); }, [expandedCore, activeExpandedCore]);
  useEffect(() => { positions.showCore(layoutCore, performance.now(), reduced); }, [positions, layoutCore, nodes, links, reduced]);
  const data = useMemo(() => ({ nodes: displayNodes.map(n => ({ ...n })), links: displayLinks.map(l => ({ ...l })) }), [displayNodes, displayLinks]);
  const motionClock = useRef<StarClock>({ seconds: 0, lastTime: null });
  const motionReduced = useRef(reduced); motionReduced.current = reduced;
  const resources = useMemo(() => ({ geometry: new PlaneGeometry(1, 1), star: starMaterial(), ring: texture("ring"), selection: texture("selection"), change: texture("change"), materials: new Map<string, SpriteMaterial>(), summaries: new Map<string, { signature: string; texture: CanvasTexture; material: SpriteMaterial }>() }), []);
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
    for (const summary of resources.summaries.values()) { summary.material.dispose(); summary.texture.dispose(); }
    resources.summaries.clear();
  }, [resources]);
  useEffect(() => {
    // ForceGraph replaces old node objects during this commit. Retire absent
    // summary textures on the following frame, after their sprites are gone.
    const frame = requestAnimationFrame(() => {
      for (const [id, glyph] of resources.summaries) if (!glyphMembers.has(id)) {
        glyph.material.dispose(); glyph.texture.dispose(); resources.summaries.delete(id);
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [glyphMembers, resources]);
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
  useEffect(() => {
    const linked = new Set(snapshot.links.flatMap(link => [link.source, link.target]));
    const eligible = new Set(semanticNodes.filter(node => knowledge(node) && !linked.has(node.id)).map(node => node.id));
    setSpatialReveal(current => {
      if (!current?.members.size) return current;
      const members = new Set([...current.members].filter(id => eligible.has(id)));
      return members.size === current.members.size ? current : members.size ? { ...current, members } : null;
    });
  }, [semanticNodes, snapshot.links]);
  useEffect(() => () => { if (spatialCloseTimer.current !== null) window.clearTimeout(spatialCloseTimer.current); }, []);
  // A relationship refresh can change the expanded core without changing node IDs.
  // Dragged coordinates are deliberately excluded so an unchanged refresh keeps the camera.
  const expandedRelationKey = useMemo(() => {
    if (!activeExpandedCore || selected) return "";
    const members = cores.find(core => core.hub === activeExpandedCore)?.members;
    return members ? expandedCoreCameraFrame(nodes, links, activeExpandedCore, members)?.key ?? "" : "";
  }, [nodes, links, cores, selected, activeExpandedCore]);
  const spatialCameraKey = spatialReveal?.members.size ? `${positions.structureEpoch}:${[...spatialReveal.members].sort().join("|")}` : "";
  const cameraKey = `${viewKey}:${selected}:${activeExpandedCore}:${fit}:${size.width}:${size.height}:${expandedRelationKey}:${spatialCameraKey}`;
  const appliedCamera = useRef("");
  const appliedFit = useRef(fit);
  const positionCamera = useCallback(() => {
    const instance = graph.current;
    if (!ready || !instance || !nodes.length || positions.dragging || appliedCamera.current === cameraKey) return;
    const hadCamera = Boolean(appliedCamera.current);
    appliedCamera.current = cameraKey;
    const fitChanged = appliedFit.current !== fit;
    appliedFit.current = fit;
    if (fitChanged) {
      setSpatialReveal(null); setExpandedCore(null);
      positions.showCore(exposesHiddenMembers ? "*" : null, performance.now(), reduced);
    }
    // Frame the settled target while nodes travel there; framing their current
    // positions would zoom into the still-collapsed core and then jump outward.
    const cameraNodes = nodes.map(node => ({ ...node, ...(positions.layoutTarget(node.id) ?? {}) }));
    const cameraDisplayNodes = semanticNodes.map(node => ({ ...node, ...(positions.layoutTarget(node.id) ?? {}) }));
    const target = fitChanged ? null : cameraNodes.find(n => n.id === (selected ?? activeExpandedCore));
    if (target && selected) {
      // Selection reveals the rotating surface while preserving any closer
      // user zoom and the current viewing direction, including deep links.
      const camera = instance.camera() as PerspectiveCamera;
      const controls = instance.controls() as OrbitControls;
      const offset = camera.position.clone().sub(controls.target);
      const distance = focusedCameraDistance(offset.length(), size.height, camera.projectionMatrix.elements[5]);
      if (offset.lengthSq() < 1e-9) offset.set(0, 0, 1);
      offset.normalize().multiplyScalar(distance);
      instance.cameraPosition({ x: target.x + offset.x, y: target.y + offset.y, z: target.z + offset.z }, { x: target.x, y: target.y, z: target.z }, reduced ? 0 : 650);
      return;
    }
    if (spatialReveal?.members.size && !fitChanged) {
      const members = cameraNodes.filter(node => spatialReveal.members.has(node.id));
      if (members.length) {
        const center = members.reduce((sum, node) => ({ x: sum.x + node.x / members.length, y: sum.y + node.y / members.length, z: sum.z + node.z / members.length }), { x: 0, y: 0, z: 0 });
        const radius = Math.max(...members.map(node => Math.hypot(node.x - center.x, node.y - center.y, node.z - center.z))) + 6;
        const frame = spatialCameraFrame(instance.camera() as PerspectiveCamera, instance.controls() as OrbitControls, { center, radius }, size);
        instance.cameraPosition(frame.position, frame.target, reduced ? 0 : 650);
        return;
      }
    }
    if (target && activeExpandedCore) {
      const members = cores.find(core => core.hub === activeExpandedCore)?.members;
      if (members) {
        const radius = expandedCoreCameraFrame(cameraNodes, links, activeExpandedCore, members)?.radius ?? 18;
        const camera = instance.camera() as PerspectiveCamera;
        const controls = instance.controls() as OrbitControls;
        const vertical = camera.fov * Math.PI / 180;
        const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * size.width / size.height);
        const distance = radius * 1.15 / Math.sin(Math.min(vertical, horizontal) / 2);
        const offset = camera.position.clone().sub(controls.target);
        if (offset.lengthSq() < 1e-9) offset.set(0, 0, 1);
        offset.normalize().multiplyScalar(distance);
        instance.cameraPosition({ x: target.x + offset.x, y: target.y + offset.y, z: target.z + offset.z }, { x: target.x, y: target.y, z: target.z }, reduced ? 0 : 650);
        return;
      }
    }
    const overviewNodes = fitChanged ? constellationView(cameraNodes, links, null, null).nodes : cameraDisplayNodes;
    const bounds = overviewNodes.reduce((b, n) => ({ minX: Math.min(b.minX, n.x), maxX: Math.max(b.maxX, n.x), minY: Math.min(b.minY, n.y), maxY: Math.max(b.maxY, n.y), minZ: Math.min(b.minZ, n.z), maxZ: Math.max(b.maxZ, n.z) }), { minX: Infinity, maxX: -Infinity, minY: Infinity, maxY: -Infinity, minZ: Infinity, maxZ: -Infinity });
    const center = { x: (bounds.minX + bounds.maxX) / 2, y: (bounds.minY + bounds.maxY) / 2, z: (bounds.minZ + bounds.maxZ) / 2 };
    const lookAt = target ?? center;
    const radius = Math.max(18, ...overviewNodes.map(n => Math.hypot(n.x - lookAt.x, n.y - lookAt.y, n.z - lookAt.z) + 6));
    const camera = instance.camera() as PerspectiveCamera;
    const vertical = camera.fov * Math.PI / 180;
    const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * size.width / size.height);
    const fitDistance = radius * 1.15 / Math.sin(Math.min(vertical, horizontal) / 2);
    // Keep the pixel-sized lattice from being magnified when it fits the viewport.
    // Larger graphs still fit in the initial overview.
    const firstOverview = positions.hasCompactInitialLayout && fit === 0 && nodes.length === snapshot.nodes.length;
    const distance = firstOverview ? Math.max(fitDistance, size.height * camera.projectionMatrix.elements[5] / 2) : fitDistance;
    const offset = hadCamera && !fitChanged ? camera.position.clone().sub((instance.controls() as OrbitControls).target) : new Vector3(0, 0, 1);
    if (offset.lengthSq() < 1e-9) offset.set(0, 0, 1);
    offset.normalize().multiplyScalar(distance);
    instance.cameraPosition({ x: lookAt.x + offset.x, y: lookAt.y + offset.y, z: lookAt.z + offset.z }, lookAt, reduced ? 0 : 650);
  }, [ready, cameraKey, nodes, semanticNodes, links, selected, activeExpandedCore, cores, reduced, size, positions, fit, snapshot.nodes.length, spatialReveal, exposesHiddenMembers]);
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
      const moved = positions.dragMoved, individual = draggedSummary.current ? null : dragged.current;
      positions.cancel();
      if (draggedSummary.current && moved && draggedSummary.current.epoch === positions.groupEpoch) {
        const group = draggedSummary.current;
        setMovedGroups(current => new Map(current).set(group.id, { members: group.members, level: group.level, epoch: group.epoch }));
      }
      if (individual && moved) setMovedGroups(current => new Map([...current].filter(([, group]) => !group.members.includes(individual))));
      dragged.current = null; draggedSummary.current = null; summaryHalo.current = null; frozenSpatial.current = null; setDraggingId(null);
      pressedNode.current = null;
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
      const unitsPerPixel = 2 * Math.max(.001, depth) / (size.height * projectionY);
      const viewport = () => ({ width: container.current?.clientWidth || size.width, height: container.current?.clientHeight || size.height });
      const group = spatialGroups.get(node.id);
      const cohorts = new Map<string, readonly string[]>([
        ...[...spatialGroups].map(([id, value]) => [id, value.members] as const),
        ...cores.filter(core => collapsedCounts.has(core.hub)).map(core => [core.hub, [...core.members]] as const),
      ]);
      summaryHalo.current = group || collapsedCounts.has(node.id)
        ? { id: node.id, startedAt: performance.now(), releasedAt: null, releaseScale: 1 } : null;
      const plane = {
        right: new Vector3().setFromMatrixColumn(camera.matrixWorld, 0),
        up: new Vector3().setFromMatrixColumn(camera.matrixWorld, 1), spacingPixels: 24,
        visible: displayNodes.map(value => value.id),
        cohorts,
        basis: () => { camera.updateMatrixWorld(); return { right: new Vector3().setFromMatrixColumn(camera.matrixWorld, 0), up: new Vector3().setFromMatrixColumn(camera.matrixWorld, 1) }; },
        viewKey: () => { camera.updateMatrixWorld(); const { width, height } = viewport(); return `${width}|${height}|${camera.matrixWorld.elements.join(",")}|${camera.projectionMatrix.elements.join(",")}`; },
        worldPerPixel: (atDepth: number) => 2 * atDepth / (viewport().height * camera.projectionMatrix.elements[5]),
        isVisible: (at: { x: number; y: number }, radius: number) => { const { width, height } = viewport(); return Math.abs(at.x) <= width / 2 + radius && Math.abs(at.y) <= height / 2 + radius; },
        radius: (value: PositionedNode, atDepth: number) => {
          const isSummary = collapsedCounts.has(value.id) || spatialCounts.has(value.id);
          const pixels = Math.max(isSummary ? 36 : 0, nodeScreenSize(value.kind, atDepth, viewport().height, camera.projectionMatrix.elements[5]));
          const metrics = nodeScreenMetrics(pixels, value.id === selected, value.changed);
          return isSummary ? Math.max(18, metrics.radius) : metrics.radius;
        },
        project: (value: { x: number; y: number; z: number }) => {
          camera.updateMatrixWorld();
          dragProjection.set(value.x, value.y, value.z).applyMatrix4(camera.matrixWorldInverse);
          const depth = camera.projectionMatrix.elements[11] === -1 ? -dragProjection.z : 1;
          dragProjection.applyMatrix4(camera.projectionMatrix);
          const { width, height } = viewport();
          return { x: dragProjection.x * width / 2, y: dragProjection.y * height / 2, depth };
        },
      };
      if (group) {
        draggedSummary.current = { id: node.id, members: group.members, level: spatialLevel, epoch: positions.groupEpoch };
        frozenSpatial.current = spatial;
        positions.beginGroup(node.id, group.members, unitsPerPixel, plane);
      } else positions.begin(node.id, unitsPerPixel, plane);
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
    const measuring = elements.find(element => element !== document.activeElement);
    if (!measuring) return;
    const labels = new Map(displayNodes.map(node => [node.id, {
      node, ...nodePresentation(node, true),
      status: `${kindName[node.kind]}${!active(node) ? ` · ${stateName(node)}` : ""}${node.changed ? " · 변경" : ""}`,
    }]));
    const fill = (element: HTMLElement, id: string) => {
      const label = labels.get(id)!;
      const summary = collapsedCounts.has(id) || spatialCounts.has(id);
      element.dataset.nodeId = id;
      element.className = `node-label ${active(label.node) ? "" : "inactive"}${summary ? " interactive" : ""}`;
      element.children[0].textContent = collapsedCounts.has(id) ? `연결된 항목 · ${collapsedCounts.get(id)}개` : spatialCounts.has(id) ? `가까운 항목 · ${spatialCounts.get(id)}개` : label.title;
      const subtitle = element.children[1] as HTMLElement;
      subtitle.textContent = summary ? "별 끌기 · 눌러 펼치기" : label.subtitle; subtitle.hidden = !subtitle.textContent;
      element.children[2].textContent = summary ? "" : label.status;
      if (summary) {
        element.setAttribute("role", "button"); element.setAttribute("aria-label", `${element.children[0].textContent}, 별을 끌어 함께 이동하거나 눌러 펼치기`);
        element.setAttribute("aria-hidden", "false"); element.tabIndex = 0;
      } else {
        element.removeAttribute("role"); element.removeAttribute("aria-label");
        element.setAttribute("aria-hidden", "true"); element.tabIndex = -1;
      }
    };
    // Measure each literal label once per model/viewport change using an existing slot.
    // Animation frames only project points and reposition the same bounded DOM pool.
    const dimensions = new Map<string, { width: number; height: number }>();
    const measuredId = measuring.dataset.nodeId, measuredHidden = measuring.hidden;
    measuring.hidden = false; measuring.style.visibility = "hidden";
    for (const node of displayNodes) {
      fill(measuring, node.id);
      dimensions.set(node.id, { width: measuring.offsetWidth, height: measuring.offsetHeight });
    }
    if (measuredId && labels.has(measuredId)) fill(measuring, measuredId);
    else { delete measuring.dataset.nodeId; measuring.className = "node-label"; measuring.removeAttribute("role"); measuring.removeAttribute("aria-label"); measuring.setAttribute("aria-hidden", "true"); measuring.tabIndex = -1; }
    measuring.hidden = measuredHidden; measuring.style.visibility = "";
    let frame = 0, lastProjection = "", lastPositions = -1, refreshLabels = true;
    const projected = new Vector3();
    const draw = () => {
      const instance = graph.current;
      instance?.camera().updateMatrixWorld();
      positions.advance(performance.now(), reduced);
      if (positions.settling) settleMovingRef.current = true;
      if (positions.layoutMoving || positions.settling || positions.dragging) movingRef.current = true;
      else if (movingRef.current) {
        movingRef.current = false;
        if (settleMovingRef.current) {
          const byId = new Map(nodes.map(node => [node.id, node]));
          const shifted = [...spatialGroups].filter(([, group]) => {
            const center = group.members.reduce((sum, id) => {
              const node = byId.get(id)!;
              return { x: sum.x + node.x / group.members.length, y: sum.y + node.y / group.members.length, z: sum.z + node.z / group.members.length };
            }, { x: 0, y: 0, z: 0 });
            return Math.hypot(center.x - group.center.x, center.y - group.center.y, center.z - group.center.z) > .001;
          });
          if (shifted.length) setMovedGroups(previous => {
            const next = new Map(previous);
            for (const [id, group] of shifted) next.set(id, { members: group.members, level: spatialLevel, epoch: positions.groupEpoch });
            return next;
          });
        }
        settleMovingRef.current = false;
        setSettledRevision(value => value + 1);
      }
      if (instance && lastPositions !== positions.revision) {
        lastPositions = positions.revision;
        const byId = new Map(nodes.map(node => [node.id, node]));
        for (const node of data.nodes) fixPosition(node, byId.get(node.id)!);
        // With cooldownTicks=0 this refreshes objects/edges without a simulation tick.
        instance.d3ReheatSimulation();
      }
      const controls = instance?.controls() as OrbitControls | undefined;
      const focusedElement = elements.find(element => element === document.activeElement);
      const focusedId = focusedElement?.dataset.nodeId;
      const focusedSummary = focusedId && (collapsedCounts.has(focusedId) || spatialCounts.has(focusedId)) ? focusedId : null;
      if (controls) controls.autoRotate = rotate && !focusedSummary && !positions.dragging && !positions.settling && !positions.layoutMoving && controls.enabled;
      const camera = instance?.camera();
      if (camera) {
        camera.updateMatrixWorld();
        if (!positions.dragging && !positions.layoutMoving && size.height > 0) {
          const controls = instance!.controls() as OrbitControls;
          const distance = camera.position.distanceTo(controls.target);
          const spacing = 31 * size.height * camera.projectionMatrix.elements[5] / (2 * distance);
          const next = nucleusLevel(spacing, lodLevelRef.current);
          if (next !== lodLevelRef.current) {
            lodLevelRef.current = next; hoveredId.current = null; setHover(null); setLodLevel(next);
          }
        }
        const key = `${camera.matrixWorld.elements.join(",")}|${camera.projectionMatrix.elements.join(",")}|${hoveredId.current}|${focusedSummary}|${dragged.current}|${positions.revision}`;
        if (key !== lastProjection) {
          lastProjection = key;
          const candidates = displayNodes.map(node => {
            projected.set(node.x, node.y, node.z).applyMatrix4(camera.matrixWorldInverse);
            const summary = collapsedCounts.has(node.id) || spatialCounts.has(node.id);
            const pixels = Math.max(summary ? 36 : 0, nodeScreenSize(node.kind, -projected.z, size.height, camera.projectionMatrix.elements[5]));
            const { radius } = nodeScreenMetrics(pixels, node.id === selected, node.changed);
            projected.applyMatrix4(camera.projectionMatrix);
            return { id: node.id, kind: node.kind, active: active(node), summary, importance: collapsedCounts.has(node.id) ? 2 : 0, x: (projected.x + 1) * size.width / 2, y: (1 - projected.y) * size.height / 2, depth: projected.z, radius, ...dimensions.get(node.id)! };
          });
          const spatialLabels = nucleusLabelIds(candidates, spatialCounts, size.width, size.height);
          const labelCandidates = candidates.filter(node => !spatialCounts.has(node.id) || spatialLabels.has(node.id) || node.id === hoveredId.current || node.id === focusedSummary);
          const visible = visibleLabels(labelCandidates, size.width, size.height, focusedSummary ?? dragged.current ?? selected, hoveredId.current);
          const remaining = new Map(visible.map(box => [box.id, box]));
          const assigned = new Map<HTMLElement, (typeof visible)[number]>();
          for (const element of elements) {
            const box = remaining.get(element.dataset.nodeId ?? "");
            if (box) { assigned.set(element, box); remaining.delete(box.id); }
          }
          if (focusedElement && (!assigned.has(focusedElement) || !focusedSummary)) focusedElement.blur();
          for (const box of remaining.values()) {
            const slot = elements.find(element => !assigned.has(element));
            if (slot) assigned.set(slot, box);
          }
          for (const element of elements) {
            const box = assigned.get(element);
            if (box && (refreshLabels || element.dataset.nodeId !== box.id)) fill(element, box.id);
            element.hidden = !box;
            element.classList.toggle("hovered", !!box && box.id === hoveredId.current);
            if (!box) continue;
            element.style.left = `${box.left}px`; element.style.top = `${box.top}px`;
          }
          refreshLabels = false;
        }
      }
      frame = requestAnimationFrame(draw);
    };
    draw(); return () => cancelAnimationFrame(frame);
  }, [ready, nodes, displayNodes, collapsedCounts, spatialCounts, spatialGroups, spatialLevel, selected, size, visible, positions, data, reduced, rotate, positionCamera]);
  const object = useCallback((node: PositionedNode) => {
    const group = new Group();
    group.userData.nodeId = node.id;
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
    const summary = collapsedCounts.has(node.id) || spatialCounts.has(node.id);
    const haloScale = summary ? () => summaryHaloScale(summaryHalo.current, node.id, performance.now(), motionReduced.current) : undefined;
    const contactMotion = () => positions.collisionReaction(node.id, performance.now(), motionReduced.current);
    const members = glyphMembers.get(node.id);
    if (summary && members?.length) {
      const samples = members.map(member => ({ id: member.id, kind: member.kind, taxonomyColor: member.taxonomyColor, opacity: active(member) ? 1 : .4 }));
      const signature = samples.map(member => `${member.id}:${starColor(member)}:${member.opacity}`).sort().join("|");
      let glyph = resources.summaries.get(node.id);
      if (!glyph) {
        const image = summaryGlyphTexture(node.id, samples);
        glyph = { signature, texture: image, material: new SpriteMaterial({ map: image, color: "#ffffff", transparent: true, opacity: 1, depthWrite: false, depthTest: true, sizeAttenuation: false }) };
        resources.summaries.set(node.id, glyph);
      } else if (glyph.signature !== signature) {
        const previous = glyph.texture;
        glyph.texture = summaryGlyphTexture(node.id, samples);
        glyph.material.map = glyph.texture;
        glyph.material.needsUpdate = true;
        glyph.signature = signature;
        previous.dispose();
      }
      group.add(screenSprite(glyph.material, node, "body", false, 0, 36,
        haloScale && (() => 1 + (haloScale() - 1) * .35),
        () => (contactMotion()?.pulse ?? 0) * .09));
    } else group.add(isKnowledge
      ? screenStar(resources.geometry, resources.star, node, motionClock.current, () => motionReduced.current, cursor, dragged, summary ? 22 : 0, haloScale, contactMotion)
      : screenSprite(material("ring", color, active(node) ? 1 : .35), node, "body", isSelected, 0, undefined,
        () => (1 + ((haloScale?.() ?? 1) - 1) * 2) * (1 + (contactMotion()?.pulse ?? 0) * .05)));
    if (spatialReveal?.members.has(node.id)) group.add(screenSprite(material("ring", "#89bad2", .7), node, "body", false, 0, 18));
    if (isSelected) group.add(screenSprite(material("selection", "#dce8f6", .52), node, "selection", isSelected));
    if (node.changed) group.add(screenSprite(material("change", "#edb66b", .9), node, "change", isSelected));
    // The invisible plane follows the star and status rings, with a 36px minimum.
    group.add(screenSprite(material("hit", "#ffffff", 0), node, "hit", isSelected));
    return group;
  }, [resources, selected, collapsedCounts, spatialCounts, spatialReveal, glyphMembers, positions]);
  const chooseNode = (id: string) => {
    hoveredId.current = null; setHover(null);
    if (disabled || performance.now() < suppressClickUntil.current) return;
    if (collapsedCounts.has(id)) {
      if (selected && !onClearSelection()) return;
      setSpatialReveal(null);
      setExpandedCore(id);
    } else if (spatialGroups.has(id)) {
      const group = spatialGroups.get(id)!, instance = graph.current;
      if (!instance) return;
      if (selected && !onClearSelection()) return;
      if (spatialCloseTimer.current !== null) window.clearTimeout(spatialCloseTimer.current);
      spatialCloseTimer.current = null;
      const camera = instance.camera() as PerspectiveCamera, controls = instance.controls() as OrbitControls;
      const previous = spatialReveal?.members.size ? spatialReveal : null;
      setSpatialReveal({
        members: new Set(group.members), level: Math.max(lodLevelRef.current, previous?.level ?? 0),
        camera: previous?.camera ?? { ...camera.position }, target: previous?.target ?? { ...controls.target },
      });
      const frame = spatialCameraFrame(camera, controls, group, size);
      instance.cameraPosition(frame.position, frame.target, reduced ? 0 : 650);
    } else {
      if (!spatialReveal?.members.has(id)) setSpatialReveal(null);
      onSelect(id);
    }
  };
  const closeSpatialReveal = () => {
    if (!spatialReveal?.members.size) return;
    const closing = { ...spatialReveal, members: new Set<string>() };
    setSpatialReveal(closing);
    graph.current?.cameraPosition(spatialReveal.camera, spatialReveal.target, reduced ? 0 : 650);
    if (spatialCloseTimer.current !== null) window.clearTimeout(spatialCloseTimer.current);
    spatialCloseTimer.current = window.setTimeout(() => {
      setSpatialReveal(current => current === closing ? null : current);
      spatialCloseTimer.current = null;
    }, reduced ? 0 : 700);
  };
  return <div className="graph-canvas" ref={container} tabIndex={-1}
    onPointerMoveCapture={event => {
      if (disabled || positions.dragging || (event.pointerType !== "mouse" && event.pointerType !== "pen")) { cursor.current = null; return; }
      const rect = event.currentTarget.getBoundingClientRect();
      cursor.current = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    }}
    onPointerLeave={() => { cursor.current = null; }}
    onPointerDownCapture={event => {
      cursor.current = null;
      // A new press is a new action; the guard only belongs to the prior drag's click.
      if (!positions.dragging) suppressClickUntil.current = 0;
      allowDrag.current = !positions.layoutMoving && (event.button === 0 || event.pointerType === "touch");
      if (!allowDrag.current) suppressClickUntil.current = performance.now() + 350;
      pointer.current = { pointerId: event.pointerId, pointerType: event.pointerType };
      pressedNode.current = null;
      const instance = graph.current, canvas = instance?.renderer().domElement;
      if (event.button === 0 && allowDrag.current && canvas && event.target === canvas) {
        const rect = canvas.getBoundingClientRect(), camera = instance!.camera();
        camera.updateMatrixWorld();
        const ray = new Raycaster();
        ray.setFromCamera(new Vector2(2 * (event.clientX - rect.left) / rect.width - 1, 1 - 2 * (event.clientY - rect.top) / rect.height), camera);
        const objects = data.nodes.flatMap(node => (node as PositionedNode & { __threeObj?: Group }).__threeObj ?? []);
        const id = ray.intersectObjects(objects, true)[0]?.object.parent?.userData.nodeId as string | undefined;
        if (id) pressedNode.current = { id, pointerId: event.pointerId, x: event.clientX, y: event.clientY };
      }
    }}
    onPointerUpCapture={event => {
      const press = pressedNode.current;
      pressedNode.current = null; pointer.current = null;
      if (!press || event.button !== 0 || press.pointerId !== event.pointerId || positions.dragging || Math.hypot(event.clientX - press.x, event.clientY - press.y) >= 6) return;
      // The renderer clicks its previous hover frame, which can miss a first tap.
      suppressClickUntil.current = 0;
      chooseNode(press.id);
      suppressClickUntil.current = performance.now() + 350;
    }}
    aria-label="3D 지식 지도. 묶음 별을 끌면 함께 이동하고 누르면 펼쳐 개별 별을 끌 수 있습니다. 빈 공간을 드래그해 회전하고 스크롤로 확대·축소합니다. Tab과 Enter로 묶음을 펼칠 수 있으며 전체 항목은 목록 보기에서 탐색할 수 있습니다.">
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
      onNodeDragEnd={node => {
        const summary = draggedSummary.current;
        const individual = summary ? null : dragged.current;
        draggedSummary.current = null;
        if (positions.dragging) suppressClickUntil.current = performance.now() + 350;
        const moved = allowDrag.current && positions.release(performance.now(), reduced);
        const halo = summaryHalo.current;
        if (halo) {
          if (moved) {
            const now = performance.now();
            halo.releaseScale = summaryHaloScale(halo, halo.id, now, reduced);
            halo.releasedAt = now;
          } else summaryHalo.current = null;
        }
        frozenSpatial.current = null;
        const current = new Map(nodes.map(value => [value.id, value]));
        for (const rendered of data.nodes) fixPosition(rendered, current.get(rendered.id)!);
        if (!allowDrag.current) {
          // Native DragControls also pans/rotates on secondary buttons; undo its visual transform.
          const object = (node as PositionedNode & { __threeObj?: Group }).__threeObj;
          const original = current.get(node.id);
          if (object && original) { object.position.set(original.x, original.y, original.z); object.quaternion.identity(); }
        }
        graph.current?.d3ReheatSimulation();
        dragged.current = null; setDraggingId(null);
        if (moved) setMovedGroups(previous => {
          const next = new Map([...previous].filter(([, group]) => !individual || !group.members.includes(individual)));
          if (summary?.epoch === positions.groupEpoch) next.set(summary.id, { members: summary.members, level: summary.level, epoch: summary.epoch });
          for (const [id, group] of spatialGroups) {
            const representative = current.get(id), target = positions.settlingTarget(id);
            const shifted = group.members.reduce((sum, member) => {
              const node = current.get(member)!;
              return { x: sum.x + node.x / group.members.length, y: sum.y + node.y / group.members.length, z: sum.z + node.z / group.members.length };
            }, { x: 0, y: 0, z: 0 });
            const willMove = target && representative && Math.hypot(target.x - representative.x, target.y - representative.y, target.z - representative.z) > .001;
            const hasMoved = Math.hypot(shifted.x - group.center.x, shifted.y - group.center.y, shifted.z - group.center.z) > .001;
            if (willMove || hasMoved) {
              next.set(id, { members: group.members, level: spatialLevel, epoch: positions.groupEpoch });
            }
          }
          return next;
        });
        const clicked = allowDrag.current && !moved ? individual ?? (summary?.epoch === positions.groupEpoch ? summary.id : null) : null;
        if (clicked) {
          suppressClickUntil.current = 0;
          chooseNode(clicked);
          suppressClickUntil.current = performance.now() + 350;
        }
      }} enablePointerInteraction={!disabled}
      cooldownTicks={0} warmupTicks={0} onEngineStop={positionCamera}
      onNodeClick={node => chooseNode(node.id)}
      onNodeHover={node => { hoveredId.current = node?.id ?? null; setHover(node ? collapsedCounts.has(node.id) ? { title: `연결된 항목 · ${collapsedCounts.get(node.id)}개`, detail: "별 끌기: 함께 이동 · 누르기: 펼치기" } : spatialCounts.has(node.id) ? { title: `가까운 항목 · ${spatialCounts.get(node.id)}개`, detail: "별 끌기: 함께 이동 · 누르기: 펼치기" } : { title: nodePresentation(node).title, detail: `${nodePresentation(node).subtitle ? `${nodePresentation(node).subtitle} · ` : ""}${kindName[node.kind]} · ${stateName(node)}` } : null); }}
      onLinkHover={link => { if (link) hoveredId.current = null; setHover(link ? { title: linkName[link.kind], detail: link.current ? "등록된 관계" : "과거 출처 근거 · 군집 계산에서 제외" } : null); }}
    />}
    <div className="node-labels" ref={labelLayer}
      onClick={event => { const label = (event.target as HTMLElement).closest<HTMLElement>(".node-label.interactive"); if (label?.dataset.nodeId) { if (event.detail === 0) suppressClickUntil.current = 0; chooseNode(label.dataset.nodeId); } }}
      onKeyDown={event => { if (event.key !== "Enter" && event.key !== " ") return; const label = (event.target as HTMLElement).closest<HTMLElement>(".node-label.interactive"); if (label?.dataset.nodeId) { event.preventDefault(); suppressClickUntil.current = 0; const id = label.dataset.nodeId, spatial = spatialCounts.has(id); chooseNode(id); requestAnimationFrame(() => (spatial ? spatialCloseButton : coreCloseButton).current?.focus()); } }}>
      {Array.from({ length: MAX_VISIBLE_LABELS }, (_, index) => <div className="node-label" hidden key={index}><strong /><span /><small /></div>)}
    </div>
    {activeExpandedCore && !selected && !spatialReveal?.members.size && <button type="button" ref={coreCloseButton} className="graph-core-close" onClick={event => { if (event.detail === 0) container.current?.focus(); hoveredId.current = null; setHover(null); setExpandedCore(null); }}>묶음 접기</button>}
    {spatialReveal && spatialReveal.members.size > 0 && !selected && <button type="button" ref={spatialCloseButton} className="graph-core-close" onClick={event => { if (event.detail === 0) container.current?.focus(); closeSpatialReveal(); }}>가까운 항목 {spatialReveal.members.size}개 접기</button>}
    {hover && <div className="graph-tooltip" role="status"><strong>{hover.title}</strong><span>{hover.detail}</span></div>}
    <div className="graph-instructions" aria-hidden="true">묶음 별 끌기: 함께 이동 · 누르기: 펼치기 · 빈 공간 회전 · 스크롤 확대·축소</div>
  </div>;
}
