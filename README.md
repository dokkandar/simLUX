# SIMLUX

A physically-based **lighting (lux) simulator** for professional lighting design —
draw rooms, place IES luminaires, and compute illuminance grids with a
ray-traced / progressive-radiosity engine.

Built with **Tauri v2 + React (react-three-fiber)** on the front, and a
**Rust** computation engine on the back.

> Status: **Phase 3.1** — IES LM-63 parsing, DXF import, and a working
> direct + one-bounce-indirect lux engine (ray-traced, rayon-parallel). See
> [ROADMAP.md](./ROADMAP.md) for the phased plan and
> [docs/Guide-for-simLUX.txt](./docs/Guide-for-simLUX.txt) for the full design
> research.

## Stack

| Layer      | Choice                                                     |
| ---------- | ---------------------------------------------------------- |
| Shell      | Tauri v2                                                   |
| UI         | React 19 + Vite + `@react-three/fiber` / `drei` (Three.js) |
| State      | zustand                                                    |
| Engine     | Rust — `glam` (math), `rayon` (parallelism), custom BVH ray tracer |
| Photometry | Custom IES LM-63 parser + bilinear candela interpolation   |
| CAD import | DXF via `cad_io` (from `dokkandar/Auto_RASM`)              |

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
│        ├─ geometry/       2D/3D primitives, meshes, box_room, calc plane
│        ├─ rt/             ray tracer: Tri/AABB/BVH + cosine sampling
│        ├─ calc/           direct + Monte-Carlo indirect lux (rayon)
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

| Command                                  | Returns to JS | Purpose                              |
| ---------------------------------------- | ------------- | ------------------------------------ |
| `engine_info()`                          | `EngineInfo`  | health check / version               |
| `get_project()`                          | `Project`     | snapshot of app state                |
| `import_ies(path)`                       | `IesProfile`  | parse + store an IES file            |
| `load_dxf(path)`                         | `Line2[]`     | load DXF plan geometry               |
| `add_luminaire(x, y, z, profile)`        | `Project`     | place a luminaire in the scene       |
| `add_demo_room(width, depth, height, …)` | `Project`     | box room + calc grid + a downlight   |
| `calculate_lux()`                        | `LuxGrid`     | compute the illuminance grid         |

### Try it

`Import IES` (e.g. `samples/T1.ies`) → `Demo Room` → `Calculate Lux`. A 4×4×3 m
room appears with a ceiling downlight, and the work-plane heatmap shows direct +
reflected illuminance. `Load DXF` (e.g. `samples/corner sofa.dxf`) draws a plan
underlay.
