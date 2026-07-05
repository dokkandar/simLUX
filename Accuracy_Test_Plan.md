# RUST_CAD — Numerical Accuracy Test Plan

> **Purpose.** Prove, with a repeatable test harness and a sales-facing
> report, that RUST_CAD computes π-dependent quantities — circumference,
> arc length, area, perimeter — to the **full precision of IEEE-754
> `f64`** (≈15.95 correct decimal digits). Not critical for drafting
> (where `EPS = 1e-9` is plenty), but a concrete differentiator at the
> sales point: *"measured to the last bit, with the report to prove it."*
>
> Status: **PLAN** (2026-06-03). Nothing here is built yet.

---

## 0. The headline answer: "how do you calculate π?"

**We don't calculate π — we use the correctly-rounded constant.**

Every π in the kernel is `std::f64::consts::PI` / `TAU` / `FRAC_PI_2`
(verified: `grep -rn "PI\|TAU" cad_kernel/src` — no hardcoded `3.14159`
literals anywhere). `std::f64::consts::PI` is the `f64` value *nearest*
to the true π — its error from real π is **< 0.5 ULP ≈ 1.1 × 10⁻¹⁶
relative**. You cannot store π more accurately in a 64-bit double; any
"more digits" would require a wider type.

So the accuracy story has exactly two sources of error, and the test
suite isolates each:

| Source | Magnitude | Controlled by |
|--------|-----------|---------------|
| **The constant π itself** | ≤ 0.5 ULP | Rust std (correctly rounded) — we just assert it |
| **The arithmetic** (r·r·π, integration, shoelace) | depends on the formula | **our code** — this is what the harness actually tests |

The interesting, defensible engineering is all in the second row.

---

## 1. Scope — what gets measured

| Shape | Quantity | Closed form? | Difficulty |
|-------|----------|--------------|------------|
| **Circle** | circumference `2πr` | exact | trivial — should be ≤ 1 ULP |
| **Circle** | area `πr²` | exact | trivial — should be ≤ 1 ULP |
| **Arc** | arc length `r·θ` | exact | trivial |
| **Arc** | sector area `½r²θ` | exact | trivial |
| **Arc** | chord length `2r·sin(θ/2)` | exact | trivial |
| **Ellipse** | area `πab` | exact | trivial — ≤ 1 ULP |
| **Ellipse** | **perimeter** | **NO elementary closed form** | **hard — the real story** |
| **EllipseArc** | **arc length** | incomplete elliptic integral E | **hard** |
| **EllipseArc** | sector area | closed form (`½ab·Δt`) | easy |
| **Polyline** | perimeter | sum of chords (+ bulge arcs) | easy; cancellation risk |
| **Polyline** | area | shoelace (+ bulge corrections) | easy; **cancellation risk** |

The two **hard** rows are where competitors cut corners (most CAD tools
ship the Ramanujan-II ellipse-perimeter approximation, good to ~3×10⁻⁵).
Doing them to full `f64` via **Carlson symmetric elliptic integrals** is
the headline sales claim.

---

## 2. Prerequisite — the measurement API does not exist yet

Today the kernel has only `Polyline::length()` (straight chords) and
`Arc::endpoints()`. There is **no `area()` / `perimeter()` /
`circumference()` / `arc_length()`**. Phase A builds it; Phases B–E test it.

### Phase A — measurement API (`cad_kernel/src/measure.rs`, new)

Pure functions on `&Geom`, zero new dependencies (kernel stays dep-free):

```rust
impl Geom {
    pub fn area(&self) -> f64;        // enclosed area; 0.0 for open shapes
    pub fn perimeter(&self) -> f64;   // closed-shape boundary length
    pub fn length(&self) -> f64;      // open-shape / curve length (extends today's)
}
```

Per-variant math:

| Variant | `area` | `length` / `perimeter` |
|---------|--------|------------------------|
| Line | 0 | `a.dist(b)` |
| Circle | `π·r²` | `2π·r` (perimeter) |
| Arc | sector `½r²·sweep` | `r·sweep` (length) |
| Ellipse | `π·a·b` where `b = a·ratio` | **Carlson** (perimeter) |
| EllipseArc | sector `½·a·b·sweep_param` | **Carlson incomplete** (length) |
| Point | 0 | 0 |
| Polyline (open) | 0 | Σ segment lengths (+ bulge arc len) |
| Polyline (closed) | shoelace (+ bulge area) | Σ segment lengths |

