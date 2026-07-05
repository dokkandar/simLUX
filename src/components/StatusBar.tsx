import { useStore } from "../store/projectStore";

export default function StatusBar() {
  const status = useStore((s) => s.status);
  const busy = useStore((s) => s.busy);

  return (
    <footer className="statusbar">
      <span className={busy ? "dot busy" : "dot"} />
      <span>{status}</span>
    </footer>
  );
}
