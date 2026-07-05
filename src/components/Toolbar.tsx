import { open } from "@tauri-apps/plugin-dialog";
import { useStore } from "../store/projectStore";
import {
  addDemoRoom,
  buildRoom,
  calculateLux,
  clearWalls,
  importIes,
  loadDxf,
} from "../api/commands";

export default function Toolbar() {
  const busy = useStore((s) => s.busy);
  const drawMode = useStore((s) => s.drawMode);
  const project = useStore((s) => s.project);
  const setProject = useStore((s) => s.setProject);
  const setDxfLines = useStore((s) => s.setDxfLines);
  const setLuxGrid = useStore((s) => s.setLuxGrid);
  const setStatus = useStore((s) => s.setStatus);
  const setBusy = useStore((s) => s.setBusy);
  const setDrawMode = useStore((s) => s.setDrawMode);
  const setPendingStart = useStore((s) => s.setPendingStart);

  async function run<T>(label: string, fn: () => Promise<T>, ok: (r: T) => void) {
    try {
      setBusy(true);
      setStatus(`${label}…`);
      ok(await fn());
    } catch (e) {
      setStatus(`${label} failed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function onLoadDxf() {
    const path = await open({ filters: [{ name: "DXF", extensions: ["dxf"] }] });
    if (typeof path !== "string") return;
    await run("Load DXF", () => loadDxf(path), (lines) => {
      setDxfLines(lines);
      setStatus(`Underlay loaded — ${lines.length} DXF segments. Trace walls over it.`);
    });
  }

  async function onImportIes() {
    const path = await open({ filters: [{ name: "IES photometry", extensions: ["ies"] }] });
    if (typeof path !== "string") return;
    await run("Import IES", () => importIes(path), (p) => {
      setStatus(`Imported IES: ${p.name} — peak ${Math.round(p.candela[0][0] * p.multiplier)} cd`);
    });
  }

  function onToggleDraw() {
    const next = !drawMode;
    setDrawMode(next);
    setPendingStart(null);
    setStatus(next ? "Draw mode — click to place wall points. Esc to finish." : "Draw mode off.");
  }

  async function onBuildRoom() {
    setDrawMode(false);
    setPendingStart(null);
    await run("Build room", () => buildRoom(project?.room_height ?? 3, 0.8), (p) => {
      setProject(p);
      setStatus(
        p.meshes.length
          ? `Room built (${p.meshes.length} faces). ${p.luminaires.length ? "Click Calculate." : "Import an IES + rebuild to add a light."}`
          : "No closed room yet — draw walls forming a loop, then Build Room.",
      );
    });
  }

  async function onClearWalls() {
    await run("Clear walls", () => clearWalls(), (p) => {
      setProject(p);
      setLuxGrid(null);
      setStatus("Walls cleared.");
    });
  }

  async function onDemoRoom() {
    setDrawMode(false);
    await run("Demo room", () => addDemoRoom(4, 4, 3, 0.8), (p) => {
      setProject(p);
      setStatus(
        p.luminaires.length
          ? "Demo room ready. Click Calculate."
          : "Demo room ready. Import an IES first to add a light.",
      );
    });
  }

  async function onCalculate() {
    await run("Calculate", () => calculateLux(), (g) => {
      setLuxGrid(g);
      setStatus(`Lux — avg ${g.avg.toFixed(0)}, min ${g.min.toFixed(0)}, max ${g.max.toFixed(0)}`);
    });
  }

  return (
    <header className="toolbar">
      <span className="brand">SIMLUX</span>
      <div className="tools">
        <button disabled={busy} onClick={onLoadDxf}>Load DXF</button>
        <button disabled={busy} onClick={onImportIes}>Import IES</button>
        <span className="sep" />
        <button className={drawMode ? "active" : ""} disabled={busy} onClick={onToggleDraw}>
          {drawMode ? "Drawing…" : "Draw Wall"}
        </button>
        <button disabled={busy} onClick={onBuildRoom}>Build Room</button>
        <button disabled={busy} onClick={onClearWalls}>Clear Walls</button>
        <span className="sep" />
        <button disabled={busy} onClick={onDemoRoom}>Demo Room</button>
        <button className="primary" disabled={busy} onClick={onCalculate}>Calculate Lux</button>
      </div>
    </header>
  );
}
