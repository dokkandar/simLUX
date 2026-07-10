# Block System — Guide for a Coding Agent

How **blocks**, **insertion**, and **smart (parametric) blocks** work in simLUX,
and how an instance is placed. Written for an agent extending this code.

- **Kernel data model** lives in `cad_kernel/src/block.rs` (pure 2D, UI-free).
- **All UI + flow** lives in `cad_app/src/app.rs` (egui). Line numbers below are
  approximate — they drift; search by symbol.
- Rule of thumb: the kernel stores *definitions + instances + deformation
  primitives*; the app drives *creation, insertion, and live preview*.

---

## 1. Mental model

A **Block** is a named group of geometry (a *definition*) stored once in
`Document.blocks`. A **BlockRef** is an *instance*: a lightweight reference
(`block` id + pose) that says "draw definition N here, scaled/rotated/mirrored."
Define once, insert many.

A **smart block** additionally carries **parameters** (`BlockParam`s). Each
parameter owns one or more **modifier vectors** (`ParamVector`) that stretch a
window of the definition's geometry. At insert time the user supplies a *value*
per parameter; the geometry is **re-derived** (deformed) from those values. This
is how one "door" definition produces doors of any width.

```
Document.blocks: BlockTable
   └─ Block { name, base, dobjects[], smart, params[], cut_edges[] }   ← definition
BlockRef { block: id, insert, scale, scale_y, rotation, mirror_x, param_values } ← instance (a Geom on the canvas)
```

**Three block kinds.** The first two are kernel flags on the *definition*; the
third is a SIMLUX-side marker, so the kernel stays lighting-agnostic:

| Kind | Marker | Lives in | What it adds |
|------|--------|----------|--------------|
| **Plain** | — | — | a reusable symbol; define once, insert many |
| **Smart** | `Block.smart` + `params[]` | `cad_kernel` | parameters that deform the geometry at insert (§2–§8) |
| **Luminaire (LUX)** | an entry in `LightState.lux_blocks` (sidecar) | `cad_app` / sidecar | a photometric identity — links **many IES, one active**; instances DERIVE fixtures for the lux calc (§11) |

`smart` and `luminaire` are **orthogonal** — a block can be neither, either, or (in
principle) both. Ticking either checkbox in the Block dialog sets its marker.

---

## 2. Kernel data model (`cad_kernel/src/block.rs`)

```rust
pub const MAX_BLOCK_PARAMS: usize = 8;   // param_values is a fixed array so BlockRef stays Copy

pub struct BlockRef {                    // an INSTANCE (a Geom::BlockRef variant)
    pub block:    u32,                   // id into Document.blocks
    pub insert:   Vec2,                  // world point the definition's `base` lands on
    pub scale:    f64,                   // X scale magnitude (>0)
    pub scale_y:  f64,                   // Y scale magnitude (>0); differs → circles become ellipses
    pub rotation: f64,                   // radians CCW
    pub mirror_x: bool,                  // reflect across local Y (through base) before scale+rot
    pub param_values: [f64; MAX_BLOCK_PARAMS],  // per-instance smart values (unused slots = 0)
}

pub struct ParamVector {                 // one modifier of a parameter
    pub win_min: Vec2, pub win_max: Vec2,// stretch window (definition space): points inside move
    pub dir:     Vec2,                   // unit-ish direction the window moves
    pub gain:    f64,                    // links vectors to one value w/ different magnitudes
}

pub struct BlockParam {                  // one named variable, e.g. "width"
    pub name:     String,
    pub original: f64,                   // the value the SOURCE block represents (displacement 0)
    pub vectors:  Vec<ParamVector>,      // ≥1 vectors this variable drives together
}

pub struct Block {                       // a DEFINITION
    pub name:     String,
    pub base:     Vec2,                  // reference point that aligns to BlockRef.insert
    pub dobjects: Vec<DObject>,          // the geometry (definition space)
    pub smart:    bool,                  // marker only; params[] is what actually deforms
    pub params:   Vec<BlockParam>,       // EMPTY until vectors are defined (see §8)
    pub cut_edges: Vec<usize>,           // jamb lines → door/window cuts the host wall
}

pub struct BlockTable { pub blocks: Vec<Block>, /* … */ }   // .get(id) .add(b)->id .find(name)->Option<id>
```

