# How LibreCAD Computes Intersections

*Research note for RUST_CAD math lab.*
*Source studied: LibreCAD 2.2.2 — `/home/HSI/workspace/Librecad/LibreCAD-master/`.*
*Date captured: 2026-05-20. LibreCAD is no longer an active reference; this file preserves the findings.*

---

## Why this matters

Before designing our own intersection routines, we wanted to see how a mature C++ CAD codebase organises this. The result is a study of one specific approach, not a recommendation to copy it. Many design choices below are worth carrying over; some are explicitly worth replacing.

---

## 1. The dispatcher — one entry point for every pair

All intersection work funnels through **one function**:

> `RS_VectorSolutions RS_Information::getIntersection(RS_Entity const* e1, RS_Entity const* e2, bool onEntities)`
> Location: `librecad/src/lib/information/rs_information.cpp:454`

It inspects each entity's runtime type (`e->rtti()`) and routes to a specialised solver. The routing tree:

```
                  getIntersection(e1, e2)
                          │
        ┌─────────────────┼───────────────────────┬──────────────────┐
        ▼                 ▼                       ▼                  ▼
  Line × Line     Arc × Arc (incl. circle)   Line × Arc        everything else
        │               │                       │              (fallback)
        ▼               ▼                       ▼                  ▼
 specialised        specialised             specialised      LC_Quadratic
  routine            routine                 routine         generic conic
                                                                  │
                                                                  ▼
                                                          if still empty:
                                                          TangentFinder (last
                                                          resort, bisection)
```

The `onEntities` flag decides whether to filter the discrete crossings to those that lie on each entity's **range** (segment endpoints, arc angle window) vs. the entity's infinite/full curve.

### Return type — one container for 0…N solutions

`RS_VectorSolutions` (in `librecad/src/lib/engine/rs_vector.h:182`) is a `std::vector<RS_Vector>` plus a single `bool tangent`. The same class represents:

- **0 solutions**: empty vector.
- **1 solution (tangent)**: one element + `tangent = true`.
- **1 solution (crossing)**: one element + `tangent = false`.
- **2 solutions**: two elements.
- **N solutions (ellipse-ellipse, quartic root)**: up to 4 elements.

The advantage is uniformity — every solver returns the same type, and the caller never special-cases tangent vs. cross.

---

## 2. Line–Line

**Method:** 2D cross-product / Cramer's rule on the parametric form
`p1 + u·(p2−p1) = p3 + v·(p4−p3)`.

```cpp
double num = ((p4.x-p3.x)*(p1.y-p3.y) - (p4.y-p3.y)*(p1.x-p3.x));
double div = ((p4.y-p3.y)*(p2.x-p1.x) - (p4.x-p3.x)*(p2.y-p1.y));
```

Parallelism is checked **twice**: once on the denominator `|div| > RS_TOLERANCE` and once on the angle difference `mod π`. Both guards must trigger before falling through to the parallel-line path. This catches the case where two nearly-parallel lines have a small but non-zero `div` that would yield a wildly large `u = num/div`.

Lines are treated as **infinite** at this stage. Segment-endpoint enforcement happens afterwards in the dispatcher via `isPointOnEntity()`. The same code therefore serves both `RS_Line` and `RS_ConstructionLine` (infinite line).

Zero-length / coincident-point degeneracies are handled by an explicit fall-through that projects one endpoint onto the other line.

---

## 3. Line–Circle and Line–Arc

**Method:** project the circle centre onto the line and use the chord-distance geometry.

Let `dP = projection_of_centre_onto_line − centre`. Then:
- `|dP| > r`: line misses the circle.
- `|dP| ≈ r`: tangent (single hit at the projection).
- `|dP| < r`: two hits, symmetric around the projection, at distance `±√(r² − |dP|²)` along the line.

```cpp
const double dr  = dP.magnitude() - r;
const double tol = 1e-5 * r;
if (dr >  tol) return {};
if (dr < -tol) {
    const double dt   = std::sqrt(r*r - dP.squared());
    const RS_Vector dT = d * (dt / d.magnitude());
    return { projection + dT, projection - dT };
}
// tangent
return RS_VectorSolutions{projection}; // tangent flag set after
```

Two interesting details:

1. **Orthogonality re-projection** `dP -= d * (d.dotP(dP) / d2)` — after the initial perpendicular-foot computation, this subtracts any residual tangential component. Pure numerical hygiene; prevents long-line × tight-circle cases from drifting.
2. **Tangent threshold is relative to radius** (`1e-5 * r`). Practical but heuristic — for a degenerate `r ≈ 1e-15` the tangent band would exceed the radius itself.

**Arc filtering** is NOT done in this routine. After the dispatcher gets the line-circle hits, it calls `isPointOnEntity()` on each, which for an arc invokes `RS_Math::isAngleBetween()` to check the swept-angle window. That function handles CW arcs via a `reversed` flag (swap start/end and reuse the CCW arithmetic) and the 0/2π discontinuity via a two-clause comparison.

