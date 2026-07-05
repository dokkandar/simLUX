// TypeScript mirror of the Rust serde model (src-tauri/src/model & engine).
// Keep field names in sync with the `#[derive(Serialize)]` structs.

export interface Point2 {
  x: number;
  y: number;
}

export interface Line2 {
  start: Point2;
  end: Point2;
}

export interface Vertex {
  x: number;
  y: number;
  z: number;
}

export type PhotometryType = "A" | "B" | "C";

export interface IesProfile {
  name: string;
  photometry: PhotometryType;
  lumens: number;
  multiplier: number;
  vertical_angles: number[];
  horizontal_angles: number[];
  candela: number[][];
  width: number;
  length: number;
  height: number;
}

export interface RayTracingSettings {
  rays_per_point: number;
  max_bounces: number;
  shadows: boolean;
}

export interface LuxGrid {
  cols: number;
  rows: number;
  values: number[];
  min: number;
  max: number;
  avg: number;
}

export interface Material {
  id: number;
  name: string;
  reflectance: number;
  color: [number, number, number];
}

export interface LuminaireInstance {
  id: number;
  profile: string;
  position: Vertex;
  rotation_deg: number;
  dimming: number;
}

export interface CalculationPlane {
  origin: Vertex;
  width: number;
  depth: number;
  cols: number;
  rows: number;
}

export interface Wall {
  centerline: Line2;
  thickness: number;
  height: number;
}

export interface Room {
  id: number;
  name: string;
  walls: Wall[];
}

export interface Triangle {
  a: number;
  b: number;
  c: number;
}

export interface Mesh {
  vertices: Vertex[];
  triangles: Triangle[];
  material: number;
}

export interface Project {
  name: string;
  rooms: Room[];
  luminaires: LuminaireInstance[];
  materials: Material[];
  profiles: Record<string, IesProfile>;
  dxf_lines: Line2[];
  meshes: Mesh[];
  calc_plane: CalculationPlane | null;
  settings: RayTracingSettings;
}

export interface EngineInfo {
  name: string;
  version: string;
  phase: string;
}
