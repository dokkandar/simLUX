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
  const project = useStore((s) => s.project);
  const dxfCount = useStore((s) => s.dxfLines.length);
  const entityCount = useStore((s) => s.geometry.length);
  const luxGrid = useStore((s) => s.luxGrid);

  const profileCount = project ? Object.keys(project.profiles).length : 0;
  const lumCount = project?.luminaires.length ?? 0;
  const plane = project?.calc_plane ?? null;

  return (
    <aside className="sidebar">
      <h2>Engine</h2>
      <div className="panel">
        <Row k="Name" v={engine?.name ?? "—"} />
        <Row k="Version" v={engine?.version ?? "—"} />
      </div>

      <h2>Scene</h2>
      <div className="panel">
        <Row k="IES profiles" v={String(profileCount)} />
        <Row k="Entities" v={String(entityCount)} />
        <Row k="Luminaires" v={String(lumCount)} />
        <Row k="Room faces" v={String(project?.meshes.length ?? 0)} />
        <Row k="DXF underlay" v={`${dxfCount} seg`} />
        {plane && <Row k="Calc grid" v={`${plane.cols} × ${plane.rows} @ ${plane.origin.z} m`} />}
      </div>

      <h2>Results (lux)</h2>
      <div className="panel">
        {luxGrid ? (
          <>
            <Row k="Average" v={luxGrid.avg.toFixed(0)} />
            <Row k="Minimum" v={luxGrid.min.toFixed(0)} />
            <Row k="Maximum" v={luxGrid.max.toFixed(0)} />
            <Row
              k="Uniformity"
              v={luxGrid.avg > 0 ? (luxGrid.min / luxGrid.avg).toFixed(2) : "—"}
            />
          </>
        ) : (
          <p className="muted">No calculation yet.</p>
        )}
      </div>

      {engine && <p className="phase">{engine.phase}</p>}
    </aside>
  );
}
