# SIMLUX

A physically-based **lighting (lux) simulator** for professional lighting design —
draw rooms, place IES luminaires, and compute illuminance grids with a
ray-traced / progressive-radiosity engine.

Built with **Tauri v2 + React (react-three-fiber)** on the front, and a
**Rust** computation engine on the back.

> Status: **Phase 3.2** — draw walls over a DXF underlay, stitch + extrude them
> to a room, place IES luminaires, and compute a ray-traced direct + indirect
> lux heatmap. See [ROADMAP.md](./ROADMAP.md) for the phased plan and
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
| CAD import | DXF via `cad_io` (from `dokkandar/Auto_RASM`) — underlay only |
| Drafting   | In-app 2D CAD view (Line/Polyline/Rect/Wall); lines extrude to surfaces |

## Layout

```
SIMLUX/
├─ src/                     React frontend
│  ├─ api/commands.ts       typed wrappers over Tauri invoke()
│  ├─ components/           Plan2D (2D CAD view), ToolPalette, Viewport (r3f),
│  │                        Toolbar (tabs), Sidebar, StatusBar
│  ├─ three/coords.ts       engine Z-up ↔ three.js Y-up mapping
│  ├─ store/projectStore.ts zustand app state (tab, tool, project, …)
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
│        ├─ wall/           extrude lines/walls → surface meshes (+floor/ceiling)
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
| `add_wall(sx, sy, ex, ey, thickness)`    | `Project`     | append a drawn wall segment          |
| `move_wall` / `offset_wall`              | `Project`     | wall modifiers                       |
| `build_room(height, plane_height)`       | `Project`     | stitch + extrude walls → room + grid |
| `clear_walls()`                          | `Project`     | remove walls + room                  |
| `add_demo_room(width, depth, height, …)` | `Project`     | box room + calc grid + a downlight   |
| `calculate_lux()`                        | `LuxGrid`     | compute the illuminance grid         |

### Try it — draft a room

In the **Construction** tab, pick the **Rect** (or **Wall**) tool and draw a
room on the grid (snaps to nodes/grid; wheel to zoom, drag to pan; Esc finishes a
chain). Optionally `Load DXF` (`samples/corner sofa.dxf`) first as a reference
underlay to trace. Then `Import IES` (`samples/T1.ies`) → `Build Room` (extrudes
each line to a surface + floor/ceiling, switches to **3D & Light**) →
`Calculate`. The heatmap shows direct + reflected illuminance.