---

## 4. Circle–Circle

**Method:** classic d/s/h decomposition along the line joining the centres.

```cpp
RS_Vector u = c2 - c1;                  // centre-to-centre
if (u.magnitude() < 1e-7*(r1 + r2)) return {};   // concentric

auto v = RS_Vector{u.y, -u.x};          // perpendicular to u
double s    = 0.5 * ((r12 - r22)/u.squared() + 1.0);   // along-u fraction
double term = r12/u.squared() - s*s;                   // (h/|u|)²
if (term < -RS_TOLERANCE) return {};                   // no intersection

double t1 = std::sqrt(std::max(0., term));
RS_Vector sol1 = c1 + u*s + v*t1;
RS_Vector sol2 = c1 + u*s - v*t1;
```

Three early-exit guards cover every degenerate case:

| Condition | Geometric meaning |
|---|---|
| `|u| < 1e-7·(r₁+r₂)` | concentric → no intersection |
| `term < −RS_TOLERANCE` | one circle entirely inside or outside the other |
| `|sol1 − sol2| < 1e-5·(r₁+r₂)` | tangent → collapse to single point + tangent flag |

---

## 5. Arc–Arc and Arc–Circle

There is **no separate arc-arc solver** — they share `getIntersectionArcArc`, which is literally the circle-circle code above (it only reads `getCenter()` and `getRadius()` from each entity). Arc angle filtering happens later in the dispatcher.

The takeaway: **"arc intersection" is circle intersection plus angle filtering**. The same predicate (`isAngleBetween`) gets applied to each candidate point against each of the two arcs.

This is exactly the approach we use in `cad_kernel/src/intersect.rs`.

---

## 6. Ellipses — where it stops being trivial

Two conics intersect in up to **4 points**, so finding them needs a **quartic** equation.

### Line–Ellipse
Rotate the line into the ellipse's principal axes, substitute the parametric line into `x²/rx² + y²/ry² = 1`, and you get a quadratic in `t`. The discriminant gives 0, 1 or 2 hits.

### Ellipse–Ellipse
LibreCAD reduces both ellipses to a 6-coefficient general-conic form and calls a custom `simultaneousQuadraticSolver`. Internally this does **Sylvester resultant elimination** of `x` to produce a quartic in `y`, then solves with Ferrari-style code.

The known hazards in this approach:
- The resultant multiplies coefficients pairwise → can cancel catastrophically for nearly-parallel conics.
- No normalisation step scales coefficients to `[-1, 1]` first — large or tiny ellipses lose precision.
- The mitigations (radical-axis pre-reduction in `LC_Quadratic`) help but aren't bulletproof.

**Lesson:** if RUST_CAD ever needs ellipse-ellipse, lean on a vetted polynomial-root library (e.g. the `roots` crate), or use a double-double precision intermediate. Don't roll Ferrari from scratch.

---

## 7. Polylines and splines

A polyline is a list of sub-entities (lines + arcs with bulges). Polyline intersection just iterates the sub-entities and dispatches each one through the normal `getIntersection`.

Splines are interesting: LibreCAD does **not** flatten them to line segments for math purposes. `LC_SplinePoints` walks the spline's quadratic Bézier segments one at a time and feeds each one to a specialised solver. For line-vs-spline, each Bézier segment (built from three consecutive control points) is intersected analytically. For spline-vs-other, the generic `LC_Quadratic` solver is invoked per segment.

This preserves curve precision but costs `O(segments × m)` where m is the per-segment solver cost.

---

## 8. Precision strategy

### Global constants — `librecad/src/lib/engine/rs.h:38`

| Constant | Value | Purpose |
|---|---|---|
| `RS_TOLERANCE`        | `1e-10` | linear distances; denominators |
| `RS_TOLERANCE2`       | `1e-20` | **squared** distances (no `sqrt`) |
| `RS_TOLERANCE_ANGLE`  | `1e-8`  | radian angle differences |
| `RS_TOLERANCE15`      | `1.5e-15` | very tight degenerate guards |

Having distinct constants for *distance*, *squared distance*, and *angle* is cleaner than one global `EPS`. Squared-distance comparisons avoid `sqrt` and are used everywhere they can be.

### Local heuristics in solvers

| Threshold | Where | Comment |
|---|---|---|
| `1e-5 * r`        | line-circle tangent  | scaled to radius |
| `1e-5 * (r₁+r₂)`  | circle-circle tangent | scaled to total size |
| `1e-7 * (r₁+r₂)`  | concentric test | tighter than the tangent test |

These local thresholds are *relative* to the entity scale — better than absolute tolerances for varied drawings, but the multiplier (`1e-5`, `1e-7`) is unjustified. No derivation in code or comments.

