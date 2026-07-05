import { useState } from "react";
import { useStore } from "../store/projectStore";
import { cancelCommand, execCommand } from "../api/commands";

export default function CommandLine() {
  const prompt = useStore((s) => s.prompt);
  const cmdLog = useStore((s) => s.cmdLog);
  const applyCmd = useStore((s) => s.applyCmd);
  const pushInput = useStore((s) => s.pushInput);
  const [text, setText] = useState("");

  async function submit() {
    const input = text;
    if (input.trim()) pushInput(input);
    setText("");
    applyCmd(await execCommand(input));
  }

  async function onKey(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Enter") {
      e.preventDefault();
      await submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setText("");
      applyCmd(await cancelCommand());
    }
  }

  return (
    <div className="cmdline">
      <div className="cmdlog">
        {cmdLog.slice(-6).map((l, i) => (
          <div key={i} className={`cmdlog-line ${l.kind}`}>
            {l.kind === "in" ? "› " : ""}
            {l.text}
          </div>
        ))}
      </div>
      <div className="cmdrow">
        <span className="cmdprompt">{prompt}</span>
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKey}
          placeholder="command (line, rect, circle, arc, wall, pline, point, clear) — or a point: 3,0  @2,0  @5<90"
          spellCheck={false}
          autoComplete="off"
        />
      </div>
    </div>
  );
}
