#!/usr/bin/env python3
"""
build_audit.py — generate Audit.html, a third-party-style parity audit
comparing RUST_CAD against LibreCAD as the established baseline.

The audit is intentionally OBJECTIVE — it's the project's compass. It
should be honest about what's missing, not promotional.

Categories audited:
  1. Entity types (Dobjects)
  2. Object snap kinds
  3. Drafting / draw tools
  4. Edit / modify tools
  5. Selection methods
  6. Dock panels
  7. File formats
  8. Print / plot
  9. Settings / SYSVARS

Run from workspace root:
  python3 build_audit.py
"""

import html
import datetime
import subprocess
from pathlib import Path

ROOT = Path(__file__).parent

# ---------- AUDIT DATA -----------------------------------------------------
#
# Status buckets per RUST_CAD cell:
#   "done"      — fully implemented, parity reached
#   "partial"   — implemented for a subset; gaps noted
#   "planned"   — explicitly on the roadmap with a slice id
#   "deferred"  — explicitly NOT planned in near term
#   "missing"   — not present, not in plan, not flagged
#
# LC = LibreCAD baseline, RC = RUST_CAD

CATEGORIES = [
    {
        "id": "entities",
        "title": "1. Entity types (Dobjects)",
        "lc_intro": (
            "19 core entity classes + 9 dimension variants. All inherit from "
            "RS_Entity; container-based composition for polylines and dimensions."
        ),
        "rc_intro": (
            "7 entity types modelled as Geom enum variants inside the DObject "
            "wrapper struct (geom + style + handle). Adding a new type is a "
            "new variant — Style infrastructure is shared."
        ),
        "rows": [
            ["RS_Line",          "● Line",          "done",     "Geom::Line { a, b } — 2D today"],
            ["RS_Circle",        "● Circle",        "done",     "Geom::Circle { center, radius }"],
            ["RS_Arc",           "● Arc",           "done",     "Geom::Arc { center, radius, start_angle, sweep_angle }"],
            ["RS_Ellipse (full)","● Ellipse",       "done",     "Geom::Ellipse { center, major, ratio }"],
            ["RS_Ellipse (arc)", "● EllipseArc",    "done",     "Split into separate variant for tighter algos"],
            ["RS_Point",         "● Point",         "done",     "Geom::Point { location, style, size } — Slice E"],
            ["RS_Polyline (LW)", "● Polyline",      "partial",  "Geom::Polyline — straight segments today; bulge field stored but not rendered as arc segments yet"],
            ["RS_Spline",        "DobjectSpline",   "planned",  "Tier 2 in Implementation_Plan.md (#13) — Phase 9; NURBS covers parabola / hyperbola too"],
            ["RS_Text",          "Text (single)",   "planned",  "Slice E.x — needs TextStyleTable on Document first"],
            ["RS_MText",         "MText",           "planned",  "Slice E.x — needs TextStyleTable"],
            ["RS_Hatch",         "Hatch",           "planned",  "Slice E.x — needs pattern lib + boundary-path representation"],
            ["RS_Insert (block)","BlockRef",        "planned",  "Slice F — needs BlockTable on Document"],
            ["RS_Block (def)",   "BlockTable entry","planned",  "Slice F"],
            ["RS_Image",         "DobjectRasterImage",  "planned", "Tier 3 in Implementation_Plan.md (#21)"],
            ["RS_Solid (2D)",    "DobjectSolid2D",      "planned", "Tier 3 in Implementation_Plan.md (#24)"],
            ["RS_Leader",        "DobjectLeader",       "planned", "Tier 2 in Implementation_Plan.md (#20)"],
            ["LC_MLeader",       "DobjectMLeader",      "planned", "Tier 2 in Implementation_Plan.md (#20)"],
            ["LC_Tolerance",     "DobjectTolerance",    "deferred","Extreme niche (GD&T); revisit on explicit demand only"],
            ["RS_ConstructionLine", "DobjectRay / DobjectXline", "planned", "Tier 3 in Implementation_Plan.md (#22, #23) — Phase 3 draw tools"],
            ["LC_Hyperbola, LC_Parabola, LC_Rect", "—", "deferred", "Explicitly OUT OF SCOPE — DobjectSpline (Tier 2 #13) covers smooth curves; Rectangle is a Polyline preset"],
            ["RS_DimLinear",     "DimRotated",      "planned",  "Slice E.x — needs DimStyleTable on Document"],
            ["RS_DimAligned",    "DimAligned",      "planned",  "Slice E.x"],
            ["RS_DimAngular",    "DimAngular",      "planned",  "Slice E.x"],
            ["RS_DimRadial",     "DimRadial",       "planned",  "Slice E.x"],
            ["RS_DimDiametric",  "DimDiameter",     "planned",  "Slice E.x"],
            ["LC_DimArc",        "DimArc",          "planned",  "Slice E.x"],
            ["LC_DimOrdinate",   "DimOrdinate",     "planned",  "Slice E.x"],
        ],
    },
    {
        "id": "snaps",
        "title": "2. Object snap kinds",
        "lc_intro": (
            "10 modes (9 standard + 1 HSI-fork PER extension) as RS_SnapMode "
            "bool struct. Restrictions (Horizontal / Vertical / Orthogonal) "
            "applied as modifiers."
        ),
        "rc_intro": (
            "8 kinds in cad_kernel::snap::SnapKind enum, with multi-candidate "
            "Tab cycling, mouse-priority vs object-priority activation rules, "
            "ellipse PER/TAN via multi-seed Newton (8 seeds, 30 iters)."
        ),
        "rows": [
            ["SnapEndpoint",       "● END", "done",  "Endpoints of line / arc / ellipse arc; polyline vertices"],
            ["SnapMiddle",         "● MID", "done",  "Midpoint of line / angular midpoint of arc / parametric midpoint of ellipse arc; per-segment midpoint of polyline"],
            ["SnapCenter",         "● CEN", "done",  "Centre of circle / arc / ellipse / ellipse arc"],
            ["—",                  "● QUA", "done",  "Quadrants of circles & arcs, axis-end points of ellipses — RUST_CAD distinction LibreCAD lacks"],
            ["SnapIntersection",   "● INT", "done",  "Pairwise across all Geom-pair combinations (10 pair types)"],
            ["—",                  "● PER (HSI ext)", "done", "Perpendicular foot from anchor; both real and imaginary-extension feet emitted with anchor for dashed cue"],
            ["—",                  "● TAN", "done",  "Tangent points from external anchor; supports ellipses via Newton"],
            ["SnapOnEntity",       "● NEA", "done",  "Nearest point on the visible curve"],
            ["SnapDistance",       "Dist",  "missing", "Step along an entity by a fixed distance"],
            ["SnapGrid",           "Grid",  "missing", "Snap to grid intersections (separate from grid display)"],
            ["SnapFree",           "Free",  "missing", "Explicit no-snap mode (RUST_CAD: just disable all kinds)"],
            ["SnapAngle",          "Angle", "missing", "Polar tracking — `PolMod` SYSVAR catalogued in Variables.md, not wired"],
            ["RestrictHorizontal/Vertical/Ortho", "Ortho restriction", "missing", "Modifier on top of snaps; not implemented"],
            ["Extension (apparent intersection)", "AppInt", "missing", "DXF/AutoCAD-style apparent intersection across extended entities"],
            ["Parallel snap",      "PAR", "missing", "Snap parallel to another line"],
        ],
    },
    {
        "id": "draw",
        "title": "3. Drafting / draw tools",
        "lc_intro": (
            "~106 separate Action subclasses across line / circle / arc / "
            "ellipse / polyline / text / hatch / dim families. Each construction "
            "method is its own action."
        ),
        "rc_intro": (
            "8 tools wired to the toolbar: Line, Circle, Arc (5 methods), "
            "Ellipse, EllipseArc, Point, Polyline. Construction math for all "
            "5 arc methods (3-point, S-C-E, C-S-E, chord+radius, chord+length) "
            "is exposed via the command line."
        ),
        "rows": [
            ["Line 2-point",                  "● Line tool",          "done", "Click two points"],
            ["Polyline multi-segment",        "● Polyline tool",      "done", "Click N points, Enter to finish, `close` to close"],
            ["Angle/orthogonal/parallel/perpendicular/tangent/bisector/freehand line", "—", "missing", "8 LibreCAD variants — none implemented"],
            ["Circle center-radius (click)",  "● Circle tool",        "done", "Click centre then radius point"],
            ["Circle 2P / 3P / TTR / TTT / inscribed", "—",           "missing", "LibreCAD has 6 circle variants; RUST_CAD has 1"],
            ["Arc 3-point",                   "● Arc tool method",    "done", "ArcMethod::ThreePoints"],
            ["Arc start-center-end",          "● Arc tool method",    "done", "ArcMethod::StartCenterEnd"],
            ["Arc center-start-end",          "● Arc tool method",    "done", "ArcMethod::CenterStartEnd"],
            ["Arc chord+radius",              "◐ via cmd line",       "partial", "`arccr` command; not wired to toolbar"],
            ["Arc chord+arclength",           "◐ via cmd line",       "partial", "`arccl` command; not wired to toolbar"],
            ["Arc 2-point+angle / +height",   "—",                    "missing", "LibreCAD-only variants"],
            ["Ellipse",                       "● Ellipse tool",       "done", "centre + major-end + minor-side click flow"],
            ["Elliptical arc",                "● EllipseArc tool",    "done", "5-click flow (centre, major, minor, start, end)"],
            ["Rectangle",                     "Rect",                 "missing", "Trivial — could ship as polyline preset"],
            ["Polygon (regular N-gon)",       "Polygon",              "missing", "Not on roadmap"],
            ["Point",                         "● Point tool",         "done", "Slice E"],
            ["Spline",                        "Spline tool",          "deferred", "Geometry kernel deferred"],
            ["Text (single-line)",            "Text tool",            "planned", "Slice E.x — needs TextStyleTable"],
            ["MText (multi-line)",            "MText tool",           "planned", "Slice E.x"],
            ["Hatch",                         "Hatch tool",           "planned", "Slice E.x — pattern library required"],
            ["Image insert",                  "Image",                "missing", "Sketched only"],
            ["Block insert (Insert)",         "BlockRef",             "planned", "Slice F"],
            ["Dimensions — 18 LC draw actions", "—",                  "planned", "Slice E.x. Today 0 of the 9 dim variants implemented"],
            ["GD&T Feature Control Frame",    "Tolerance",            "missing", "Sketched only"],
            ["Leader / MLeader",              "—",                    "missing", "Sketched only"],
            ["LC Hyperbola / Parabola",       "—",                    "missing", "LibreCAD extensions, not on roadmap"],
        ],
    },
    {
        "id": "edit",
        "title": "4. Edit / modify tools",
        "lc_intro": (
            "~48 modify Action classes covering transform / trim / join / "
            "polyline-segment ops / explode / order / attribute-edit. Global "
            "undo/redo transaction system."
        ),
        "rc_intro": (
            "Edit loop substantially complete after Slices J / K / L / M.1–M.5. "
            "20+ editing commands wired, each running through a basket-based "
            "QueuedOp flow with snapshot undo + redo. EdgMod / FltRad / "
            "ChmDs1 / ChmDs2 SYSVARS persist the relevant defaults."
        ),
        "rows": [
            ["Move",            "● `move` / `m` cmd",        "done",     "Slice J — translate selection by (end - base)"],
            ["Copy",            "● `copy` / `cp` / `co` / `c` cmd", "done", "Slice J — leaves originals; appends translated copies"],
            ["Duplicate",       "≡ Copy with zero offset",  "partial",  "No dedicated cmd; achievable via copy with same start+end click"],
            ["Rotate",          "● `rotate` / `ro` cmd",    "done",     "Slice J — pivot + reference + target; Geom::rotated handles ellipse major-axis"],
            ["Scale",           "● `scale` / `sc` cmd",     "done",     "Slice J — uniform factor via pivot + reference + target distances"],
            ["Mirror",          "● `mirror` / `mi` cmd",    "done",     "Slice J — two clicks define the axis; preserves Arc CCW convention"],
            ["Stretch",         "● `stretch` cmd",          "done",     "Slice L — window/crossing-select vertices, drag delta translates them"],
            ["Offset (parallel)", "● `offset` / `o` cmd",   "done",     "Slice L — Line / Circle / Arc native; Ellipse / EllipseArc produce Polyline approximation; Polyline corner-aware"],
            ["Align (3 variants)", "● `align` cmd",         "done",     "Slice L — two ref points → two target points; computes translate+rotate+scale"],
            ["Lengthen / shorten", "● `lengthen` / `len` cmd", "done",  "Slice L — Line endpoint extend, Arc sweep extend, EllipseArc chord-scale approximation"],
            ["Trim",            "● `trim` / `tr` cmd",      "done",     "Slice M.1 — two-basket flow (cutters + targets), Geom::trim_at + extended_for_edgemode; closed Ellipse → N EllipseArcs; Polyline explode-then-trim"],
            ["Extend",          "● `extend` / `ex` cmd",    "done",     "Slice M.2 — symmetric counterpart, Geom::extend_to with EdgMod"],
            ["Cut / Break",     "● `break` / `br` cmd",     "done",     "Slice L — one-pick variant (two-point break supported)"],
            ["Divide",          "Divide",                   "missing",  "Equal-distance subdivision — not yet"],
            ["Fillet (round)",  "● `fillet` / `flt` / `f` cmd", "done", "Slice M.3 — line-line; FltRad SYSVAR persists; arbitrary-angle + right-angle + sharp (r=0) all verified"],
            ["Chamfer (bevel)", "● `chamfer` cmd",          "done",     "Slice M.4 — line-line; ChmDs1 / ChmDs2 SYSVARS; emits g1' + bridge line + g2'"],
            ["Join",            "● `join` cmd",             "done",     "Slice M.5 — collinear lines, concentric arcs, chain-to-polyline"],
            ["MatchProperties", "● `matchprop` / `mp` cmd", "done",     "Slice K — paint style from a clicked source dobject onto the basket"],
            ["Reverse direction", "● `reverse` cmd",        "done",     "Slice K — swap line endpoints, flip arc sweep"],
            ["Change layer",    "● `chlayer` cmd",          "done",     "Slice K — reassign basket to a named layer"],
            ["Polyline segment ops (add / append / delete / change type / offset)", "◐ partial", "partial", "Polyline trim and intersect landed; segment-level add/delete/append/type-flip still missing"],
            ["Delete",          "● `delete` / `erase` / `e` / `del N`", "done", "Slice J — bulk on selection + indexed single via cmd line"],
            ["Explode (blocks → entities)", "Explode",      "missing",  "Depends on Slice F (Blocks)"],
            ["Explode text",    "ExplodeText",              "missing",  "Depends on text + outline conversion"],
            ["Attributes (pen/layer edit)", "● Entity Info panel", "done", "Slice D — per-Dobject layer / color / linetype / lineweight editing"],
            ["Array (rect / polar)", "● Rect array",        "partial",  "Rect array on single source; polar array missing"],
            ["Order (Z-order)", "Order",                    "missing",  "Send-to-back / bring-to-front"],
            ["Undo",            "● `undo` / `u` cmd",       "done",     "Slice J — 64-deep snapshot stack"],
            ["Redo",            "● `redo` / `y` cmd",       "done",     "Slice K — symmetric redo_stack; any new edit clears it (branch-cut semantics)"],
            ["Grips (visual handles)", "● Grips v1 + v2",   "done",     "Per-role grip semantics (endpoint / midpoint / center / quad / vertex), click-to-grab, role-aware drag preview"],
        ],
    },
    {
        "id": "selection",
        "title": "5. Selection methods",
        "lc_intro": "10 selection actions including window, crossing, contour, intersected, lasso, by-layer.",
        "rc_intro": (
            "Selection backbone wired via SelectMode + QueuedOp. AutoCAD-style "
            "sub-commands during a selection session: window, crossing, single, "
            "previous (`before`), all, none, add/remove modes, Shift+click for "
            "remove."
        ),
        "rows": [
            ["Single entity click",   "● Single click",       "done", "click_select(i, shift)"],
            ["Window (inside) drag",  "● `inside` window",    "done", "L→R drag — only fully-enclosed dobjects"],
            ["Crossing window drag",  "● `crossing` window",  "done", "R→L drag — anything touching the rect"],
            ["All",                   "● `all` cmd",          "done", "AutoCAD-style"],
            ["Invert",                "Invert",               "missing", "Not yet"],
            ["Previous (`before`)",   "● `before` cmd",       "done", "Re-adds the last finalised selection"],
            ["None / deselect",       "● `none` cmd",         "done", "Clear selection"],
            ["Add / Remove mode toggles", "● `add` / `remove` cmds", "done", "AutoCAD-style during session"],
            ["By layer",              "By layer",             "missing", "Not yet; trivial follow-up since LayerTable exists"],
            ["By block",              "By block",             "missing", "Depends on Slice F"],
            ["Contour (trace edges)", "Contour",              "missing", "Walk connected entities"],
            ["Intersected (SAT)",     "Intersected",          "missing", "True SAT polygon test"],
            ["Lasso (freehand)",      "Lasso",                "missing", "Freehand outline"],
        ],
    },
    {
        "id": "panels",
        "title": "6. Dock panels (UI)",
        "lc_intro": (
            "13 core dock widgets + 10 status-bar widgets. QDockWidget pattern "
            "with custom view renderers (tree models for layers/blocks)."
        ),
        "rc_intro": (
            "3 dock panels (Layer, Pen palette, Entity Info) — first-pass "
            "implementations matching LibreCAD's core read/write capability. "
            "Status bar still minimal."
        ),
        "rows": [
            ["Layers (grid)",           "● Layer panel",     "done",    "Slice B — add/rename/delete, visibility/lock/freeze, active radio, color swatch"],
            ["Layers Tree (hierarchy)", "—",                 "missing", "Flat layer model only; hierarchical extension if needed later"],
            ["Pen Palette",             "● Pen palette",     "done",    "Slice C — 7 default presets, apply-to-selection"],
            ["Entity Info / Properties","● Entity Info",     "done",    "Slice D — single + multi-sel, editable layer/color/linetype/lineweight"],
            ["Pen Wizard",              "—",                 "missing", "Click-to-set-pen-by-existing-entity workflow"],
            ["Blocks",                  "—",                 "planned", "Slice F"],
            ["Library (DXF browser)",   "—",                 "planned", "Slice G"],
            ["Command Line dock",       "● command line",    "partial", "Always-listen text input wired; no separate dock widget — embedded in main area"],
            ["Named Views",             "—",                 "planned", "Slice G"],
            ["UCS List",                "—",                 "planned", "Slice G"],
            ["Workspaces / Layout manager", "—",             "missing", "Egui dock layout is built-in; LibreCAD-style workspace persistence not yet"],
            ["Debug Log (HSI fork)",    "◐ debug window",    "partial", "RUST_CAD has render-mode debug + index stats; no general log viewer"],
            ["Status bar — coords",     "● coord display",   "done",    "Bottom-bar widget"],
            ["Status bar — selection count", "◐",            "partial", "Shown in Entity Info, not as separate widget"],
            ["Status bar — active layer", "◐",               "partial", "Visible in Layer panel"],
            ["Status bar — grid / UCS / angles / rel-zero", "—", "missing", "All shown in LibreCAD's status bar"],
        ],
    },
    {
        "id": "files",
        "title": "7. File formats",
        "lc_intro": (
            "5 filters registered: DXF (libdxfrw, multi-version), legacy DXF, "
            "LFF, JWW, CXF. DWG via libdxfrw (marked unsafe). No 3D / STEP."
        ),
        "rc_intro": (
            "1 filter: DXF (cad_io::dxf). Round-trips currently supported "
            "entity types + LAYER + LTYPE tables. Files written by RUST_CAD "
            "open cleanly in LibreCAD. Native .rsm planned (Slice I)."
        ),
        "rows": [
            ["DXF read",  "● dxf::read_dxf",  "partial", "Slice H — Line/Circle/Arc/Ellipse/EllipseArc/Point/LWPolyline + LAYER + LTYPE. Untested: blocks, dim styles, xdata, complex hatches, text, splines, inserts."],
            ["DXF write", "● dxf::write_dxf", "partial", "Same surface as read; LibreCAD opens output cleanly"],
            ["DXF versions", "ASCII",         "partial", "Current writer emits ASCII; specific R-version tagging tentative — needs verification"],
            ["DWG",       "—",                "missing", "Would need a libdxfrw equivalent or pure-Rust DWG reader (none mature)"],
            ["LFF (LibreCAD native)", "—",    "missing", "Not on roadmap"],
            ["JWW",       "—",                "missing", "Niche"],
            ["CXF",       "—",                "missing", "Niche"],
            [".rsm (native binary)", "● rsm::read / write", "done", "Slice I — RUST_CAD-native binary format for fast load/save"],
            ["File dialog (rfd / native)", "—", "missing", "Today: `open <path>` / `save <path>` via cmd line only"],
            ["3D formats (STEP / IGES)", "—", "deferred", "Out of scope until RUST_CAD goes 3D"],
        ],
    },
    {
        "id": "plot",
        "title": "8. Print / plot",
        "lc_intro": (
            "Print preview, page setup, scale-to-fit, PDF export via "
            "LC_Printing, batch dxf2pdf CLI. Per-layer plot enable."
        ),
        "rc_intro": "Not started. Nothing on roadmap before Slice K+.",
        "rows": [
            ["Print preview",   "—", "missing", "No plot subsystem"],
            ["Page setup",      "—", "missing", "—"],
            ["Scale-to-fit",    "—", "missing", "—"],
            ["PDF export",      "—", "missing", "—"],
            ["Multi-page",      "—", "missing", "—"],
            ["Plot styles per layer", "—", "missing", "Layer.plottable bit exists in struct; no consumer"],
            ["Batch CLI converter", "—", "missing", "Future possibility once cad_io is solid"],
        ],
    },
    {
        "id": "settings",
        "title": "9. Settings / SYSVARS",
        "lc_intro": (
            "RS_Settings singleton (Qt QSettings) — hierarchical key-value store. "
            "Estimated 200+ distinct keys across grid, snap, colors, UI, units, "
            "text/dim defaults."
        ),
        "rc_intro": (
            "UserEnv struct in cad_app/src/settings.rs — cryptic AutoCAD-style "
            "short names (SpTGSZ, GrpEnb). 209 entries catalogued in "
            "Variables.md. Persisted to ~/.config/rust_cad/user_env.txt."
        ),
        "rows": [
            ["Total settings exposed",                "≈ 200+",            "—",       "LibreCAD"],
            ["Total catalogued",                      "209",               "—",       "Variables.md is the contract"],
            ["Actually wired (●)",                    "≈ 11",              "partial", "SpTGSZ, GrpEnb, GrClrU/S, GrpSz, plus EdgMod / FltRad / ChmDs1 / ChmDs2 (from Slice M) and pen-panel defaults. Most others still ◐ Planned."],
            ["Persistence",                           "● user_env.txt",    "done",    "Format: one KEY=VALUE per line, forward-compatible (unknown keys ignored)"],
            ["Settings UI",                           "● Settings window", "partial", "Lists all UserEnv fields with cryptic name + plain-English label + live preview pane on the right"],
            ["Grid (spacing / snap / iso)",           "—",                 "missing", "Grid renders; no SYSVAR-driven spacing yet"],
            ["Snap kinds (running osnaps)",           "● SnapSet panel",   "done",    "Floating window; toggles persist via UserEnv"],
            ["Color preferences (bg / grid / snap)",  "◐",                 "partial", "Bg / accent colours hardcoded; some catalogued (CrsACol, IntsCol, …)"],
            ["Toolbar / palette visibility",          "—",                 "missing", "Egui auto-handles dock state; not persisted"],
            ["Units (mm / inch / drawing unit)",      "—",                 "missing", "No unit system yet"],
            ["Angle display format (deg / rad / DMS)","—",                 "missing", "Not yet"],
            ["Text / dim style defaults",             "—",                 "planned", "Depends on Text/Dim slices"],
        ],
    },
]


