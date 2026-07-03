#!/usr/bin/env bash
# Convenience wrapper: run the built dwgconv against the local .NET install.
#   dwgconv.sh <in.(dwg|dxf)> <out.(dwg|dxf)>
# Use from the import sandbox as:
#   --converter "/abs/path/tools/dwgconv/dwgconv.sh {in} {out}"
set -euo pipefail
# Resolve symlinks so this works when invoked via PATH or a symlink.
here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
dotnet_root="${DOTNET_ROOT:-$HOME/.dotnet}"
export DOTNET_ROOT="$dotnet_root"
exec "$dotnet_root/dotnet" "$here/bin/Release/net10.0/dwgconv.dll" "$@"
