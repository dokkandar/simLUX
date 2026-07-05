import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../store/projectStore";
import { cancelCommand, pickPoint } from "../api/commands";
import type { GeomDto } from "../types";

interface View {
  scale: number; // pixels per metre
  tx: number;
  ty: number;
}
type Pt = [number, number];
type Snap = { p: Pt; kind: "end" | "mid" | "cen" | "int" | null };

function arcPoints(c: Pt, r: number, startDeg: number, sweepDeg: number, n = 40): Pt[] {
  const out: Pt[] = [];
  for (let i = 0; i <= n; i++) {
    const a = ((startDeg + (sweepDeg * i) / n) * Math.PI) / 180;
    out.push([c[0] + r * Math.cos(a), c[1] + r * Math.sin(a)]);
  }
  return out;
}

function segInt(a: [Pt, Pt], b: [Pt, Pt]): Pt | null {
  const [p1, p2] = a, [p3, p4] = b;
  const d1x = p2[0] - p1[0], d1y = p2[1] - p1[1];
  const d2x = p4[0] - p3[0], d2y = p4[1] - p3[1];
  const den = d1x * d2y - d1y * d2x;
  if (Math.abs(den) < 1e-9) return null;
  const t = ((p3[0] - p1[0]) * d2y - (p3[1] - p1[1]) * d2x) / den;
  const u = ((p3[0] - p1[0]) * d1y - (p3[1] - p1[1]) * d1x) / den;
  if (t < 0 || t > 1 || u < 0 || u > 1) return null;
  return [p1[0] + t * d1x, p1[1] + t * d1y];
}