# Architectural assessment data (what's good / what's a risk).
STRENGTHS = [
    ("Pure Rust, single binary",
     "No Qt baggage, no webview, no IPC. eframe + glow gives a GL context on "
     "the main thread. Cross-compiles cleanly; no system-package dependencies "
     "beyond the desktop GL stack."),

    ("Property foundation is structurally right",
     "DObject = geom + style + handle. Future entity types add a Geom variant; "
     "they get layer/color/linetype/lineweight/visibility for free. This is the "
     "key payoff of Slice A — it avoids per-variant retrofits down the line."),

    ("Geometry math is independently testable",
     "70 kernel tests + 15 cad_io tests + 2 doctests = 87 total, all passing. "
     "Snap helpers, intersection pairs, and ellipse PER/TAN/INT (multi-seed "
     "Newton) all under #[cfg(test)] coverage. cad_cli REPL lets a human pipe "
     "commands in and diff intersection output line-by-line."),

    ("Spatial index built in",
     "UniformGrid bucketing into all overlapping cells, auto-cell-size targeting "
     "~10 dobjects per cell. PAIR_LIMIT cap (5M) protects against runaway "
     "intersection sweeps. Lazy rebuild on edit."),

    ("DXF round-trip with LibreCAD compat verified",
     "Slice H emits DXF files that LibreCAD opens cleanly. Reader handles the "
     "current 7 entity types + LAYER + LTYPE tables. Foundation for serious "
     "interop is in."),

    ("Editing loop substantially complete (Slices J–M)",
     "20+ editing commands wired in 24 hours: Move / Copy / Rotate / Scale / "
     "Mirror / Delete / Undo / Redo (Slice K) / MatchProperties / Reverse / "
     "ChangeLayer / Offset / Lengthen / Break / Align / Stretch (Slice L) / "
     "Trim / Extend with EdgMod (Slice M.1–M.2) / Fillet / Chamfer / Join "
     "(Slice M.3–M.5). All run through the basket + QueuedOp pattern with "
     "snapshot undo+redo. Trim handles closed Ellipse → N EllipseArcs and "
     "Polyline explode-then-trim. Grips v1+v2 add visual editing handles with "
     "per-role drag semantics. EdgMod / FltRad / ChmDs1 / ChmDs2 SYSVARS "
     "persist the relevant defaults across sessions."),

    ("Full 256-color ACI palette",
     "color.rs::aci_palette now resolves all 256 indices via the canonical "
     "wheel (24 hues × 10 steps + grays at 250–255 + named at 0–9). DXF files "
     "using non-trivial palette indices now look right on import."),

    ("Native binary format alongside DXF",
     "Slice I added .rsm — a RUST_CAD-native binary format for fast load/save, "
     "complementing the DXF interop path. Two-track persistence: round-trip "
     "with the AutoCAD world via DXF, fast local saves via .rsm."),

    ("Reference documents are real contracts",
     "Variables.md (209 SYSVARS), Dobject_DXF.md (~310 group codes), "
     "Dobject_Properties.md (per-type fields). Three docs, plus ROADMAP.md, "
     "plus Plan_Report.html and this Audit.html. Every architectural decision "
     "has a documented home."),
]

