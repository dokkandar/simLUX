# π, Length, and Curve Representation in CAD Math

*Research note for RUST_CAD math lab.*
*Date: 2026-05-20.*

---

## The two questions

> What is the value of π that intersection and length calculations actually use?
> Are curves treated as many short line segments, or computed differently?

Short answer to both:

1. **π is a single 64-bit floating-point constant** — `std::f64::consts::PI = 3.141592653589793` — accurate to about 15–16 decimal digits. The `sin`/`cos` library functions internally use a higher-precision π for argument reduction.
2. **For the simple shapes (line / circle / arc / ellipse), curves are NOT discretised**. Their lengths and intersections are closed-form analytic expressions. Discretisation only happens (a) on the screen during rendering, and (b) for genuinely non-analytic curves like splines and Bézier paths, where it's a numerical-integration choice, not a representation choice.

The rest of this note unpacks both points and what they mean for RUST_CAD.

---

## 1. What is π, exactly?

### As a floating-point number

In `cad_kernel/src/math.rs` we use `std::f64::consts::PI` and `std::f64::consts::TAU = 2π`. Both are compile-time constants built into the Rust standard library:

```
PI  = 3.141592653589793
TAU = 6.283185307179586
```

These are the closest representable `f64` values to π and 2π. The true value of π differs from the stored value by roughly `1.2 × 10⁻¹⁶` (about half an ULP — unit in the last place). For all CAD-scale geometry, this is utterly negligible: even at a 100-km drawing, half an ULP of π corresponds to a position error of ~1.2 × 10⁻¹¹ metres.

### Inside the trig functions

When you call `theta.cos()` or `theta.sin()`, the math library doesn't actually use `f64::PI`. It uses a much higher-precision π (typically split into two `f64`s, giving ~31 decimal digits) for *argument reduction* — the step that maps any input angle into `[-π/4, +π/4]` before computing the polynomial approximation. This matters for very large angles (e.g. `(1e15).sin()`); for the angles a CAD app typically sees (0 to ~10·2π), it's already much more accurate than `f64` can represent.

**Implication for RUST_CAD:** we never need to think about π precision. The bottleneck in our math is always somewhere else — accumulation of rounding errors over many operations, or catastrophic cancellation in expressions like `(a − b)` where `a ≈ b`.

---

## 2. Are curves stored as line segments?

### Lines and circles and arcs: NO

In our kernel, the types are:

```rust
pub struct Line   { pub a: Vec2, pub b: Vec2 }
pub struct Circle { pub center: Vec2, pub radius: f64 }
pub struct Arc    { pub center: Vec2, pub radius: f64,
                    pub start_angle: f64, pub sweep_angle: f64 }
```

Each is a handful of `f64` values. **No discretisation.** A circle of radius 10 stored as `(0, 0, 10)` represents the *exact* mathematical circle, not a 100-sided polygon approximation of it.

### What this means for *length*

Closed-form formulas, no integration, no summing of small pieces:

| Shape | Length |
|---|---|
| Line segment | `(b − a).len()` |
| Circle (circumference) | `2 · π · r` |
| Arc | `r · sweep_angle` *(when `sweep_angle` is in radians)* |

That last one is the reason "arcs are sized in radians" matters. The arc length formula `s = r·θ` is *only* clean when θ is in radians. In degrees it becomes `s = r·θ·π/180`, which costs one extra multiplication and introduces an extra rounding step. Internally we store radians; the parser converts degrees to radians once at input time and the rest of the system stays in radians.

### What this means for *intersection*

The intersection routines we've already implemented operate on the analytic representation:

- **Line-line** — Cramer's rule on a 2×2 system of two parametric line equations. Three multiplications, two subtractions, one division. No iteration.
- **Line-circle** — substitute the parametric line into `|p − c|² = r²`, get a quadratic in `t`, take the discriminant. No iteration.
- **Circle-circle** — geometric construction along the centre-to-centre vector. No iteration.
- **Arc filtering** — after computing the underlying circle intersection, test each hit's angle against the arc's angular window. Still no iteration.

None of these treat the curve as a polyline. They all use the closed-form mathematical description.

### Why this matters for accuracy

If you *were* to approximate a circle as a 360-sided polygon and intersect it with a line, you would find a point on the **polygon**, not on the true circle. For a typical CAD precision target of 10⁻⁹ units, you'd need on the order of 100,000 polygon sides per circle to even get close — and you'd still be wrong in the last few digits, because polygon corners aren't smooth.

Analytic intersection gives you a point on the **true** circle, with error bounded only by `f64` arithmetic (~10⁻¹⁶ relative).

---

## 3. Where discretisation *does* happen

There are two legitimate places where curves become line segments:

### A. Rendering to a screen

In `cad_app/src/app.rs`, the function `draw_entity` approximates an arc with a polyline before handing it to egui:

