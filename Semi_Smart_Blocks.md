# Semi-Smart Blocks (Parametric Blocks) — design proposal

> **Status: PROPOSAL — do not implement until asked.**
> Companion to `Smart_Dobjects.md` (walls = smart member #1). This doc defines
> the *semi-smart* category and its first member, the **Door** block.
> Naming reminder: drawing entities are **Dobjects**, never "entities".

---

## 1. The problem (user's door example)

A door symbol drawn as a classic static block is wrong the moment the wall
changes:

- The **frame profile thickness** (e.g. 15 mm) is a manufacturing constant —
  it must NEVER scale.
- The **frame depth** must equal the **host wall thickness** (70 / 100 / 150 mm).
- The **leaf length** and **swing arc radius** follow the **door width** W.

Uniform scale distorts the profile. Non-uniform scale distorts it differently.
No static block can be correct in two different walls. Different parts of the
block obey **different rules** — that is the defining property of the
semi-smart category.

---

## 2. Taxonomy — three tiers of Dobject intelligence

| Tier | Geometry comes from | Example | Authoring |
|---|---|---|---|
| **Dumb** | stored directly | Line, Arc, Polyline | drawn |
| **Semi-smart** | template + **declarative rules** replayed over parameters | Door, Window, bolt, title-block | drawn once + rules attached (data, not code) |
| **Smart** | **procedural solver** in Rust | Wall (junction solver) | code (`cad_wall`) |

Shared principle (from `Smart_Dobjects.md`): **identity + parameters are the
permanent stored truth; visible geometry is DERIVED and transient.** Editing a
parameter re-derives; the identity never degrades. For a wall the identity is
the centerline + style; for a semi-smart block it is
*(block name, insertion, rotation, parameter values)*.

The difference is only *how* derivation happens: walls need real geometry
solving (miters, T-junctions) → code. A door needs "move these vertices by
(T − T₀)" → data. Rules-as-data means users can eventually author new
semi-smart blocks **in-app, without writing Rust** (§9, BLK-5).

---

## 3. Design options considered

- **A. Full 2D constraint solver** (Revit-family style: dimensions +
  coincident/parallel constraints, iterative solve). Most general, but a
  constraint solver is a project of its own, failure modes are non-local
  (over/under-constrained), and the door/window class of problem doesn't
  need it. **Rejected for now** (revisit if sketch-constraints ever become a
  feature in their own right).
- **B. Procedural Rust per block type** (a `derive_door()` like the wall
  solver). Fastest to ship, but every new symbol needs a Rust release —
  the library can never grow in user-space. **Rejected as the general
  mechanism** (kept as an escape hatch, §5.4).
- **C. Template + parameters + declarative rules** — AutoCAD *dynamic
  blocks* model (linear parameter + stretch action). Template geometry is
  drawn normally; rules describe how tagged sub-geometry responds when a
  parameter changes. Covers door/window/furniture/fixtures; user-authorable;
  testable headless. **CHOSEN.**

Option C deliberately reuses an existing proven machine: our `stretch`
command (Slice L) already moves *only the vertices inside a crossing window*
by a delta. A `Rule::Stretch` is exactly that operation, **recorded and
replayed parametrically**. The rule engine is "edit operations as data".

---

## 4. Data model (kernel: `cad_kernel/src/block.rs`)

Classic blocks first — params/rules are a strict superset. A `BlockDef` with
empty `params`/`rules` IS a classic static block; one code path serves both.

```rust
pub struct BlockDef {
    pub name: String,
    pub base_point: Vec2,
    pub dobjects: Vec<DObject>,     // template, block-local coords, block-local HANDLES
    pub params: Vec<ParamDef>,      // empty → classic static block
    pub rules: Vec<Rule>,           // empty → classic static block
    pub description: String,
}

pub struct BlockTable { /* defs: Vec<BlockDef>, id-addressed like DimStyleTable */ }

pub struct ParamDef {
    pub name: String,               // "W", "T", "Swing", "Flip"
    pub label: String,              // UI: "Door width"
    pub kind: ParamKind,
    pub binding: Option<HostBinding>, // §6 — e.g. WallThickness
}

pub enum ParamKind {
    Length  { default: f64, min: f64, max: f64 },
    Angle   { default: f64 },
    Count   { default: u32, min: u32, max: u32 },
    Choice  { options: Vec<String>, default: usize },
    Flag    { default: bool },
    Derived { expr: String },       // "W - 2*15 - 5" — computed, not user-set (§5.3)
}

/// Which template dobjects a rule touches — BY BLOCK-LOCAL HANDLE.
/// (Handle-first identity again — the recurring keystone.)
pub struct RuleTargets { pub handles: Vec<Handle> }

pub enum Rule {
    /// Move only the grip points inside `zone` (block-local AABB) of the
    /// targets, by (value(param) - default(param)) along `dir`.
    /// == the existing `stretch` command, parameterized.
    Stretch    { param: String, zone: Aabb, dir: Vec2, targets: RuleTargets },
    /// Translate whole targets by (value - default) * dir.
    Move       { param: String, dir: Vec2, targets: RuleTargets },
    /// Rotate whole targets about `pivot` by (value - default) (Angle param).
    Rotate     { param: String, pivot: Vec2, targets: RuleTargets },
    /// Set circle/arc radius (or line length) to value.
    SetRadius  { param: String, targets: RuleTargets },
    /// Mirror targets about an axis through base_point when Flag/Choice matches.
    Mirror     { param: String, axis: Vec2, targets: RuleTargets },
    /// Show targets only when param matches (Choice/Flag) — swing-open vs closed, handing variants.
    Visibility { param: String, on: ParamValue, targets: RuleTargets },
    /// Repeat targets `count` times stepped by `step` (Count param) — e.g. mullions.
    Array      { param: String, step: Vec2, targets: RuleTargets },
}
```

The instance:

```rust
pub struct BlockRef {
    pub def: u32,                          // BlockTable id
    pub insert: Vec2,
    pub rotation: f64,
    pub scale: Vec2,                       // classic uniform/non-uniform, still allowed
    pub values: Vec<(String, ParamValue)>, // instance overrides; absent → ParamDef default
    pub host: Option<Handle>,              // §6 — the wall this ref is anchored to
    pub host_t: f64,                       // param position along host centerline
}
// Geom gains: Geom::BlockRef(BlockRef)  → match arms across the kernel (the usual tax)
// Document gains: pub blocks: BlockTable  (the commented-out line in document.rs comes alive)
```

---

## 5. Derive pipeline (feature crate: `cad_block`)

Per `ARCHITECTURE.md`: data above lives in `cad_kernel`; the UI-free
algorithm lives in a new feature crate **`cad_block`** (same pattern
`cad_wall` proved).

```text
derive(def: &BlockDef, values: &ResolvedParams) -> Vec<DObject>
  1. resolve params: defaults ← instance values ← host bindings ← Derived exprs
  2. clone template dobjects
  3. apply rules IN DECLARATION ORDER (each mutates its targets)
  4. return block-local result
world(ref) = transform(derive(...), insert/rotation/scale)
```

### 5.1 Determinism & ordering
Rules apply in the order stored. Order matters (stretch then mirror ≠ mirror
then stretch); the authoring UI presents rules as an ordered list. No solver,
no iteration → derivation is O(rules × targets), totally predictable, and a
bad rule set degrades visibly, not mysteriously.

### 5.2 Caching (the millions-of-Dobjects north star)
`derive` is pure → cache key = (def id, def version counter, resolved-params
hash). 500 identical doors = 1 derivation + 500 cheap transforms. The cache
lives app-side (like wall faces today, computed per frame); a `version: u32`
bump on `BlockDef` edit invalidates.

### 5.3 Derived params — tiny expression language
`Derived { expr }` needs only: numbers, param names, `+ - * /`, parens.
~100 lines of recursive-descent in `cad_block`, no dependency. This is what
encodes *leaf_length = W − 2·frame − clearance* without hardcoding doors.
**Explicitly NOT a scripting language** — no conditionals, no loops (v1).

### 5.4 Escape hatch
`Rule::Builtin { name: String }` → registry of named Rust post-passes for the
rare geometry no declarative rule covers. Built-ins ship with the binary;
user-authored blocks simply can't use them. Keeps the door open without
turning rules into code.

---

## 6. Host coupling — the wall relationship

Two separable mechanisms:

**6.1 Binding (parameter ← host query).**
`ParamDef.binding = Some(HostBinding::WallThickness)` → on insert and on every
re-derive, the param value is read from the host wall (style-resolved
thickness). User never types T; editing the wall (70→100) re-derives every
door on it automatically. Other bindings later: `WallStyleField(name)`,
`HostLength`, …

**6.2 Anchor (position + opening).**
A hosted ref stores `(host: Handle, host_t: f64)` — *position along the
centerline*, not a frozen world point. Placement: world pos = centerline
point at `t`, rotation = wall direction. Dragging the door slides `t`.

The **opening**: the door publishes its span `[t − W/2, t + W/2]` on the host
centerline; `cad_wall::solve_faces` subtracts published spans from the faces
and poché. Dependency direction stays clean — **walls never know about
doors**; the app collects `(wall_handle, span)` pairs from hosted BlockRefs
each frame and passes them into the wall solver as plain data. This lands the
"openings" item already on the `cad_wall` backlog with no new coupling.

Host deleted → ref keeps last derived geometry, flagged "orphaned" (Dobject
Info shows it; re-host by dragging onto another wall).

---

## 7. Worked example — DOOR

Template drawn at defaults **W=900, T=100**, base point = hinge-side wall face,
frame profile **15** (a plain number in the template — constants stay drawn):

| Template part | block-local handles | rule(s) |
|---|---|---|
| Hinge jamb 15×T rect | h1 | `Stretch{param:"T", zone: far edge, dir:(0,1)}` — depth follows wall, profile width 15 untouched (zone captures only the far-edge vertices) |
| Latch jamb 15×T rect | h2 | same T-stretch **+** `Stretch{param:"W", zone: whole jamb, dir:(1,0)}` — rides out with width |
| Leaf (rect/line) | h3 | length = `Derived leaf = "W - 2*15 - 5"` via `SetRadius`/stretch; `Rotate{param:"Swing"}` |
| Swing arc | h4 | `SetRadius{param:"leaf"}` + W-stretch moves its center? No — arc center sits on hinge jamb (outside W-zone), only radius changes |
| Handing | h1–h4 | `Mirror{param:"FlipX", axis:(0,1)}`, `Mirror{param:"FlipY", axis:(1,0)}` |

Params: `W` Length (bindable to nothing — user-set), `T` Length
**bound to WallThickness**, `Swing` Angle (default 90), `FlipX/FlipY` Flags,
`leaf` Derived.

Acceptance test (headless, in `cad_block`): derive at T=70/100/150 → jamb
profile width is exactly 15 in all three; derive at W=800/900 → swing radius
tracks leaf length; flip twice = identity.

**Window** is the same shape of problem (W stretch along wall, T binding,
mullions = `Array`) — member #2, zero new engine work.

---

## 8. Editing UX

- **Insert**: pick block → hover a wall → ghost auto-orients + auto-T live →
  click sets `t` → type W or accept default. (Reuses the dim-style ghost
  pattern.)
- **Edit**: double-click → parameter dialog (generated from `ParamDef`s —
  Length=DragValue, Choice=combo, Flag=checkbox; same dialog family as
  WallStyle/DimStyle). Grips: insertion grip + one grip per Stretch/Rotate
  rule (drag W-arrow, drag swing) + flip widgets.
- **Author** (BLK-5, last): in-app Block Editor — draw template, define
  params in a table, paint Stretch zones exactly like a crossing window
  (machinery exists), assign targets by clicking dobjects, reorder rules.

---

## 9. IO

- **RSM (native, lossless — non-negotiable)**: BlockTable round-trips
  defs + params + rules + per-ref values + host handle. Handles make this
  possible — DXF can't, RSM can. *(Note: this makes the outstanding
  "style tables don't round-trip" debt worse if unpaid — pay it in BLK-1.)*
