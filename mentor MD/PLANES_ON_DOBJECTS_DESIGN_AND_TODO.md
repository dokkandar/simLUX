# Planes on DObjects — design + comprehensive TODO

**Author:** 3d mentor · **Date:** 2026-07-15 · **Status:** design; needs 2 owner decisions (§8).
**Trigger:** *"add a new attribute to our dobjects as Plane … how are we going to define planes in
our in-file datastructure … make a comprehensive todo list"* + 3 UI asks (§6).

---

## 0. TL;DR

| Question | Answer |
|---|---|
| Is "plane on the dobject" the right call? | **Yes — and it is FORCED, not a preference.** `Geom` is `Vec2`. A dobject on a tilted plane *cannot* store world coords. §1 |
| Does it conflict with existing plans? | **Yes — head-on with `UCS_Roadmap.md`**, which says UCS is "not 3D" and "not per-dobject". That spec assumed a 2D-only project. §2 |
| What shape? | **Mirror the LAYER pattern exactly**: `Document.planes: PlaneTable` + `DObject.plane: PlaneId`, id 0 = World. §3 |
| On-disk? | **RSM v7 → v8**: a `planes` table + a per-dobject id. `#[serde(default)]` ⇒ v7 files still load. §4 |
| Bonus | It **deletes the doc-swap** and the entire crash class we just fixed. §5 |
| Cost | **Diverges `cad_kernel` from RUST_CAD** (byte-identical today). The one real price. §8 |

---

## 1. The decisive constraint — `Geom` is 2D

```rust
pub struct Line { pub a: Vec2, pub b: Vec2 }     // cad_kernel/src/geom.rs
```

Every `Geom` variant is `Vec2`. So:

> A line drawn on a **tilted** plane has **3D** world coordinates.
> **It is impossible to store that in a `Vec2`.**

There are exactly two ways out:

| Option | Consequence |
|---|---|
| Make `Geom` 3D (`Vec3`) | Rewrites the kernel **and** all ~31.8k lines of 2D tools, and destroys `cad_kernel`'s byte-identity with RUST_CAD. **Catastrophic. Reject.** |
| **Store plane-local `(u,v)` + a plane reference on the dobject** | **What you asked for.** The dobject stays `Vec2`; the plane supplies the lift to 3D. |

So the design isn't a preference call — **the type system already decided it.** Your instinct is right.

### 1.1 The elegant property that makes this safe
For **plane 0 = World XY**, `(u, v) ≡ (world x, world y)`. So:

> **For every existing pure-2D drawing the change is a literal no-op.**

That is what makes this survivable: the 2D app never notices, and the merge back is additive.

---

## 2. ⚠️ This collides with `UCS_Roadmap.md` — reconcile it deliberately

`UCS_Roadmap.md` (in **both** simLUX and RUST_CAD) already specs "alternate coordinate system",
and `cad_kernel/src/document.rs` already **reserves the slot**:

```rust
// Reserved for future slices — leave the field list extensible:
// pub ucs_list:    UcsList,
```

But that spec explicitly forbids what you're asking for:

> **Not 3D.** UCS is 2D rotation + translation only … this whole project is 2D.
> **Not per-dobject.** Dobjects always store world coords. The UCS is an **editor lens, not a geometry attribute**.
> **Storage rule: all dobjects stay in WORLD coords on disk and in memory.**

**Who's right?** Both — for different projects. That spec's reasoning is sound *given its stated
premise*: "this whole project is 2D". **3D_Factory broke that premise.** Its storage rule cannot
survive 3D, for the reason in §1: there is no `Vec2` world coord for a tilted plane.

**Therefore: UCS and Plane are TWO DIFFERENT CONCEPTS. Do not merge them.**

| | **UCS** (existing spec, unbuilt) | **Plane** (this design) |
|---|---|---|
| What | An **editor lens** — type/read coords in a rotated 2D frame | A **geometry attribute** — which surface a dobject lives on |
| Storage | Nothing per-dobject | `PlaneId` per dobject |
| Dimensionality | 2D rotation + offset | Full 3D frame (origin + u + v) |
| Affects the file? | Only a UCS table + active index | Dobject coords **mean nothing** without it |

