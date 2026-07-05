// Typed wrappers around the Tauri command layer (src-tauri/src/commands.rs).
// Tauri maps camelCase JS keys to snake_case Rust params automatically.
import { invoke } from "@tauri-apps/api/core";
import type { EngineInfo, IesProfile, Line2, LuxGrid, Project } from "../types";

export const engineInfo = () => invoke<EngineInfo>("engine_info");

export const getProject = () => invoke<Project>("get_project");

export const importIes = (path: string) =>
  invoke<IesProfile>("import_ies", { path });

export const loadDxf = (path: string) => invoke<Line2[]>("load_dxf", { path });

export const calculateLux = () => invoke<LuxGrid>("calculate_lux");
