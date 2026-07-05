# dwgconv — DWG↔DXF converter (ACadSharp) for the RUST_CAD import sandbox

RUST_CAD reads **DXF** natively but has **no DWG reader** (DWG is a closed,
reverse-engineered binary format). DWG support is therefore handled as a
**conversion step**, not a linked library:

```
DWG ──(dwgconv)──▶ DXF ──▶ cad_io::dxf::read_dxf ──▶ Document
```

`dwgconv` is a tiny C# program built on **[ACadSharp](https://github.com/DomCR/ACadSharp)**
(MIT, pure managed C#). ACadSharp reads DWG **AC1014 → AC1032** (AutoCAD R14
through **2018+**) and writes several versions — a superset of LibreCAD's
bundled libdxfrw (which tops out at AC1027 / 2017 and cannot write DWG).

Because ACadSharp is C#, it can't be linked into RUST_CAD (Rust) or LibreCAD
(C++). We run it as a **separate process** — the cleanest, most robust bridge
(process isolation; a parser crash can't take down the app).

## Prerequisites — install the .NET SDK

Not currently installed on this machine. On Arch (package `dotnet-sdk`, ~110 MiB,
currently 10.x — the project targets `net10.0` to match):

```bash
sudo pacman -S dotnet-sdk
dotnet --version   # expect 10.x
```

## Build a portable binary

```bash
cd ~/workspace/RUST_CAD/tools/dwgconv
dotnet publish -c Release -r linux-x64 --self-contained -o dist
# → dist/dwgconv  (self-contained: no .NET install needed to RUN it)
```

(During development you can just `dotnet run -- in.dwg out.dxf`.)

## Use it with the import sandbox

```bash
cd ~/workspace/RUST_CAD
cargo run -p cad_io --example import_sandbox -- \
    "/home/HSI/Downloads/PROPOSED LIGHTING LAYOUT- ANNEX VILLA.dwg" \
    --converter "$PWD/tools/dwgconv/dist/dwgconv {in} {out}" \
    --out annex.rsm
```

The sandbox converts the DWG to DXF, parses it, reports which entities survived
vs were dropped by RUST_CAD's reader, and (with `--out`) writes an `.rsm` you can
open in the app.

## Notes / limitations

- ACadSharp **read** range: AC1014–AC1032. **Write** range: AC1014/1015/1024/1027/1032.
- DWG is reverse-engineered — exotic entities may not round-trip perfectly.
  The sandbox's drop report is the source of truth for "what made it in".
- NativeAOT (`-p:PublishAot=true`) yields a smaller standalone binary but
  ACadSharp's reflection use may need trimming hints — start with the
  self-contained publish above; treat AOT as an optimisation.
- This is a **sandbox/testbed tool**, intentionally outside the Cargo workspace
  (`tools/`, not a workspace member) so the C#/.NET toolchain never touches the
  Rust build.