A UCS answers *"what's my typing frame?"*. A Plane answers *"where in 3D does this line exist?"*.
You can have a UCS **on** a plane later; they compose.

**Action:** add a "Superseded for 3D" note to `UCS_Roadmap.md` §"What this isn't", pointing here,
or the next agent will implement the 2D rule and fight this design. **Do this before coding.**

---

## 3. The design — mirror the LAYER pattern exactly

The kernel already has this exact shape and the app already has the machinery. **Reuse it; invent nothing.**

| Layers (exists today) | Planes (this design) |
|---|---|
| `Document.layers: LayerTable` | `Document.planes: PlaneTable` |
| `DObject.style.layer: LayerId` | `DObject.plane: PlaneId` |
| layer 0 reserved (`Layer::layer_zero()`) | **plane 0 = World XY, reserved** |
| current/active layer | **active plane** |
| `doc.layers.renders(e.style.layer)` render gate | `doc.planes.renders(...)` / active-plane gate |
| lock / freeze / visible | **other planes = reference (greyed, snappable, not editable)** — identical semantics to a locked layer |
| serialised as a table in RSM | same |

```rust
// cad_kernel::plane (new)
pub struct PlaneId(pub u16);                 // 0 = World XY, always present

pub struct DPlane {
    pub id:     PlaneId,
    pub name:   String,      // "Top of box", "Wall A"
    pub origin: [f32; 3],    // world
    pub u:      [f32; 3],    // unit, in-plane +U
    pub v:      [f32; 3],    // unit, in-plane +V  (normal = u × v)
    pub color:  u8,          // ACI — for the plane list + 3D outline
}

pub struct PlaneTable { planes: Vec<DPlane> }   // planes[0] == World XY

// cad_kernel::Document
pub planes: PlaneTable,

// cad_kernel::DObject
pub plane: PlaneId,          // #[serde(default)] → 0 = World
```

**Why `DObject.plane` and not `DObject.style.plane`:** `Style` is presentation (colour, linetype,
visible). A plane changes what the coordinates **mean** — that is geometry, not style. Keep it a
peer of `geom`.

**Frame math already exists** — `cad_solid::Frame { origin, u, v }` with `to_uv` / `from_uv` /
`from_point_normal`. `DPlane` is that struct plus identity. Move `Frame` down into `cad_kernel`
(or make `DPlane` convert to it) so both crates share one definition — **do not write a second one.**

### 3.1 The hazard this creates — tools must filter by plane
One document means the tools now see **every** plane's dobjects. That is **wrong** for anything
geometric: TRIM would take cutters from a plane that isn't coplanar, OSNAP would snap to a point
that isn't on your surface, a window-select would grab another wall.

