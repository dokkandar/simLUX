# GPU render — push status & branch clarification

**TL;DR:** The full GPU renderer **is committed and pushed** to HSI-Lighting. It lives on branch
**`windows-ui-session-2026-06-20`**, *not* on `main`. Nothing is unpushed and nothing needs
re-implementing. The dokkandar agent's "origin doesn't have it" is because it diffed HSI **`main`**
(stale) instead of the dev branch.

Date: 2026-07-04 · Origin: `github.com/HSI-Lighting/RUST-AutoRASM`

---

## The evidence

| HSI origin branch | tip | `cad_app/src/gpu.rs` | Renderer | Has GPU merge (`7aecd7d`)? |
|---|---|---|---|---|
| `main` | `3157d1a` (old "Parametric auto-solve") | **228 lines** | `GpuCircleRenderer` (circle-only) | ❌ No |
| **`windows-ui-session-2026-06-20`** | `0d07081` | **607 lines** | **`GpuShapeRenderer`** (full) | ✅ **Yes** |

The dokkandar report's "Origin = 228 lines, `GpuCircleRenderer` only" **exactly matches HSI `main`**.
`main` is a *pre-GPU* commit (`3157d1a`), so its `gpu.rs` is naturally the circle-only version.
A plain `windows-ui-session` branch (no date) **does not exist** on HSI origin — so tooling that
asked for it fell back to `main`/HEAD.

### Direction of the work
The GPU render **originated in dokkandar**. HSI **pulled it in** on 2026-07-03 (see commits below).
So HSI got it *from* dokkandar — it now exists on **both** sides. There is nothing for another
agent to implement.

---

## Verify it yourself

```bash
git fetch origin
git show origin/windows-ui-session-2026-06-20:cad_app/src/gpu.rs | wc -l   # → 607
git branch -r --contains 7aecd7d                                          # → windows-ui-session-2026-06-20
git show origin/main:cad_app/src/gpu.rs | wc -l                           # → 228 (old, circle-only)
```

---

## The 13 merged commits (`7aecd7d..8ecbe47`, on `windows-ui-session-2026-06-20`)

```
7aecd7d gpu: merge full GPU renderer from dokkandar/Auto_RASM
f15b0c6 fix(parser): bare `ellipse` enters the tool + add `ellipsearc` command
7b8930f Hatch: .pat pattern extractor (parser + CLI + standard starter patterns)
90c0701 backend: bring ext backend + cad_param/cad_raster wholesale
9145cb9 fix(grips): drag-only grip editing — a plain click no longer warps a dobject
1b1414a feat(open): auto zoom-to-fit after loading a drawing
99d3c9f feat(pline/spline): commit-on-end + Esc drops only the last segment
8efd2b6 feat(wall): wall-xjunction — ported app.rs portion of dokkandar dcbb59b
8de5418 feat(wall): explode a wall into its boundary particles (faces + caps)
afcc0f8 docs+tools: bring ext handoff guides + dwgconv (.NET) with Windows .cmd wrapper
ffdf853 feat(open): DWG open via external converter (cross-platform)
63df464 feat(raster): raster→vector editor + image underlays (from dokkandar)
8ecbe47 feat(param): parametric constraint mode + sketch panel (from dokkandar)
```
(`0d07081` on top adds the reconciliation handoff doc.)

---

## Message to paste to the dokkandar agent

> You're comparing against the wrong HSI branch. HSI's `main` (`3157d1a`) is stale — an old
> pre-GPU commit, so its `gpu.rs` is the 228-line circle-only `GpuCircleRenderer`. The GPU render
> is **not** on `main`; it's on **`origin/windows-ui-session-2026-06-20`** (`0d07081`), where HSI
> already merged it in: `gpu.rs` there is the full **607-line `GpuShapeRenderer`** (circle/arc/
> ellipse/line/fill pipelines, hatch cache, unified 3-way `RenderMode`). Verify with `git fetch`
> then `git show origin/windows-ui-session-2026-06-20:cad_app/src/gpu.rs | wc -l` (→ 607) or
> `git branch -r --contains 7aecd7d`. So nothing is unpushed and nothing needs re-implementing —
> the render originated on dokkandar and HSI **pulled it in** from you; it now exists on both
> sides. Please re-run your diff against `origin/windows-ui-session-2026-06-20` (note the date
> suffix — a plain `windows-ui-session` branch doesn't exist on HSI origin, which is why your
> tooling fell back to `main`).

---

## Optional: make it unambiguous on `main`

If you want anyone diffing HSI `main` to see the full renderer, merge/fast-forward
`windows-ui-session-2026-06-20` → `main` and push. Not done here — updating `main` is the owner's
call (the standing instruction was to leave `main` alone).
