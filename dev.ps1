# Auto-rebuild + relaunch the CAD app on every source change.
#
# On each save, cargo-watch stops the running app, rebuilds, and relaunches it,
# so you never have to close/reopen the window yourself. Stop with Ctrl+C.
#
# Requires cargo-watch:  cargo install cargo-watch
#
# Usage:  .\dev.ps1            (debug build — fast rebuilds, recommended)
#         .\dev.ps1 -Release   (optimized build — slower rebuilds)
param([switch]$Release)

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Set-Location $PSScriptRoot

if ($Release) {
    cargo watch -c -x "run -p cad_app --release"
} else {
    cargo watch -c -x "run -p cad_app"
}