RISKS = [
    ("No UI tests",
     "egui is hard to drive headlessly. CPU renderer ByLayer resolution, "
     "selection toggles, and panel edits are verified manually. Plan: introduce "
     "snapshot tests on canvas painter output OR integration tests on the "
     "underlying CadApp methods (without instantiating the egui Context)."),

    ("GPU renderer doesn't honour ByLayer yet",
     "CPU path resolves Color::ByLayer correctly. GPU path (CircleInstance "
     "color field is u32, but populated from sel/snap/default hardcoded values). "
     "Mixed-mode (CPU lines/arcs + GPU circles) shows the gap visibly when GPU "
     "is enabled."),

    ("Handle counter is process-global, not per-Document",
     "Won't round-trip cleanly when DXF files carry their own hex handles. "
     "Will need to either preserve source handles verbatim or remap on import. "
     "Decision pending — flagged in Plan_Report.html Open Questions."),

    ("Most catalogued SYSVARS are not wired",
     "209 entries in Variables.md — only ~7 actually drive behaviour. The "
     "Settings panel surfaces them all (so future wiring is one-line per row), "
     "but a user expecting AutoCAD parity today will hit a lot of inert "
     "toggles."),

    ("DXF I/O is one-pass and shallow",
     "Slice H proves the round-trip pipeline but doesn't handle xdata, blocks, "
     "complex hatches, dim styles, text styles, splines, inserts, or proxy "
     "graphics. Real-world DXF files routinely use all of these. Treat current "
     "cad_io as a proof-of-life; harden against AutoCAD-authored files before "
     "any 'open DXF' marketing claim."),

    ("Polyline bulge rendering",
     "PolyVertex.bulge field is stored but not interpreted as arc segments at "
     "render or snap time. Imported polylines with bulges will look like "
     "polylines of straight chords instead of arcs."),

    ("No layer-tree hierarchy",
     "LibreCAD has both flat and hierarchical layer panels. RUST_CAD's "
     "LayerTable is flat. If real drawings use grouped layers, this becomes a "
     "panel limitation, not a kernel one."),

    ("Print / plot not even sketched",
     "Roadmap places this after .rsm and editing ops. Engineers expecting a "
     "workable CAD will note its absence — this is a 'before any real adoption' "
     "category, not an optional one."),
]


