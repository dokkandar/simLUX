# Background Operations Pattern

**Status:** vital architectural rule, established 2026-06-05.
**Origin:** Hatch trace worker (`HatchWorker` + `HatchWorkerResult`,
commit `0c8683d`). First real implementation is in
[`cad_app/src/app.rs`](cad_app/src/app.rs) — search for
`spawn_hatch_worker` / `poll_hatch_worker`.

This document is the canonical reference for how RUST_CAD handles any
operation that might exceed a frame budget (~16 ms). Any heavy
command — hatch trace, intersect-all, region boolean, save/load,
massive trim/extend, future spatial-index rebuild — MUST follow this
shape so the UI stays responsive and Esc actually cancels.

---

## 1. The one-paragraph recipe

> Every heavy command splits into **PURE** (reads the doc, computes,
> never writes) and **APPLY** (one-frame on the main thread, mutates
> `self.doc` with the result). PURE goes on a worker thread that
> reads a snapshot of the doc; APPLY runs from `poll_*_worker()` on
> the main thread when the worker reports Success. Cancel = drop the
> result without ever entering APPLY. **No save/restore is needed —
> the doc was never modified during PURE.**

---

## 2. The three safety invariants

These three properties guarantee Esc just works for any heavy op. If
the implementation violates any of them, the lifecycle becomes
fragile and you'll fight race conditions.

### I-1. Single-writer invariant

`self.doc` is mutated by ONE thread only — the main UI thread — and
ONLY inside the `Success` branch of the per-op `poll_*_worker()`.
Workers never write to `self.doc`; they write to their own snapshot
or produce a result value that the main thread applies.

### I-2. Snapshot at spawn

The worker either takes a `Document::clone()` (cheap O(N) at spawn
time) OR borrows the doc through an `Arc<Document>` with read-only
semantics. Edits the user makes mid-op stay on the live doc and DO
NOT affect the worker. When the worker returns, its result is based
on the snapshot's geometry — accept this, or detect & restart.

### I-3. Cancel = drop the result

On `Cancelled`, `poll_*_worker()` does nothing to `self.doc`. There
is no rollback because there is nothing to roll back. The cancel
flag's only job is to make the worker exit faster; correctness
doesn't depend on it firing within any particular deadline.

---

## 3. The generalised abstraction (sketch)

Today we have exactly one backgrounded op (hatch trace). When a
second one lands, refactor to:

```rust
trait BackgroundOp {
    type Args:   Send + 'static;   // input (seed, delta, cutter set, …)
    type Result: Send + 'static;   // pure output (loops, new positions, edges, …)

    fn name() -> &'static str;     // for prompt + debug

    /// PURE: runs on the worker thread, reads doc_snapshot, never writes.
    fn run(
        doc_snapshot: Document,
        args:         Self::Args,
        cancel:       Arc<AtomicBool>,
    ) -> WorkerOutcome<Self::Result>;

    /// APPLY: runs on the main thread inside poll_active_op's Success branch.
    fn apply(app: &mut CadApp, result: Self::Result);
}

enum WorkerOutcome<R> {
    Success(R,        Vec<String>),    // result + log lines accumulated by worker
    Failure(String,   Vec<String>),    // reason + log
    Cancelled(/* */    Vec<String>),    // log only
}

// One generic handle on CadApp:
struct ActiveOp<R> {
    name:   &'static str,
    cancel: Arc<AtomicBool>,
    rx:     mpsc::Receiver<WorkerOutcome<R>>,
}
```

CadApp ends up with at most a couple of `Option<ActiveOp<...>>`
fields (one per result type) instead of a zoo of per-op structs.

**DO NOT extract this abstraction until at least two heavy ops use
the pattern.** Two examples justify it; one doesn't.

---

## 4. Which commands actually need backgrounding

Operations split cleanly into two cost classes. Most are O(N) and
DON'T need a worker.

| Class | Examples | Backgrounding? |
| --- | --- | --- |
| **O(1) per-click drafting** | line / circle / arc / polyline / spline vertices | No |
| **O(N) per-dobject loops** | move / copy / rotate / scale / mirror / change-layer / change-color / delete-selected | **No.** At 1M dobjects, 10–100 ms. Frame stutter, not freeze. |
| **O(N·k) bounded** | array (N·count where count<1000), trim/extend with small cutter sets | **No.** Same regime. |
| **O(N²) pairwise** | intersect-all, **hatch trace split**, region boolean, fillet-all, smart trim with "all-as-cutters" | **YES.** At 1M dobjects = 10¹² ops. Use the pattern. |
| **O(N) IO-bound** | save big drawing, DXF import/export, PDF export | **YES** when files are large or disk is slow. The worker does `fs::write` instead of geometry math; same pattern. |
| **Interactive live preview** | drag-to-move, drag-to-scale, rubber-band selection | **No — stays sync.** Each preview frame is cheap (just transform under matrix). The FINAL apply might background if N is huge. |

### Today's shortlist of ops that genuinely need this:

