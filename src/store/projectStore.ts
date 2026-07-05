import { create } from "zustand";
import type { EngineInfo, Line2, LuxGrid, Project } from "../types";

export type Tool = "select" | "line" | "polyline" | "rect" | "wall";
export type Tab = "construction" | "view3d";

interface AppStore {
  engine: EngineInfo | null;
  project: Project | null;
  dxfLines: Line2[];
  luxGrid: LuxGrid | null;
  status: string;
  busy: boolean;
  tab: Tab;
  tool: Tool;
  wallThickness: number;
  setEngine: (e: EngineInfo) => void;
  setProject: (p: Project) => void;
  setDxfLines: (l: Line2[]) => void;
  setLuxGrid: (g: LuxGrid | null) => void;
  setStatus: (s: string) => void;
  setBusy: (b: boolean) => void;
  setTab: (t: Tab) => void;
  setTool: (t: Tool) => void;
  setWallThickness: (t: number) => void;
}

export const useStore = create<AppStore>((set) => ({
  engine: null,
  project: null,
  dxfLines: [],
  luxGrid: null,
  status: "Ready.",
  busy: false,
  tab: "construction",
  tool: "wall",
  wallThickness: 0.1,
  setEngine: (engine) => set({ engine }),
  setProject: (project) => set({ project }),
  setDxfLines: (dxfLines) => set({ dxfLines }),
  setLuxGrid: (luxGrid) => set({ luxGrid }),
  setStatus: (status) => set({ status }),
  setBusy: (busy) => set({ busy }),
  setTab: (tab) => set({ tab }),
  setTool: (tool) => set({ tool }),
  setWallThickness: (wallThickness) => set({ wallThickness }),
}));
