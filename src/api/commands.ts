// Typed wrappers around the Tauri command layer (src-tauri/src/commands.rs).
// Tauri maps camelCase JS keys to snake_case Rust params automatically.
import { invoke } from "@tauri-apps/api/core";
import type { CmdResult, EngineInfo, IesProfile, Line2, LuxGrid, Project } from "../types";

export const engineInfo = () => invoke<EngineInfo>("engine_info");

export const getProject = () => invoke<Project>("get_project");

export const importIes = (path: string) => invoke<IesProfile>("import_ies", { path });

export const loadDxf = (path: string) => invoke<Line2[]>("load_dxf", { path });

// --- command-line drafting ---
export const execCommand = (input: string) => invoke<CmdResult>("exec_command", { input });
export const pickPoint = (x: number, y: number) => invoke<CmdResult>("pick_point", { x, y });
export const cancelCommand = () => invoke<CmdResult>("cancel_command");
export const getGeometry = () => invoke<CmdResult>("get_geometry");

// --- scene ---
export const addLuminaire = (x: number, y: number, z: number, profile: string) =>
  invoke<Project>("add_luminaire", { x, y, z, profile });

export const addDemoRoom = (width: number, depth: number, height: number, planeHeight: number) =>
  invoke<Project>("add_demo_room", { width, depth, height, planeHeight });

export const buildRoom = (height: number, planeHeight: number) =>
  invoke<Project>("build_room", { height, planeHeight });

export const calculateLux = () => invoke<LuxGrid>("calculate_lux");
