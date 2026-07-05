# SIMLUX

A physically-based **lighting (lux) simulator** for professional lighting design —
draw rooms, place IES luminaires, and compute illuminance grids with a
ray-traced / progressive-radiosity engine.

Built with **Tauri v2 + React (react-three-fiber)** on the front, and a
**Rust** computation engine on the back.

> Status: **Scaffold** — architecture in place, engine stubs return
> `not yet implemented`. See [ROADMAP.md](./ROADMAP.md) for the phased plan and
> [docs/Guide-for-simLUX.txt](./docs/Guide-for-simLUX.txt) for the full design
> research.

## Stack

| Layer      | Choice                                                     |
| ---------- | ---------------------------------------------------------- |
| Shell      | Tauri v2                                                   |
| UI         | React 19 + Vite + `@react-three/fiber` / `drei` (Three.js) |
| State      | zustand                                                    |
| Engine     | Rust — `glam` (math), `rayon` (parallelism), `thiserror`   |
| Photometry | Custom IES LM-63 parser _(Phase 3.1)_                      |
| CAD import | DXF _(Phase 3.1)_                                          |

## Layout

```
SIMLUX/
├─ src/                     React frontend
│  ├─ api/commands.ts       typed wrappers over Tauri invoke()
│  ├─ components/           Toolbar, Sidebar, Viewport (r3f), StatusBar
│  ├─ store/projectStore.ts zustand app state
│  └─ types.ts              mirror of the Rust serde model
├─ src-tauri/
│  └─ src/
│     ├─ commands.rs        Tauri command surface (engine_info, import_ies, …)
│     ├─ model/             serialisable project state
│     ├─ state.rs           AppState (Mutex<Project>)
│     ├─ error.rs           EngineError (serialised to the UI)
│     └─ engine/
│        ├─ ies/            IES LM-63 photometry
│        ├─ dxf/            DXF plan import
│        ├─ geometry/       2D/3D primitives, meshes, calc plane
│        ├─ calc/           direct lux → progressive radiosity
│        └─ math.rs         vector + photometry helpers
├─ docs/Guide-for-simLUX.txt  design research
└─ ROADMAP.md
```

## Prerequisites

- [Rust](https://rustup.rs/) (stable) + the platform's Tauri build deps
  (WebView2 is preinstalled on Windows 11).
- [Node.js](https://nodejs.org/) 18+.

## Develop

```bash
npm install
npm run tauri dev      # launches the desktop app with hot reload
```

## Build

```bash
npm run tauri build    # produces a native installer under src-tauri/target/release
```

## The command bridge

| Command             | Returns to JS  | Purpose                               |
| ------------------- | -------------- | ------------------------------------- |
| `engine_info()`     | `EngineInfo`   | health check / version                |
| `get_project()`     | `Project`      | snapshot of app state                 |
| `import_ies(path)`  | `IesProfile`   | parse + store an IES file _(stub)_    |
| `load_dxf(path)`    | `Line2[]`      | load DXF plan geometry _(stub)_       |
| `calculate_lux()`   | `LuxGrid`      | compute the illuminance grid _(stub)_ |

The engine stubs currently reject with a readable "not yet implemented" message
that the UI surfaces in the status bar — proving the full JS ⇄ Rust pipeline is
wired end to end. Implementing them is [Phase 3.1](./ROADMAP.md).