**Mitigation = the layer gate, reused.** Everything geometric filters to the **active plane**:
- candidate iteration (the culling loop — the one that just crashed),
- pick / window-select,
- trim/extend/fillet/chamfer cutters + boundaries,
- osnap targets (with an opt-in "snap to other planes" later, like AutoCAD's projected snaps).

**This is the bulk of the work** and the main risk. It is ~1 predicate applied at ~6 choke points,
*not* 31.8k lines of edits — because the tools all funnel through the candidate list + selection.

---

## 4. On-disk — RSM v7 → v8

Today: `cad_io/src/rsm.rs` → `const VERSION: u16 = 7`.

```
RSM v8 adds:
  TABLE  planes:
      id, name, origin[3], u[3], v[3], color
      (id 0 = World XY — written explicitly so the file is self-describing)
  DOBJECT record gains:
      plane: u16          #[serde(default)] → 0
```

**Compatibility rules (both directions):**
- **v7 → v8 (read old file):** no `planes` table ⇒ synthesise World only; every dobject `plane = 0`.
  Because of §1.1 this is **exactly** the current behaviour. Old drawings are untouched.
- **v8 → v7 (old app reads new file):** dobjects on plane ≠ 0 would be silently misread as World —
  their `(u,v)` would be drawn as world XY. **Decide:** either bump the "minimum reader version"
  so v7 readers refuse, or accept the flattening. **I recommend refusing** — silent geometry
  corruption is worse than a clear error.

**"Keep previews for the user":** the `planes` table + per-dobject id is exactly what makes a
preview possible — on reopen you can rebuild every plane's frame and lift its geometry into 3D
without re-deriving anything. Add to the plane record only what a preview needs and nothing more
(`name`, `color`). **Do not** store a bitmap thumbnail — it's derivable, and it will go stale.

**Also needed:** the 3D solid features themselves (`cad_solid::Model`) still have no persistence
(`SIMLUX_3D_SOLIDS_PLAN.md` proposed "RSM v9"). **Sequence planes as v8 first** — they're a smaller,
self-contained change — then solids. Don't do both in one version bump.

---

## 5. The prize: this DELETES the doc-swap and its crash class

Today `factory_enter_sketch` swaps `self.doc`, which is why we needed `factory_reset_doc_state()`
and why a stale **spatial index** crashed the app (commit `07120ac`).

With one document + a plane id:
- **no swap** → no parked doc, no parked undo stacks, no stale index, **no `factory_reset_doc_state`**;
- **one undo stack** across 2D and 3D — Ctrl+Z just works everywhere;
- everything is displayable **together**, which is what you asked for;
- entering a plane becomes *exactly* as cheap and safe as **switching the current layer**.

That is the strongest argument for this design after §1. **The crash we just fixed becomes
structurally impossible.**

---

## 6. Your three UI asks — all reuse something that exists

1. **"3D Factory active → split display in equal half."** ✅ Already built: the SIMLUX workspace does
   exactly this — `let half = ctx.screen_rect().width() * 0.5; base.exact_width(half)`, gated on
   `light.simlux_mode`. **Reuse that pattern**, don't write a second splitter. Open question: 3D
   Factory and SIMLUX-workspace both want the right half — they must be **mutually exclusive**
   (one workspace mode enum), or they'll fight over the panel.
2. **"Basic 3D objects should have controllers."** Needs a **feature inspector**: pick a feature in
   the 3D view → edit its `Placement { u, v, lift, spin_deg }` + `Primitive` params (box `w/d/h`,
   cylinder `r/h/sides`) + its `BoolOp`. The app already has `param_editor.rs` and an Inspector
   panel — **follow the Inspector's shape**. Must re-eval CSG **only on release**, never per-drag
   frame (csgrs walks a BSP per boolean — this is the known lag source).
3. **"3D viewpoint setter same as sandbox."** The app has **no** viewcube; the sandbox's is
   `sandbox.rs:2150 fn navigator`. **Port it verbatim** — it is the only one of the three that is a
   genuine port rather than a reuse.

---

## 7. THE COMPREHENSIVE TODO

Ordered so each slice is independently mergeable and independently *provable*.

### Phase A — decide + document (before any code)
| # | Task | Done when |
|---|---|---|
| A1 | **Decide §8.1**: accept `cad_kernel` divergence from RUST_CAD | written down in the memo |
| A2 | **Decide §8.2**: v7 readers refuse v8, or flatten | written down |
| A3 | Add a "**superseded for 3D**" note to `UCS_Roadmap.md` pointing here — Plane ≠ UCS (§2) | the next agent can't mis-implement |
| A4 | Decide the workspace-mode enum (3D Factory vs SIMLUX split are exclusive, §6.1) | one enum, not two bools |

### Phase B — kernel (pure, testable, no UI)
| # | Task | Done when |
|---|---|---|
| B1 | `PlaneId`, `DPlane`, `PlaneTable` in `cad_kernel`; `planes[0] = World XY` | unit tests |
| B2 | Move/share `Frame` (origin/u/v, `to_uv`/`from_uv`/`from_point_normal`) into the kernel; `cad_solid::Frame` re-exports it — **one definition** | `cad_solid` compiles against it |
| B3 | `Document.planes`; `DObject.plane: PlaneId` with `#[serde(default)]` | `cargo test -p cad_kernel` green |
| B4 | `DPlane::to_world(uv) -> Vec3`, `from_world(Vec3) -> Vec2`, round-trip tests incl. tilted planes | property test: `from_world(to_world(p)) ≈ p` |
| B5 | **Prove §1.1**: every existing test still passes with plane 0 ⇒ 2D is a no-op | full kernel suite green, unchanged |

### Phase C — persistence (RSM v8)
| # | Task | Done when |
|---|---|---|
| C1 | RSM `VERSION 7 → 8`; write the `planes` table + per-dobject id | round-trip test |
| C2 | **Read v7** → synthesise World, all dobjects plane 0 | golden v7 file loads byte-identical |
| C3 | v7-reader policy from A2 (refuse or flatten) | explicit test for the chosen behaviour |
| C4 | Round-trip test: 2 planes + geometry on each → save → load → identical frames + ids | test green |

### Phase D — the app: active plane (mirror the layer machinery)
| # | Task | Done when |
|---|---|---|
| D1 | `active_plane: PlaneId` on `CadApp` (peer of the current layer) | — |
| D2 | **Filter the candidate iteration by active plane** (the culling loop) | other planes don't render as if flat |
| D3 | Filter **pick + window-select** | can't select through a plane |
| D4 | Filter **osnap targets** | can't snap to a non-coplanar point |
| D5 | Filter **trim/extend/fillet/chamfer** cutters + boundaries | ⚠️ the correctness-critical one |
| D6 | New dobjects inherit `active_plane` (like the current layer) | draw on a plane → correct id |
| D7 | Other planes render as **reference** (greyed, snap-optional) — reuse locked-layer semantics | visible but not editable |
| D8 | **Delete `factory_enter_sketch`/`exit`/`factory_reset_doc_state`** + the `SketchSession` doc-swap; "enter a plane" = set `active_plane` | the §5 prize; crash class gone |

### Phase E — 3D view + UI (your §6 asks)
| # | Task | Done when |
|---|---|---|
| E1 | Workspace mode enum; **3D Factory = equal half split** (reuse `simlux_mode`'s `exact_width(half)`) | toggling splits 50/50 |
| E2 | **Port the sandbox viewcube** (`sandbox.rs:2150 fn navigator`) | standard views snap |
| E3 | 3D view lifts each dobject by its plane frame (replaces `factory::sketch_lines`) | 2D work appears on its plane in 3D |
| E4 | **Feature inspector/controllers**: pick a feature → edit Placement + Primitive + BoolOp | drag `w` → box resizes |
| E5 | CSG re-eval **on release only**, never per-drag frame | no lag while dragging |
| E6 | Plane list panel (name, colour, active, visible) — mirror the Layers panel | switch plane from the UI |
| E7 | `plane` commands: `plane` (list), `plane <name>`, `plane new` (3-pick), `plane world`, `plane del` — mirror `UCS_Roadmap` §"How users interact" | typed flow works |

### Phase F — close the loop
| # | Task | Done when |
|---|---|---|
| F1 | Extrude a plane's closed profile → a `cad_solid` feature | 2D → solid |
| F2 | 3D modifiers on the app command line via `run_command`'s cascade (Slice 4, still owed) | move/rotate a solid by typing |
| F3 | `cad_solid::Model` persistence (**RSM v9**, after v8 lands) | solids survive save/reopen |

### Acceptance test for the whole design
```
box → right-click top face → plane created + active
f → r → 10                      (the app's own fillet, on that plane)
save → reopen                   → the plane + its filleted profile are back, in 3D
switch to World                 → the 2D drawing is untouched
```

---

## 8. Decisions I need from you

**8.1 — `cad_kernel` divergence (the real cost).**
`cad_kernel` is **byte-identical to RUST_CAD** today; adding `DObject.plane` + `Document.planes`
breaks that, so the merge spreads to a second repo. Options:
- **(a) Accept it** ← my recommendation. This feature has to go upstream eventually; §1.1 makes it
  additive and a no-op for 2D, so it's the cheapest it will ever be.
- (b) Side-table (`HashMap<Handle, PlaneId>` in `cad_solid`). Kernel untouched — but it **desyncs**
  the moment any of the 31.8k lines of tools creates a dobject without telling the side-table.
  **Reject: it reintroduces exactly the "two sources of truth" bug class this fork exists to kill.**

**8.2 — v7 readers meeting a v8 file:** refuse (my recommendation — silent geometry corruption is
worse than an error), or flatten to World?

**8.3 — Scope check:** Phases B–D are the real work (D5 especially). Confirm you want the full
one-document model, **or** say the word and I'll keep the current per-sketch documents — they work
today, at the cost of the doc-swap, split undo, and no combined display.

*MD only — no code changed by this review.*
