import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../store/projectStore";
import { addWall } from "../api/commands";

interface View {
  scale: number; // pixels per metre
  tx: number;
  ty: number;
}
type Pt = [number, number];

/** Top-down 2D CAD drafting view (SVG). World is metres, Y-up. */
export default function Plan2D() {
  const tool = useStore((s) => s.tool);
  const thickness = useStore((s) => s.wallThickness);
  const project = useStore((s) => s.project);
  const dxfLines = useStore((s) => s.dxfLines);
  const setProject = useStore((s) => s.setProject);
  const setStatus = useStore((s) => s.setStatus);

  const svgRef = useRef<SVGSVGElement | null>(null);
  const [size, setSize] = useState({ w: 800, h: 600 });
  const [view, setView] = useState<View>({ scale: 40, tx: 400, ty: 300 });
  const [pts, setPts] = useState<Pt[]>([]);
  const [hover, setHover] = useState<Pt | null>(null);
  const pan = useRef<{ x: number; y: number; tx: number; ty: number } | null>(null);
  const down = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const fitted = useRef(false);

  const toScreen = useCallback(
    (wx: number, wy: number): Pt => [view.tx + wx * view.scale, view.ty - wy * view.scale],
    [view],
  );
  const toWorld = useCallback(
    (sx: number, sy: number): Pt => [(sx - view.tx) / view.scale, (view.ty - sy) / view.scale],
    [view],
  );

  const gridSpacing = useMemo(() => {
    for (const s of [0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 20, 50, 100]) {
      if (s * view.scale >= 45) return s;
    }
    return 200;
  }, [view.scale]);

  const snapNodes = useMemo(() => {
    const out: Pt[] = [];
    project?.walls.forEach((w) => out.push([w.start.x, w.start.y], [w.end.x, w.end.y]));
    dxfLines.forEach((l) => out.push([l.start.x, l.start.y], [l.end.x, l.end.y]));
    return out;
  }, [project?.walls, dxfLines]);

  const snap = useCallback(
    (wx: number, wy: number): Pt => {
      let best: Pt | null = null;
      let bd = 12 / view.scale;
      for (const [nx, ny] of snapNodes) {
        const d = Math.hypot(nx - wx, ny - wy);
        if (d < bd) { bd = d; best = [nx, ny]; }
      }
      if (best) return best;
      return [Math.round(wx / gridSpacing) * gridSpacing, Math.round(wy / gridSpacing) * gridSpacing];
    },
    [snapNodes, gridSpacing, view.scale],
  );

  const fit = useCallback(() => {
    let mnx = Infinity, mny = Infinity, mxx = -Infinity, mxy = -Infinity, any = false;
    const add = (x: number, y: number) => {
      any = true;
      mnx = Math.min(mnx, x); mny = Math.min(mny, y);
      mxx = Math.max(mxx, x); mxy = Math.max(mxy, y);
    };
    project?.walls.forEach((w) => { add(w.start.x, w.start.y); add(w.end.x, w.end.y); });
    dxfLines.forEach((l) => { add(l.start.x, l.start.y); add(l.end.x, l.end.y); });
    if (!any) { setView({ scale: 40, tx: size.w / 2, ty: size.h / 2 }); return; }
    const bw = Math.max(mxx - mnx, 0.001), bh = Math.max(mxy - mny, 0.001);
    const scale = Math.max(2, Math.min(size.w / bw, size.h / bh) * 0.85);
    const cx = (mnx + mxx) / 2, cy = (mny + mxy) / 2;
    setView({ scale, tx: size.w / 2 - cx * scale, ty: size.h / 2 + cy * scale });
  }, [project?.walls, dxfLines, size]);

  // Track element size.
  useEffect(() => {
    const el = svgRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    setSize({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, []);

  // Auto-fit once, when there's content to frame.
  useEffect(() => {
    if (!fitted.current && snapNodes.length > 0 && size.w > 0) {
      fit();
      fitted.current = true;
    }
  }, [snapNodes.length, size.w, fit]);

  // Reset in-progress geometry on tool change; Esc/Enter finishes.
  useEffect(() => setPts([]), [tool]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" || e.key === "Enter") setPts([]);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Native non-passive wheel zoom (about the cursor).
  useEffect(() => {
    const el = svgRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = el.getBoundingClientRect();
      const sx = e.clientX - r.left, sy = e.clientY - r.top;
      setView((v) => {
        const wx = (sx - v.tx) / v.scale, wy = (v.ty - sy) / v.scale;
        const scale = Math.max(2, Math.min(6000, v.scale * (e.deltaY < 0 ? 1.1 : 1 / 1.1)));
        return { scale, tx: sx - wx * scale, ty: sy + wy * scale };
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const eventWorld = (e: React.PointerEvent): Pt => {
    const r = svgRef.current!.getBoundingClientRect();
    return toWorld(e.clientX - r.left, e.clientY - r.top);
  };

  async function commitWall(a: Pt, b: Pt, t: number) {
    const proj = await addWall(a[0], a[1], b[0], b[1], t);
    setProject(proj);
    setStatus(`Segment added — ${proj.walls.length} total.`);
  }

  async function addRect(c0: Pt, c1: Pt) {
    const corners: Pt[] = [[c0[0], c0[1]], [c1[0], c0[1]], [c1[0], c1[1]], [c0[0], c1[1]]];
    let proj = project;
    for (let i = 0; i < 4; i++) {
      const a = corners[i], b = corners[(i + 1) % 4];
      proj = await addWall(a[0], a[1], b[0], b[1], thickness);
    }
    if (proj) setProject(proj);
    setStatus("Rectangle added.");
  }

  function place(p: Pt) {
    if (tool === "select") return;
    if (tool === "rect") {
      if (pts.length === 0) { setPts([p]); setStatus("Rectangle — click the opposite corner."); }
      else { addRect(pts[0], p); setPts([]); }
      return;
    }
    if (tool === "line") {
      if (pts.length === 0) { setPts([p]); setStatus("Line — click the end point."); }
      else { commitWall(pts[0], p, 0); setPts([]); }
      return;
    }
    // wall / polyline: chained segments
    const t = tool === "wall" ? thickness : 0;
    if (pts.length === 0) {
      setPts([p]);
      setStatus(`${tool === "wall" ? "Wall" : "Polyline"} — click next point (Esc/Enter to finish).`);
      return;
    }
    const last = pts[pts.length - 1];
    if (Math.hypot(p[0] - last[0], p[1] - last[1]) < 1e-4) return;
    commitWall(last, p, t);
    setPts((prev) => [...prev, p]);
  }

  const onPointerDown = (e: React.PointerEvent) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    if (e.button === 1 || e.button === 2) {
      pan.current = { x: e.clientX, y: e.clientY, tx: view.tx, ty: view.ty };
    } else if (e.button === 0) {
      down.current = { x: e.clientX, y: e.clientY, moved: false };
    }
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (pan.current) {
      const { x, y, tx, ty } = pan.current;
      setView((v) => ({ ...v, tx: tx + (e.clientX - x), ty: ty + (e.clientY - y) }));
      return;
    }
    const [wx, wy] = eventWorld(e);
    setHover(snap(wx, wy));
    if (down.current && Math.hypot(e.clientX - down.current.x, e.clientY - down.current.y) > 4) {
      down.current.moved = true;
    }
  };
  const onPointerUp = (e: React.PointerEvent) => {
    if (pan.current) { pan.current = null; return; }
    if (e.button !== 0) return;
    const d = down.current;
    down.current = null;
    if (d?.moved) return; // drag, not a click
    const [wx, wy] = eventWorld(e);
    place(snap(wx, wy));
  };

  // --- render helpers ---
  const gridLines = useMemo(() => {
    const out: Array<{ k: string; x1: number; y1: number; x2: number; y2: number; major: boolean }> = [];
    const sp = gridSpacing;
    const [wl, wt] = toWorld(0, 0);
    const [wr, wb] = toWorld(size.w, size.h);
    const left = Math.min(wl, wr), right = Math.max(wl, wr);
    const bottom = Math.min(wt, wb), top = Math.max(wt, wb);
    for (let x = Math.floor(left / sp) * sp; x <= right; x += sp) {
      const [sx] = toScreen(x, 0);
      out.push({ k: `vx${x.toFixed(3)}`, x1: sx, y1: 0, x2: sx, y2: size.h, major: Math.round(x / sp) % 5 === 0 });
    }
    for (let y = Math.floor(bottom / sp) * sp; y <= top; y += sp) {
      const [, sy] = toScreen(0, y);
      out.push({ k: `hy${y.toFixed(3)}`, x1: 0, y1: sy, x2: size.w, y2: sy, major: Math.round(y / sp) % 5 === 0 });
    }
    return out;
  }, [gridSpacing, toScreen, toWorld, size]);

  return (
    <div className="plan2d">
      <svg
        ref={svgRef}
        className="plan2d-svg"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerLeave={() => { pan.current = null; down.current = null; setHover(null); }}
        onContextMenu={(e) => e.preventDefault()}
      >
        <rect x={0} y={0} width={size.w} height={size.h} fill="#0e1116" />

        {gridLines.map((l) => (
          <line key={l.k} x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2}
            stroke={l.major ? "#2a3542" : "#1a2029"} strokeWidth={1} />
        ))}
        {/* world axes */}
        <line x1={toScreen(0, 0)[0]} y1={0} x2={toScreen(0, 0)[0]} y2={size.h} stroke="#3a4a5a" strokeWidth={1.3} />
        <line x1={0} y1={toScreen(0, 0)[1]} x2={size.w} y2={toScreen(0, 0)[1]} stroke="#3a4a5a" strokeWidth={1.3} />

        {/* DXF underlay */}
        {dxfLines.map((l, i) => {
          const [x1, y1] = toScreen(l.start.x, l.start.y);
          const [x2, y2] = toScreen(l.end.x, l.end.y);
          return <line key={`d${i}`} x1={x1} y1={y1} x2={x2} y2={y2} stroke="#31424f" strokeWidth={1} />;
        })}

        {/* Walls */}
        {project?.walls.map((w, i) => {
          const [x1, y1] = toScreen(w.start.x, w.start.y);
          const [x2, y2] = toScreen(w.end.x, w.end.y);
          const sw = Math.max(2, w.thickness * view.scale);
          return (
            <line key={`w${i}`} x1={x1} y1={y1} x2={x2} y2={y2}
              stroke="#9aa4b2" strokeWidth={sw} strokeLinecap="round" />
          );
        })}

        {/* In-progress preview: last placed point → cursor */}
        {pts.length >= 1 && hover && tool !== "rect" && tool !== "select" && (() => {
          const [x1, y1] = toScreen(pts[pts.length - 1][0], pts[pts.length - 1][1]);
          const [x2, y2] = toScreen(hover[0], hover[1]);
          return <line x1={x1} y1={y1} x2={x2} y2={y2} stroke="#ffd24a" strokeWidth={1.5} strokeDasharray="5 4" />;
        })()}
        {tool === "rect" && pts.length === 1 && hover && (() => {
          const [ax, ay] = toScreen(pts[0][0], pts[0][1]);
          const [bx, by] = toScreen(hover[0], hover[1]);
          return <rect x={Math.min(ax, bx)} y={Math.min(ay, by)} width={Math.abs(bx - ax)} height={Math.abs(by - ay)}
            fill="none" stroke="#ffd24a" strokeWidth={1.5} strokeDasharray="5 4" />;
        })()}

        {hover && tool !== "select" && (
          <circle cx={toScreen(hover[0], hover[1])[0]} cy={toScreen(hover[0], hover[1])[1]} r={4} fill="#ffd24a" />
        )}
      </svg>

      <div className="plan2d-hud">
        <span className="tag">{tool}</span>
        {hover && <span>{hover[0].toFixed(2)}, {hover[1].toFixed(2)} m</span>}
        <span className="spacer" />
        <span>grid {gridSpacing} m</span>
        <button onClick={fit}>Fit</button>
      </div>
    </div>
  );
}
