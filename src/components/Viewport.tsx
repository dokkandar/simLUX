import { useMemo } from "react";
import { Canvas } from "@react-three/fiber";
import { Grid, OrbitControls, GizmoHelper, GizmoViewport } from "@react-three/drei";
import * as THREE from "three";
import { useStore } from "../store/projectStore";
import type { Line2 } from "../types";

/** Renders imported DXF plan geometry as flat line segments on the ground plane. */
function DxfLines({ lines }: { lines: Line2[] }) {
  const geometry = useMemo(() => {
    const positions = new Float32Array(lines.length * 6);
    lines.forEach((l, i) => {
      // DXF plan (x, y) -> world (x, up, -y): +y in plan reads as "north".
      positions.set(
        [l.start.x, 0.01, -l.start.y, l.end.x, 0.01, -l.end.y],
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
      <lineBasicMaterial color="#4ea1ff" />
    </lineSegments>
  );
}

export default function Viewport() {
  const dxfLines = useStore((s) => s.dxfLines);

  return (
    <Canvas camera={{ position: [8, 8, 8], fov: 45 }}>
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
        fadeDistance={60}
        followCamera={false}
      />

      <DxfLines lines={dxfLines} />

      <OrbitControls makeDefault />
      <GizmoHelper alignment="bottom-right" margin={[70, 70]}>
        <GizmoViewport axisColors={["#e0576b", "#57e08a", "#4ea1ff"]} labelColor="white" />
      </GizmoHelper>
    </Canvas>
  );
}