1. **Hatch trace** — done (`HatchWorker`)
2. **Spatial-index broad-phase + the O(N²) split scan** — once we hit
   a doc big enough that even the broad-phase exceeds ~100 ms,
   background that with the same pattern.
3. **Intersect-everything** — already O(N²); the `∩ view` command is
   the natural next worker.
4. **Region boolean ops** (when Region entity lands) — same shape.
5. **DXF / PDF import-export** (when `cad_io::dxf` exists) — same
   shape, IO-bound instead of CPU-bound.

Everything else stays synchronous and is fine.

---

## 5. The interactive-vs-final split (for drag commands)

Drag-style operations like move/scale/rotate have TWO phases. Handle
them separately:

- **During drag:** live preview ghosts the selection at the cursor.
  Render-side only — it does not touch the kernel data. Always
  synchronous, always cheap (~ms per frame regardless of N).
- **On release (final apply):** if N is small, sync apply is fine.
  If N is huge, spawn a worker that produces
  `Vec<(dobject_idx, new_geom)>` and apply on the main thread when
  ready.

So the user never notices — preview is responsive; the actual
data-write may take a moment with an Esc-able "applying move…"
prompt at the extreme end.

---

## 6. The 30-second mental model

```
T₀  user clicks → spawn_worker():
       doc_snapshot = self.doc.clone()
       cancel       = Arc<AtomicBool>(false)        ← shared
       thread::spawn(move || {
           let result = run_pure(&doc_snapshot, args, &cancel);
           tx.send(result);
       });
       store rx on CadApp; return immediately.

T₀..Tₙ  every frame, update() runs:
        poll_worker():
            try_recv():
                Empty       → return (worker still running)
                Ok(Success) → APPLY: push results to self.doc
                Ok(Failure) → log / fall back to a cheap path
                Ok(Cancelled) → log

Tₖ  user presses Esc:
       op_cancel.store(true)      ← single atomic write, instant
       Esc handler returns; UI keeps spinning.

       Worker thread (in background): inside CANCEL_CHECK_STRIDE loop
       sees cancel == true → sends Cancelled → exits.

Tₖ₊₁  next frame: poll_worker receives Cancelled → log only.
       self.doc is byte-identical to T₀ (worker never wrote).
```

---

## 7. Cancel-check overhead is negligible

The worker does `cancel.load(Ordering::Relaxed)` every
`CANCEL_CHECK_STRIDE = 256` inner iterations. The load is ~1 ns; the
inner geometric work is ~50 ns per iteration. Cancel-check overhead
is well under 0.1 %. **Do not skip the check to "save cycles" — it
costs nothing and gives correctness.**

---

## 8. Common pitfalls (do not do these)

- **Mutate `self.doc` from inside the worker thread.** Compiler
  won't let you — Document isn't shared via mutex. If you find
  yourself wanting to, you're not following I-1.
- **Skip the snapshot and read `self.doc` through a borrow.** The
  borrow checker says no. But if you used `Arc<Document>` instead,
  you could — at the cost of locking down user edits while the worker
  runs. We chose snapshot precisely to avoid that lock.
- **Make the worker call back into the main thread.** No. The worker
  returns one result via the channel, then exits. The main thread
  decides what to do with it.
- **Restore the doc on cancel.** There is nothing to restore. If
  your code has a "rollback" path, you violated I-1.
- **Generalise after one example.** Wait for the second op. The
  current HatchWorker shape is fine as the prototype; copy-paste
  for the second op; THEN extract the trait. (Three is the magic
  number for abstraction, but two is the minimum.)
- **Cancel the worker's thread directly via `JoinHandle::abort`.**
  Doesn't exist in std. Use the cooperative flag — it's portable,
  drop-safe, and works on every platform.

---

## 9. Where this lives in the codebase today

| Piece | File | Notes |
| --- | --- | --- |
| `op_cancel: Arc<AtomicBool>` | `cad_app/src/app.rs` (CadApp field) | Single global cancel flag; set by Esc handler. Will become per-op when we add a second worker. |
| `HatchWorker`, `HatchWorkerResult` | `cad_app/src/app.rs` | Concrete prototype of the pattern. |
| `spawn_hatch_worker`, `poll_hatch_worker` | `cad_app/src/app.rs` | The spawn/poll pair. |
| Cancellable variants in `hatch_trace` | `cad_app/src/hatch_trace.rs` | `tessellate_doc_cancellable`, `split_at_intersections_cancellable`, `cluster_endpoints_cancellable`. CANCEL_CHECK_STRIDE = 256. |
| Esc handler | `cad_app/src/app.rs` (global Esc) | Sets `op_cancel`. |

---

## 10. Reference

- Hatch worker commit: `0c8683d` (2026-06-05).
- Cancellation primitive: `d8d2431` (2026-06-04).
- Bug Report covering the journey: [`Bug_Report_2026-06-04.html`](Bug_Report_2026-06-04.html).
- Daily Report mapping the day's work: [`Daily_Report_2026-06-04.html`](Daily_Report_2026-06-04.html).

**When you add the second backgrounded operation, update §3 to
reflect the extracted `BackgroundOp` trait and remove the "don't
generalise yet" warning.**
