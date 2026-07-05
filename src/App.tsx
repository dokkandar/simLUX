import { useEffect } from "react";
import Toolbar from "./components/Toolbar";
import ToolPalette from "./components/ToolPalette";
import Plan2D from "./components/Plan2D";
import CommandLine from "./components/CommandLine";
import Viewport from "./components/Viewport";
import Sidebar from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import { engineInfo, getGeometry } from "./api/commands";
import { useStore } from "./store/projectStore";
import "./App.css";

export default function App() {
  const setEngine = useStore((s) => s.setEngine);
  const setStatus = useStore((s) => s.setStatus);
  const applyCmd = useStore((s) => s.applyCmd);
  const tab = useStore((s) => s.tab);

  useEffect(() => {
    engineInfo()
      .then((info) => {
        setEngine(info);
        setStatus(`${info.name} v${info.version} — type a command (e.g. rect) or use the tools.`);
      })
      .catch((e) => setStatus(`Engine unavailable: ${String(e)}`));
    getGeometry().then(applyCmd).catch(() => {});
  }, [setEngine, setStatus, applyCmd]);

  return (
    <div className="app">
      <Toolbar />
      <div className="body">
        {tab === "construction" ? (
          <>
            <ToolPalette />
            <div className="stage construction">
              <div className="plan-wrap">
                <Plan2D />
              </div>
              <CommandLine />
            </div>
          </>
        ) : (
          <main className="stage">
            <Viewport />
          </main>
        )}
        <Sidebar />
      </div>
      <StatusBar />
    </div>
  );
}
