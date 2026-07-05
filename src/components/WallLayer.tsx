import { useMemo, useRef, useState } from "react";
import type { ThreeEvent } from "@react-three/fiber";
import * as THREE from "three";
import { useStore } from "../store/projectStore";
import { addWall } from "../api/commands";
import type { WallSeg } from "../types";
import { toThree } from "../three/coords";

const SNAP_DIST = 0.2; // metres

function rectCorners(w: WallSeg): Array<[number, number]> {
  const dx = w.end.x - w.start.x;
  const dy = w.end.y - w.start.y;
  const len = Math.hypot(dx, dy) || 1;
  const nx = (-dy / len) * (w.thickness / 2);
  const ny = (dx / len) * (w.thickness / 2);
  return [
    [w.start.x + nx, w.start.y + ny],
    [w.end.x + nx, w.end.y + ny],
    [w.end.x - nx, w.end.y - ny],
    [w.start.x - nx, w.start.y - ny],
  ];
}

/** Flat filled rectangles showing each drawn wall's thickness on the ground. */
export function WallFootprints({ walls }: { walls: WallSeg[] }) {
  const geometry = useMemo(() => {
    if (!walls.length) return null;
    const pos: number[] = [];
    for (const w of walls) {
      const c = rectCorners(w).map(([x, y]) => toThree(x, y, 0.03));
      pos.push(...c[0], ...c[1], ...c[2], ...c[0], ...c[2], ...c[3]);
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(pos), 3));
    return g;
  }, [walls]);

  if (!geometry) return null;
  return (
    <mesh geometry={geometry}>
      <meshBasicMaterial color="#8b93a1" transparent opacity={0.85} side={THREE.DoubleSide} />
    </mesh>
  );
}

/** Click-to-draw walls on the ground plane (polyline), with endpoint snapping. */
export function WallDraw() {
  const drawMode = useStore((s) => s.drawMode);
  const pendingStart = useStore((s) => s.pendingStart);
  const setPendingStart = useStore((s) => s.setPendingStart);
  const setProject = useStore((s) => s.setProject);
  const setStatus = useStore((s) => s.setStatus);
  const thickness = useStore((s) => s.wallThickness);
  const project = useStore((s) => s.project);
  const dxfLines = useStore((s) => s.dxfLines);
  const [hover, setHover] = useState<[number, number] | null>(null);
  const downPos = useRef<{ x: number; y: number } | null>(null);

  // Snap targets in the drawing frame: existing wall nodes + centred DXF endpoints.
  const snapPts = useMemo(() => {
    const pts: Array<[number, number]> = [];
    project?.walls.forEach((w) => {
      pts.push([w.start.x, w.start.y], [w.end.x, w.end.y]);
    });
    if (dxfLines.length) {
      let mnx = Infinity, mny = Infinity, mxx = -Infinity, mxy = -Infinity;
      for (const l of dxfLines) {
        mnx = Math.min(mnx, l.start.x, l.end.x); mxx = Math.max(mxx, l.start.x, l.end.x);
        mny = Math.min(mny, l.start.y, l.end.y); mxy = Math.max(mxy, l.start.y, l.end.y);
      }
      const cx = (mnx + mxx) / 2, cy = (mny + mxy) / 2;
      for (const l of dxfLines) {
        pts.push([l.start.x - cx, l.start.y - cy], [l.end.x - cx, l.end.y - cy]);
      }
    }
    return pts;
  }, [project?.walls, dxfLines]);

  const snap = (x: number, y: number): [number, number] => {
    let best: [number, number] = [x, y];
    let bd = SNAP_DIST;
    for (const [px, py] of snapPts) {
      const d = Math.hypot(px - x, py - y);
      if (d < bd) { bd = d; best = [px, py]; }
    }
    return best;
  };

  const previewGeom = useMemo(() => {
    if (!pendingStart || !hover) return null;
    const a = toThree(pendingStart[0], pendingStart[1], 0.05);
    const b = toThree(hover[0], hover[1], 0.05);
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(new Float32Array([...a, ...b]), 3));
    return g;
  }, [pendingStart, hover]);

  if (!drawMode) return null;

  const onMove = (e: ThreeEvent<PointerEvent>) => {
    setHover(snap(e.point.x, -e.point.z));
  };
  const onClick = (e: ThreeEvent<MouseEvent>) => {
    e.stopPropagation();
    const d = downPos.current;
    if (d && Math.hypot(e.nativeEvent.clientX - d.x, e.nativeEvent.clientY - d.y) > 4) {
      return; // a drag (orbit), not a placement click
    }
    const [x, y] = snap(e.point.x, -e.point.z);
    if (!pendingStart) {
      setPendingStart([x, y]);
      setStatus("Wall start set — click the next point (Esc to finish).");
      return;
    }
    const [sx, sy] = pendingStart;
    if (Math.hypot(x - sx, y - sy) < 1e-4) return;
    addWall(sx, sy, x, y, thickness).then((p) => {
      setProject(p);
      setPendingStart([x, y]);
      setStatus(`Wall added (${p.walls.length} total). Click next, Esc to finish.`);
    });
  };

  return (
    <>
      <mesh
        rotation={[-Math.PI / 2, 0, 0]}
        onPointerMove={onMove}
        onPointerDown={(e) => {
          downPos.current = { x: e.nativeEvent.clientX, y: e.nativeEvent.clientY };
        }}
        onClick={onClick}
      >
        <planeGeometry args={[4000, 4000]} />
        <meshBasicMaterial transparent opacity={0} depthWrite={false} />
      </mesh>

      {hover && (
        <mesh position={toThree(hover[0], hover[1], 0.06)}>
          <sphereGeometry args={[0.06, 12, 12]} />
          <meshBasicMaterial color="#ffd24a" />
        </mesh>
      )}
      {previewGeom && (
        <lineSegments geometry={previewGeom}>
          <lineBasicMaterial color="#ffd24a" />
        </lineSegments>
      )}
    </>
  );
}
