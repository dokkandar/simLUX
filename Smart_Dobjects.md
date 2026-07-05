# Smart Dobjects — category spec

**Status:** design (do NOT implement until asked). Started 2026-06-10.
**Module:** the wall implementation lives in its own module (`wall.rs`); see
[the wall plan in Map_HSI_LibreCAD.md].

---

## 1. What a "smart dobject" is

A smart dobject stores a **compact parametric definition** and **derives** its
visible geometry from it. Three parts:

| Part | Meaning | Wall example |
|---|---|---|
| **generator** (identity) | an internal "imaginary" line/path — the spine | the **centerline** `start → end` |
| **params** | the parametric knobs | `thickness`, `justification` |
| **derived geometry** | recomputed render primitives — NEVER the source of truth | the two (mitred) face lines |

### Rules
1. **The generator (centerline) is permanent identity.** It is NOT a
   construction guide and is never deleted independently. Deleting the smart
   dobject deletes its spine; the spine cannot be orphaned.
2. **Derive, don't store.** Visible geometry (wall faces, dim arrows, hatch
   fill) is recomputed from generator + params. Editing a param re-derives.
3. **Parametric edits operate on the generator.** e.g. "change wall width A→B"
   = set `thickness = B`, keep the centerline, re-derive faces, re-solve joins.
   No geometry surgery, because the spine survives.
4. **Joins are computed on the derived geometry** (Model A — independent
   segments + per-frame coincidence-recomputed joins), so they update
   automatically when a width changes or a centerline endpoint moves.

### Likely API shape
A `SmartDObject` trait: `generator() -> Path`, `params()`, 
`derive(&Document) -> Vec<RenderPrim>`, plus parametric setters
(`set_width`, …). Today's `Geom::Wall { start, end, thickness }` already has
this shape (stores centerline implicitly, derives `left_line()/right_line()`),
so this formalizes an existing pattern. Dimensions and hatches are already
"smart" in the same spirit and are natural category members.

### Consequences to wire
- **Snapping must see the centerline.** Today snap only hits face endpoints
  (`cad_kernel/src/snap.rs`). Add centerline endpoints + the centerline itself
  ("the imaginary line") so a second wall can be drawn/joined onto the spine.
- **Centerline is selectable/grippable but not independently deletable.**

---

## 2. Junction algorithm — extracted from user demos

The user demonstrates a scenario by hand (offset + fillet@0 + erase) and we
extract the rule. The hand-demo shows the **visual result**; storage stays the
smart-dobject model (centerlines kept).

### Scenario #1 — L corner, SHARP miter  ✅ IMPLEMENTED (cad_app/src/wall.rs)
*Source: session dump 2026-06-10 (offset #6/#7 → faces #8–#11, fillet r0 the
adjacent face pairs, erase centerlines).*

```
Input: centerlines C1, C2 sharing corner node N; thickness t (each).
1. OFFSET each centerline by ±t/2            → 4 face lines.
2. Pair faces by side of the corner:
       inner pair  = (C1 inner face, C2 inner face)
       outer pair  = (C1 outer face, C2 outer face)
3. For each pair: FILLET radius 0 = extend/trim BOTH faces to their
   intersection point (= the miter).
4. Centerlines KEPT as identity (demo erased them only to show the result).
Output: 4 face segments mitred into a clean L.
```

**Built as** `wall::solve_faces(this, all_walls)` — Model A, per-frame from
endpoint coincidence (`JOIN_TOL`). Miter rule (symmetric, order-independent),
relative to each wall's OUTGOING dir at the node:
`inner = this.leftOut ∩ neighbour.rightOut`,
`outer = this.rightOut ∩ neighbour.leftOut`; move each face's node-side
endpoint to its miter. The render Wall arm calls it; falls back to raw ±t/2
faces when there's no neighbour. Tests: 90° miter, **any-angle meet-at-a-point
(covers the non-90 case)**, lone-wall-unchanged. Centerline kept (identity).
*Still single-segment faces only — N-way nodes pick the first neighbour.*

