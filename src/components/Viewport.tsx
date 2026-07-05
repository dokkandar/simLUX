import { useEffect, useMemo } from "react";
import { Canvas, useThree } from "@react-three/fiber";
import { Grid, OrbitControls, GizmoHelper, GizmoViewport } from "@react-three/drei";
import * as THREE from "three";
import { useStore } from "../store/projectStore";
import type { CalculationPlane, Line2, LuxGrid, Mesh as SceneMesh, Project } from "../types";
import { toThree } from "../three/coords";
import { WallDraw, WallFootprints } from "./WallLayer";

const MATERIAL_COLOR: Record<number, string> = { 0: "#5a5f66", 1: "#8a9098", 2: "#c9ced6" };

/** 5-stop perceptual-ish ramp: dark-blue -> blue -> green -> yellow -> red. */
function ramp(t: number): [number, number, number] {
  const x = Math.min(1, Math.max(0, t));
  const stops: Array<[number, [number, number, number]]> = [
    [0.0, [0.05, 0.05, 0.28]],
    [0.25, [0.13, 0.56, 0.94]],
    [0.5, [0.15, 0.86, 0.42]],
    [0.75, [0.98, 0.87, 0.2]],
    [1.0, [0.9, 0.16, 0.12]],
  ];
  for (let i = 0; i < stops.length - 1; i++) {
    const [t0, c0] = stops[i];
    const [t1, c1] = stops[i + 1];
    if (x <= t1) {
      const f = (x - t0) / (t1 - t0 || 1);
      return [c0[0] + (c1[0] - c0[0]) * f, c0[1] + (c1[1] - c0[1]) * f, c0[2] + (c1[2] - c0[2]) * f];
    }
  }
  return stops[stops.length - 1][1];
}

function RoomMesh({ mesh }: { mesh: SceneMesh }) {
  const geometry = useMemo(() => {
    const pos = new Float32Array(mesh.vertices.length * 3);
    mesh.vertices.forEach((v, i) => {
      const [x, y, z] = toThree(v.x, v.y, v.z);
      pos.set([x, y, z], i * 3);
    });
    const idx = mesh.triangles.flatMap((t) => [t.a, t.b, t.c]);
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(pos, 3));
    g.setIndex(idx);
    g.computeVertexNormals();
    return g;
  }, [mesh]);

  return (
    <mesh geometry={geometry}>
      <meshStandardMaterial
        color={MATERIAL_COLOR[mesh.material] ?? "#7a7f88"}
        transparent
        opacity={0.12}
        side={THREE.DoubleSide}
        depthWrite={false}
      />
    </mesh>
  );
}

function Luminaires({ project }: { project: Project }) {
  return (
    <>
      {project.luminaires.map((l) => {
        const [x, y, z] = toThree(l.position.x, l.position.y, l.position.z);
        return (
          <group key={l.id} position={[x, y, z]}>
            <mesh>
              <sphereGeometry args={[0.09, 16, 16]} />
              <meshStandardMaterial color="#ffd24a" emissive="#ffb300" emissiveIntensity={1.4} />
            </mesh>
            {/* Downlight indicator cone (three cones point +Y; flip to -Y). */}
            <mesh position={[0, -0.16, 0]} rotation={[Math.PI, 0, 0]}>
              <coneGeometry args={[0.12, 0.28, 20, 1, true]} />
              <meshBasicMaterial color="#ffcf5c" transparent opacity={0.28} side={THREE.DoubleSide} />
            </mesh>
          </group>
        );
      })}
    </>
  );
}

function Heatmap({ grid, plane }: { grid: LuxGrid; plane: CalculationPlane }) {
  const geometry = useMemo(() => {
    const { cols, rows, values, max } = grid;
    const dx = plane.width / cols;
    const dy = plane.depth / rows;
    const { x: ox, y: oy, z: oz } = plane.origin;
    const h = oz + 0.02;
    const scale = max > 0 ? max : 1;

    const positions = new Float32Array(cols * rows * 6 * 3);
    const colors = new Float32Array(cols * rows * 6 * 3);
    let vi = 0;
    const push = (px: number, py: number, c: [number, number, number]) => {
      const [tx, ty, tz] = toThree(px, py, h);
      positions.set([tx, ty, tz], vi * 3);
      colors.set(c, vi * 3);
      vi++;
    };
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const col = ramp(values[r * cols + c] / scale);
        const x0 = ox + c * dx, x1 = ox + (c + 1) * dx;
        const y0 = oy + r * dy, y1 = oy + (r + 1) * dy;
        push(x0, y0, col); push(x1, y0, col); push(x1, y1, col);
        push(x0, y0, col); push(x1, y1, col); push(x0, y1, col);
      }
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    g.setAttribute("color", new THREE.BufferAttribute(colors, 3));
    return g;
  }, [grid, plane]);

  return (
    <mesh geometry={geometry}>
      <meshBasicMaterial vertexColors transparent opacity={0.92} side={THREE.DoubleSide} />
    </mesh>
  );
}

