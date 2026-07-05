// Typed wrappers around the Tauri command layer (src-tauri/src/commands.rs).
// Tauri maps camelCase JS keys to snake_case Rust params automatically.
import { invoke } from "@tauri-apps/api/core";
import type { EngineInfo, IesProfile, Line2, LuxGrid, Project } from "../types";

export const engineInfo = () => invoke<EngineInfo>("engine_info");

export const getProject = () => invoke<Project>("get_project");

export const importIes = (path: string) =>
  invoke<IesProfile>("import_ies", { path });

export const loadDxf = (path: string) => invoke<Line2[]>("load_dxf", { path });

export const addLuminaire = (x: number, y: number, z: number, profile: string) =>
  invoke<Project>("add_luminaire", { x, y, z, profile });

export const addDemoRoom = (
  width: number,
  depth: number,
  height: number,
  planeHeight: number,
) => invoke<Project>("add_demo_room", { width, depth, height, planeHeight });

export const addWall = (
  startX: number,
  startY: number,
  endX: number,
  endY: number,
  thickness: number,
) => invoke<Project>("add_wall", { startX, startY, endX, endY, thickness });

export const moveWall = (index: number, dx: number, dy: number) =>
  invoke<Project>("move_wall", { index, dx, dy });

export const offsetWall = (index: number, dist: number) =>
  invoke<Project>("offset_wall", { index, dist });

export const clearWalls = () => invoke<Project>("clear_walls");

export const buildRoom = (height: number, planeHeight: number) =>
  invoke<Project>("build_room", { height, planeHeight });

export const calculateLux = () => invoke<LuxGrid>("calculate_lux");