### Scenario #1b — L corner, ROUNDED (radius)  *(captured; not yet built)*
*Source: session dump 2026-06-10 — `fillet` the two CENTERLINES with radius
R (=50) FIRST, then OFFSET line+arc+line by ±t/2; erase centerlines.*

```
Input: centerlines C1, C2 at node N; thickness t; corner radius R.
1. FILLET the two CENTERLINES with radius R → centerline = C1' + arc(R) + C2'
   (tangent arc on the inside of the corner).
2. OFFSET the whole centerline path by ±t/2:
       outer face = offset lines + arc(R + t/2)
       inner face = offset lines + arc(R − t/2)   (R must exceed t/2)
3. Centerlines kept as identity.
Output: rounded corner; faces are tangent line-arc-line.
```

**Confirms the unifying model:** a wall = `offset(centerline, ±t/2)`. The
corner radius lives on the **centerline/spine** (the identity). Sharp = sharp
centerline vertex → mitred faces (#1); rounded = filleted centerline vertex →
concentric-arc faces (#1b). Build plan: reuse `cad_kernel::fillet_lines` for
the centerline arc, then offset; attribute the shared corner arcs to one wall
of the pair to avoid double-draw. Radius source: a SYSVAR (`WlCorR`) to start,
per-corner override later.

### Scenario #2 — T junction (90° or any angle)  *(captured; not yet built)*
*Source: session dump 2026-06-10 — offset both centerlines to faces, then
`trim` (NOT fillet): branch faces trimmed to the through face; the through
near-face split/opened where the branch enters.*

```
Input: through-wall A; branch-wall B whose END lands on A's centerline
       (T-node, any angle); thicknesses tA, tB.
1. OFFSET both centerlines ±t/2 → faces.
2. TRIM B's two faces to A's NEAR face (the side B approaches).
3. OPEN A's near face: split it at B's two faces and remove the segment
   between them (so A and B interiors connect).
4. A's FAR face stays continuous; centerlines kept.
Output: branch meets through cleanly with an opening into it.
```

**Rule differs from L:** not a miter — it's trim-branch + open-through.
Detection: a wall END coincides with a point in the INTERIOR of another wall's
centerline (vs L where two ENDS coincide). Build: needs point-on-segment test
+ split the through near-face.

### Scenario #1r — REACH + corner (two walls with a GAP)  ✅ SHARP done
*Source: user tried `fillet` (r=50) on two non-touching walls; it no-op'd
because `apply_fillet` only accepted Line+Line.*

The fix: **fillet now operates on the wall CENTERLINE.** A wall corner IS a
fillet of its spine.
```
Pick two walls (fillet), radius r:
  r = 0 (SHARP, built): intersect the two centerlines as INFINITE lines → I;
        fillet_lines extends/trims each centerline so its picked side reaches
        I; write the new centerlines back into each Wall.start/end (re-wrapped
        as Walls, identity preserved). Now the ends coincide → solve_faces
        mitres the faces. "Reach" = the extend half of fillet.
  r > 0 (ROUNDED): rejected for now — needs an arc in the centerline spine
        (a single straight Wall can't carry it) = scenario 1b.
```
Built in `apply_fillet` (cad_app/src/app.rs): `centerline()` extracts a Line +
optional thickness from Line/Wall; trimmed centerlines are re-wrapped via a
`rebuild` closure so walls stay smart dobjects. Honors `TrmMd` (reach needs
trim on). Works wall+wall and wall+line.

### Scenario #3 — X crossing  *(TODO — awaiting demo)*
### Scenario #4 — collinear continuation  *(TODO — awaiting demo)*

---

## 2.5 Wall as a full smart dobject — styles, dialog, fill, convert
*(roadmap, user 2026-06-10; do NOT build until asked)*

Goal: walls carry a **style** (a "type": Dry Wall, Structural, …) the same way
`Dim` carries a `DimStyle`. Mirror that existing machinery rather than invent.

**Data model (mirror DimStyle):**
- `WallStyle { name, thickness, justification, fill: (HatchPattern, color),
  face_color, face_lineweight, … }`.
- `WallStyleTable` on `Document`, STANDARD at id 0 (analog of `DimStyleTable`).
- `Wall.style: u32` (like `Dim.style`). Thickness/fill derive from the style
  (optional per-wall override). Editing a style re-derives all walls of that
  type — the smart-dobject payoff.

**Wall Style Manager dialog:** adapt the **DimStyle Manager** (already built in
`cad_app/src/app.rs`) — styles list + live preview (sample wall + corner with
its fill) + New / Modify / Set Current; colors via the shared ACI wheel
(`AciPickRequest`). The `wall` command opens/uses it; "current wall style"
drives new walls. Styles read like Hatch attribute choices.

**Hatch fill inside (poché):** fill the wall **footprint polygon** with the
style's `HatchPattern` (structural = concrete, drywall = light). Reuses the
hatch fill renderer. **Prereq:** `wall::solve_faces` must also emit a CLOSED
footprint polygon (left face + end caps + right face), not just two segments —
this same polygon later enables room detection.

**Convert closed dobject → wall:** pick a closed Polyline/Circle/rect → use its
path as the centerline → offset ±t/2 (reuse kernel polyline offset) → walls.
Turns a room outline into walls in one step.

**Reuse map:** DimStyle/DimStyleTable → WallStyle/WallStyleTable; Dim.style →
Wall.style; DimStyle Manager dialog → Wall Style dialog; hatch fill + HatchPattern
→ poché; ACI wheel → colors; polyline offset → convert-to-wall.

## 2.6 Opening styles — doors, windows, niches
*(plan, user 2026-06-11; do NOT build until asked)*

Openings are smart **sub-dobjects that ride on a wall**: they don't own
free geometry — they own a position on a host wall's centerline and derive
their cut from the wall's thickness. Third member of the style-table family
(after DimStyle and WallStyle); surfaced in the **Styles menu** (placeholder
entry already present, disabled).

**Data model (mirrors the established pattern):**
- `Opening { wall: handle, t: f64 /* 0..1 along centerline */, width: f64,
  style: u32, flip: bool }` — `t` parameterizes along the host centerline so
  the opening *rides the wall* when the wall moves/stretches (smart-dobject
  identity rule: the anchor is parametric, not absolute coords).
- `OpeningStyle { name, kind: Door | Window | Niche, width_default, depth,
  sill_height, swing: Option<…>, fill/face colors, description }`
- `OpeningStyleTable` on `Document`, STANDARD presets ("Door 0.9",
  "Window 1.2", "Niche 0.6"); `wall_styles`-style Manager dialog (third
  clone of the DimStyle Manager).

**Kind semantics (the user's "niche" point):**
- **Door / Window** — cut the FULL thickness: both faces open between the
  opening's two ends (window additionally draws sill lines, door a swing arc).
- **Niche** — `depth < thickness`: only the NEAR face opens; a recess
  polyline (3 segments at `depth` into the wall) replaces it. The far face
  stays continuous. This falls out of the same face-splitting primitive the
  T-junction needs ("open a face between two stations") — build T first,
  reuse the splitter here.

**Derive pipeline:** wall face derivation (cad_wall) gains a post-pass:
collect the host wall's openings, sort by `t`, split each affected face at
the opening stations, drop/replace the gap segments per kind. Pure logic →
lives in `cad_wall`; data type + Geom variant (or wall-attached list) →
cad_kernel; dialog → cad_app (per ARCHITECTURE.md).

**Storage decision to make at build time:** `Geom::Opening` as its own
dobject referencing the wall by handle (selectable/erasable on its own —
preferred) vs `Vec<Opening>` inside `Wall` (simpler, but invisible to
selection). Preferred = own dobject; needs handle lookup, which the
handle-first Document API work also wants → schedule together.

**Order:** T-junction (#2) → face-splitting primitive exists → openings
slice 1 (Door, full cut, no swing) → niche → window/sill → swing/flip UI.

## 3. Open questions
- **Auto-join vs explicit:** join automatically when two wall ends coincide
  (within tol), or require an explicit pick like the demo's fillet clicks?
  (Leaning auto, since coincidence is the trigger in Model A.)
- Justification default (center) and how it interacts with joins.
- Unequal-thickness joins (T: through wall wins).