function DxfLines({ lines }: { lines: Line2[] }) {
  const geometry = useMemo(() => {
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const l of lines) {
      minX = Math.min(minX, l.start.x, l.end.x);
      maxX = Math.max(maxX, l.start.x, l.end.x);
      minY = Math.min(minY, l.start.y, l.end.y);
      maxY = Math.max(maxY, l.start.y, l.end.y);
    }
    const cx = (minX + maxX) / 2, cy = (minY + maxY) / 2;
    const positions = new Float32Array(lines.length * 6);
    lines.forEach((l, i) => {
      positions.set(
        [l.start.x - cx, 0.01, -(l.start.y - cy), l.end.x - cx, 0.01, -(l.end.y - cy)],
        i * 6,
      );
    });
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    return g;
  }, [lines]);

  if (lines.length === 0) return null;
  return (
    <lineSegments geometry={geometry}>
      {/* Dimmed — the DXF is a reference underlay, not lit geometry. */}
      <lineBasicMaterial color="#3c586f" transparent opacity={0.85} />
    </lineSegments>
  );
}

interface Bounds {
  center: [number, number, number];
  size: number;
}

function useBounds(project: Project | null, dxfLines: Line2[]): Bounds | null {
  return useMemo(() => {
    let any = false;
    const min = [Infinity, Infinity, Infinity];
    const max = [-Infinity, -Infinity, -Infinity];
    const add = (x: number, y: number, z: number) => {
      any = true;
      min[0] = Math.min(min[0], x); max[0] = Math.max(max[0], x);
      min[1] = Math.min(min[1], y); max[1] = Math.max(max[1], y);
      min[2] = Math.min(min[2], z); max[2] = Math.max(max[2], z);
    };
    const t = (x: number, y: number, z: number) => {
      const [a, b, c] = toThree(x, y, z);
      add(a, b, c);
    };

    project?.meshes.forEach((m) => m.vertices.forEach((v) => t(v.x, v.y, v.z)));
    project?.luminaires.forEach((l) => t(l.position.x, l.position.y, l.position.z));
    const pl = project?.calc_plane;
    if (pl) {
      const { origin: o, width: w, depth: d } = pl;
      t(o.x, o.y, o.z); t(o.x + w, o.y, o.z); t(o.x + w, o.y + d, o.z); t(o.x, o.y + d, o.z);
    }
    if (dxfLines.length) {
      let mnx = Infinity, mny = Infinity, mxx = -Infinity, mxy = -Infinity;
      for (const l of dxfLines) {
        mnx = Math.min(mnx, l.start.x, l.end.x); mxx = Math.max(mxx, l.start.x, l.end.x);
        mny = Math.min(mny, l.start.y, l.end.y); mxy = Math.max(mxy, l.start.y, l.end.y);
      }
      const cx = (mnx + mxx) / 2, cy = (mny + mxy) / 2;
      for (const l of dxfLines) {
        add(l.start.x - cx, 0, -(l.start.y - cy));
        add(l.end.x - cx, 0, -(l.end.y - cy));
      }
    }
    if (!any) return null;
    return {
      center: [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2],
      size: Math.max(max[0] - min[0], max[1] - min[1], max[2] - min[2]),
    };
  }, [project, dxfLines]);
}

function FitView({ bounds }: { bounds: Bounds | null }) {
  const camera = useThree((s) => s.camera);
  const controls = useThree((s) => s.controls) as
    | { target: THREE.Vector3; update: () => void }
    | null;

  useEffect(() => {
    if (!bounds) return;
    const [cx, cy, cz] = bounds.center;
    const d = Math.max(bounds.size, 0.5) * 1.4;
    camera.position.set(cx + d, cy + d, cz + d);
    if (camera instanceof THREE.PerspectiveCamera) {
      camera.near = Math.max(0.01, d / 1000);
      camera.far = d * 40;
      camera.updateProjectionMatrix();
    }
    if (controls) {
      controls.target.set(cx, cy, cz);
      controls.update();
    }
  }, [bounds, camera, controls]);

  return null;
}

export default function Viewport() {
  const project = useStore((s) => s.project);
  const dxfLines = useStore((s) => s.dxfLines);
  const luxGrid = useStore((s) => s.luxGrid);
  const bounds = useBounds(project, dxfLines);

  return (
    <Canvas camera={{ position: [8, 8, 8], fov: 45, near: 0.1, far: 5000 }}>
      <color attach="background" args={["#0e1116"]} />
      <ambientLight intensity={0.6} />
      <directionalLight position={[10, 20, 10]} intensity={0.8} />

      <Grid
        args={[50, 50]}
        cellSize={1}
        cellThickness={0.6}
        cellColor="#26303a"
        sectionSize={5}
        sectionThickness={1}
        sectionColor="#3a4a5a"
        infiniteGrid
        fadeDistance={Math.max(60, (bounds?.size ?? 0) * 2.5)}
      />

      {project?.meshes.map((m, i) => <RoomMesh key={i} mesh={m} />)}
      {luxGrid && project?.calc_plane && <Heatmap grid={luxGrid} plane={project.calc_plane} />}
      {project && <Luminaires project={project} />}
      {project && <WallFootprints walls={project.walls} />}
      <DxfLines lines={dxfLines} />
      <WallDraw />

      <FitView bounds={bounds} />
      <OrbitControls makeDefault />
      <GizmoHelper alignment="bottom-right" margin={[70, 70]}>
        <GizmoViewport axisColors={["#e0576b", "#57e08a", "#4ea1ff"]} labelColor="white" />
      </GizmoHelper>
    </Canvas>
  );
}
