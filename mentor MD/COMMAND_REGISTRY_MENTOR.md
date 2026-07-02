# COMMAND REGISTRY — MENTOR (phased build plan)

Companion to `COMMAND_SYSTEM.md`. It builds the CommandRegistry in **small,
reviewed phases**, each independently shippable and revertable. Sits alongside
the design docs; the owner defines each phase and it is pressure-tested before
the code is written.

**Core principle (the whole design in one line):** a **metadata registry** +
**your existing dispatch**. The registry *describes* commands (id, title,
category, aliases, shortcut, icon, context predicates). It does **not** execute
them — `run_command(id)` still does, unchanged. This sidesteps the Rust
borrow-checker trap of storing `&mut CadApp` closures inside `CadApp`.

---

## Global invariants (hold across every phase)

`run_command(id)` is the **ONE dispatch point**. A UI click and a typed command
both produce an `id`, and both funnel through `run_command`. The registry never
gains a second execution path.

**Dependencies flow UP, definitions live DOWN.** `cad_app` may depend on
`cad_kernel`; the kernel NEVER depends on the app (ARCHITECTURE §2 — the kernel
doesn't know the registry exists). So any data both layers must share lives in
the kernel, and the app reads it.

**Mental model:** Registry ids organize the UI; dispatch tokens execute the tool.
Nothing else crosses that boundary.

**Presentation ≠ parsing.** Parser shortcuts (`"l"`) belong to the parser — they are
NOT registry data. UI search metadata (`keywords`) belongs to the registry —
independent of parser syntax. The same command can have rich UI keywords without
exposing or depending on any parser shorthand.

---

## Phase 0 — FREEZE (locked ✔ 2026-07-02)

Before touching anything, define what will **not** change. These stay exactly as
they are — only the *description* of commands moves into the registry:

- command **execution** (`run_command`)
- **undo**
- active **tool state machines**
- **parser** logic
- command-line **prompt system** (multi-stage flows, direct-distance entry, sysvars)
- command **history**

### Target architecture (corrected)

```
UI (Rails · Menus · Palette)
   │  read metadata → RENDER          │  on click → command id
   ▼                                  ▼
Registry (metadata only) ───────────► run_command(id) ──► CadApp
   ▲                                  ▲
   │  (later phase) owns alias → id   │
Command line / Parser ────────────────┘   ← frozen now
```

The correction vs the first sketch: the registry is **not** a dead end. A rail/
menu click resolves the command's `id` from the registry, then calls
`run_command(id)`. The registry feeds **rendering**; the **click still dispatches
through `run_command`**. Execution is untouched.

### Locked decisions

1. **Dispatch — reuse what exists.** *(Refined 2026-07-02 — see the Phase 1 id/
   dispatch amendment.)* Execution keeps flowing through `run_command`, called
   with the command's **`dispatch` token** (`"line"`), which is the string
   `run_command` already expects. The registry `id` is **namespaced** (`draw.line`)
   for stable identity, but is NEVER passed to `run_command`. Every surface
   invokes via one `dispatch(id)` helper → `run_command(cmd.dispatch)`. Rails
   already do essentially this (`rail_dispatch(cmd) → run_command(cmd)`), so the
   execution edge is **already there** — migration changes only where a surface
   gets its *list + metadata*, never how it executes. Store **no closures** in
   the registry.

2. **Aliases — there are TWO unrelated systems; the registry needs neither joined.**
   ⚠️ **Resolved 2026-07-02 after verifying the parser AND correcting an over-reach.**
   `cad_kernel::parser` holds `"line" | "l" => parse_line` (`parser.rs:312`) — aliases
   are match-arm literals. That is a **kernel input-parsing** concern. It is a **category
   error** to fuse it with UI discoverability. Keep them separate:

   | "alias" | Lives in | Purpose | UI's business? |
   |---|---|---|---|
   | `"l"` → line | kernel parser match arms | typing/parsing input | **No** |
   | `"(L)"` hint | tooltip / registry metadata | palette search, hints | Yes |

   - **The registry needs NO alias field.** The palette (Phase 7) searches the metadata
     it already has — `title` / `tooltip` / `category`. It never reaches into the parser.
   - **Parser aliases stay frozen forever** (for this migration) — kernel-internal, untouched.
   - **"Phase 5 / alias ownership" was a PHANTOM problem** born of merging the two. It is
     **dissolved, not deferred** — nothing in the registry migration ever depended on parser
     aliases. The `aliases` field cut in Phase 1 stays cut, correctly.
   - *Footnote (out of scope):* the parser `"l"` and the tooltip `"(L)"` are the same fact
     stored twice, so they can drift — but that's pre-existing and NOT a migration concern.
     A shared `cad_kernel` command-def table (the old "Option C") is relevant **only** as a
     far-future nicety *if* you ever want palette alias-search guaranteed in sync with the
     parser. Not needed, not now, not part of this work.

**Phase 0 exit criteria:** nothing built yet — this is the contract. Any phase
that would change an item in the freeze list, add a second dispatch path, or
hand-duplicate aliases is **out of scope** and must be raised before proceeding.

---

## Phase 1 — CREATE THE METADATA REGISTRY (schema only, locked ✔ 2026-07-02)

Define the **types only** — NO data (that's Phase 2), NO wiring. The app stays
**visually identical** after Phase 1.

**The struct — final shape** *(amended 2026-07-02: split id from dispatch — see note):*

```
CommandInfo {
    id,        // stable, namespaced: "draw.line" — identity for registry/UI/palette
    dispatch,  // the run_command token: "line"   — the ONLY value passed to run_command
    title,     // display name  ("Line")
    tooltip,   // hover text     ("Line  (L)")
    category,  // Draw | Modify
    icon,      // IconId
}
```

> **Amendment (id vs dispatch).** Registry identity and the execution token are
> two different things. `id` is **namespaced** (`draw.line`) per COMMAND_SYSTEM §2
> — stable, unique across categories/plugins (`layer.copy` vs `modify.copy`), and
> it's what gets **persisted** in `draw_items` from Phase 3 (so it must be stable
> BEFORE persistence — that's why we do it now, not later). `dispatch` is today's
> `run_command` token (`line`). **Invariant: surfaces call
> `run_command(cmd.dispatch)`, NEVER `run_command(cmd.id)`** — enforce via one
> `dispatch(id)` helper. `run_command` stays untouched (freeze intact). Both `id`
> and `dispatch` are DERIVED from the arrays in Phase 2 (`dispatch` = command
> string; `id` = `"<category>." + dispatch`) — no hand-typing, one source.
> `CommandId` stays a **string** for now (not a `CommandId::DrawLine` enum).

**Cut from the agent's draft** (moved to their real phases — do NOT include now):
- `aliases` → **Phase 5** (alias ownership). A hand-fillable alias field next to
  the parser's is the drift trap Phase 0 froze against.
- `visible` / `enabled` → the **context-predicates phase**, and they return as
  **`fn(&Ctx) -> bool`**, NOT static `bool` (a static bool can't express
  "enabled only when 2 lines are selected").

**Four types, pinned:**
1. **`CommandId` = the namespaced id, an OWNED `String`** (`"draw.line"`). It must be
   `String`, not `&'static str`, because Phase 2 DERIVES it by concatenation
   (`"<category>." + dispatch`), which allocates. The **`dispatch`** token stays
   `&'static str`. `CommandId` is a string, never a new int/enum (an enum would break
   "reuse existing dispatch"). *(Corrected 2026-07-02 — the earlier "&'static str"
   note predated the runtime-derived id.)*
2. **`IconId` = enum over BOTH glyph families** → `DrawGlyph(…)` |
   `ModifyGlyph(GlyphKind)` (Lucide later). A flat id can't serve both; Phase 3
   needs this.
3. **`CommandCategory` = enum `{ Draw, Modify }`**, extensible.
4. **`CommandRegistry { commands: HashMap<CommandId, CommandInfo> }` — lookup by id.**
   *(Amended in Phase 6:)* it ALSO keeps a **canonical ordered index** (`Vec<CommandId>`
   in seed order) so `by_category` returns a **deterministic** order for menus (a raw
   HashMap iterates randomly). **Rail** order is separate — the user-custom order in
   `draw_items` / `modify_items` (Phase 3). Two orders: canonical (menus) vs custom (rails).

**Invariants:** NO `handler` / closure / `FnMut` / `&mut CadApp` in the registry;
`run_command` untouched; ONE source of truth (Phase 2 *derives* from the arrays —
no second hand-written command list); app visually identical.

**Verify:** it compiles and the fields/types match this shape. (The debug-dump
check belongs to Phase 2, where there is data.)

**Exit criteria:** types defined, registry empty, compiles, nothing wired, freeze intact.

---

## Phase 2 — POPULATE FROM THE ARRAYS (locked ✔ 2026-07-02)

Fill the registry from `DRAW_CMDS` + `MODIFY_CMDS`. Still **invisible** — nothing
renders from it; the app stays identical.

- **Derive, don't replace.** Leave `DRAW_CMDS`/`MODIFY_CMDS` **untouched** (rails
  still read them until Phase 3). Build the registry *from* them at startup — the
  arrays stay the single source; the registry is a derived copy. Arrays retire in Phase 3.
- **Populate BOTH categories** — Draw *and* Modify (not just Draw).
- **Per entry (all derived — no hand-typing):** `dispatch` = command string (col 2);
  `id` = `"<category>." + dispatch` (`draw.line`); `tooltip` = col 3; `icon` =
  `DrawGlyph`/`ModifyGlyph` ref (col 1); `title` = derived from the tooltip (strip a
  trailing `(KEY)`) — refine wording later.
- **Verify:** a temporary **debug dump** (Tools ▸ Debug) lists every entry
  (id / dispatch / title / category / icon) — the proof it populated, since nothing renders yet.
- **Invariants:** execution untouched; NO second hand-written command list; app visually identical.

**Exit criteria:** registry fully populated (Draw + Modify) & dump-verified; arrays
intact; rails still read the arrays; freeze intact.

---

## Phase 3 — RAILS USE IDS INSTEAD OF INDEXES (locked ✔ 2026-07-02)

First real migration. `draw_items`/`modify_items`: `Vec<usize>` → `Vec<CommandId>`;
rails read `registry.get(id)` instead of `DRAW_CMDS[index]`. First behavior-touching
phase, but **user-visibly identical**.

- **Read side:** rail renders from `registry.get(id)` — icon via `IconId`
  (`DrawGlyph → draw_draw_glyph`, `ModifyGlyph → draw_cmd_glyph`), tooltip/title from
  the entry. Order preserved (ordered `Vec`; order lives in the list, not the HashMap).
- **Execution seam — one `execute(id)` helper.** Every surface calls `execute(id)`,
  never `run_command` or `dispatch` directly. Today `execute(id)` resolves to
  `run_command(cmd.dispatch)`; a future executor change touches ONLY this helper.
  Abstraction = "UI → command execution API"; `dispatch` is the internal detail, not
  permanent architecture. **Never pass `id` to `run_command`.** ⚠️ **`run_command` is
  OUTSIDE the migration surface** — it never sees registry ids, now or ever in this
  design. Its `match` arms stay keyed on the **dispatch token** (`"line"`); do NOT
  rewrite them to the namespaced id (`"draw.line"`).

> **Mental model:** Registry ids organize the UI; dispatch tokens execute the tool.
> Nothing else crosses that boundary.
- **Defensive lookup — never panic.** `if let Some(cmd) = registry.get(id)` everywhere.
  A stale/unregistered id is **skipped on render** (per WORKSPACE_SYSTEM "unknown ids
  dropped silently") and a **graceful no-op on click**. Optionally debug-log; never
  `unwrap`. (Plugins can remove commands — Phase 3 opens this door.)
- **Support logic → id-based:** add/remove/reorder/reset switch index → id
  (sites `app.rs:10434/10441/10516/10525`); the "available tools" list comes from
  `registry.by_category(cat)`.
- **Flyouts** (arc/circle/fillet) key on `cmd.dispatch`, not `id` — must still fire.
- **Arrays stay as the registry SEED** — rails stop reading `DRAW_CMDS`/`MODIFY_CMDS`
  directly, but they are NOT deleted (they populate the registry). Inlining is optional/later.
- **Persistence:** none today (`draw_items` resets to default each launch), so the
  `usize → CommandId` change needs no migration.

**Exit checklist:**
- [ ] Same commands appear in the same order
- [ ] Drag-and-drop preserves order
- [ ] Add / remove / reset works
- [ ] **Every stored `CommandId` resolves** in the registry (no missing lookups / panics)
- [ ] Rail clicks execute the correct command via `execute(id)` (the execution-API seam)
- [ ] Flyouts (Arc, Circle, Fillet…) behave identically
- [ ] Icons render correctly via `IconId`
- [ ] No user-visible behavior change

---

## Phase 5 — UI METADATA ENRICHMENT (locked ✔ 2026-07-02)

Reframed from the dissolved "alias ownership." Goal: enrich the registry so UI
surfaces (menus, palette, help, future plugins) present commands consistently —
**without touching parsing or execution.**

**Scope — add two STATIC presentation fields:**

```
CommandInfo { id, dispatch, title, tooltip, category, icon, keywords, group }
```

- **`keywords`** — UI search terms for the palette (`segment`, `straight`, `edge`
  → Line). **NOT parser aliases.** Hand-authored (the first *non-derived* registry
  data; lives in ONE place → no drift). Typing "segment" may find Line; "draw" may
  list drawing tools. The parser never changes.
- **`group`** — optional sub-group within a category for richer menus
  (Draw ▸ Basic ▸ Line/Polyline; Draw ▸ Curves ▸ Circle/Arc). Presentation only.

**NOT in this phase:**
- **`visible` / `enabled`** → the **context-predicates phase (6b)**, built as
  `fn(&Ctx) -> bool` (NOT static bools — the Phase 1 cut reason still holds; context
  can't be a fixed flag). Don't pull `Ctx` machinery into this static-metadata phase.
- **No `aliases` field** — aliases stay dissolved (parser-internal; see Phase 0 decision #2).

**Unchanged:** parser, `run_command`, dispatch, tool state machines, kernel. This
only enriches app-owned metadata.

**Exit criteria:** registry carries `keywords` + `group`; menus/palette *can*
consume them; parser / dispatch / `run_command` untouched; kernel unaware of the
registry; registry unaware of parser internals.

---

## Phase 6 — CONVERT MENUS (Draw & Modify only) (locked ✔ 2026-07-02)

Generate the **plain command buttons** of the Draw and Modify menus from the
registry — the rails pattern (Phase 3) applied to the menu surface. Only *after*
rails work.

**Scope: Draw + Modify menus ONLY.** File / Edit / View / Formative / Utilities /
Tools / Help stay hand-authored — their items aren't registry commands.

**Three rules (what "nothing else changes" glosses):**

1. **Ordered `by_category`.** A `HashMap` iterates randomly → menu items would
   shuffle each launch. The registry keeps a **canonical ordered index**
   (`Vec<CommandId>` in seed/array order, populated in Phase 2); `by_category`
   iterates *that*, filtered. Menus use this canonical order; **rails keep their
   custom order from `draw_items`**. (Two orders: canonical for menus, custom for rails.)
2. **Hybrid, NOT flat.** Draw/Modify menus interleave special items a flat loop
   would drop: dialogs (`Hatch…`, `Block…`, `Array…`), a dynamic submenu
   (`Insert Block ▸`), separators, chained/special (`Wall`, `Inspector…`,
   `Match Properties`, `Change Layer`). Convert ONLY the plain command items to a
   registry loop; keep the special items **hand-authored, interleaved**. "Nothing
   else changes" must NOT mean "flatten the menu."
3. **Dispatch via the `execute(id)` seam** — each generated item's click →
   `execute(id)` → `run_command(cmd.dispatch)`. Never `run_command(id)`; never the
   old hardcoded call.

**Good news:** menus enumerate the registry (`by_category`), not stored ids — so
there's no stale-id/panic risk (no persisted menu config). The Phase 3 defensive-
lookup rule isn't critical here.

**Optional:** Phase 5's `group` can add sub-headers to the Draw menu (Basic / Curves).

**Exit criteria:** Draw/Modify plain commands render from the registry in canonical
order; all dialogs/submenus/separators/special items preserved; every item dispatches
via `execute(id)`; other menus untouched; app behaves identically.

---

## Phase 6b — CONTEXT PREDICATES (locked ✔ 2026-07-02)

Turn `visible`/`enabled` (the fields reserved in Phase 5) into **computed predicates
over a read-only context**. The registry stops describing UI *state* and starts
describing UI *rules*.

**Struct — EXTEND the frozen schema (never re-declare it):**

```
CommandInfo {
    id, dispatch, title, tooltip, category, icon,   // Phases 1–2
    keywords, group,                                 // Phase 5
    visible: fn(&Ctx) -> bool,                       // 6b
    enabled: fn(&Ctx) -> bool,                       // 6b
}
```
⚠️ Keep `dispatch` / `keywords` / `group`; **NO `aliases`** (dissolved). Predicates
are **fn POINTERS, not capturing closures** — `|c| … self.x` smuggles hidden state
into what must be static rules. Forbidden.

**`Ctx` — read-only projection, built ONCE PER FRAME by `CadApp`:**

```
Ctx<'a> { selection, active_tool, has_clipboard, snap_enabled, mode, … }  // only what predicates read
```
HARD RULES (borrow safety): NO `&mut CadApp`, NO `Rc<RefCell>`, NO interior
mutability, NO mutating methods. **`Ctx` is a projection, NOT a gateway** —
`ctx.app.anything()` = the architecture is broken. If a predicate needs a value,
COMPUTE it when building `Ctx`; never fetch from the app inside a predicate.

**Defaults (prevent predicate explosion):** `always_visible` / `always_enabled`;
most commands point at these. Add real predicates only where behavior differs.

**`visible` vs `enabled` (strict):**
- `visible(ctx)` → should it appear at all? (selection exists, mode supports it, flag)
- `enabled(ctx)` → clickable right now? (prerequisites met, valid sub-state)
- UI: `if visible { if enabled { render_active } else { render_disabled } }`.

**Feeds EVERY surface** — greys-out/hides in rails, menus, palette AND the right-click
menu (not just the context menu). Filtering is **PURE** and iterates the **canonical
ordered index** (never HashMap `.values()` — unordered): no mutation, no execution,
no side effects. If filtering ever triggers tool state, the model is violated.

**Staging = no behavior change first:** add the fields defaulting to `always_*`
(app identical), THEN add real predicates command-by-command.

**The one-way graph (the whole point):**
`CadApp → builds Ctx → feeds registry → UI filters → dispatch executes`. No predicate
affects execution. The **parser stays blind** — never evaluates predicates, queries the
registry, or decides visibility/enablement.

**Exit criteria:** predicates are pure `fn(&Ctx)`; `Ctx` read-only & borrow-safe;
UI filtering uses registry + ctx only (ordered); execution & parser unchanged; no
`CadApp` leaks into the registry.

---

## Phase 7 — COMMAND PALETTE (+ keyboard shortcuts) (locked ✔ 2026-07-02)

The last surface. Only after everything else.

**Flow:** `registry.iter() → filter(visible) → search(title + keywords) → execute(id)`.
- Searches **`title` + `keywords`** (Phase 5 metadata) — **NOT parser aliases.**
  "segment" finds Line; "round" finds Fillet.
- Dispatch via the **`execute(id)` seam** → `run_command(cmd.dispatch)`. Never `run_command(id)`.
- The palette only DISPLAYS metadata; it never knows how a command executes.
- The keyboard **shortcut map** (`Accel → id`, conflict detection) folds in here.

**`IconId` (guardrail — enum, never a String):**
`enum IconId { Cad(CadGlyph), Lucide(LucideIcon), None }` — `Cad` must address BOTH
the draw-glyph and modify-`GlyphKind` painters; `Lucide` is reserved until Lucide is
wired in; rails render `Cad` glyphs, menus/palette may render `Lucide`.

**Exit criteria:** palette searches title+keywords and dispatches via `execute(id)`;
shortcut map derived from the registry with conflict detection; parser / dispatch /
tools untouched.

---

## Scope boundaries — NEVER migrate into the registry

The registry knows only *that `draw.line` exists* and how it looks. It must NEVER
absorb (this is the Phase 0 freeze, restated as a wall):

- **Multi-stage prompt flows** — LINE's first-point / second-point / close / cancel,
  etc. The registry knows `draw.line`; the **tool owns its prompt state machine.**
- **Undo** — `run_command → snapshot → execute → undo history` stays exactly as-is.
- **Active tool logic** — Mirror's pick → base → second → keep-copy, and every tool's
  interaction. The registry never knows these exist.
- **Parser alias resolution** — the parser NEVER reads the registry
  (`cad_kernel → cad_app` = forbidden, ARCHITECTURE §2). **"Parser aliases read the
  registry" is a permanently rejected idea** (see the "presentation ≠ parsing" invariant).

---

## Roadmap (agent's phasing + mentor corrections — locked only when reviewed)

| Phase | What | Status / mentor note |
|---|---|---|
| **0** | Freeze current architecture | ✔ **locked** |
| **1** | Create metadata registry (schema) | ✔ **locked** |
| **2** | Populate from `DRAW_CMDS` / `MODIFY_CMDS` | ✔ **locked** · derive both id+dispatch; debug-dump verify; arrays untouched |
| **3** | Rails use ids instead of indexes | ✔ **locked** · `execute(id)` seam; defensive lookup (no panic); full exit checklist |
| ~~4~~ | ~~Keep dispatch where it is~~ | **NOT a phase** — it's an invariant; folded into Phase 3's `execute(id)` seam |
| **5** | UI Metadata Enrichment | ✔ **locked** · add `keywords` (palette search) + `group` (menu sub-grouping) — static presentation only. NO `aliases`; `visible`/`enabled` reserved for 6b (predicates) |
| **6** | Convert menus | ✔ **locked** · Draw/Modify plain commands from ordered `by_category`; special items (dialogs/submenus/separators) stay hand-authored; `execute(id)` seam |
| **6b** | Context predicates `enabled/visible(ctx)` + right-click context menu | ✔ **locked** · `fn(&Ctx)->bool` (not bools); read-only `Ctx` projection (no `&mut`/gateway); defaults + staging; feeds all surfaces |
| **7** | Command palette (+ shortcut map) | ✔ **locked** · search `title`+`keywords`, dispatch via `execute(id)`; NO parser/alias dependency |
| ✗ | ~~Parser aliases read registry~~ | **PERMANENTLY REJECTED** — kernel→app dependency (ARCHITECTURE §2); parser never reads the registry |
| later | Plugin registration + reserved fields | deferred (owner's call) |

> Stop-anywhere: shipping through Phase 2–3 already delivers registry-driven,
> customizable rails — the rest is additive when wanted.