**Ellipse perimeter — the gold algorithm.** Use Carlson's symmetric
integral form, which is exact to machine precision and numerically
stable for all eccentricities (unlike Ramanujan approximations or naïve
AGM near `ratio→0`):

```
perimeter = 4a · E(e)            // E = complete elliptic integral 2nd kind
          = 4a · [ R_F(0,1-e²,1) − (e²/3)·R_D(0,1-e²,1) ]
```

where `R_F` and `R_D` are Carlson's symmetric integrals (≈15-line
iterative duplication algorithms, converge in ~6 iterations to `f64`
epsilon). `EllipseArc` length uses the **incomplete** Carlson form over
`[start_param, start_param+sweep_param]`.

> Note: the existing `EPS = 1e-9` `approx_eq` is a **drafting** tolerance
> and must NOT be used to validate these — accuracy tests compare in ULP.

---

## 3. The ground-truth oracle (test-only)

The crux of any accuracy test: *what do you compare against?* Three
independent oracles, used together so no single one can mask an error:

1. **Closed-form analytic** (for circle/arc/ellipse-area): the formula
   *is* the truth; we only measure the FP rounding of our arithmetic.
   Compare our `f64` result to the same expression evaluated in higher
   precision and rounded back.

2. **High-precision reference** (`[dev-dependencies]` ONLY — never ships
   in the kernel): an arbitrary-precision float crate — **`astro-float`**
   or **`dashu-float`** (both pure-Rust, no C) — computes each quantity
   at 256-bit precision, then rounds to `f64` to get the
   *correctly-rounded answer*. Our value is compared to that.

3. **Published constants** (for the elliptic-integral path): hardcode
   known values — e.g. the perimeter of an ellipse with `a=1, b=0` is
   exactly `4`, with `a=b` is `2πr`, and Gauss's constant / lemniscate
   cases have published 30-digit references — to independently confirm
   the Carlson implementation before trusting it as an oracle.

---

## 4. Phase B — the accuracy harness (`cad_kernel/tests/accuracy.rs`)

### 4.1 Metrics (the report columns)

For each (quantity, input) pair, compute against the oracle:

- **ULP distance** — `(ours.to_bits() as i64 − truth.to_bits() as i64).abs()`.
  The rigorous FP metric. Target: **≤ 1 ULP** for closed-form quantities,
  **≤ 4 ULP** for the integral-based ones.
- **Relative error** — `|ours − truth| / |truth|`.
- **Correct significant digits** — `−log10(relative_error)`. The number
  the sales deck quotes ("15.9 correct digits").

### 4.2 Test matrix — scale & shape coverage

Each quantity is tested across a grid so we catch scale-dependent error:

- **Radii / semi-axes**: `1e-9, 1e-3, 1.0, 7.389…, 1e3, 1e6, 1e12`
  (tiny → huge; catches overflow in `r²` and underflow).
- **Angles / sweeps**: `0+, π/6, π/2, π, 3π/2, TAU−ε, exactly TAU`
  (full-turn boundary is a known wrap-around trap — see `norm_angle`).
- **Ellipse ratios**: `1.0` (=circle, must match `2πr` exactly),
  `0.999, 0.5, 0.1, 1e-3, 1e-9` (degenerate sliver → tests Carlson
  stability where Ramanujan diverges).
- **Polygon position**: centred at origin AND offset to `(1e8, 1e8)`
  — see §5.

### 4.3 Invariant / identity tests (oracle-free sanity)

These need no reference value — they must hold by mathematics, and
catch whole classes of bugs:

- **Scale**: `area(scaled k) == k²·area` and `perimeter(scaled k) == k·perimeter` to ≤ few ULP.
- **Rotation**: area & perimeter **invariant** under `Geom::rotated` (reuses existing transform).
- **Translation**: area & perimeter invariant under move.
- **Circle = degenerate ellipse**: `Ellipse{ratio:1}.perimeter()` == `Circle.circumference()` to ≤ 1 ULP.
- **Arc additivity**: two arcs splitting a circle sum to `2πr`.
- **Convergence**: a regular N-gon inscribed in a circle has
  area → `πr²` and perimeter → `2πr` as N grows, at the analytic
  `O(1/N²)` rate (also validates the shoelace path).

---

## 5. Phase C — the catastrophic-cancellation tests (where accuracy is actually lost)