**Instance → world transform** — `BlockRef::transform_geom(&self, g, base)`
(block.rs): mirror (optional) → scale about base → rotate about base → translate
`insert − base`. Works for every `Geom`, including nested `BlockRef`s.

---

## 3. Deformation math — how a value bends the geometry

`CadApp::block_derived_geoms(blk, param_values) -> Vec<Geom>` (app.rs) produces
the definition geometry deformed by the given values, in **definition space**:

```
for each param k:
    amount = param_values[k] - param.original
    for each vector v in param.vectors:
        disp = v.dir * (v.gain * amount)
        geom = stretch_one(geom, v.win_min, v.win_max, disp)   // move pts inside the window by disp
```

So `param_values[k]` is the **target value**; displacement is
`dir · gain · (value − original)`. To draw/place a deformed instance:
`block_derived_geoms(...)` → then `BlockRef::transform_geom` each result to world.

A plain block has `params == []`, so `block_derived_geoms` returns the geometry
unchanged and `param_values` is ignored.

---

## 4. Creating a block

Command `block <name>` / bare `block` (opens the dialog). Handler: `Command::BlockDef`.

- **Dialog** `BlockDialog` (`render_block_dialog`): name, base point (X/Y or
  Pick⊕), instance colour, **Smart block** checkbox. Default base =
  **`selection_centroid()`** — the selection's gravity centre (rule: an undefined
  insertion point puts the block on its own centre).
- **OK** → `apply_block_create(name, base, color_aci, smart)`:
  clones the selected dobjects into a new `Block`, `doc.blocks.add(...)`,
  replaces the originals with ONE `BlockRef` at `insert == base` (AutoCAD BLOCK
  behaviour), so the drawing looks unchanged.
- If **Smart** was ticked → immediately `open_block_editor(id)` (§8) so the user
  can define the parameter vectors.

`smart = true` only sets a flag/badge; it does **not** create params. Params come
from the editor (§8). A smart block with `params == []` behaves like a plain one.

---

## 5. Rendering a block instance

`Geom::BlockRef` is drawn in the main render loop (`RenderMode::Cpu`/`Gpu`), on
the egui painter, by resolving the definition through the block table.

- **Bbox**: a raw `BlockRef` bbox is a degenerate point (insertion point). Always
  use **`resolved_blockref_bbox(br)`** for cull/hit-test/selection — otherwise the
  sub-pixel micro-cull (`bbox_px < 1.0`) skips every unselected instance and it
  renders invisible. (This was a real bug; the fix is in both render paths.)
- **Explode / snap-through / cut**: `expand_cutter_geoms(&geom, out, depth)`
  resolves a `BlockRef` (recursively) into its world-space child geoms. Reused by
  trim/extend cutters, object-snap phantoms, and the insert **preview**.

---

## 6. Insertion — the state machine

State fields on `CadApp` (all in app.rs):

| Field | Meaning |
|-------|---------|
| `insert_state: InsertState` | `Off` / `WaitingForPoint{block}` / `WaitingForAngle{block,insert}` |
| `insert_dialog: Option<InsertDialog>` | the **Insert Block** dialog |
| `pending_insert: Option<PendingInsert>` | block+scale+rotation+params armed, awaiting the base CLICK |
| `insert_param_pick: Option<(usize, Option<Vec2>)>` | dialog "↔": 2 clicks → distance = a param value |
| `insert_live: Option<InsertLive>` | LIVE parametric drag phase (smart blocks) |

**INVARIANT: a block is NEVER placed without a clicked insertion point.** The
dialog never places on OK — it *arms* a pending insert; a canvas click places it.

### Flow

```
insert            → Command::Insert(None)  → open InsertDialog (first block preselected)
insert <name>     → Command::Insert(Some)  → open InsertDialog with <name> preselected
```

`InsertDialog` (`render_insert_dialog`): block dropdown, Scale, Rotation°, and —
for a smart block — one value box per parameter, each with a **↔** button
(`insert_param_pick`: click two points, their distance fills the value).
`Insert` button →

```rust
pending_insert = Some(PendingInsert { block, scale, rotation, param_values });
insert_state   = WaitingForPoint { block };
// dialog closes; prompt "click the insertion point"
```