- **DXF (interop, lossy by policy)**: export each distinct
  (def, resolved-params) as an anonymous static block (`*U…`) with INSERTs —
  drawings look right everywhere. Optionally stash params in XDATA so
  RUST_CAD can re-hydrate its own DXF. Import: plain INSERT → classic
  static BlockDef.

---

## 10. Phasing

| Slice | Scope | Proves |
|---|---|---|
| **BLK-1** | Classic blocks: `BlockTable`, `BlockDef` (no params), `Geom::BlockRef`, make/insert/explode commands, Blocks panel, RSM + DXF INSERT round-trip. **Includes the overdue RSM style-table round-trip.** | The architectural step everything waits on |
| **BLK-2** | Param + rule engine in `cad_block`, expression eval, derive cache, built-in DOOR (authored as Rust *data* — template + rules constants, NOT procedural code), headless acceptance tests | The semi-smart thesis |
| **BLK-3** | Host binding + anchor + wall-opening integration with `solve_faces` | The wall↔door story (user's actual ask) |
| **BLK-4** | Param dialog, grips, flip/visibility states, WINDOW built-in | Editing feel; engine generality |
| **BLK-5** | In-app Block Editor (zone painting, rule list, save to library) | User-space authoring endgame |

Each slice compiles green and is independently shippable; BLK-1 has value
even if the semi-smart tiers never ship.

---

## 11. Open questions (for the user)

1. **Expression scope** — is the §5.3 arithmetic-only language enough for v1,
   or do you want `min/max(a,b)` from day one (door clearances often clamp)?
2. **Built-in library form** — DOOR/WINDOW as Rust data constants (proposed),
   or as `.rsm` library files loaded at startup (more user-like, but needs
   BLK-5's serialization earlier)?
3. **Anchor storage** — `host`/`host_t` on `BlockRef` (proposed, simple) vs a
   Document-level relation table (more general, heavier). Relation table
   becomes right if anchors ever go beyond block-on-wall.
4. **DXF policy sign-off** — anonymous-block export + XDATA re-hydration
   acceptable as the permanent policy?
5. **Category name** — "semi-smart Dobjects" in docs/UI, or "parametric
   blocks"? (SYSVAR names come later either way.)
