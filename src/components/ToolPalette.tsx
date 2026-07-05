import { useStore } from "../store/projectStore";
import type { Tool } from "../store/projectStore";

const TOOLS: Array<{ id: Tool; label: string; hint: string }> = [
  { id: "select", label: "Select", hint: "Pan / select (drag to pan; wheel to zoom)" },
  { id: "wall", label: "Wall", hint: "Chain walls with thickness (Esc/Enter to finish)" },
  { id: "line", label: "Line", hint: "Single line segment (extrudes to a surface)" },
  { id: "polyline", label: "Polyline", hint: "Chain of lines (Esc/Enter to finish)" },
  { id: "rect", label: "Rect", hint: "Rectangular room from two corners" },
];

export default function ToolPalette() {
  const tool = useStore((s) => s.tool);
  const setTool = useStore((s) => s.setTool);
  const thickness = useStore((s) => s.wallThickness);
  const setThickness = useStore((s) => s.setWallThickness);

  return (
    <aside className="palette">
      {TOOLS.map((t) => (
        <button
          key={t.id}
          title={t.hint}
          className={tool === t.id ? "tool active" : "tool"}
          onClick={() => setTool(t.id)}
        >
          {t.label}
        </button>
      ))}
      <div className="palette-field">
        <label>Wall t</label>
        <input
          type="number"
          min={0}
          step={0.05}
          value={thickness}
          onChange={(e) => setThickness(Math.max(0, Number(e.target.value) || 0))}
        />
        <span>m</span>
      </div>
    </aside>
  );
}
