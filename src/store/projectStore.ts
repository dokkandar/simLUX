import { create } from "zustand";
import type { EngineInfo, Line2, LuxGrid, Project } from "../types";

interface AppStore {
  engine: EngineInfo | null;
  project: Project | null;
  dxfLines: Line2[];
  luxGrid: LuxGrid | null;
  status: string;
  busy: boolean;
  setEngine: (e: EngineInfo) => void;
  setProject: (p: Project) => void;
  setDxfLines: (l: Line2[]) => void;
  setLuxGrid: (g: LuxGrid) => void;
  setStatus: (s: string) => void;
  setBusy: (b: boolean) => void;
}

export const useStore = create<AppStore>((set) => ({
  engine: null,
  project: null,
  dxfLines: [],
  luxGrid: null,
  status: "Ready.",
  busy: false,
  setEngine: (engine) => set({ engine }),
  setProject: (project) => set({ project }),
  setDxfLines: (dxfLines) => set({ dxfLines }),
  setLuxGrid: (luxGrid) => set({ luxGrid }),
  setStatus: (status) => set({ status }),
  setBusy: (busy) => set({ busy }),
}));