# Score calculation: weighted across categories
SCORE_WEIGHTS = {
    "entities":  2.0,   # core to any drafting workflow
    "snaps":     1.5,   # foundational interactive accuracy
    "draw":      1.5,
    "edit":      2.0,   # huge gap right now
    "selection": 1.0,
    "panels":    1.0,
    "files":     1.5,   # interop is critical
    "plot":      0.8,
    "settings":  0.5,
}

def score_category(cat):
    """Return (done, partial, planned+deferred+missing, total)."""
    done = partial = pending = 0
    for row in cat["rows"]:
        status = row[2]
        if status == "done":
            done += 1
        elif status == "partial":
            partial += 1
        else:
            pending += 1
    total = done + partial + pending
    return done, partial, pending, total


def overall_score():
    """Weighted parity percentage. 'done' = 1.0, 'partial' = 0.4, else 0."""
    num = denom = 0.0
    by_cat = []
    for cat in CATEGORIES:
        w = SCORE_WEIGHTS.get(cat["id"], 1.0)
        d, p, _, t = score_category(cat)
        if t == 0:
            continue
        cat_score = (d * 1.0 + p * 0.4) / t
        by_cat.append((cat["id"], cat["title"], cat_score, d, p, t, w))
        num   += cat_score * w
        denom += w
    return (num / denom if denom else 0.0), by_cat