```rust
let r_px = (a.radius as f32 * app.scale).max(1.0);
let n = ((r_px * 0.5).clamp(8.0, 256.0)) as usize;
let mut pts = Vec::with_capacity(n + 1);
for i in 0..=n {
    let t = a.start_angle + (i as f64 / n as f64) * a.sweep_angle;
    let p = Vec2::new(
        a.center.x + a.radius * t.cos(),
        a.center.y + a.radius * t.sin(),
    );
    pts.push(app.w2s(p, rect));
}
```

This is **purely visual**. The polyline only exists for the duration of one frame, is rebuilt each repaint, and never feeds back into the kernel. The error is bounded by pixel size — at `n` segments per arc, the worst-case sagitta is `r · (1 − cos(θ/2n)) ≈ r·θ²/(8n²)`. With our `n ∝ pixel radius`, the visible error is fractional pixels.

A more rigorous renderer would compute `n` from a *sagitta tolerance* (subdivide until the chord is within e.g. 0.25 px of the true arc). For CAD-grade plotting this is the standard approach. Egui paints circles natively (`circle_stroke`), so for those we skip discretisation entirely; only arcs need the polyline.

### B. Curves without a closed-form solution

Some shapes genuinely don't have an analytic length or intersection routine, and there numerical methods (which always discretise *something*) are unavoidable:

| Curve | Length | Closed-form? |
|---|---|---|
| Line | `|b − a|` | yes |
| Circular arc | `r · θ` | yes |
| **Elliptical arc** | `∫₀^θ √(a²sin²t + b²cos²t) dt` | **no** — incomplete elliptic integral of the second kind |
| **Quadratic Bézier** | `∫₀¹ |B′(t)| dt` | yes but messy (closed-form with `asinh` for special cases, numerical in general) |
| **Cubic Bézier / NURBS** | as above | no closed form |

For these, the integral is evaluated numerically. Common methods:

- **Gauss–Legendre quadrature** — sample the integrand at a small set of carefully-chosen points (often 5–10), weight and sum. Highly accurate for smooth integrands; standard in vetted libraries.
- **Adaptive subdivision** — split the curve until each piece is "straight enough" (chord ≈ control-polygon), then sum chord lengths. Slower but robust for curves with sharp features.
- **Romberg integration** — repeated trapezoid-rule refinements with Richardson extrapolation.

For *intersection* of these curves, the equivalent toolkit is:

- **Bézier clipping** (Sederberg & Nishita 1990) — recursive subdivision using bounding-box overlap to prune.
- **Polynomial root-finding** — convert both curves to implicit polynomial form, eliminate one variable to get a univariate polynomial, solve it. This is what LibreCAD does for ellipse-ellipse, and it's where most precision is lost.

All of these are **fundamentally different from "treating the curve as N line segments"** — they're principled numerical methods with provable convergence and error bounds, not crude approximations.

---

## 4. Summary table — what RUST_CAD does today, and why

| Quantity | Method | Discretised? | Error source |
|---|---|---|---|
| `line.length()` | `(b-a).len()` | no | f64 rounding (~16 digits) |
| `circle.circumference()` (future) | `2.0 * PI * r` | no | f64 rounding of `2πr` |
| `arc.length()` (future) | `r * sweep_angle` | no | f64 rounding of one multiplication |
| `intersect_line_line` | Cramer's rule | no | catastrophic cancellation in `denom` for near-parallel lines |
| `intersect_line_circle` | quadratic discriminant | no | discriminant near zero loses precision for tangents |
| `intersect_circle_circle` | d/s/h decomposition | no | similar tangent-precision issue |
| `draw_entity(arc)` on screen | 8–256 polyline segments | YES (visual only) | < 1 px sagitta |
| ellipse / spline (future) | numerical integration / Bézier clipping | partially | quadrature order, subdivision tolerance |

**The principled stance:** treat each shape with the most exact tool you have, and accept numerical methods only when no closed form exists. Don't be tempted to "just polygonise everything" — it would simplify the code at the cost of an unrecoverable factor of 10⁴–10⁶ in precision.

---

## 5. Practical consequences for the math lab

1. **Stay in radians internally.** Convert degrees on input/output only. Halves the number of multiplications by π/180 throughout the engine.
2. **Use `TAU` not `2 * PI`.** `f64::consts::TAU` is the exact-rounded constant for 2π; computing `2.0 * PI` adds one rounding step. Trivial but free.
3. **When adding new analytic shapes** (ellipse, ellipse-arc), follow the same pattern: store the analytic parameters, write closed-form length / intersection. Only fall back to numerics for ellipse-ellipse intersection (quartic) or ellipse arc length (incomplete elliptic integral).
4. **When adding splines or arbitrary curves**, build a `Curve` trait with `bbox()`, `evaluate(t)`, `derivative(t)`, and intersect via Bézier clipping. Use Gauss-Legendre (with order 7 or 10) for length.
5. **Never let the renderer feed back into the math.** The visual polyline is a one-way, throwaway artefact. If it ever leaks into intersection or snap logic, accuracy collapses.

---

*End of note.*
