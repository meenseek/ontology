import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
import type { ForceGraphMethods } from "react-force-graph-3d";
import { CanvasTexture, Color, Group, LineBasicMaterial, Mesh, MOUSE, PlaneGeometry, Raycaster, ShaderMaterial, Sprite, SpriteMaterial, TOUCH, Vector2, Vector3 } from "three";
import type { Camera, Object3D, PerspectiveCamera } from "three";
import type { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { active, constellationView, diagramLinks, expandedCoreCameraFrame, kindName, knowledge, linkColor, linkName, stateName } from "./graph";
import { nucleusLabelIds, nucleusLevel, nucleusView } from "./nuclei";
import type { NucleusView } from "./nuclei";
import { fixPosition } from "./positions";
import type { Positions } from "./positions";
import type { GraphLink, GraphView, Model, PositionedNode } from "./graph";
import { LAYOUT_WORLD_SPACING, MAX_VISIBLE_LABELS, advanceStarClock, coreCameraDistance, focusedCameraDistance, nodePresentation, nodeScreenMetrics, nodeScreenSize, nodeVisualRadius, spriteScale, screenPickDistance, starColor, starMotion, starPhase, starShape, summaryAppearance, summaryHaloScale, visibleLabels, type StarClock, type SummaryHaloMotion } from "./presentation";
import { summaryGlyphTexture } from "./summary-glyph";
import { planCoreView, screenPlane, spatialCameraFrame } from "./view-layout";
import { CameraMotion, zoomCameraPose } from "./camera-motion";
import type { CameraPose } from "./camera-motion";
import MiniMap from "./MiniMap";
import { REVEAL_DURATION, retargetReveal, revealOpacity, revealState } from "./reveal-transition";

type RenderLink = Omit<GraphLink, "source" | "target">;
type Props = { positions: Positions; snapshot: Model; nodes: PositionedNode[]; links: GraphLink[]; selected: string | null; rotate: boolean; reduced: boolean; visible: boolean; fit: number; disabled: boolean; onSelect: (id: string) => void; onClearSelection: () => boolean; onFit?: () => void; onFailure: () => void };
type SpatialReveal = { members: ReadonlySet<string>; level: number; camera: { x: number; y: number; z: number }; target: { x: number; y: number; z: number } };
const coreLabel = (node: PositionedNode | undefined, count: number, view?: GraphView) => node?.kind === "folder" ? `폴더 묶음 · ${count - 1}개` : view === "purpose" ? `${node?.subject_name ?? node?.label ?? "목적 묶음"} · ${count}개` : `관계 묶음 · ${count}개`;
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
function screenStar(geometry: PlaneGeometry, material: ShaderMaterial, node: PositionedNode, clock: StarClock, isReduced: () => boolean, cursor: { current: { x: number; y: number } | null }, dragged: { current: string | null }, minimumPixels = 0, haloMotion?: () => number, contactMotion?: () => { x: number; y: number; glow: number } | null, fade?: () => number) {
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
    material.uniforms.uOpacity.value = opacity * (fade?.() ?? 1);
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
function screenSprite(material: SpriteMaterial, node: PositionedNode, part: "body" | "selection" | "change" | "hit", selected: boolean, minimumPixels = 0, fixedPixels?: number, scaleMotion?: () => number, rotationMotion?: () => number, paint?: () => void, fade?: () => number, pickable?: () => boolean) {
  const sprite = new Sprite(material), viewport = new Vector2(), position = new Vector3(), cursor = new Vector3();
  const opacity = (material.userData.baseOpacity ??= material.opacity) as number;
  const resize = (camera: Camera) => {
    sprite.getWorldPosition(position).applyMatrix4(camera.matrixWorldInverse);
    const pixels = Math.max(minimumPixels, nodeScreenSize(node.kind, -position.z, viewport.y, camera.projectionMatrix.elements[5]));
    const renderedPixels = (fixedPixels ?? nodeScreenMetrics(pixels, selected, node.changed)[part]) * (scaleMotion?.() ?? 1);
    sprite.scale.setScalar(spriteScale(renderedPixels, viewport.y, camera.projectionMatrix.elements[5]));
    sprite.updateMatrixWorld();
  };
  sprite.onBeforeRender = (renderer, _scene, camera) => {
    renderer.getSize(viewport); resize(camera);
    material.opacity = opacity * (fade?.() ?? 1);
    paint?.();
    if (rotationMotion) material.rotation = rotationMotion();
  };
  // Draw luminous bodies over native relation lines while retaining their positions.
  sprite.renderOrder = part === "hit" ? 0 : 1;
  const raycast = sprite.raycast;
  sprite.raycast = part === "hit" ? (raycaster, intersections) => {
    // Picking can precede a render after the camera moves, including behind a node.
    if (!raycaster.camera || pickable && !pickable()) return;
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
export default function Graph({ positions, snapshot, nodes, links, selected, rotate, reduced, visible, fit, disabled, onSelect, onClearSelection, onFit, onFailure }: Props) {
  const container = useRef<HTMLDivElement>(null);
  const graph = useRef<ForceGraphMethods<PositionedNode, RenderLink> | undefined>(undefined);
  const cameraMotion = useRef(new CameraMotion());
  const coreReturn = useRef<CameraPose | null>(null);
  const pendingReturn = useRef<CameraPose | null>(null);
  const getCamera = useCallback(() => {
    const instance = graph.current;
    if (!instance) return null;
    const camera = instance.camera() as PerspectiveCamera, controls = instance.controls() as OrbitControls;
    return { camera, pose: { position: { x: camera.position.x, y: camera.position.y, z: camera.position.z }, target: { x: controls.target.x, y: controls.target.y, z: controls.target.z } } };
  }, []);
  const writeCamera = useCallback((pose: CameraPose) => {
    const instance = graph.current;
    if (!instance) return;
    const controls = instance.controls() as OrbitControls;
    instance.camera().position.set(pose.position.x, pose.position.y, pose.position.z);
    controls.target.set(pose.target.x, pose.target.y, pose.target.z);
    controls.update(); instance.camera().updateMatrixWorld();
  }, []);
  const moveCamera = useCallback((position: CameraPose["position"], target: CameraPose["target"], duration: number) => {
    const view = getCamera();
    if (view) {
      (graph.current!.controls() as OrbitControls).autoRotate = false;
      writeCamera(cameraMotion.current.move(view.pose, { position, target }, performance.now(), duration));
    }
  }, [getCamera, writeCamera]);
  const panMiniMap = useCallback((pose: CameraPose) => { cameraMotion.current.stop(); pendingReturn.current = null; writeCamera(pose); }, [writeCamera]);
  const zoomMiniMap = useCallback((factor: number) => {
    const view = getCamera();
    if (!view) return;
    const controls = graph.current!.controls() as OrbitControls;
    cameraMotion.current.stop(); pendingReturn.current = null;
    const pose = zoomCameraPose(view.pose, factor, Math.max(1, controls.minDistance), controls.maxDistance);
    moveCamera(pose.position, pose.target, reduced ? 0 : 160);
  }, [getCamera, moveCamera, reduced]);
  useEffect(() => {
    if (reduced) { const pose = cameraMotion.current.finish(); if (pose) writeCamera(pose); }
  }, [reduced, writeCamera]);
  const labelLayer = useRef<HTMLDivElement>(null);
  const coreCloseButton = useRef<HTMLButtonElement>(null);
  const spatialCloseButton = useRef<HTMLButtonElement>(null);
  const hoveredId = useRef<string | null>(null);
  const dragged = useRef<string | null>(null);
  const draggedSummary = useRef<{ id: string; members: readonly string[]; level: number; epoch: number } | null>(null);
  const summaryHalo = useRef<SummaryHaloMotion | null>(null);
  const packedOverview = useRef("");
  const overviewZoom = useRef({ scale: 0, changedAt: 0, checkedFootprints: "" });
  const frozenSpatial = useRef<NucleusView | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [expandedCore, setExpandedCore] = useState<string | null>(null);
  const [corePage, setCorePage] = useState(0);
  const [showAllCore, setShowAllCore] = useState(false);
  const [spatialReveal, setSpatialReveal] = useState<SpatialReveal | null>(null);
  const [movedGroups, setMovedGroups] = useState<Map<string, { members: readonly string[]; level: number; epoch: number }>>(new Map());
  const [lodLevel, setLodLevel] = useState(0);
  const [settledRevision, setSettledRevision] = useState(0);
  const lodLevelRef = useRef(0), movingRef = useRef(false), settleMovingRef = useRef(false);
  const spatialCloseTimer = useRef<number | null>(null);
  const allowDrag = useRef(true), suppressClickUntil = useRef(0);
  const pointer = useRef<{ pointerId: number; pointerType: string } | null>(null);
  const controlPointer = useRef<{ pointerId: number; pointerType: string } | null>(null);
  const activePointers = useRef(new Map<number, string>());
  const pressedNode = useRef<{ id: string; pointerId: number; x: number; y: number } | null>(null);
  const cursor = useRef<{ x: number; y: number } | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [hover, setHover] = useState<{ title: string; detail: string } | null>(null);
  const snapshotLinks = useMemo(() => diagramLinks(snapshot), [snapshot]);
  // Renderer endpoint mutation stays out of the reconciled model.
  const presentation = useMemo(() => constellationView(nodes, links, selected, expandedCore, showAllCore ? undefined : corePage, snapshot.view),
    [nodes, links, selected, expandedCore, showAllCore, corePage, snapshot.view]);
  const { nodes: semanticNodes, links: displayLinks, counts: collapsedCounts, cores, disclosure } = presentation;
  const spatialLevel = Math.max(lodLevel, spatialReveal?.level ?? 0);
  const spatialLocks = useMemo(() => new Map([...movedGroups].filter(([, group]) => group.level === spatialLevel && group.epoch === positions.groupEpoch).map(([id, group]) => [id, group.members])), [movedGroups, spatialLevel, positions.groupEpoch]);
  const spatial = useMemo(() => frozenSpatial.current ?? nucleusView(semanticNodes, snapshotLinks,
    spatialLevel, selected, spatialReveal?.members, spatialLocks), [semanticNodes, snapshotLinks, spatialLevel, selected, settledRevision, spatialReveal, spatialLocks]);
  const { nodes: currentDisplayNodes, counts: spatialCounts, groups: spatialGroups } = spatial;
  const revealKey = currentDisplayNodes.map(node => node.id).sort().join("|");
  // Context/filter changes must never retain content no longer in the current source.
  const revealContext = `${snapshot.scope}:${snapshot.view}:${nodes.map(node => node.id).sort().join("|")}`;
  const [reveal, setReveal] = useState(() => revealState(revealKey, revealContext, currentDisplayNodes.map(node => node.id)));
  if (reveal.key !== revealKey || reveal.context !== revealContext || reduced && (reveal.retiring.size > 0 || reveal.entering.size > 0)) {
    setReveal(retargetReveal(reveal, revealKey, revealContext, currentDisplayNodes.map(node => node.id), new Set(nodes.map(node => node.id)), performance.now(), reduced));
  }
  const revealRef = useRef(reveal); revealRef.current = reveal;
  useEffect(() => {
    const starts = [...reveal.entering.values(), ...reveal.retiring.values()];
    if (!starts.length) return;
    const timer = window.setTimeout(() => setReveal(current => retargetReveal(current, current.key, current.context,
      [...current.current], new Set(nodes.map(node => node.id)), performance.now(), reduced)), Math.max(0, Math.max(...starts) + REVEAL_DURATION - performance.now()) + 16);
    return () => window.clearTimeout(timer);
  }, [reveal, nodes, reduced]);
  const displayNodes = useMemo(() => [...currentDisplayNodes, ...nodes.filter(node => reveal.retiring.has(node.id))], [currentDisplayNodes, nodes, reveal]);
  const visualLinks = useMemo(() => {
    const ids = new Set(displayNodes.map(node => node.id));
    return [...displayLinks, ...links.filter(link => ids.has(link.source) && ids.has(link.target) &&
      (reveal.retiring.has(link.source) || reveal.retiring.has(link.target)) && !displayLinks.some(current => current.source === link.source && current.target === link.target && current.kind === link.kind))];
  }, [displayNodes, displayLinks, links, reveal]);
  // Camera detail bands change visibility, not the content/status of the snapshot.
  const changedFootprints = useMemo(() => JSON.stringify(nodes.filter(node => node.changed).map(node => node.id).sort()), [nodes]);
  const summaryCounts = useMemo(() => {
    const counts = new Map(spatialCounts), byId = new Map(nodes.map(node => [node.id, node]));
    for (const [id, count] of collapsedCounts) counts.set(id, count - Number(byId.get(id)?.kind === "folder"));
    return counts;
  }, [nodes, collapsedCounts, spatialCounts]);
  const glyphMembers = useMemo(() => {
    const byId = new Map(nodes.map(node => [node.id, node]));
    const members = new Map<string, PositionedNode[]>();
    for (const id of collapsedCounts.keys()) {
      const core = cores.find(candidate => candidate.hub === id);
      if (core) members.set(id, [...core.members].map(memberId => byId.get(memberId)).filter((node): node is PositionedNode => Boolean(node) && !(node!.id === id && (node!.kind === "folder" || snapshot.view === "purpose" && node!.kind === "subject"))));
    }
    for (const [id, group] of spatialGroups) members.set(id, group.members.map(memberId => byId.get(memberId)).filter((node): node is PositionedNode => Boolean(node)));
    return members;
  }, [nodes, collapsedCounts, cores, spatialGroups, snapshot.view]);
  const collapsibleCore = useMemo(() => !selected && expandedCore && constellationView(nodes, links, null, null, undefined, snapshot.view).counts.has(expandedCore), [nodes, links, selected, expandedCore, snapshot.view]);
  const activeExpandedCore = collapsibleCore ? expandedCore : null;
  // A filter can remove a hub and expose members hidden in the full overview.
  // Give those visible nodes their own positions instead of stacking them at the missing hub.
  const exposesHiddenMembers = useMemo(() => {
    if (nodes.length === snapshot.nodes.length) return false;
    const overview = constellationView(snapshot.nodes, snapshotLinks, null, null, undefined, snapshot.view);
    const visible = new Set(overview.nodes.map(node => node.id));
    return displayNodes.some(node => !visible.has(node.id));
  }, [nodes, snapshot, displayNodes]);
  const layoutCore = activeExpandedCore ?? cores.find(core => selected && core.members.has(selected))?.hub ?? (exposesHiddenMembers ? "*" : null);
  useEffect(() => { if (expandedCore && !activeExpandedCore) { setExpandedCore(null); coreReturn.current = null; pendingReturn.current = null; } }, [expandedCore, activeExpandedCore]);
  const data = useMemo(() => ({ nodes: displayNodes.map(n => ({ ...n })), links: visualLinks.map(l => ({ ...l })) }), [displayNodes, visualLinks]);
  const motionClock = useRef<StarClock>({ seconds: 0, lastTime: null });
  const motionReduced = useRef(reduced); motionReduced.current = reduced;
  const resources = useMemo(() => ({ geometry: new PlaneGeometry(1, 1), star: starMaterial(), ring: texture("ring"), selection: texture("selection"), change: texture("change"), materials: new Map<string, SpriteMaterial>(), lines: new Map<string, LineBasicMaterial>(), summaries: new Map<string, ReturnType<typeof summaryGlyphTexture> & { signature: string; material: SpriteMaterial }>() }), []);
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
    for (const material of resources.lines.values()) material.dispose();
    resources.lines.clear();
    for (const summary of resources.summaries.values()) { summary.material.dispose(); summary.texture.dispose(); }
    resources.summaries.clear();
  }, [resources]);
  useEffect(() => {
    // ForceGraph replaces old node objects during this commit. Retire absent
    // summary textures on the following frame, after their sprites are gone.
    const frame = requestAnimationFrame(() => {
      for (const [id, glyph] of resources.summaries) if (!glyphMembers.has(id) && !reveal.entering.size && !reveal.retiring.size) {
        glyph.material.dispose(); glyph.texture.dispose(); resources.summaries.delete(id);
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [glyphMembers, resources, reveal]);
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
    if (controls) {
      controls.zoomToCursor = true;
      controls.enableRotate = false;
      controls.enablePan = true;
      controls.screenSpacePanning = true;
      controls.mouseButtons.LEFT = MOUSE.PAN;
      controls.touches.ONE = TOUCH.PAN;
      controls.touches.TWO = TOUCH.DOLLY_PAN;
      controls.autoRotate = rotate;
      controls.autoRotateSpeed = .2;
      controls.enableDamping = !reduced;
    }
  }, [ready, rotate, reduced]);
  const viewKey = nodes.map(n => n.id).sort().join("|");
  useEffect(() => {
    const linked = new Set(snapshotLinks.flatMap(link => [link.source, link.target]));
    const eligible = new Set(semanticNodes.filter(node => knowledge(node) && !linked.has(node.id)).map(node => node.id));
    setSpatialReveal(current => {
      if (!current?.members.size) return current;
      const members = new Set([...current.members].filter(id => eligible.has(id)));
      return members.size === current.members.size ? current : members.size ? { ...current, members } : null;
    });
  }, [semanticNodes, snapshotLinks]);
  useEffect(() => () => { if (spatialCloseTimer.current !== null) window.clearTimeout(spatialCloseTimer.current); }, []);
  // A relationship refresh can change the expanded core without changing node IDs.
  // Dragged coordinates are deliberately excluded so an unchanged refresh keeps the camera.
  const expandedRelationKey = useMemo(() => {
    if (!activeExpandedCore || selected) return "";
    const members = cores.find(core => core.hub === activeExpandedCore)?.members;
    return members ? expandedCoreCameraFrame(nodes, links, activeExpandedCore, members)?.key ?? "" : "";
  }, [nodes, links, cores, selected, activeExpandedCore]);
  const spatialCameraKey = spatialReveal?.members.size ? `${positions.structureEpoch}:${[...spatialReveal.members].sort().join("|")}` : "";
  const cameraKey = `${viewKey}:${selected}:${activeExpandedCore}:${disclosure?.index ?? "all"}:${fit}:${size.width}:${size.height}:${expandedRelationKey}:${spatialCameraKey}`;
  const appliedCamera = useRef("");
  const appliedFit = useRef(fit);
  const plannedCamera = useRef<{ key: string; camera: PerspectiveCamera; target: { x: number; y: number; z: number } } | null>(null);
  const renderKey = JSON.stringify([cameraKey, changedFootprints, positions.groupEpoch]);
  useEffect(() => {
    const instance = graph.current;
    if (!ready || !instance) return;
    const group = cores.find(core => core.hub === layoutCore);
    positions.showCore(layoutCore, performance.now(), reduced, disclosure?.index, disclosure?.visible,
      nodes.length !== snapshot.nodes.length ? { nodes, links, view: snapshot.view } : undefined, group ? {
        key: renderKey,
        resolve: (desired, movable, hub) => {
          const automatic = appliedCamera.current !== cameraKey;
          const plan = planCoreView(desired, movable, hub, {
            camera: instance.camera() as PerspectiveCamera, cameraTarget: (instance.controls() as OrbitControls).target,
            size, automatic, nodes: semanticNodes, links: snapshotLinks, counts: collapsedCounts,
            members: disclosure?.hub === hub ? disclosure.visible : group.members, selected, lod: lodLevelRef.current,
            cohorts: new Map(cores.filter(core => collapsedCounts.has(core.hub)).map(core => [core.hub, [...core.members]])),
            spatial: spatialReveal, locks: new Map([...movedGroups].filter(([, value]) => value.epoch === positions.groupEpoch)),
          }, (desired, movable, held, plane) => positions.clearLayout(desired, movable, held, plane));
          if (automatic) plannedCamera.current = { key: cameraKey, camera: plan.camera, target: plan.target };
          // The committed view keeps the exact cohorts whose footprints were
          // cleared, even when their translation crosses a spatial cell boundary.
          if (plan.memberships.size) setMovedGroups(previous => {
            const next = new Map(previous);
            for (const [id, members] of plan.memberships) next.set(id, { members, level: plan.level, epoch: positions.groupEpoch });
            return next;
          });
          return plan.positions;
        },
      } : undefined);
  }, [positions, layoutCore, disclosure, nodes, links, snapshot.nodes.length, snapshotLinks, reduced, ready, semanticNodes,
    cores, collapsedCounts, renderKey, cameraKey, size, selected, spatialReveal, movedGroups]);
  const positionCamera = useCallback(() => {
    const instance = graph.current;
    if (!ready || !instance || !nodes.length || positions.dragging || appliedCamera.current === cameraKey) return;
    const hadCamera = Boolean(appliedCamera.current);
    appliedCamera.current = cameraKey;
    const fitChanged = appliedFit.current !== fit;
    appliedFit.current = fit;
    if (!fitChanged && pendingReturn.current) {
      const pose = pendingReturn.current; pendingReturn.current = null;
      moveCamera(pose.position, pose.target, reduced ? 0 : REVEAL_DURATION);
      return;
    }
    // Empty members denotes a spatial return. Its own camera motion owns this interval.
    if (!fitChanged && spatialReveal && !spatialReveal.members.size) return;
    if (fitChanged) {
      setSpatialReveal(null); setExpandedCore(null); coreReturn.current = null; pendingReturn.current = null;
      positions.showCore(exposesHiddenMembers ? "*" : null, performance.now(), reduced);
    }
    // Frame the settled target while nodes travel there; framing their current
    // positions would zoom into the still-collapsed core and then jump outward.
    const cameraNodes = nodes.map(node => ({ ...node, ...(positions.layoutTarget(node.id) ?? {}) }));
    const cameraDisplayNodes = semanticNodes.map(node => ({ ...node, ...(positions.layoutTarget(node.id) ?? {}) }));
    const target = fitChanged ? null : cameraNodes.find(n => n.id === (selected ?? activeExpandedCore));
    const planned = plannedCamera.current;
    if (target && !fitChanged && planned?.key === cameraKey) {
      moveCamera({ x: planned.camera.position.x, y: planned.camera.position.y, z: planned.camera.position.z }, planned.target, reduced ? 0 : 650);
      return;
    }
    if (target && selected) {
      // Selection reveals the rotating surface while preserving any closer
      // user zoom and the current viewing direction, including deep links.
      const camera = instance.camera() as PerspectiveCamera;
      const controls = instance.controls() as OrbitControls;
      const offset = camera.position.clone().sub(controls.target);
      const distance = focusedCameraDistance(offset.length(), size.height, camera.projectionMatrix.elements[5]);
      if (offset.lengthSq() < 1e-9) offset.set(0, 0, 1);
      offset.normalize().multiplyScalar(distance);
      moveCamera({ x: target.x + offset.x, y: target.y + offset.y, z: target.z + offset.z }, { x: target.x, y: target.y, z: target.z }, reduced ? 0 : 650);
      return;
    }
    if (spatialReveal?.members.size && !fitChanged) {
      const members = cameraNodes.filter(node => spatialReveal.members.has(node.id));
      if (members.length) {
        const center = members.reduce((sum, node) => ({ x: sum.x + node.x / members.length, y: sum.y + node.y / members.length, z: sum.z + node.z / members.length }), { x: 0, y: 0, z: 0 });
        const radius = Math.max(...members.map(node => Math.hypot(node.x - center.x, node.y - center.y, node.z - center.z))) + 6;
        const frame = spatialCameraFrame(instance.camera() as PerspectiveCamera, (instance.controls() as OrbitControls).target, { center, radius }, size);
        moveCamera(frame.position, frame.target, reduced ? 0 : 650);
        return;
      }
    }
    if (target && activeExpandedCore) {
      const members = cores.find(core => core.hub === activeExpandedCore)?.members;
      if (members) {
        const radius = expandedCoreCameraFrame(cameraNodes, links, activeExpandedCore,
          disclosure?.hub === activeExpandedCore ? disclosure.visible : members)?.radius ?? 18;
        const camera = instance.camera() as PerspectiveCamera;
        const controls = instance.controls() as OrbitControls;
        const vertical = camera.fov * Math.PI / 180;
        const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * size.width / size.height);
        // Fit large cores, but do not magnify a compact page until ordinary
        // 31-unit layout steps look like stretched links. Manual zoom stays free.
        const distance = coreCameraDistance(radius * 1.15 / Math.sin(Math.min(vertical, horizontal) / 2),
          size.height, camera.projectionMatrix.elements[5]);
        const offset = camera.position.clone().sub(controls.target);
        if (offset.lengthSq() < 1e-9) offset.set(0, 0, 1);
        offset.normalize().multiplyScalar(distance);
        moveCamera({ x: target.x + offset.x, y: target.y + offset.y, z: target.z + offset.z }, { x: target.x, y: target.y, z: target.z }, reduced ? 0 : 650);
        return;
      }
    }
    const overviewNodes = fitChanged ? constellationView(cameraNodes, links, null, null, undefined, snapshot.view).nodes : cameraDisplayNodes;
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
    // Establish the overview before packing fixed-size glyphs in its screen plane.
    moveCamera({ x: lookAt.x + offset.x, y: lookAt.y + offset.y, z: lookAt.z + offset.z }, lookAt, 0);
  }, [ready, cameraKey, nodes, semanticNodes, links, selected, activeExpandedCore, cores, disclosure, reduced, size, positions, fit, snapshot.nodes.length, spatialReveal, exposesHiddenMembers, moveCamera]);
  useEffect(() => {
    let second = 0;
    const first = requestAnimationFrame(() => { second = requestAnimationFrame(positionCamera); });
    return () => { cancelAnimationFrame(first); cancelAnimationFrame(second); };
  }, [positionCamera]);
  useEffect(() => {
    const canvas = graph.current?.renderer().domElement;
    const owner = canvas?.ownerDocument;
    // ForceGraph emits a document-only touch release with no pointer ID. OrbitControls
    // must release the original drag pointer, including when DragControls ends on leave.
    const releaseControlPointer = (event: PointerEvent) => {
      const input = controlPointer.current;
      if (input && !event.isTrusted && event.target === owner && event.pointerType === "touch" && event.pointerId === 0) {
        event.stopImmediatePropagation(); controlPointer.current = null;
        owner?.dispatchEvent(new PointerEvent("pointerup", input));
      }
    };
    const clearControlPointer = (event: PointerEvent) => {
      activePointers.current.delete(event.pointerId);
      if (event.pointerId === controlPointer.current?.pointerId) controlPointer.current = null;
    };
    owner?.addEventListener("pointerup", releaseControlPointer, true);
    // Clear after canvas DragControls dispatches its compatibility release.
    owner?.addEventListener("pointerup", clearControlPointer);
    owner?.addEventListener("pointercancel", clearControlPointer);
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
      const input = controlPointer.current ?? pointer.current;
      const inputs = new Map(activePointers.current);
      if (input) inputs.set(input.pointerId, input.pointerType);
      for (const [pointerId, pointerType] of inputs) canvas?.dispatchEvent(new PointerEvent("pointerup", { pointerId, pointerType, bubbles: true }));
      pointer.current = null; controlPointer.current = null; activePointers.current.clear();
    };
    window.addEventListener("blur", cancel);
    canvas?.addEventListener("pointercancel", cancel);
    if (!visible || disabled) cancel();
    return () => {
      window.removeEventListener("blur", cancel); canvas?.removeEventListener("pointercancel", cancel); cancel();
      owner?.removeEventListener("pointerup", releaseControlPointer, true);
      owner?.removeEventListener("pointerup", clearControlPointer); owner?.removeEventListener("pointercancel", clearControlPointer);
      controlPointer.current = null; activePointers.current.clear();
    };
  }, [ready, positions, snapshot, nodes, visible, disabled, fit]);
  const dragNode = (node: PositionedNode) => {
    const instance = graph.current;
    if (!allowDrag.current || disabled || !instance) return;
    if (!positions.dragging) {
      const camera = instance.camera(), controls = instance.controls() as OrbitControls;
      const origin = nodes.find(n => n.id === node.id)!;
      const depth = -new Vector3(origin.x, origin.y, origin.z).applyMatrix4(camera.matrixWorldInverse).z;
      const projectionY = camera.projectionMatrix.elements[5];
      const unitsPerPixel = 2 * Math.max(.001, depth) / (size.height * projectionY);
      const viewport = () => ({ width: container.current?.clientWidth || size.width, height: container.current?.clientHeight || size.height });
      const group = spatialGroups.get(node.id);
      const cohorts = new Map<string, readonly string[]>([
        ...[...spatialGroups].map(([id, value]) => [id, value.members] as const),
        ...cores.filter(core => collapsedCounts.has(core.hub)).map(core => [core.hub, [...core.members]] as const),
      ]);
      summaryHalo.current = group || collapsedCounts.has(node.id)
        ? { id: node.id, startedAt: performance.now(), releasedAt: null, releaseScale: 1 } : null;
      const plane = { ...screenPlane(camera, viewport, selected, summaryCounts),
        visible: displayNodes.map(value => value.id), cohorts };
      if (group) {
        draggedSummary.current = { id: node.id, members: group.members, level: spatialLevel, epoch: positions.groupEpoch };
        frozenSpatial.current = spatial;
        positions.beginGroup(node.id, group.members, unitsPerPixel, plane);
      } else positions.begin(node.id, unitsPerPixel, plane);
      if (positions.dragging) { dragged.current = node.id; setDraggingId(node.id); }
      cameraMotion.current.stop();
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
      element.children[0].textContent = collapsedCounts.has(id) ? coreLabel(label.node, collapsedCounts.get(id)!, snapshot.view) : spatialCounts.has(id) ? `근접 묶음 · ${spatialCounts.get(id)}개` : label.title;
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
      const pose = cameraMotion.current.advance(performance.now());
      if (pose) writeCamera(pose);
      // Built-in uniforms need distinct materials for independently fading objects.
      for (const link of data.links) for (const [key, opacity] of [["__lineObj", .65], ["__arrowObj", 1.95]] as const) {
        const object = (link as unknown as Record<string, Object3D>)[key] as (Object3D & { material?: { opacity: number } }) | undefined;
        if (!object?.material || object.userData.revealFade) continue;
        object.userData.revealFade = true;
        const from = String(endpoint(link.source)), to = String(endpoint(link.target));
        object.onBeforeRender = () => { if (object.material) object.material.opacity = opacity * Math.min(revealOpacity(revealRef.current, from, performance.now()), revealOpacity(revealRef.current, to, performance.now())); };
      }
      instance?.camera().updateMatrixWorld();
      positions.advance(performance.now(), reduced);
      advanceStarClock(motionClock.current, performance.now(), reduced);
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
      if (controls) controls.autoRotate = rotate && !focusedSummary && !positions.dragging && !positions.settling && !positions.layoutMoving && !cameraMotion.current.moving && controls.enabled;
      const camera = instance?.camera();
      if (camera) {
        camera.updateMatrixWorld();
        const distance = camera.position.distanceTo((instance!.controls() as OrbitControls).target);
        const scale = size.height * camera.projectionMatrix.elements[5] / (2 * distance);
        const zoom = overviewZoom.current;
        if (Number.isFinite(scale) && scale > 0 && Math.abs(scale - zoom.scale) > Math.max(scale, zoom.scale) * 1e-5) {
          zoom.scale = scale; zoom.changedAt = performance.now();
        }
        if (!positions.dragging && !positions.layoutMoving && !cameraMotion.current.moving && size.height > 0) {
          const next = nucleusLevel(LAYOUT_WORLD_SPACING * scale, lodLevelRef.current);
          if (next !== lodLevelRef.current) {
            lodLevelRef.current = next; hoveredId.current = null; setHover(null); setLodLevel(next);
          }
        }
        if (!selected && !activeExpandedCore && !spatialReveal?.members.size &&
          spatialLevel === lodLevelRef.current && appliedCamera.current === cameraKey &&
          !positions.dragging && !positions.settling && !positions.layoutMoving && !cameraMotion.current.moving && !reveal.retiring.size) {
          const cohorts = new Map([...glyphMembers].map(([id, members]) => [id, members.map(member => member.id)]));
          // Compact a new view once. Zoom/LOD only reveals the existing world positions.
          const packingKey = JSON.stringify([positions.structureEpoch, viewKey, fit, size]);
          const overviewChanged = packedOverview.current !== packingKey;
          if (performance.now() - zoom.changedAt >= 150 && (overviewChanged || zoom.checkedFootprints !== changedFootprints)) {
            const discs = displayNodes.map(node => {
              projected.set(node.x, node.y, node.z).applyMatrix4(camera.matrixWorldInverse);
              const depth = -projected.z, pixels = nodeScreenSize(node.kind, depth, size.height, camera.projectionMatrix.elements[5]);
              const radius = nodeVisualRadius(pixels, false, node.changed, summaryCounts.get(node.id));
              projected.applyMatrix4(camera.projectionMatrix);
              return { id: node.id, x: projected.x * size.width / 2, y: -projected.y * size.height / 2, depth, radius };
            });
            // Only real content/status changes can require clearance in an existing view.
            const overlap = discs.some((a, index) => discs.slice(index + 1).some(b =>
              Math.hypot(a.x - b.x, a.y - b.y) < a.radius + b.radius + 6 - .01));
            const packed = (overviewChanged || overlap) && positions.packOverview(discs, displayLinks, cohorts, {
              right: new Vector3().setFromMatrixColumn(camera.matrixWorld, 0),
              up: new Vector3().setFromMatrixColumn(camera.matrixWorld, 1).multiplyScalar(-1),
              worldPerPixel: depth => 2 * depth / (size.height * camera.projectionMatrix.elements[5]),
            }, performance.now(), reduced);
            if (packed || !overlap) {
              // Ungrouped views also compact once; a later first nucleus uses
              // the same coordinates instead of restarting the layout.
              packedOverview.current = packingKey;
              zoom.checkedFootprints = changedFootprints;
            }
            if (packed) {
              setMovedGroups(previous => {
                const next = new Map(previous);
                for (const [id, group] of spatialGroups) next.set(id, { members: group.members, level: spatialLevel, epoch: positions.groupEpoch });
                return next;
              });
            }
          }
        }
        const key = `${camera.matrixWorld.elements.join(",")}|${camera.projectionMatrix.elements.join(",")}|${hoveredId.current}|${focusedSummary}|${dragged.current}|${positions.revision}`;
        if (key !== lastProjection) {
          lastProjection = key;
          const candidates = displayNodes.map(node => {
            projected.set(node.x, node.y, node.z).applyMatrix4(camera.matrixWorldInverse);
            const summary = collapsedCounts.has(node.id) || spatialCounts.has(node.id);
            const pixels = nodeScreenSize(node.kind, -projected.z, size.height, camera.projectionMatrix.elements[5]);
            const radius = nodeVisualRadius(pixels, node.id === selected, node.changed, summaryCounts.get(node.id));
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
        for (const element of elements) {
          const id = element.dataset.nodeId;
          if (id) {
            const opacity = revealOpacity(revealRef.current, id, performance.now());
            element.style.opacity = String(opacity);
            element.style.pointerEvents = revealRef.current.retiring.has(id) ? "none" : "";
            if (revealRef.current.retiring.has(id)) { element.tabIndex = -1; element.setAttribute("aria-hidden", "true"); }
          }
        }
      }
      frame = requestAnimationFrame(draw);
    };
    draw(); return () => cancelAnimationFrame(frame);
  }, [ready, nodes, displayNodes, displayLinks, collapsedCounts, spatialCounts, spatialGroups, spatialLevel, summaryCounts, changedFootprints, glyphMembers, selected, size, visible, positions, data, reduced, rotate, positionCamera, cameraKey, viewKey, fit, activeExpandedCore, spatialReveal, writeCamera, reveal]);
  const object = useCallback((node: PositionedNode) => {
    const group = new Group();
    group.userData.nodeId = node.id;
    const material = (kind: "ring" | "selection" | "change" | "hit", color: string, opacity: number) => {
      const key = `${kind}:${color}:${opacity}:${kind === "hit" ? "shared" : node.id}`;
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
    const summaryPixels = summaryAppearance(summaryCounts.get(node.id) ?? 0).pixels;
    const haloScale = summary ? () => summaryHaloScale(summaryHalo.current, node.id, performance.now(), motionReduced.current) : undefined;
    const contactMotion = () => positions.collisionReaction(node.id, performance.now(), motionReduced.current);
    const fade = () => revealOpacity(revealRef.current, node.id, performance.now());
    group.userData.opacity = fade;
    group.userData.retiring = () => revealRef.current.retiring.has(node.id);
    const members = glyphMembers.get(node.id);
    if (summary && members?.length) {
      const samples = members.map(member => ({ id: member.id, kind: member.kind, taxonomyColor: member.taxonomyColor, opacity: active(member) ? 1 : .4 }));
      const signature = samples.map(member => `${member.id}:${starColor(member)}:${member.opacity}`).sort().join("|");
      let glyph = resources.summaries.get(node.id);
      if (!glyph) {
        const image = summaryGlyphTexture(node.id, samples);
        glyph = { signature, ...image, material: new SpriteMaterial({ map: image.texture, color: "#ffffff", transparent: true, opacity: 1, depthWrite: false, depthTest: true, sizeAttenuation: false }) };
        resources.summaries.set(node.id, glyph);
      } else if (glyph.signature !== signature) {
        const previous = glyph.texture;
        const image = summaryGlyphTexture(node.id, samples);
        glyph.texture = image.texture; glyph.updateMotion = image.updateMotion;
        glyph.material.map = glyph.texture;
        glyph.material.needsUpdate = true;
        glyph.signature = signature;
        previous.dispose();
      }
      group.add(screenSprite(glyph.material, node, "body", false, 0, summaryPixels,
        haloScale && (() => 1 + (haloScale() - 1) * .35),
        () => (contactMotion()?.pulse ?? 0) * .09,
        () => glyph.updateMotion(hoveredId.current === node.id && !!cursor.current && !positions.dragging,
          motionClock.current.seconds, motionReduced.current), fade));
    } else group.add(isKnowledge
      ? screenStar(resources.geometry, resources.star, node, motionClock.current, () => motionReduced.current, cursor, dragged, summary ? 22 : 0, haloScale, contactMotion, fade)
      : screenSprite(material("ring", color, active(node) ? 1 : .35), node, "body", isSelected, 0, undefined,
        () => (1 + ((haloScale?.() ?? 1) - 1) * 2) * (1 + (contactMotion()?.pulse ?? 0) * .05), undefined, undefined, fade));
    const oldGlyph = !summary && resources.summaries.get(node.id);
    if (oldGlyph && node.id === activeExpandedCore && reveal.entering.size) {
      const start = Math.min(...reveal.entering.values());
      group.add(screenSprite(oldGlyph.material, node, "body", false, 0, 26, undefined, undefined, undefined,
        () => Math.max(0, 1 - (performance.now() - start) / 180)));
    }
    if (spatialReveal?.members.has(node.id)) group.add(screenSprite(material("ring", "#89bad2", .7), node, "body", false, 0, 18));
    const summaryMetrics = summaryPixels ? nodeScreenMetrics(summaryPixels, isSelected, node.changed) : null;
    if (isSelected) group.add(screenSprite(material("selection", "#dce8f6", .52), node, "selection", isSelected, 0, summaryMetrics?.selection, undefined, undefined, undefined, fade));
    if (node.changed) group.add(screenSprite(material("change", "#edb66b", .9), node, "change", isSelected, 0, summaryMetrics?.change, undefined, undefined, undefined, fade));
    // The invisible plane follows the star and status rings, with a 36px minimum.
    group.add(screenSprite(material("hit", "#ffffff", 0), node, "hit", isSelected, 0, summaryMetrics?.hit, undefined, undefined, undefined, undefined,
      () => revealRef.current.current.has(node.id)));
    return group;
  }, [resources, selected, collapsedCounts, spatialCounts, spatialReveal, glyphMembers, summaryCounts, positions, activeExpandedCore, reveal]);
  const chooseNode = (id: string) => {
    hoveredId.current = null; setHover(null);
    if (disabled || !revealRef.current.current.has(id) || performance.now() < suppressClickUntil.current) return;
    if (collapsedCounts.has(id)) {
      if (selected && !onClearSelection()) return;
      if (!coreReturn.current) coreReturn.current = getCamera()?.pose ?? null;
      pendingReturn.current = null;
      setSpatialReveal(null);
      setCorePage(0); setShowAllCore(false);
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
      const frame = spatialCameraFrame(camera, controls.target, group, size);
      moveCamera(frame.position, frame.target, reduced ? 0 : 650);
    } else {
      coreReturn.current = null;
      if (!spatialReveal?.members.has(id)) setSpatialReveal(null);
      onSelect(id);
    }
  };
  const closeSpatialReveal = () => {
    if (!spatialReveal?.members.size) return;
    const closing = { ...spatialReveal, members: new Set<string>() };
    setSpatialReveal(closing);
    moveCamera(spatialReveal.camera, spatialReveal.target, reduced ? 0 : 650);
    if (spatialCloseTimer.current !== null) window.clearTimeout(spatialCloseTimer.current);
    spatialCloseTimer.current = window.setTimeout(() => {
      setSpatialReveal(current => current === closing ? null : current);
      spatialCloseTimer.current = null;
    }, reduced ? 0 : REVEAL_DURATION + 16);
  };
  return <div className="graph-canvas" ref={container} tabIndex={-1}
    onPointerMoveCapture={event => {
      if (disabled || positions.dragging || (event.pointerType !== "mouse" && event.pointerType !== "pen")) { cursor.current = null; return; }
      const rect = event.currentTarget.getBoundingClientRect();
      cursor.current = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    }}
    onPointerLeave={() => { cursor.current = null; }}
    onWheelCapture={event => { if (event.target === graph.current?.renderer().domElement) cameraMotion.current.stop(); }}
    onPointerDownCapture={event => {
      cursor.current = null;
      // A new press is a new action; the guard only belongs to the prior drag's click.
      if (!positions.dragging) suppressClickUntil.current = 0;
      allowDrag.current = !positions.layoutMoving && (event.button === 0 || event.pointerType === "touch");
      if (!allowDrag.current) suppressClickUntil.current = performance.now() + 350;
      pointer.current ??= { pointerId: event.pointerId, pointerType: event.pointerType };
      pressedNode.current = null;
      const instance = graph.current, canvas = instance?.renderer().domElement;
      if (canvas && event.target === canvas) {
        cameraMotion.current.stop(); pendingReturn.current = null;
        if (activePointers.current.size === 0) controlPointer.current = null;
        activePointers.current.set(event.pointerId, event.pointerType);
        const rect = canvas.getBoundingClientRect(), camera = instance!.camera();
        camera.updateMatrixWorld();
        const ray = new Raycaster();
        ray.setFromCamera(new Vector2(2 * (event.clientX - rect.left) / rect.width - 1, 1 - 2 * (event.clientY - rect.top) / rect.height), camera);
        const objects = data.nodes.flatMap(node => (node as PositionedNode & { __threeObj?: Group }).__threeObj ?? []);
        const id = ray.intersectObjects(objects, true)[0]?.object.parent?.userData.nodeId as string | undefined;
        if (id) {
          if (!controlPointer.current) controlPointer.current = { pointerId: event.pointerId, pointerType: event.pointerType };
          if (event.button === 0 && allowDrag.current) pressedNode.current = { id, pointerId: event.pointerId, x: event.clientX, y: event.clientY };
        }
      }
    }}
    onPointerUpCapture={event => {
      const press = pressedNode.current;
      pressedNode.current = null;
      if (pointer.current?.pointerId === event.pointerId) pointer.current = null;
      if (!press || event.button !== 0 || press.pointerId !== event.pointerId || positions.dragging || Math.hypot(event.clientX - press.x, event.clientY - press.y) >= 6) return;
      // The renderer clicks its previous hover frame, which can miss a first tap.
      suppressClickUntil.current = 0;
      chooseNode(press.id);
      suppressClickUntil.current = performance.now() + 350;
    }}
    aria-label="3D 지식 지도. 빈 공간을 끌면 지도가 이동합니다. 묶음 별을 끌면 함께 이동하고 누르면 펼쳐 개별 별을 끌 수 있습니다. 스크롤로 확대·축소하고 보기 설정에서 느린 지도 회전을 켤 수 있습니다. Tab과 Enter로 묶음을 펼칠 수 있으며 전체 항목은 목록 보기에서 탐색할 수 있습니다.">
    {ready && <ForceGraph3D<PositionedNode, RenderLink>
      ref={graph} width={size.width} height={size.height} graphData={data}
      backgroundColor="rgba(0,0,0,0)" controlType="orbit" showNavInfo={false}
      nodeLabel={() => ""} linkLabel={() => ""} nodeThreeObject={object}
      linkMaterial={link => {
        const from = String(endpoint(link.source)), to = String(endpoint(link.target)), key = JSON.stringify([from, to, link.kind, link.current]);
        let material = resources.lines.get(key);
        if (!material) { material = new LineBasicMaterial({ transparent: true, depthWrite: false, opacity: .65 }); resources.lines.set(key, material); }
        const focus = draggingId ?? selected;
        material.color.set(focus && from !== focus && to !== focus ? "#35404b" : link.current ? linkColor[link.kind] : "#947867");
        return material;
      }}
      linkColor={link => {
        const focus = draggingId ?? selected;
        return focus && endpoint(link.source) !== focus && endpoint(link.target) !== focus ? "#35404b" : link.current ? linkColor[link.kind] : "#947867";
      }}
      linkWidth={0}
      linkOpacity={.65} linkDirectionalArrowLength={link => link.kind === "evidence" || link.kind === "parent" || link.kind === "reference" ? 2 : 0} linkDirectionalArrowRelPos={.8}
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
      onNodeHover={node => { hoveredId.current = node?.id ?? null; setHover(node ? collapsedCounts.has(node.id) ? { title: coreLabel(node, collapsedCounts.get(node.id)!, snapshot.view), detail: "별 끌기: 함께 이동 · 누르기: 펼치기" } : spatialCounts.has(node.id) ? { title: `근접 묶음 · ${spatialCounts.get(node.id)}개`, detail: "별 끌기: 함께 이동 · 누르기: 펼치기" } : { title: nodePresentation(node).title, detail: `${nodePresentation(node).subtitle ? `${nodePresentation(node).subtitle} · ` : ""}${kindName[node.kind]} · ${stateName(node)}` } : null); }}
      onLinkHover={link => { if (link) hoveredId.current = null; setHover(link ? { title: linkName[link.kind], detail: link.kind === "parent" ? "경로에서 계산 · 화살표는 상위 폴더 방향" : link.current ? "등록된 관계" : "과거 출처 근거 · 군집 계산에서 제외" } : null); }}
    />}
    <div className="node-labels" ref={labelLayer}
      onClick={event => { const label = (event.target as HTMLElement).closest<HTMLElement>(".node-label.interactive"); if (label?.dataset.nodeId) { if (event.detail === 0) suppressClickUntil.current = 0; chooseNode(label.dataset.nodeId); } }}
      onKeyDown={event => { if (event.key !== "Enter" && event.key !== " ") return; const label = (event.target as HTMLElement).closest<HTMLElement>(".node-label.interactive"); if (label?.dataset.nodeId) { event.preventDefault(); suppressClickUntil.current = 0; const id = label.dataset.nodeId, spatial = spatialCounts.has(id); chooseNode(id); requestAnimationFrame(() => (spatial ? spatialCloseButton : coreCloseButton).current?.focus()); } }}>
      {Array.from({ length: MAX_VISIBLE_LABELS }, (_, index) => <div className="node-label" hidden key={index}><strong /><span /><small /></div>)}
    </div>
    {activeExpandedCore && !selected && !spatialReveal?.members.size && <div className="graph-core-actions">
      <button type="button" ref={coreCloseButton} onClick={event => { if (event.detail === 0) container.current?.focus(); hoveredId.current = null; setHover(null); pendingReturn.current = coreReturn.current; coreReturn.current = null; setExpandedCore(null); }}>묶음 접기</button>
      {disclosure?.hub === activeExpandedCore && <>
        <button type="button" disabled={disclosure.index === 0} onClick={() => setCorePage(disclosure.index - 1)}>이전</button>
        <span>{coreLabel(nodes.find(node => node.id === disclosure.hub), disclosure.total, snapshot.view)} · {disclosure.index + 1}/{disclosure.pages}</span>
        <button type="button" disabled={disclosure.index + 1 === disclosure.pages} onClick={() => setCorePage(disclosure.index + 1)}>다음</button>
        <button type="button" onClick={() => setShowAllCore(true)}>전체 보기</button>
      </>}
      {showAllCore && cores.find(core => core.hub === activeExpandedCore && core.count > 36) &&
        <button type="button" onClick={() => setShowAllCore(false)}>나눠 보기</button>}
    </div>}
    {spatialReveal && spatialReveal.members.size > 0 && !selected && <button type="button" ref={spatialCloseButton} className="graph-core-close" onClick={event => { if (event.detail === 0) container.current?.focus(); closeSpatialReveal(); }}>근접 묶음 {spatialReveal.members.size}개 접기</button>}
    {ready && <MiniMap nodes={currentDisplayNodes} getCamera={getCamera} onMove={panMiniMap} onZoom={zoomMiniMap} onFit={onFit} disabled={disabled || positions.dragging} visible={visible} />}
    {hover && <div className="graph-tooltip" role="status"><strong>{hover.title}</strong><span>{hover.detail}</span></div>}
    <div className="graph-instructions" aria-hidden="true">빈 공간 끌기: 지도 이동 · 묶음 별 끌기: 함께 이동 · 누르기: 펼치기 · 스크롤 확대·축소</div>
  </div>;
}
