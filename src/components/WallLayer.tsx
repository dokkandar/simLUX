import { useMemo } from "react";
import * as THREE from "three";
import type { WallSeg } from "../types";
import { toThree } from "../three/coords";

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

/** Flat wall footprints on the ground (a pre-extrude preview of the drawn plan). */
export function WallFootprints({ walls }: { walls: WallSeg[] }) {
  const geometry = useMemo(() => {
    if (!walls.length) return null;
    const pos: number[] = [];
    for (const w of walls) {
      if (w.thickness > 0.001) {
        const c = rectCorners(w).map(([x, y]) => toThree(x, y, 0.03));
        pos.push(...c[0], ...c[1], ...c[2], ...c[0], ...c[2], ...c[3]);
      }
    }
    if (!pos.length) return null;
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(pos), 3));
    return g;
  }, [walls]);

  if (!geometry) return null;
  return (
    <mesh geometry={geometry}>
      <meshBasicMaterial color="#8b93a1" transparent opacity={0.7} side={THREE.DoubleSide} />
    </mesh>
  );
}