Canvas click while `WaitingForPoint` + `pending_insert` (see the click handler,
`else if let InsertState::WaitingForPoint`):

```
nparams == 0  → place_block_full(block, click, scale, rotation, param_values)   // plain: done
nparams >  0  → enter LIVE phase (§7): insert_live = InsertLive{…, values, idx:0}
```

`place_block_full(block, insert, scale, rotation, param_values)` builds the
`BlockRef`, `add_dobject`, then `apply_block_cut` (door/window cuts the host).
It ALWAYS places (unlike the legacy `apply_insert`, whose parametric branch
waited for a command-line prompt).

The **shade preview** `paint_insert_preview` draws a dashed ghost of the block
following the cursor during `WaitingForPoint` (using the pending scale/rotation).

---

## 7. Smart-block LIVE insertion (the dynamic-block feel)

After the base click, a smart block enters `insert_live`:

```rust
pub struct InsertLive { block, insert, scale, rotation, values: Vec<f64>, idx: usize }
```

Each frame, `paint_insert_live` reads the cursor, computes the **current**
parameter's value, re-derives the block, and draws it deforming (dashed ghost +
value readout). One parameter at a time:

- **cursor → value**: `live_param_value(live, cursor)` = project `(cursor − insert)`
  onto the parameter's first vector `dir` (rotated by `rotation`, de-scaled),
  added to `original`. Drag away along the direction → value grows.
- **click** (click handler, `else if self.insert_live.is_some()`) FIXES
  `values[idx]`, `idx += 1`; when `idx == values.len()` →
  `place_block_full(block, insert, scale, rotation, values)`.
- **typed number** (command handler, `if self.insert_live.is_some()`) sets the
  current value manually and advances — same as a click.
- **Esc** clears `insert_live` (and every other insert state).

```
insert door → dialog(Insert) → click BASE → drag → door frame widens live → click = fix width → placed
```

Both `insert_live` and `insert_param_pick` phases are added to the **click-only**
gate (so a press-release is a click, not a window-select) and to the Esc reset.

---

## 8. Where parameters (vectors) come from — the current gap

A freshly smart-marked block has `params == []`, so there is nothing to drag.
Params are authored in the **isolated Block Editor**:

