import { open } from "@tauri-apps/plugin-dialog";
import { useStore } from "../store/projectStore";
import { buildRoom, calculateLux, execCommand, importIes, loadDxf } from "../api/commands";

export default function Toolbar() {
  const busy = useStore((s) => s.busy);
  const tab = useStore((s) => s.tab);
  const project = useStore((s) => s.project);
  const setTab = useStore((s) => s.setTab);
  const setProject = useStore((s) => s.setProject);
  const setDxfLines = useStore((s) => s.setDxfLines);
  const setLuxGrid = useStore((s) => s.setLuxGrid);
  const setStatus = useStore((s) => s.setStatus);
  const setBusy = useStore((s) => s.setBusy);
  const applyCmd = useStore((s) => s.applyCmd);

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

  async function onImportIes() {
    const path = await open({ filters: [{ name: "IES photometry", extensions: ["ies"] }] });
    if (typeof path !== "string") return;
    await run("Import IES", () => importIes(path), (p) => {
      setStatus(`Imported IES: ${p.name} — peak ${Math.round(p.candela[0][0] * p.multiplier)} cd`);
    });
  }

  async function onLoadDxf() {
    const path = await open({ filters: [{ name: "DXF", extensions: ["dxf"] }] });
    if (typeof path !== "string") return;
    await run("Load DXF", () => loadDxf(path), (lines) => {
      setDxfLines(lines);
      setStatus(`Underlay loaded — ${lines.length} DXF segments. Trace walls over it.`);
    });
  }

  async function onBuildRoom() {
    await run("Build room", () => buildRoom(project?.room_height ?? 3, 0.8), (p) => {
      setProject(p);
      setTab("view3d");
      setStatus(
        p.meshes.length
          ? `Room built — ${p.meshes.length} surfaces. ${p.luminaires.length ? "Click Calculate." : "Import an IES + rebuild to add a light."}`
          : "Draw walls forming a closed loop, then Build Room.",
      );
    });
  }

  async function onClear() {
    await run("Clear", () => execCommand("clear"), (r) => {
      applyCmd(r);
      setLuxGrid(null);
      setStatus("Drawing cleared.");
    });
  }

  async function onCalculate() {
    await run("Calculate", () => calculateLux(), (g) => {
      setLuxGrid(g);
      setTab("view3d");
      setStatus(`Lux — avg ${g.avg.toFixed(0)}, min ${g.min.toFixed(0)}, max ${g.max.toFixed(0)}`);
    });
  }

  return (
    <header className="toolbar">
      <span className="brand">SIMLUX</span>
      <div className="tabs">
        <button className={tab === "construction" ? "tab active" : "tab"} onClick={() => setTab("construction")}>
          Construction
        </button>
        <button className={tab === "view3d" ? "tab active" : "tab"} onClick={() => setTab("view3d")}>
          3D &amp; Light
        </button>
      </div>
      <span className="sep" />
      <div className="tools">
        <button disabled={busy} onClick={onImportIes}>Import IES</button>
        <button disabled={busy} onClick={onLoadDxf}>Load DXF</button>
        <button disabled={busy} onClick={onBuildRoom}>Build Room</button>
        <button disabled={busy} onClick={onClear}>Clear</button>
        <button className="primary" disabled={busy} onClick={onCalculate}>Calculate</button>
      </div>
    </header>
  );
}