# ---------- HTML rendering --------------------------------------------------

CSS = """
:root {
  --bg:#0f1419; --bg-elev:#161b22; --bg-tbl:#11161d;
  --fg:#d4d4d8; --fg-dim:#8b949e; --accent:#79c0ff; --border:#30363d;
  --ok:#3fb950; --partial:#d29922; --planned:#8b949e;
  --deferred:#a371f7; --missing:#ff7b72;
  --score-bar:#21262d;
}
* { box-sizing: border-box; }
body {
  background: var(--bg); color: var(--fg);
  font: 14.5px/1.55 -apple-system, "Segoe UI", system-ui, sans-serif;
  margin: 0; padding: 0;
}
.layout { display: grid; grid-template-columns: 260px 1fr; min-height: 100vh; }
nav.toc {
  background: var(--bg-elev); border-right: 1px solid var(--border);
  padding: 24px 18px; position: sticky; top: 0; align-self: start;
  height: 100vh; overflow-y: auto;
}
nav.toc h2 { font-size: 11px; text-transform: uppercase; letter-spacing: 1.5px; color: var(--fg-dim); margin: 18px 0 8px; }
nav.toc h2:first-child { margin-top: 0; }
nav.toc a { display: block; color: var(--fg); text-decoration: none; padding: 4px 8px; border-radius: 4px; font-size: 13px; }
nav.toc a:hover { background: rgba(255,255,255,0.06); color: var(--accent); }
main { padding: 32px 48px 80px; max-width: 1200px; }

.cover {
  background: linear-gradient(180deg, rgba(255,123,114,0.10), transparent);
  border: 1px solid var(--border); border-radius: 10px;
  padding: 24px 32px; margin-bottom: 28px;
}
.cover h1 { margin: 0 0 6px; font-size: 26px; }
.cover .subtitle { color: var(--fg-dim); font-size: 13px; }
.cover p { margin-top: 12px; }

.score-block {
  display: grid; grid-template-columns: 1fr 320px; gap: 28px;
  margin: 28px 0;
}
.score-headline {
  background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px;
  padding: 22px 26px;
}
.score-big {
  font-size: 56px; font-weight: 700; color: var(--accent); line-height: 1;
  display: flex; align-items: baseline; gap: 8px;
}
.score-big span.label { font-size: 14px; color: var(--fg-dim); font-weight: 400; }
.score-byline { color: var(--fg-dim); font-size: 13px; margin-top: 8px; }

.score-bars { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px; padding: 18px 22px; }
.score-bars h4 { margin: 0 0 12px; font-size: 12px; text-transform: uppercase; letter-spacing: 1.5px; color: var(--fg-dim); }
.score-bar-row { display: grid; grid-template-columns: 100px 1fr 40px; gap: 8px; align-items: center; margin-bottom: 8px; font-size: 12.5px; }
.score-bar-name { color: var(--fg-dim); }
.score-bar-track { background: var(--score-bar); border-radius: 3px; height: 10px; overflow: hidden; }
.score-bar-fill { height: 100%; background: linear-gradient(90deg, var(--missing) 0%, var(--partial) 50%, var(--ok) 100%); }
.score-bar-pct { color: var(--fg); text-align: right; font-family: monospace; font-size: 11.5px; }

h1 { font-size: 24px; border-bottom: 1px solid var(--border); padding-bottom: 10px; margin: 48px 0 14px; }
h2 { font-size: 19px; margin: 32px 0 10px; }
h3 { font-size: 16px; margin: 22px 0 8px; color: var(--accent); }
h4 { font-size: 13px; margin: 16px 0 6px; color: var(--fg-dim); text-transform: uppercase; letter-spacing: 1px; }
a { color: var(--accent); }
code {
  background: rgba(110,118,129,0.18); padding: 1px 6px; border-radius: 4px;
  font-family: "JetBrains Mono", "SF Mono", Menlo, Consolas, monospace; font-size: 12.5px;
}

.cat-intro { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin: 12px 0 18px; }
.cat-intro .col {
  background: var(--bg-elev); border: 1px solid var(--border); border-radius: 6px;
  padding: 12px 16px; font-size: 13px;
}
.cat-intro .col strong { color: var(--accent); }

.tbl-wrap { overflow-x: auto; margin: 12px 0 20px; border: 1px solid var(--border); border-radius: 6px; }
table { border-collapse: collapse; width: 100%; background: var(--bg-tbl); }
th, td { padding: 8px 12px; text-align: left; vertical-align: top; border-bottom: 1px solid var(--border); font-size: 13.5px; }
th { background: var(--bg-elev); color: var(--accent); font-weight: 600; }
tr:last-child td { border-bottom: none; }
tr:hover td { background: rgba(255,255,255,0.025); }
td.status { white-space: nowrap; font-weight: 600; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.5px; }
.s-done     { color: var(--ok); }
.s-partial  { color: var(--partial); }
.s-planned  { color: var(--planned); }
.s-deferred { color: var(--deferred); }
.s-missing  { color: var(--missing); }

.legend {
  display: flex; gap: 22px; flex-wrap: wrap;
  background: var(--bg-elev); border: 1px solid var(--border);
  border-radius: 6px; padding: 12px 18px; margin: 14px 0 28px;
  font-size: 13px;
}
.legend .item { display: flex; align-items: center; gap: 6px; }

.assessment { display: grid; grid-template-columns: 1fr 1fr; gap: 24px; }
.assessment .panel {
  background: var(--bg-elev); border: 1px solid var(--border); border-radius: 8px;
  padding: 18px 22px;
}
.assessment .panel.strengths { border-left: 3px solid var(--ok); }
.assessment .panel.risks     { border-left: 3px solid var(--missing); }
.assessment h3 { margin-top: 0; color: var(--fg); font-size: 17px; }
.assessment .item { margin-bottom: 16px; padding-bottom: 16px; border-bottom: 1px solid var(--border); }
.assessment .item:last-child { border-bottom: none; margin-bottom: 0; padding-bottom: 0; }
.assessment .item .head { font-weight: 600; color: var(--accent); margin-bottom: 5px; font-size: 14px; }
.assessment .item.s-risks .head { color: var(--missing); }
.assessment .item .body { font-size: 13px; color: var(--fg); }

.summary-numbers {
  display: grid; grid-template-columns: repeat(4, 1fr); gap: 14px;
  margin: 18px 0;
}
.summary-numbers .stat {
  background: var(--bg-elev); border: 1px solid var(--border); border-radius: 6px;
  padding: 14px 16px; text-align: center;
}
.summary-numbers .stat .n { font-size: 26px; font-weight: 700; color: var(--accent); }
.summary-numbers .stat .l { font-size: 11px; color: var(--fg-dim); text-transform: uppercase; letter-spacing: 1px; margin-top: 4px; }

.recommendation { background: rgba(121,192,255,0.05); border-left: 3px solid var(--accent);
  padding: 12px 18px; border-radius: 0 6px 6px 0; margin: 14px 0; }
.recommendation strong { color: var(--accent); }

.method { background: var(--bg-elev); border: 1px solid var(--border); border-radius: 6px;
  padding: 16px 20px; font-size: 13px; color: var(--fg-dim); }
.method p { margin: 6px 0; }
.method strong { color: var(--fg); }
"""