- `open_block_editor(id)` (opened by Smart+OK, by `btr`/BlockTaskRecorder on a
  selected instance, or the Block dialog's **Edit ▶**). Seeds from existing
  `blk.params`.
- The user **demonstrates stretches** (Block Task Recorder → `ParamRow`s), names
  them, and **Save** → `save_block_params(id, rows)` writes `blk.params`
  (windows re-based by `blk.base`).

**Open task:** a simpler "Add vector" UI (pick a direction — e.g. Up/Down — a
window, gain, name) so a block gets params without demonstrating a drag. Until
that exists, test the live flow on a block whose `params` are already populated.

---

## 9. Key symbols (search these)

| Symbol | File | Role |
|--------|------|------|
| `BlockRef` / `Block` / `BlockParam` / `ParamVector` | `cad_kernel/src/block.rs` | data model |
| `BlockRef::transform_geom` | `cad_kernel/src/block.rs` | definition → world |
| `apply_block_create` / `selection_centroid` | `cad_app/src/app.rs` | create a block, centroid base |
| `render_block_dialog` / `BlockDialog` | app.rs | create dialog (+ Smart, Edit ▶) |
| `render_insert_dialog` / `InsertDialog` | app.rs | insert dialog (block/scale/rot/params/↔) |
| `place_block_full` | app.rs | always-place a configured instance |
| `apply_insert` | app.rs | legacy click flow (point→angle; parametric = cmd-line prompt) |
| `block_derived_geoms` | app.rs | deform geometry from param values |
| `live_param_value` / `paint_insert_live` / `InsertLive` | app.rs | live parametric insert |
| `paint_insert_preview` | app.rs | dashed shade during WaitingForPoint |
| `resolved_blockref_bbox` / `expand_cutter_geoms` | app.rs | correct bbox / explode |
| `open_block_editor` / `save_block_params` | app.rs | author parameter vectors |
| `LuxBlock` / `lux_blocks` | `cad_app/src/simlux_io.rs`, `light.rs` | luminaire block: many IES linked, one active |
| `derived_luminaires` / `derived_fixture_count` | `cad_app/src/light.rs` | derive fixtures from LUX-block instances |

## 10. Invariants / rules
- **No block without a clicked insertion point** — the dialog arms; a click places.
- **Default base = gravity centre** (`selection_centroid`) when the user doesn't set one.
- **`param_values` is a fixed `[f64; 8]`** so `BlockRef` stays `Copy`; parallel to `params` by index.
- **Always resolve `BlockRef` bbox** via `resolved_blockref_bbox` before any cull/hit-test.
- **Kernel stays 2D and UI-free**; new interaction goes in `cad_app`, not `cad_kernel`.
- **Luminaire blocks keep photometry in the sidecar** (`lux_blocks`), never in the kernel; many IES linked, one active (§11).

---

## 11. Luminaire (LUX) blocks — photometry on a block

A **luminaire block** is an ordinary block that also carries a **photometric
identity** for the lux calculation. Its 2D geometry is the symbol that prints on
the lighting layout; the assigned **IES** is what the calc uses; a future visual
mesh is what a render uses — **one block, up to three consumers** (the
"two-representation" rule, `SIMLUX_SCENE_AND_DAYLIGHT_PLAN` §3.2).

### 11.1 Data model — kernel-clean, in the sidecar
The kernel `Block` is UNCHANGED. Whether a block is a luminaire — and which IES it
carries — lives SIMLUX-side, keyed by the block DEFINITION NAME:

```
// cad_app/src/simlux_io.rs — persisted in <drawing>.simlux.json
LightState.lux_blocks: BTreeMap<BlockName, LuxBlock>
LuxBlock {
    ies_options: Vec<String>,    // MANY IES linked (names into the shared ies_library)
    active:      Option<String>, // exactly ONE active (must be in ies_options) — calc + render use it
}
```

- **A block IS a luminaire iff it has an entry in `lux_blocks`.** No kernel flag.
- **Many linked, one active.** A fixture can hold several lamp/optic options
  (e.g. one IES per mounting height 10′/20′/30′); only the **active** one is
  calculated/rendered. `LuxBlock::active_ies()` returns the active name *iff* it is
  still one of the options.
- **IES referenced by NAME**, never copied — entered once into `ies_library`,
  shared by every instance; swap the library entry → every fixture updates.

### 11.2 How it wires (create → link → derive → calc)
```
Block dialog: tick "Luminaire (IES) block"      ─► lux_blocks[name] = LuxBlock::default()      (apply_block_create, then register)
Light panel ▸ "Luminaire blocks"                ─► link IES options + pick the ◉ active         (panel_ui)
Place instances (insert / copy / array)         ─► BlockRef(name) dobjects in the Document
Calculate                                       ─► derived_luminaires(doc): each BlockRef whose def
                                                   is a lux block with an active IES emits a
                                                   Luminaire{ profile=active, pos=insert@mount, rot } (light.rs)
                                                └► lums = derived + manually-placed; else auto-centre
```
The payoff: **placing luminaire blocks IS placing the calc's fixtures** — no
separate luminaire list to keep in sync. (Manual fixtures still add on top; the
old manual side-list is no longer the primary source.)

### 11.3 Command line
- `luxblock` — list luminaire blocks, each block's linked-IES count + active IES,
  and how many fixtures currently derive from the drawing. (Linking/activation is
  in the Light panel; a full `luxblock <name> <ies>` setter waits on quoted-name
  parsing, since IES/block names contain spaces.)

### 11.4 Pose of a derived fixture
`position = (BlockRef.insert.x, .y, LightState.mount_height)` — the symbol sits on
the plan; mount height lifts it to the ceiling — and `rotation_deg =
BlockRef.rotation`. Per-instance mount height / dimming is a future refinement
(today all derived fixtures share `mount_height`, dimming 1.0).

### 11.5 Invariants
- **Kernel stays lighting-agnostic** — luminaire state is sidecar-only, keyed by
  block name (survives save/reopen; `cad_kernel` / `cad_io` untouched).
- **One active IES** — `active` must be one of `ies_options`; removing the active
  option re-points `active` to the first remaining (or `None`).
- **No photometry ⇒ no light** — a luminaire block with `active_ies() == None`
  still draws + prints its symbol but contributes nothing to the calc.
- **IES by reference, never copied** — always a name into `ies_library`.