π is exact; the *arithmetic* is where digits die. The harness must
include the adversarial cases, because a naïve implementation passes
the origin-centred tests and fails these:

- **Shoelace far from origin.** A 1×1 square at `(1e8, 1e8)` computed by
  the raw shoelace sum loses ~8 digits to cancellation (the products are
  ~1e16, the area is 1). **Mitigation to implement & test:** subtract the
  centroid (or first vertex) before summing. Test asserts the offset
  square's area still matches `1.0` to ≤ 2 ULP.
- **Huge-radius circle area.** `r = 1e12` → `r² = 1e24`, still exact in
  `f64` (mantissa holds it), but verify no premature overflow vs `1e154`.
- **Near-degenerate ellipse** (`ratio = 1e-9`): Ramanujan-II relative
  error blows up here; Carlson stays ≤ 4 ULP. This single comparison is
  the strongest sales slide — put both numbers side by side.
- **Full-turn boundary** (`sweep = TAU` exactly vs `TAU − 1e-15`):
  arc length must be `2πr`, not `0`, at the wrap point.

---

## 6. Phase D — the constant-verification test (cheap, high-trust)

A one-file test that asserts our constants ARE the correctly-rounded
reals, so the sales claim "we use π to the last bit" is itself tested:

```rust
// π to 50 digits, rounded to nearest f64, must equal std::f64::consts::PI bit-for-bit.
assert_eq!(std::f64::consts::PI.to_bits(),  pi_ref_f64().to_bits());
assert_eq!(std::f64::consts::TAU.to_bits(), tau_ref_f64().to_bits());
```

where `pi_ref_f64()` parses the 50-digit literal via the dev-only
high-precision crate and rounds. Result: green check = "π is bit-exact."

---

## 7. Phase E — the sales artifact (`build_accuracy_report.py` → `Accuracy_Report.html`)

Reuse the existing report-generator pattern (`build_audit.py`,
`build_report.py`). A `cargo test` run with a `--emit-json` feature dumps
each (quantity, input, ours, truth, ulp, digits) row to JSON; the Python
script renders a presentation-grade single-file HTML deck:

- **Headline tiles**: "Circle area — 0 ULP, 15.95 digits", "Ellipse
  perimeter — 2 ULP, 15.6 digits".
- **The money table**: quantity · formula · sample input · our value ·
  reference value · ULP · correct digits.
- **The competitor slide**: our Carlson ellipse perimeter vs the
  Ramanujan-II approximation that ships in most tools, at `ratio=0.1`
  and `ratio=1e-6` — show the digit gap.
- **Methodology footnote**: f64 = IEEE-754 binary64, `std::f64::consts::PI`,
  256-bit oracle, ULP definition.

---

## 8. Deliverables & sequencing

| Step | Deliverable | Depends on | Est. |
|------|-------------|-----------|------|
| A | `cad_kernel/src/measure.rs` — `area`/`length`/`perimeter` + Carlson `R_F`/`R_D` | — | 1–2 sessions |
| B | `cad_kernel/tests/accuracy.rs` — matrix + invariants; `astro-float` dev-dep | A | 1–2 sessions |
| C | cancellation tests + shoelace centroid-shift mitigation | A, B | ~1 session |
| D | `tests/constants.rs` — bit-exact π/τ assertions | — (independent) | ~½ session |
| E | `build_accuracy_report.py` → `Accuracy_Report.html` | B | ~1 session |

Total ≈ 5–6 sessions. **Step D is independent** and could land first as
a quick confidence win. **Step A doubles as a real feature** — the
measurement API is needed anyway for a future "Properties / Inquiry"
panel (AutoCAD `AREA` / `DIST` / `LIST` commands), so this plan advances
the product, not just the marketing.

---

## 9. Open decisions (need a call before Phase A)

1. **Oracle crate**: `astro-float` vs `dashu-float`? Both pure-Rust;
   pick one for the dev-dependency. (Recommend `astro-float` — simpler
   constant/elliptic API.)
2. **Ship the measurement API publicly now, or keep internal until the
   Inquiry panel slice?** (Recommend public — it's stable math.)
3. **Bulge-arc handling in polyline area/perimeter**: implement true
   bulge arc-length/area now, or document "straight-segment approximation
   until bulge math lands" (consistent with today's `Polyline::length`)?
4. **Report cadence**: regenerate `Accuracy_Report.html` every release,
   or only on demand?
