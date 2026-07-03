// dwgconv — DWG↔DXF converter for the RUST_CAD import sandbox.
//
//   dwgconv <in> <out>
//
// Direction is inferred from extensions:
//   in.dwg  out.dxf   →  read DWG  (AC1014..AC1032), write DXF   (the import path)
//   in.dxf  out.dwg   →  read DXF, write DWG                      (export path)
//   in.dwg  out.dwg / in.dxf out.dxf  →  re-save (version normalise)
//
// Built on ACadSharp (MIT, pure C#). RUST_CAD cannot link C#, so it invokes
// this as an external process: DWG → DXF → cad_io::dxf::read_dxf.

using ACadSharp;
using ACadSharp.IO;

if (args.Length < 2)
{
    Console.Error.WriteLine("usage: dwgconv <in.(dwg|dxf)> <out.(dwg|dxf)>");
    return 1;
}

string inPath = args[0];
string outPath = args[1];
string inExt = Path.GetExtension(inPath).ToLowerInvariant();
string outExt = Path.GetExtension(outPath).ToLowerInvariant();

try
{
    // ---- read --------------------------------------------------------
    CadDocument doc;
    if (inExt == ".dwg")
    {
        using var r = new DwgReader(inPath);
        doc = r.Read();
    }
    else if (inExt == ".dxf")
    {
        using var r = new DxfReader(inPath);
        doc = r.Read();
    }
    else
    {
        Console.Error.WriteLine($"dwgconv: unknown input extension '{inExt}'");
        return 2;
    }

    // ---- write -------------------------------------------------------
    if (outExt == ".dxf")
    {
        using var w = new DxfWriter(outPath, doc, false);   // false = ASCII DXF
        w.Write();
    }
    else if (outExt == ".dwg")
    {
        using var w = new DwgWriter(outPath, doc);
        w.Write();
    }
    else
    {
        Console.Error.WriteLine($"dwgconv: unknown output extension '{outExt}'");
        return 2;
    }

    Console.Error.WriteLine($"dwgconv: {inExt} → {outExt}  ok");
    return 0;
}
catch (Exception ex)
{
    Console.Error.WriteLine($"dwgconv: FAILED: {ex.Message}");
    return 3;
}
