import type * as THREE from "three";

// Engine world is Z-up; three.js is Y-up. (ex, ey, ez) -> (ex, ez, -ey).
export const toThree = (x: number, y: number, z: number): [number, number, number] => [x, z, -y];

// Inverse for a point on the ground plane: three (x, _, z) -> engine (x, -z).
export const engineXY = (p: THREE.Vector3): [number, number] => [p.x, -p.z];
