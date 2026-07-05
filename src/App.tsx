import { useEffect } from "react";
import Toolbar from "./components/Toolbar";
import ToolPalette from "./components/ToolPalette";
import Plan2D from "./components/Plan2D";
import Viewport from "./components/Viewport";
import Sidebar from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import { engineInfo } from "./api/commands";
import { useStore } from "./store/projectStore";
import "./App.css";

export default function App() {
  const setEngine = useStore((s) => s.setEngine);
  const setStatus = useStore((s) => s.setStatus);
  const tab = useStore((s) => s.tab);

  useEffect(() => {
    engineInfo()
      .then((info) => {
        setEngine(info);
        setStatus(`${info.name} v${info.version} ready — Construction: draw walls, then Build Room.`);
      })
      .catch((e) => setStatus(`Engine unavailable: ${String(e)}`));
  }, [setEngine, setStatus]);

  return (
    <div className="app">
      <Toolbar />
      <div className="body">
        {tab === "construction" && <ToolPalette />}
        <main className="stage">{tab === "construction" ? <Plan2D /> : <Viewport />}</main>
        <Sidebar />
      </div>
      <StatusBar />
    </div>
  );
}