STATUS_LABELS = {
    "done":     ("● DONE",     "s-done"),
    "partial":  ("◐ PARTIAL",  "s-partial"),
    "planned":  ("○ PLANNED",  "s-planned"),
    "deferred": ("◌ DEFERRED", "s-deferred"),
    "missing":  ("✗ MISSING",  "s-missing"),
}


def render_status(s: str) -> str:
    label, cls = STATUS_LABELS.get(s, (s, "s-planned"))
    return f'<td class="status {cls}">{html.escape(label)}</td>'


def render_category(cat) -> str:
    out = [f'<section id="{cat["id"]}">',
           f'<h1>{html.escape(cat["title"])}</h1>']
    out.append('<div class="cat-intro">')
    out.append(f'<div class="col"><strong>LibreCAD:</strong> {html.escape(cat["lc_intro"])}</div>')
    out.append(f'<div class="col"><strong>RUST_CAD:</strong> {html.escape(cat["rc_intro"])}</div>')
    out.append('</div>')

    d, p, pend, total = score_category(cat)
    pct = round(((d * 1.0 + p * 0.4) / total) * 100, 1) if total else 0
    out.append('<div class="summary-numbers">')
    for n, l in [(total, "items audited"), (d, "● done"),
                 (p, "◐ partial"), (pend, "○/◌/✗ pending")]:
        out.append(f'<div class="stat"><div class="n">{n}</div><div class="l">{l}</div></div>')
    out.append('</div>')

    out.append('<div class="tbl-wrap"><table>')
    out.append('<thead><tr>'
               '<th>LibreCAD feature</th>'
               '<th>RUST_CAD analog</th>'
               '<th>Status</th>'
               '<th>Notes</th>'
               '</tr></thead><tbody>')
    for row in cat["rows"]:
        lc_feat, rc_analog, status, notes = row
        out.append(
            '<tr>'
            f'<td><code>{html.escape(lc_feat)}</code></td>'
            f'<td>{html.escape(rc_analog)}</td>'
            f'{render_status(status)}'
            f'<td>{html.escape(notes)}</td>'
            '</tr>'
        )
    out.append('</tbody></table></div>')
    out.append('</section>')
    return "\n".join(out)


