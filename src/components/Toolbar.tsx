import { open } from "@tauri-apps/plugin-dialog";
import { useStore } from "../store/projectStore";
import { addDemoRoom, calculateLux, importIes, loadDxf } from "../api/commands";

export default function Toolbar() {
  const busy = useStore((s) => s.busy);
  const setProject = useStore((s) => s.setProject);
  const setDxfLines = useStore((s) => s.setDxfLines);
  const setLuxGrid = useStore((s) => s.setLuxGrid);
  const setStatus = useStore((s) => s.setStatus);
  const setBusy = useStore((s) => s.setBusy);

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
      setStatus(`Loaded ${lines.length} DXF segments.`);
    });
  }

  async function onImportIes() {
    const path = await open({ filters: [{ name: "IES photometry", extensions: ["ies"] }] });
    if (typeof path !== "string") return;
    await run("Import IES", () => importIes(path), (p) => {
      setStatus(`Imported IES: ${p.name} — peak ${Math.round(p.candela[0][0] * p.multiplier)} cd`);
    });
  }

  async function onDemoRoom() {
    await run("Demo room", () => addDemoRoom(4, 4, 3, 0.8), (proj) => {
      setProject(proj);
      const hasLum = proj.luminaires.length > 0;
      setStatus(
        hasLum
          ? "Demo room ready (4×4×3 m, light at ceiling). Click Calculate."
          : "Demo room ready. Import an IES first, then re-run to place a light.",
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
        <button disabled={busy} onClick={onDemoRoom}>Demo Room</button>
        <button className="primary" disabled={busy} onClick={onCalculate}>
          Calculate Lux
        </button>
      </div>
    </header>
  );
}
