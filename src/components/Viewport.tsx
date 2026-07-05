import { useEffect, useMemo } from "react";
import { Canvas, useThree } from "@react-three/fiber";
import { Grid, OrbitControls, GizmoHelper, GizmoViewport } from "@react-three/drei";
import * as THREE from "three";
import { useStore } from "../store/projectStore";
import type { Line2 } from "../types";

interface Flattened {
  geometry: THREE.BufferGeometry | null;
  size: number; // largest plan dimension, world units
}

/** Build a line-segment geometry recentred on the drawing's bbox centre so any
 *  DXF (which may sit at large absolute coordinates) lands around the origin. */
function useFlattened(lines: Line2[]): Flattened {
  return useMemo(() => {
    if (lines.length === 0) return { geometry: null, size: 0 };

    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const l of lines) {
      minX = Math.min(minX, l.start.x, l.end.x);
      maxX = Math.max(maxX, l.start.x, l.end.x);
      minY = Math.min(minY, l.start.y, l.end.y);
      maxY = Math.max(maxY, l.start.y, l.end.y);
    }
    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;

    const positions = new Float32Array(lines.length * 6);
    lines.forEach((l, i) => {
      // plan (x, y) -> world (x, up, -y), recentred on (cx, cy).
      positions.set(
        [l.start.x - cx, 0.01, -(l.start.y - cy), l.end.x - cx, 0.01, -(l.end.y - cy)],
        i * 6,
      );
    });
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
    return { geometry, size: Math.max(maxX - minX, maxY - minY) };
  }, [lines]);
}

/** Frames the camera to the drawing whenever its size changes. */
function FitView({ size }: { size: number }) {
  const camera = useThree((s) => s.camera);
  const controls = useThree((s) => s.controls) as
    | { target: THREE.Vector3; update: () => void }
    | null;

  useEffect(() => {
    if (size <= 0) return;
    const d = size * 1.2;
    camera.position.set(d, d, d);
    if (camera instanceof THREE.PerspectiveCamera) {
      camera.near = Math.max(0.01, d / 1000);
      camera.far = d * 20;
      camera.updateProjectionMatrix();
    }
    if (controls) {
      controls.target.set(0, 0, 0);
      controls.update();
    }
  }, [size, camera, controls]);

  return null;
}

export default function Viewport() {
  const dxfLines = useStore((s) => s.dxfLines);
  const { geometry, size } = useFlattened(dxfLines);

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
        fadeDistance={Math.max(60, size * 2.5)}
        followCamera={false}
      />

      {geometry && (
        <lineSegments geometry={geometry}>
          <lineBasicMaterial color="#4ea1ff" />
        </lineSegments>
      )}

      <FitView size={size} />
      <OrbitControls makeDefault />
      <GizmoHelper alignment="bottom-right" margin={[70, 70]}>
        <GizmoViewport axisColors={["#e0576b", "#57e08a", "#4ea1ff"]} labelColor="white" />
      </GizmoHelper>
    </Canvas>
  );
}
