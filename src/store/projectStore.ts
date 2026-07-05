import { create } from "zustand";
import type { EngineInfo, Line2, LuxGrid, Project } from "../types";

interface AppStore {
  engine: EngineInfo | null;
  project: Project | null;
  dxfLines: Line2[];
  luxGrid: LuxGrid | null;
  status: string;
  busy: boolean;
  // Wall-drawing interaction state.
  drawMode: boolean;
  pendingStart: [number, number] | null;
  wallThickness: number;
  setEngine: (e: EngineInfo) => void;
  setProject: (p: Project) => void;
  setDxfLines: (l: Line2[]) => void;
  setLuxGrid: (g: LuxGrid | null) => void;
  setStatus: (s: string) => void;
  setBusy: (b: boolean) => void;
  setDrawMode: (b: boolean) => void;
  setPendingStart: (p: [number, number] | null) => void;
  setWallThickness: (t: number) => void;
}

export const useStore = create<AppStore>((set) => ({
  engine: null,
  project: null,
  dxfLines: [],
  luxGrid: null,
  status: "Ready.",
  busy: false,
  drawMode: false,
  pendingStart: null,
  wallThickness: 0.1,
  setEngine: (engine) => set({ engine }),
  setProject: (project) => set({ project }),
  setDxfLines: (dxfLines) => set({ dxfLines }),
  setLuxGrid: (luxGrid) => set({ luxGrid }),
  setStatus: (status) => set({ status }),
  setBusy: (busy) => set({ busy }),
  setDrawMode: (drawMode) => set({ drawMode }),
  setPendingStart: (pendingStart) => set({ pendingStart }),
  setWallThickness: (wallThickness) => set({ wallThickness }),
}));
