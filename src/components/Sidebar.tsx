import { useStore } from "../store/projectStore";

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="row-kv">
      <span>{k}</span>
      <span>{v}</span>
    </div>
  );
}

export default function Sidebar() {
  const engine = useStore((s) => s.engine);
  const dxfCount = useStore((s) => s.dxfLines.length);
  const luxGrid = useStore((s) => s.luxGrid);

  return (
    <aside className="sidebar">
      <h2>Engine</h2>
      <div className="panel">
        <Row k="Name" v={engine?.name ?? "—"} />
        <Row k="Version" v={engine?.version ?? "—"} />
      </div>

      <h2>Geometry</h2>
      <div className="panel">
        <Row k="DXF segments" v={String(dxfCount)} />
        <Row k="Rooms" v="0" />
      </div>

      <h2>Results</h2>
      <div className="panel">
        {luxGrid ? (
          <>
            <Row k="Avg lux" v={luxGrid.avg.toFixed(1)} />
            <Row k="Min lux" v={luxGrid.min.toFixed(1)} />
            <Row k="Max lux" v={luxGrid.max.toFixed(1)} />
            <Row k="Grid" v={`${luxGrid.cols} × ${luxGrid.rows}`} />
          </>
        ) : (
          <p className="muted">No calculation yet.</p>
        )}
      </div>

      {engine && <p className="phase">{engine.phase}</p>}
    </aside>
  );
}
