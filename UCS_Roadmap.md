# User-Defined Coordinate Systems (UCS) — Roadmap

Captured 2026-06-06. Spec for the future feature; nothing in this
document is implemented yet beyond the basic UCS *indicator* (the
red-dot icon and its toggle).

## What "UCS" means here

In AutoCAD and every traditional CAD tool a User Coordinate System is
a per-user, per-drawing alternate origin + rotation. The user is
working on, say, a slanted wall face — they want to **type
coordinates relative to that wall**, **see distances along its local
axes**, **draw rectangles aligned with it**, and **switch back to the
World coordinate system any time**.

A UCS in RUST_CAD would consist of three numbers:

```rust
struct UserCS {
    name:   String,        // e.g. "Wall A", "Roof Panel 3"
    origin: Vec2,          // world coords of THIS UCS's (0,0)
    x_axis: Vec2,          // unit vector defining THIS UCS's +X
}
```

`y_axis` is implicit (`x_axis.perp()` — rotate 90° CCW).

## Storage rule

**All dobjects stay in WORLD coords on disk and in memory.** UCS is
purely a **display + input transform**. This keeps DXF/DWG
interoperability simple, undo cheap, and the spatial index unaffected.
The active UCS is stored on the Document (so it persists across saves)
plus a session-only "current" pointer.

```rust
// In cad_kernel::Document:
pub user_coord_systems: Vec<UserCS>,
pub active_ucs: Option<usize>,        // None = World; index into Vec
```

## How users interact

### Creating a UCS — three-point flow
```
> ucs new "Wall A"
  ucs: click ORIGIN
  ucs: click point on +X axis
  ucs: click point on +Y axis  (used only to confirm orientation,
                                NOT a free choice — the system locks
                                Y = +90° from X to stay orthogonal)
  ✓ UCS "Wall A" saved
```

### Activating / switching
```
> ucs                      → list named UCSs, ask which to activate
> ucs "Wall A"             → activate "Wall A"
> ucs world  (or ucs w)    → back to the world UCS
> ucs prev                 → toggle between current and previous
```

### Inline coord entry
While a UCS is active, all coordinates the user TYPES are interpreted
in that UCS. The parser converts to world before storing. Example with
"Wall A" rotated 30° CCW:

```
> line 0,0 100,0
   ← line endpoints are (0,0) and (100,0) IN UCS "Wall A"
   ← stored in doc as the world equivalents (after rotation + offset)
```

### Status-bar feedback
- Cursor coordinates in the bottom-left switch to UCS coords (white)
  with the world equivalent in a dimmer tone next to them.
- A small "UCS: Wall A" pill appears in the status bar, click to
  switch back to World.

### The UCS icon
The existing red-dot icon (currently corner-pinned) gets a second
purpose: when a non-World UCS is active, the X / Y arrows rotate
to point in the active UCS's axis directions. The label gains the
UCS name.

## DXF / DWG round-trip

DXF has a UCS table (`UCS`) plus per-entity ExtMin/ExtMax in
arbitrary planes. RUST_CAD's UCS list maps 1:1 to the UCS table on
write. On read, named UCSs come in; the active UCS on read is World
(matches AutoCAD's default behaviour).

## Implementation slicing (when this is scheduled)

| Slice | Scope | Effort |
|---|---|---|
| **1. Kernel** | Add `UserCS` struct + `Document.user_coord_systems` + `active_ucs` index. Helper methods: `to_world(p, ucs)`, `from_world(p, ucs)`, `to_world_vec(v, ucs)`. Pure math, no UI. | small (~half day) |
| **2. Command parser** | `ucs` / `ucs new <name>` / `ucs <name>` / `ucs world` / `ucs prev` / `ucs list` / `ucs delete <name>`. | small |
| **3. Coord input transform** | Every coord parsed from the command line (`line 0,0 10,5`, etc.) passes through `to_world(.., active_ucs)`. Add at the parser-to-app boundary. | medium (touches all draw commands) |
| **4. UCS icon rotation + label** | `draw_ucs_icon` reads the active UCS, rotates the X/Y arrows by `x_axis.angle()`, shows the UCS name. Replaces "User logo" box behaviour when active. | small |
| **5. Status-bar coord readout** | When a UCS is active, the cursor-coord label shows UCS coords first, world coords in dimmer tone after. | small |
| **6. UCS persistence (settings)** | Active UCS name persisted on document save. | small |
| **7. DXF read/write** | UCS table support in `cad_io::dxf` (Wait until DXF I/O lands). | medium |

Total for slices 1-6: roughly 2-3 days. Slice 7 piggybacks on DXF I/O work.

## What this isn't

- Not 3D. UCS in RUST_CAD is 2D rotation + translation only.
  AutoCAD's 3D UCS (with a Z axis you can tilt) is out of scope —
  this whole project is 2D.
- Not per-dobject. Dobjects always store world coords. The UCS is
  an editor lens, not a geometry attribute.
- Not a layer of indirection on undo / spatial index — those run on
  world coords and don't need to know UCS exists.

## Related memory

- [[rust-cad-universal-selection-model]] — the input-classifier rule
  the UCS coord-input layer needs to respect.
- [[rust-cad-settings-naming]] — naming convention for any new
  SYSVARs (`UcsAct` = active UCS index? — TBD when implementing).