Results are **never snapped** — what the float math produces is what you get. The only post-processing is the optional `isPointOnEntity()` filter and the last-resort `TangentFinder` heuristic.

---

## 9. Worth carrying over vs. worth replacing

### Carry over (mentally)

- **Unified return type** with `points + tangent_flag`. We may extend ours to add an `overlaps` channel separately.
- **Tiered tolerance constants** (distance / squared / angle).
- **Hand-written analytic solvers for the common shapes, generic conic solver as fallback.** Specialised wins on precision and speed; fallback covers everything else.
- **Arc filtering as a separate predicate**, reused by every arc-related solver. Don't duplicate angle-in-arc logic.
- **The orthogonality re-projection trick** in line-circle. Cheap numerical hygiene.
- **`TangentFinder` offset-bisection** as a last-resort tangent-recovery pass.

### Replace / redesign

- **Segments vs. infinite lines are conflated.** A clear `Segment` / `Ray` / `Line` distinction in the type system would be cleaner than a runtime flag.
- **Collinear-overlapping segments silently return nothing.** Their information is real and should be surfaced through an explicit `Overlap(...)` channel.
- **Magic-number tangent thresholds.** Replace `1e-5 * r` with `k * eps_relative * length_scale` and document `k`.
- **Ellipse-ellipse coefficient scaling.** Normalise to `[-1, 1]` before quartic solving.
- **Two-phase "is this on the entity" filtering** does the work twice. Fold the angle filter into the arc solver and you save a function call per hit.

---

## Direct quotes (verbatim from the source)

These are the critical 5–25 line excerpts, preserved in case the LibreCAD tree is ever wiped or relocated.

### Line-line core (rs_information.cpp:592–625)

```cpp
RS_Vector p1 = e1->getStartpoint();
RS_Vector p2 = e1->getEndpoint();
RS_Vector p3 = e2->getStartpoint();
RS_Vector p4 = e2->getEndpoint();

double num = ((p4.x-p3.x)*(p1.y-p3.y) - (p4.y-p3.y)*(p1.x-p3.x));
double div = ((p4.y-p3.y)*(p2.x-p1.x) - (p4.x-p3.x)*(p2.y-p1.y));

const double dAngle = static_cast<const RS_Line*>(e1)->getAngle1()
                    - static_cast<const RS_Line*>(e2)->getAngle1();
if (std::abs(div) > RS_TOLERANCE &&
    std::abs(std::remainder(dAngle, M_PI)) >= RS_TOLERANCE_ANGLE) {
    double u  = num / div;
    double xs = p1.x + u * (p2.x - p1.x);
    double ys = p1.y + u * (p2.y - p1.y);
    return { RS_Vector{xs, ys} };
}
```

### Line-circle core (rs_information.cpp:670–698)

```cpp
RS_Vector projection = line->getNearestPointOnEntity(c, false, &dist);
RS_Vector dP = projection - c;
dP -= d * (d.dotP(dP) / d2);                    // re-orthogonalise
projection = c + dP;

const double dr  = dP.magnitude() - r;
const double tol = 1e-5 * r;
if (dr >  tol) return {};
if (dr < -tol) {
    const double dt   = std::sqrt(r*r - dP.squared());
    const RS_Vector dT = d * (dt / d.magnitude());
    return RS_VectorSolutions({ projection + dT, projection - dT });
}
RS_VectorSolutions ret{projection};
ret.setTangent(true);
return ret;
```

### Circle-circle core (rs_information.cpp:720–753)

```cpp
RS_Vector u = c2 - c1;
if (u.magnitude() < 1e-7*(r1 + r2)) return {};

auto v = RS_Vector{u.y, -u.x};
double s    = 0.5 * ((r12 - r22)/u.squared() + 1.0);
double term = r12/u.squared() - s*s;
if (term < -RS_TOLERANCE) return {};

double t1 = std::sqrt(std::max(0., term));
RS_Vector sol1 = c1 + u*s + v*t1;
RS_Vector sol2 = c1 + u*s - v*t1;

if (sol1.distanceTo(sol2) < 1e-5*(r1+r2)) {
    RS_VectorSolutions ret{sol1};
    ret.setTangent(true);
    return ret;
}
return {sol1, sol2};
```

### Angle-in-arc predicate (rs_math.cpp:149)

```cpp
bool RS_Math::isAngleBetween(double a, double a1, double a2, bool reversed = false) {
    if (reversed) std::swap(a1, a2);
    if (getAngleDifferenceU(a2, a1) < RS_TOLERANCE_ANGLE)
        return true;                              // full circle
    const double tol   = 0.5 * RS_TOLERANCE_ANGLE;
    const double diff0 = correctAngle(a2 - a1) + tol;
    return diff0 >= correctAngle(a - a1) || diff0 >= correctAngle(a2 - a);
}
```

---

*End of note.*
