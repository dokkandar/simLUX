import { useStore } from "../store/projectStore";
import { cancelCommand, execCommand } from "../api/commands";

const TOOLS: Array<{ id: string; label: string; cmd: string | null; hint: string }> = [
  { id: "select", label: "Select", cmd: null, hint: "Cancel active command (Esc)" },
  { id: "line", label: "Line", cmd: "line", hint: "Line chain — each segment a surface" },
  { id: "polyline", label: "Pline", cmd: "pline", hint: "Polyline (Close to make a room)" },
  { id: "rectangle", label: "Rect", cmd: "rectangle", hint: "Rectangle from two corners" },
  { id: "circle", label: "Circle", cmd: "circle", hint: "Circle: centre + radius point" },
  { id: "arc", label: "Arc", cmd: "arc", hint: "Arc through three points" },
  { id: "wall", label: "Wall", cmd: "wall", hint: "Wall chain (has thickness)" },
  { id: "point", label: "Point", cmd: "point", hint: "Place a point" },
];

export default function ToolPalette() {
  const activeTool = useStore((s) => s.activeTool);
  const applyCmd = useStore((s) => s.applyCmd);
  const thickness = useStore((s) => s.wallThickness);
  const setThickness = useStore((s) => s.setWallThickness);

  async function pick(id: string, cmd: string | null) {
    if (!cmd) {
      applyCmd(await cancelCommand());
      return;
    }
    const line = id === "wall" ? `wall ${thickness}` : cmd;
    applyCmd(await execCommand(line));
  }

  return (
    <aside className="palette">
      {TOOLS.map((t) => (
        <button
          key={t.id}
          title={t.hint}
          className={activeTool === t.id ? "tool active" : "tool"}
          onClick={() => pick(t.id, t.cmd)}
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
