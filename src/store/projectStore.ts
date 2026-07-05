import { create } from "zustand";
import type { CmdResult, EngineInfo, GeomDto, Line2, LuxGrid, Project } from "../types";

export type Tab = "construction" | "view3d";
export interface LogLine {
  text: string;
  kind: "in" | "out" | "err";
}

interface AppStore {
  engine: EngineInfo | null;
  project: Project | null;
  dxfLines: Line2[];
  luxGrid: LuxGrid | null;
  status: string;
  busy: boolean;
  tab: Tab;
  // Command-line drafting state (mirrors the backend session).
  geometry: GeomDto[];
  prompt: string;
  activeTool: string | null;
  activePts: [number, number][];
  cmdLog: LogLine[];
  wallThickness: number;
  setEngine: (e: EngineInfo) => void;
  setProject: (p: Project) => void;
  setDxfLines: (l: Line2[]) => void;
  setLuxGrid: (g: LuxGrid | null) => void;
  setStatus: (s: string) => void;
  setBusy: (b: boolean) => void;
  setTab: (t: Tab) => void;
  setWallThickness: (t: number) => void;
  pushInput: (text: string) => void;
  applyCmd: (r: CmdResult) => void;
}

export const useStore = create<AppStore>((set) => ({
  engine: null,
  project: null,
  dxfLines: [],
  luxGrid: null,
  status: "Ready.",
  busy: false,
  tab: "construction",
  geometry: [],
  prompt: "Command:",
  activeTool: null,
  activePts: [],
  cmdLog: [],
  wallThickness: 0.1,
  setEngine: (engine) => set({ engine }),
  setProject: (project) => set({ project }),
  setDxfLines: (dxfLines) => set({ dxfLines }),
  setLuxGrid: (luxGrid) => set({ luxGrid }),
  setStatus: (status) => set({ status }),
  setBusy: (busy) => set({ busy }),
  setTab: (tab) => set({ tab }),
  setWallThickness: (wallThickness) => set({ wallThickness }),
  pushInput: (text) =>
    set((s) => ({ cmdLog: [...s.cmdLog, { text, kind: "in" as const }].slice(-40) })),
  applyCmd: (r) =>
    set((s) => ({
      geometry: r.geometry,
      prompt: r.prompt || "Command:",
      activeTool: r.active_tool,
      activePts: r.active_pts,
      status: r.message || r.prompt || s.status,
      cmdLog: r.message
        ? [...s.cmdLog, { text: r.message, kind: (r.ok ? "out" : "err") as LogLine["kind"] }].slice(-40)
        : s.cmdLog,
    })),
}));