/** Top-down 2D CAD drafting view (SVG). World is metres, Y-up. */
export default function Plan2D() {
  const geometry = useStore((s) => s.geometry);
  const selected = useStore((s) => s.selected);
  const dxfLines = useStore((s) => s.dxfLines);
  const activeTool = useStore((s) => s.activeTool);
  const activePts = useStore((s) => s.activePts);
  const applyCmd = useStore((s) => s.applyCmd);

  const svgRef = useRef<SVGSVGElement | null>(null);
  const [size, setSize] = useState({ w: 800, h: 600 });
  const [view, setView] = useState<View>({ scale: 40, tx: 400, ty: 300 });
  const [hover, setHover] = useState<Snap | null>(null);
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

  // OSNAP candidates (same kinds as Auto_RASM): END, MID, CEN, INT.
  const osnap = useMemo(() => {
    const ends: Pt[] = [], mids: Pt[] = [], cens: Pt[] = [], segs: [Pt, Pt][] = [];
    const addSeg = (a: Pt, b: Pt) => {
      segs.push([a, b]);
      ends.push(a, b);
      mids.push([(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]);
    };
    for (const g of geometry) {
      if (g.kind === "line" || g.kind === "wall") addSeg(g.a, g.b);
      else if (g.kind === "polyline") {
        for (let i = 0; i < g.pts.length - 1; i++) addSeg(g.pts[i], g.pts[i + 1]);
        if (g.closed && g.pts.length > 2) addSeg(g.pts[g.pts.length - 1], g.pts[0]);
      } else if (g.kind === "circle") cens.push(g.c);
      else if (g.kind === "arc") {
        cens.push(g.c);
        const a0 = (g.start_deg * Math.PI) / 180, a1 = ((g.start_deg + g.sweep_deg) * Math.PI) / 180;
        ends.push([g.c[0] + g.r * Math.cos(a0), g.c[1] + g.r * Math.sin(a0)]);
        ends.push([g.c[0] + g.r * Math.cos(a1), g.c[1] + g.r * Math.sin(a1)]);
      } else if (g.kind === "point") ends.push(g.p);
    }
    for (const l of dxfLines) addSeg([l.start.x, l.start.y], [l.end.x, l.end.y]);
    const ints: Pt[] = [];
    for (let i = 0; i < segs.length; i++)
      for (let j = i + 1; j < segs.length; j++) {
        const p = segInt(segs[i], segs[j]);
        if (p) ints.push(p);
      }
    return { ends, mids, cens, ints };
  }, [geometry, dxfLines]);

  const snap = useCallback(
    (wx: number, wy: number): Snap => {
      const r = 12 / view.scale;
      const groups: Array<[Snap["kind"], Pt[]]> = [
        ["end", osnap.ends], ["mid", osnap.mids], ["cen", osnap.cens], ["int", osnap.ints],
      ];
      for (const [kind, arr] of groups) {
        let best: Pt | null = null;
        let bd = r;
        for (const p of arr) {
          const d = Math.hypot(p[0] - wx, p[1] - wy);
          if (d < bd) { bd = d; best = p; }
        }
        if (best) return { p: best, kind };
      }
      return { p: [Math.round(wx / gridSpacing) * gridSpacing, Math.round(wy / gridSpacing) * gridSpacing], kind: null };
    },
    [osnap, gridSpacing, view.scale],
  );

  const fit = useCallback(() => {
    const pts = [...osnap.ends, ...osnap.cens];
    if (!pts.length) { setView({ scale: 40, tx: size.w / 2, ty: size.h / 2 }); return; }
    let mnx = Infinity, mny = Infinity, mxx = -Infinity, mxy = -Infinity;
    for (const [x, y] of pts) { mnx = Math.min(mnx, x); mny = Math.min(mny, y); mxx = Math.max(mxx, x); mxy = Math.max(mxy, y); }
    const bw = Math.max(mxx - mnx, 0.001), bh = Math.max(mxy - mny, 0.001);
    const scale = Math.max(2, Math.min(size.w / bw, size.h / bh) * 0.85);
    const cx = (mnx + mxx) / 2, cy = (mny + mxy) / 2;
    setView({ scale, tx: size.w / 2 - cx * scale, ty: size.h / 2 + cy * scale });
  }, [osnap, size]);

  useEffect(() => {
    const el = svgRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    setSize({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (!fitted.current && osnap.ends.length > 0 && size.w > 0) {
      fit();
      fitted.current = true;
    }
  }, [osnap.ends.length, size.w, fit]);

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
  const onPointerUp = async (e: React.PointerEvent) => {
    if (pan.current) { pan.current = null; return; }
    if (e.button !== 0) return;
    const d = down.current;
    down.current = null;
    if (d?.moved) return;
    const [wx, wy] = eventWorld(e);
    const { p } = snap(wx, wy);
    applyCmd(await pickPoint(p[0], p[1], 12 / view.scale));
  };

  useEffect(() => {
    const onKey = async (e: KeyboardEvent) => {
      if (e.key === "Escape") applyCmd(await cancelCommand());
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [applyCmd]);

  const gridLines = useMemo(() => {
    const out: Array<{ k: string; x1: number; y1: number; x2: number; y2: number; major: boolean }> = [];
    const sp = gridSpacing;
    const [wl, wt] = toWorld(0, 0);
    const [wr, wb] = toWorld(size.w, size.h);
    const left = Math.min(wl, wr), right = Math.max(wl, wr);
    const bottom = Math.min(wt, wb), top = Math.max(wt, wb);
    for (let x = Math.floor(left / sp) * sp; x <= right; x += sp) {
      const [px] = toScreen(x, 0);
      out.push({ k: `vx${x.toFixed(3)}`, x1: px, y1: 0, x2: px, y2: size.h, major: Math.round(x / sp) % 5 === 0 });
    }
    for (let y = Math.floor(bottom / sp) * sp; y <= top; y += sp) {
      const [, py] = toScreen(0, y);
      out.push({ k: `hy${y.toFixed(3)}`, x1: 0, y1: py, x2: size.w, y2: py, major: Math.round(y / sp) % 5 === 0 });
    }
    return out;
  }, [gridSpacing, toScreen, toWorld, size]);

  const poly = (pts: Pt[]) => pts.map((p) => toScreen(p[0], p[1]).join(",")).join(" ");
  const selSet = useMemo(() => new Set(selected), [selected]);

  function renderGeom(g: GeomDto, i: number) {
    const sel = selSet.has(i);
    const stroke = sel ? "#57e08a" : "#9aa4b2";
    const w = sel ? 2.5 : 1.5;
    if (g.kind === "line" || g.kind === "wall") {
      const [x1, y1] = toScreen(g.a[0], g.a[1]);
      const [x2, y2] = toScreen(g.b[0], g.b[1]);
      const sw = g.kind === "wall" ? Math.max(2, g.thickness * view.scale) : w;
      return <line key={i} x1={x1} y1={y1} x2={x2} y2={y2} stroke={stroke} strokeWidth={sw} strokeLinecap="round" />;
    }
    if (g.kind === "polyline") {
      const pts = g.closed ? [...g.pts, g.pts[0]] : g.pts;
      return <polyline key={i} points={poly(pts)} fill="none" stroke={stroke} strokeWidth={w} />;
    }
    if (g.kind === "circle") {
      const [cx, cy] = toScreen(g.c[0], g.c[1]);
      return <circle key={i} cx={cx} cy={cy} r={g.r * view.scale} fill="none" stroke={stroke} strokeWidth={w} />;
    }
    if (g.kind === "arc") {
      return <polyline key={i} points={poly(arcPoints(g.c, g.r, g.start_deg, g.sweep_deg))} fill="none" stroke={stroke} strokeWidth={w} />;
    }
    const [px, py] = toScreen(g.p[0], g.p[1]);
    return <circle key={i} cx={px} cy={py} r={2.5} fill={stroke} />;
  }

  function snapMarker(s: Snap) {
    const [x, y] = toScreen(s.p[0], s.p[1]);
    const c = "#ffd24a";
    if (s.kind === "end") return <rect x={x - 5} y={y - 5} width={10} height={10} fill="none" stroke={c} strokeWidth={1.5} />;
    if (s.kind === "mid") return <polygon points={`${x},${y - 6} ${x - 6},${y + 5} ${x + 6},${y + 5}`} fill="none" stroke={c} strokeWidth={1.5} />;
    if (s.kind === "cen") return <circle cx={x} cy={y} r={6} fill="none" stroke={c} strokeWidth={1.5} />;
    if (s.kind === "int") return <g stroke={c} strokeWidth={1.5}><line x1={x - 5} y1={y - 5} x2={x + 5} y2={y + 5} /><line x1={x - 5} y1={y + 5} x2={x + 5} y2={y - 5} /></g>;
    return <circle cx={x} cy={y} r={3} fill={c} />;
  }

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
          <line key={l.k} x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2} stroke={l.major ? "#2a3542" : "#1a2029"} strokeWidth={1} />
        ))}
        <line x1={toScreen(0, 0)[0]} y1={0} x2={toScreen(0, 0)[0]} y2={size.h} stroke="#3a4a5a" strokeWidth={1.3} />
        <line x1={0} y1={toScreen(0, 0)[1]} x2={size.w} y2={toScreen(0, 0)[1]} stroke="#3a4a5a" strokeWidth={1.3} />

        {dxfLines.map((l, i) => {
          const [x1, y1] = toScreen(l.start.x, l.start.y);
          const [x2, y2] = toScreen(l.end.x, l.end.y);
          return <line key={`d${i}`} x1={x1} y1={y1} x2={x2} y2={y2} stroke="#31424f" strokeWidth={1} />;
        })}

        {geometry.map((g, i) => renderGeom(g, i))}

        {activePts.length >= 1 && hover && activeTool === "rectangle" && (() => {
          const [ax, ay] = toScreen(activePts[0][0], activePts[0][1]);
          const [bx, by] = toScreen(hover.p[0], hover.p[1]);
          return <rect x={Math.min(ax, bx)} y={Math.min(ay, by)} width={Math.abs(bx - ax)} height={Math.abs(by - ay)} fill="none" stroke="#ffd24a" strokeWidth={1.5} strokeDasharray="5 4" />;
        })()}
        {activePts.length >= 1 && hover && activeTool === "circle" && (() => {
          const [cx, cy] = toScreen(activePts[0][0], activePts[0][1]);
          const r = Math.hypot(hover.p[0] - activePts[0][0], hover.p[1] - activePts[0][1]) * view.scale;
          return <circle cx={cx} cy={cy} r={r} fill="none" stroke="#ffd24a" strokeWidth={1.5} strokeDasharray="5 4" />;
        })()}
        {activePts.length >= 1 && hover && activeTool !== "rectangle" && activeTool !== "circle" && (
          <polyline points={poly([...activePts, hover.p])} fill="none" stroke="#ffd24a" strokeWidth={1.5} strokeDasharray="5 4" />
        )}

        {hover && snapMarker(hover)}
      </svg>

      <div className="plan2d-hud">
        <span className="tag">{activeTool ?? "ready"}</span>
        {hover && <span>{hover.p[0].toFixed(2)}, {hover.p[1].toFixed(2)} m{hover.kind ? ` · ${hover.kind.toUpperCase()}` : ""}</span>}
        <span className="spacer" />
        {selected.length > 0 && <span>{selected.length} selected</span>}
        <span>grid {gridSpacing} m</span>
        <button onClick={fit}>Fit</button>
      </div>
    </div>
  );
}
