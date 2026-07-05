import { useEffect } from "react";
import Toolbar from "./components/Toolbar";
import Sidebar from "./components/Sidebar";
import Viewport from "./components/Viewport";
import StatusBar from "./components/StatusBar";
import { engineInfo } from "./api/commands";
import { useStore } from "./store/projectStore";
import "./App.css";

export default function App() {
  const setEngine = useStore((s) => s.setEngine);
  const setStatus = useStore((s) => s.setStatus);
  const setDrawMode = useStore((s) => s.setDrawMode);
  const setPendingStart = useStore((s) => s.setPendingStart);

  useEffect(() => {
    engineInfo()
      .then((info) => {
        setEngine(info);
        setStatus(`${info.name} v${info.version} ready.`);
      })
      .catch((e) => setStatus(`Engine unavailable: ${String(e)}`));
  }, [setEngine, setStatus]);

  // Esc finishes / exits wall-draw mode.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setDrawMode(false);
        setPendingStart(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setDrawMode, setPendingStart]);

  return (
    <div className="app">
      <Toolbar />
      <div className="body">
        <Sidebar />
        <main className="viewport">
          <Viewport />
        </main>
      </div>
      <StatusBar />
    </div>
  );
}
