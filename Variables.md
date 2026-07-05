# RUST_CAD — Variables (RETIRED → see SETTINGS.md)

> **This file is retired.** The variable system is now driven by the registry
> `cad_app/src/varreg.rs` (single source of truth), and the full reference —
> architecture, status badges, command-line usage, the settings page, the
> 240-variable catalog, **and the detailed per-variable briefings** — lives in:
>
> ### → [`SETTINGS.md`](SETTINGS.md)
>
> What used to be here was folded into `SETTINGS.md`:
> - the AutoCAD-SYSVAR catalog → the generated "Full catalog (240)" tables
> - the implemented-vs-awaiting status → the `Status`/`wired` columns + §12
>   "Detailed briefings"
> - the "run the same" code-derived status check → `SETTINGS.md` §13
> - the hardcoded-values-to-promote list → the "Code-Audit Hardcoded" section + §14
>
> To change a variable, edit `varreg.rs` (and `settings.rs`/`app.rs` if wiring
> it) and regenerate `SETTINGS.md`'s tables — see `SETTINGS.md` §9–§10. Do not
> add new content here.
