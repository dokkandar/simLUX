import { open } from "@tauri-apps/plugin-dialog";
import { useStore } from "../store/projectStore";
import { calculateLux, importIes, loadDxf } from "../api/commands";

export default function Toolbar() {
  const busy = useStore((s) => s.busy);
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
      setStatus(`Imported IES: ${p.name}`);
    });
  }

  async function onCalculate() {
    await run("Calculate", () => calculateLux(), (g) => {
      setLuxGrid(g);
      setStatus(`Lux — avg ${g.avg.toFixed(1)}, max ${g.max.toFixed(1)}`);
    });
  }

  return (
    <header className="toolbar">
      <span className="brand">SIMLUX</span>
      <div className="tools">
        <button disabled={busy} onClick={onLoadDxf}>Load DXF</button>
        <button disabled={busy} onClick={onImportIes}>Import IES</button>
        <button className="primary" disabled={busy} onClick={onCalculate}>
          Calculate Lux
        </button>
      </div>
    </header>
  );
}