def render_assessment(title: str, items: list, klass: str) -> str:
    out = [f'<div class="panel {klass}"><h3>{title}</h3>']
    for head, body in items:
        item_class = "item s-risks" if klass == "risks" else "item"
        out.append(f'<div class="{item_class}">')
        out.append(f'<div class="head">{html.escape(head)}</div>')
        out.append(f'<div class="body">{html.escape(body)}</div>')
        out.append('</div>')
    out.append('</div>')
    return "\n".join(out)


def get_git_state() -> str:
    try:
        log = subprocess.check_output(
            ["git", "-C", str(ROOT), "log", "--oneline", "-5"],
            text=True).strip()
        return log
    except Exception:
        return "(git log unavailable)"


def get_test_count() -> str:
    """Sum 'NN passed' from cargo test output."""
    try:
        out = subprocess.check_output(
            ["cargo", "test", "--workspace", "--quiet"],
            cwd=str(ROOT), text=True, stderr=subprocess.STDOUT, timeout=180)
        import re
        nums = [int(m.group(1)) for m in
                re.finditer(r"(\d+) passed", out)]
        return str(sum(nums))
    except Exception:
        return "?"


def main():
    when  = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    score, by_cat = overall_score()
    git_log = get_git_state()
    test_count = get_test_count()

    # Sidebar TOC
    toc = ['<nav class="toc">',
           '<h2>Audit</h2>',
           '<a href="#summary">Executive summary</a>',
           '<a href="#method">Methodology</a>',
           '<a href="#assessment">Architectural assessment</a>',
           '<a href="#recommendations">Recommendations</a>',
           '<h2>Categories</h2>']
    for cat in CATEGORIES:
        toc.append(f'<a href="#{cat["id"]}">{html.escape(cat["title"])}</a>')
    toc.append('</nav>')

    # Cover
    cover = f"""
<section class="cover">
<h1>RUST_CAD — Third-Party Audit vs LibreCAD baseline</h1>
<div class="subtitle">
Generated <strong>{when}</strong> &nbsp;•&nbsp;
Workspace: <code>~/workspace/RUST_CAD/</code> &nbsp;•&nbsp;
Tests passing: <strong>{test_count}</strong> &nbsp;•&nbsp;
Recent commits:
</div>
<pre style="margin-top:10px;font-size:11.5px;">{html.escape(git_log)}</pre>
<p>
This document evaluates the RUST_CAD project as if reviewed by an outside
engineer, using LibreCAD as the established 2D-CAD baseline. It is the
project's <strong>compass</strong>: read it whenever you suspect the
slice plan is drifting, or before adding new scope. Each category lists
LibreCAD's feature surface, the corresponding RUST_CAD status, and notes
on gaps. Categories are weighted; the overall score reflects rough parity.
</p>
<p>Audit philosophy: be honest about what isn't there. A roadmap is only
useful when it stays attached to reality.</p>
</section>
"""

    # Score block
    score_pct = round(score * 100, 1)
    bars = []
    for cid, ctitle, cscore, d, p, t, w in by_cat:
        pct = round(cscore * 100)
        bars.append(
            f'<div class="score-bar-row">'
            f'<div class="score-bar-name">{html.escape(cid)}</div>'
            f'<div class="score-bar-track"><div class="score-bar-fill" style="width:{pct}%"></div></div>'
            f'<div class="score-bar-pct">{pct}%</div>'
            f'</div>'
        )
    score_block = f"""
<section id="summary">
<div class="score-block">
<div class="score-headline">
<div class="score-big">{score_pct}% <span class="label">weighted LibreCAD parity</span></div>
<div class="score-byline">
Scoring: <code>done</code> = 1.0, <code>partial</code> = 0.4, everything else = 0.
Per-category weights reflect importance to a working CAD workflow
(entities &amp; edit-ops heaviest; print/plot lightest). The number above
is a directional indicator — not a benchmark.
</div>
</div>
<div class="score-bars">
<h4>Per-category</h4>
{''.join(bars)}
</div>
</div>
</section>

<section class="legend">
<div class="item"><span class="s-done">● done</span></div>
<div class="item"><span class="s-partial">◐ partial</span></div>
<div class="item"><span class="s-planned">○ planned</span></div>
<div class="item"><span class="s-deferred">◌ deferred</span></div>
<div class="item"><span class="s-missing">✗ missing</span></div>
</section>
"""

    # Methodology
    method = """
<section id="method">
<h1>Methodology</h1>
<div class="method">
<p><strong>Baseline:</strong> LibreCAD (<code>~/workspace/Librecad/LibreCAD-master/</code>),
HSI fork branch. Feature surface surveyed across <code>src/lib/engine/</code>,
<code>src/actions/</code>, <code>src/ui/dock_widgets/</code>, <code>src/lib/fileio/</code>.</p>

<p><strong>Subject:</strong> RUST_CAD (<code>~/workspace/RUST_CAD/</code>),
<code>main</code> branch. State as of generation timestamp. Sources of truth:
<code>cad_kernel/src/</code> (~16 modules), <code>cad_app/src/app.rs</code>,
<code>cad_io/src/</code>, the three reference docs at workspace root
(<code>Variables.md</code>, <code>Dobject_DXF.md</code>, <code>Dobject_Properties.md</code>),
and <code>ROADMAP.md</code> for declared intent.</p>

<p><strong>What counts as &quot;done&quot;:</strong> the capability is in code, exercised
by a test or wired to the UI, and behaves the way a CAD user would expect.
&quot;Partial&quot; means a subset works or the data is stored but a consumer is
missing. &quot;Planned&quot;, &quot;deferred&quot;, &quot;missing&quot; are non-overlapping —
explicit roadmap intent vs deliberately out-of-scope vs not on anyone's
list yet.</p>

<p><strong>What's NOT audited:</strong> 3D modelling, BIM, parametric
constraints, rendering quality, plug-in extensibility, Python/LISP
scripting. RUST_CAD is a 2D drafting workbench; comparisons against 3D
CAD or BIM tools belong in a different document.</p>
</div>
</section>
"""

    # Category sections
    cats_html = "\n".join(render_category(c) for c in CATEGORIES)

    # Architectural assessment
    assessment = f"""
<section id="assessment">
<h1>Architectural assessment</h1>
<div class="assessment">
{render_assessment("Strengths", STRENGTHS, "strengths")}
{render_assessment("Risks &amp; gaps", RISKS, "risks")}
</div>
</section>
"""

    # Recommendations
    recommendations = """
<section id="recommendations">
<h1>Recommendations</h1>

<div class="recommendation">
<strong>1. Pivot to Phase 1 of Implementation_Plan.md.</strong>
Edit ops, Undo/Redo, ACI palette — three of yesterday's top
recommendations all landed in the last 24 hours. The edit-tools category
is now ~85% complete (Explode / Divide / Order / polyline segment ops
remain, plus polar array). Diminishing returns from squeezing the last
items. Switch focus to <strong>Phase 1 (Rectangle / Polygon / Circle 2P
+ 3P / Arc TTR)</strong>: zero new Dobject types, ~3–4 dev sessions,
doubles the visible toolbar surface.
</div>

<div class="recommendation">
<strong>2. Harden DXF I/O before claiming &quot;DXF support&quot;.</strong>
Slice H proves the pipeline. Real-world DXF parity needs: xdata, blocks
(blocked behind Slice F), basic text + dim styles, hatch boundary loops,
SPLINE preservation (pass-through even if we can't render it), proxy
entity skip. Add a regression suite of 20+ real DXF files (AutoCAD,
LibreCAD, QCAD, BricsCAD outputs) and require round-trip diff to be
zero or known-safe.
</div>

<div class="recommendation">
<strong>4. Bring the GPU path to ByLayer parity.</strong>
CircleInstance already carries a u32 color; populate it from
resolve_color() the same way the CPU path does. Pre-condition for any
benchmarking comparison against LibreCAD on million-entity drawings.
</div>

<div class="recommendation">
<strong>5. Move handle allocation to per-Document.</strong>
Process-global counter breaks DXF round-trip the moment two files are
open. Refactor before Block / Insert work (Slice F) starts.
</div>

<div class="recommendation">
<strong>6. Wire the panel of catalogued SYSVARS in batches by theme.</strong>
209 entries — wire them in themed groups (grip colors / sizes / snap radii
first since those are visible; then editing-display flags; then save /
backup). Each batch is 1–2 hours and visibly increases the &quot;feels like
AutoCAD&quot; quotient.
</div>

<div class="recommendation">
<strong>7. Don't open Slice F (Blocks) until E.text + dimensions land.</strong>
Blocks intersect with text-heavy real drawings; landing INSERT without
TEXT means imported blocks render with placeholders. Sequence:
Text → MText → DimRotated → Block table.
</div>

<div class="recommendation">
<strong>8. Treat this Audit.html as a checkpoint, not a one-off.</strong>
Regenerate after each slice with <code>python3 build_audit.py</code>.
The bars on the cover should move in one direction over time. If they
plateau, the slice plan is drifting away from parity work and into
nice-to-haves — that's a planning signal, not a comment on velocity.
</div>
</section>
"""

    html_doc = f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>RUST_CAD — Audit vs LibreCAD</title>
<style>{CSS}</style>
</head>
<body>
<div class="layout">
{"".join(toc)}
<main>
{cover}
{score_block}
{method}
{cats_html}
{assessment}
{recommendations}
</main>
</div>
</body>
</html>
"""
    out = ROOT / "Audit.html"
    out.write_text(html_doc)
    print(f"wrote {out}  ({len(html_doc):,} bytes, {score_pct}% weighted parity)")


if __name__ == "__main__":
    main()
