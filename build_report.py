#!/usr/bin/env python3
"""
build_report.py — generate Plan_Report.html from the workspace's MD sources.

Reads:
  ROADMAP.md, Variables.md, Dobject_DXF.md, Dobject_Properties.md
Writes:
  Plan_Report.html — single-file, self-contained, engineer-friendly snapshot.

Run from the workspace root:
  python3 build_report.py
"""

import re
import html
import datetime
from pathlib import Path

ROOT = Path(__file__).parent
SOURCES = [
    ("roadmap",     "ROADMAP.md",            "Roadmap — What / Why / How"),
    ("variables",   "Variables.md",          "User-Environment Variables (SYSVARS)"),
    ("dxf",         "Dobject_DXF.md",        "Dobject ↔ DXF Group-Code Dictionary"),
    ("properties",  "Dobject_Properties.md", "Dobject Property Model (in-memory)"),
]

# Decision points / uncertainties we want flagged at the top.
OPEN_QUESTIONS = [
    {
        "topic": "Keep AtDlgM / AtPrmM / PkBxSz in Variables.md?",
        "status": "Tentative — kept for now",
        "detail": (
            "All three are legitimate AutoCAD SYSVARS (ATTDIA, ATTREQ, PICKBOX) "
            "already in the UserEnv code, but neither attribute system nor "
            "centralized hit-test exists in RUST_CAD yet. Marked '◌ Tentative' "
            "in <code>Variables.md</code> with TENTATIVE notes in "
            "<code>settings.rs</code>. Decide when (or if) the relevant "
            "subsystem lands."
        ),
    },
    {
        "topic": "13 'Possibly missing' AutoCAD entity types — adopt any?",
        "status": "Parked",
        "detail": (
            "REGION, MLINE, HELIX, 3DFACE, POLYFACEMESH, POLYGONMESH, UNDERLAY, "
            "GEODATA, SHAPE, TRACE, FIELD, LIGHT, CAMERA. Documented in "
            "<code>Dobject_Properties.md</code> § 'Possibly missing'. Reopen "
            "only when a real drawing needs one, DXF import hits one, or "
            "someone explicitly asks."
        ),
    },
    {
        "topic": "Handle preservation on DXF import — when?",
        "status": "Question",
        "detail": (
            "Current implementation uses a process-global atomic counter "
            "(<code>cad_kernel/src/dobject.rs::HANDLE_COUNTER</code>). When "
            "DXF import lands, files carry their own hex handles that must "
            "round-trip. Plan: move the counter to per-<code>Document</code>. "
            "Open: do we preserve the source-file handles verbatim, or "
            "remap on import and persist the mapping?"
        ),
    },
    {
        "topic": "ACI palette completeness",
        "status": "Partial — needs filling",
        "detail": (
            "<code>cad_kernel/src/color.rs::aci_palette()</code> currently "
            "covers ACI 0–9 only; indices 10–255 fall back to white. Real "
            "AutoCAD has all 256 mapped. Fill from the standard ACI table "
            "before any meaningful DXF import."
        ),
    },
    {
        "topic": "GPU renderer per-instance color",
        "status": "Question",
        "detail": (
            "CPU renderer resolves <code>Color::ByLayer</code> already. GPU "
            "path (<code>cad_app/src/gpu.rs</code>) hardcodes def/sel/snap "
            "colors. Plan: add per-instance color to <code>CircleInstance</code> "
            "(already a u32 field — just plumb resolved color in)."
        ),
    },
    {
        "topic": "Polyline bulge model — DXF-compatible or simplified?",
        "status": "Question",
        "detail": (
            "DXF LWPOLYLINE uses bulge = tan(angle/4) per vertex for arc "
            "segments. Plan in <code>Dobject_Properties.md</code> matches "
            "this. Confirm before implementing Slice E — engineers may "
            "prefer storing arc params directly."
        ),
    },
    {
        "topic": "Block table — when?",
        "status": "Question",
        "detail": (
            "BlockTable is Slice F. It's the biggest architectural step "
            "after Slice A because INSERT references introduce document "
            "hierarchy. Confirm priority: do we land it before/after "
            "DimRotated (Slice E.5) or in parallel?"
        ),
    },
    {
        "topic": "Line.a / Line.b → Line.start / Line.end rename",
        "status": "Tracked, deferred",
        "detail": (
            "Flagged in <code>Dobject_Properties.md</code> § Line and saved "
            "to memory (<code>project_rust_cad_future_cleanups</code>). "
            "Mechanical rename, wide-reach. Do it in a dedicated cleanup "
            "pass — never inside a feature PR."
        ),
    },
    {
        "topic": ".rsm binary format — when and what shape?",
        "status": "Question",
        "detail": (
            "ROADMAP lists this as Slice I (after DXF I/O). Open: serde "
            "binary, custom zero-copy layout (rkyv / postcard), or "
            "memory-mapped flat format? Performance target: load 5M dobjects "
            "in &lt; 1 second."
        ),
    },
    {
        "topic": "Spline — really deferred, or quietly required?",
        "status": "Parked",
        "detail": (
            "<code>Dobject_Properties.md</code> § DobjectSpline marks it "
            "deferred indefinitely. Spline math (NURBS evaluation + de Boor + "
            "intersection) is heavy. If any real drawing needs splines, this "
            "gets a slice of its own — not folded into Slice E."
        ),
    },
]

# ---------- minimal MD → HTML converter ------------------------------------

INLINE_CODE_RE   = re.compile(r"`([^`]+)`")
LINK_RE          = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
BOLD_RE          = re.compile(r"\*\*([^*]+)\*\*")
ITALIC_RE        = re.compile(r"(?<!\*)\*([^*\n]+)\*(?!\*)")

def _inline(s: str) -> str:
    """Apply inline MD substitutions to an already-escaped string fragment.

    We escape FIRST (so user text is safe), then re-inject the structural
    tags we want.
    """
    s = html.escape(s)
    # `code` — must come before bold/italic so backtick contents aren't styled.
    s = INLINE_CODE_RE.sub(
        lambda m: f"<code>{m.group(1)}</code>", s)
    # [text](href)
    s = LINK_RE.sub(
        lambda m: f'<a href="{html.escape(m.group(2), quote=True)}">'
                  f'{m.group(1)}</a>', s)
    # **bold**
    s = BOLD_RE.sub(lambda m: f"<strong>{m.group(1)}</strong>", s)
    # *italic*
    s = ITALIC_RE.sub(lambda m: f"<em>{m.group(1)}</em>", s)
    # status glyphs → tinted spans
    for glyph, cls in [("●", "ok"), ("◐", "partial"),
                       ("○", "planned"), ("◌", "tentative")]:
        s = s.replace(glyph, f'<span class="status-{cls}">{glyph}</span>')
    return s


def _slug(text: str) -> str:
    s = re.sub(r"[^a-zA-Z0-9\s-]", "", text).strip().lower()
    return re.sub(r"\s+", "-", s)[:80]


def md_to_html(md: str, section_id_prefix: str) -> str:
    """Convert a markdown document to HTML. Supports the subset our MD uses:
    headings, tables, blockquotes, ordered / unordered lists, hrs, paragraphs,
    inline code / links / bold / italic."""
    lines = md.split("\n")
    out: list[str] = []
    i = 0
    in_para: list[str] = []

    def flush_para():
        nonlocal in_para
        if in_para:
            text = " ".join(in_para).strip()
            if text:
                out.append(f"<p>{_inline(text)}</p>")
            in_para = []

    while i < len(lines):
        line = lines[i]
        stripped = line.rstrip()

        # --- horizontal rule
        if re.fullmatch(r"\s*-{3,}\s*", stripped):
            flush_para()
            out.append("<hr/>")
            i += 1
            continue

        # --- code fence
        if stripped.startswith("```"):
            flush_para()
            i += 1
            buf = []
            while i < len(lines) and not lines[i].startswith("```"):
                buf.append(lines[i])
                i += 1
            i += 1  # consume closing fence
            out.append(
                "<pre><code>"
                + html.escape("\n".join(buf))
                + "</code></pre>"
            )
            continue

        # --- headings
        m = re.match(r"^(#{1,6})\s+(.+)$", stripped)
        if m:
            flush_para()
            level = len(m.group(1))
            title = m.group(2).strip()
            anchor = f"{section_id_prefix}-{_slug(title)}"
            out.append(
                f'<h{level} id="{anchor}">'
                f'{_inline(title)}'
                f'<a class="anchor" href="#{anchor}">¶</a>'
                f'</h{level}>'
            )
            i += 1
            continue

        # --- table (header | row | --- delimiter | body…)
        if "|" in stripped and i + 1 < len(lines) and \
                re.search(r"\|\s*:?-+:?\s*\|", lines[i + 1]):
            flush_para()
            header_line = stripped
            i += 2   # skip delimiter
            body = []
            while i < len(lines) and "|" in lines[i] and lines[i].strip():
                body.append(lines[i])
                i += 1

            def split_row(s: str) -> list[str]:
                t = s.strip()
                if t.startswith("|"): t = t[1:]
                if t.endswith("|"):   t = t[:-1]
                return [c.strip() for c in t.split("|")]

            headers = split_row(header_line)
            out.append('<div class="tbl-wrap"><table>')
            out.append("<thead><tr>")
            for h in headers:
                out.append(f"<th>{_inline(h)}</th>")
            out.append("</tr></thead><tbody>")
            for row in body:
                cells = split_row(row)
                out.append("<tr>")
                for c in cells:
                    out.append(f"<td>{_inline(c)}</td>")
                out.append("</tr>")
            out.append("</tbody></table></div>")
            continue

        # --- blockquote
        if stripped.startswith(">"):
            flush_para()
            buf = []
            while i < len(lines) and lines[i].lstrip().startswith(">"):
                buf.append(re.sub(r"^\s*>\s?", "", lines[i]))
                i += 1
            inner = md_to_html("\n".join(buf), section_id_prefix + "-bq")
            out.append(f"<blockquote>{inner}</blockquote>")
            continue

        # --- list (unordered or ordered)
        if re.match(r"^\s*[-*]\s+", stripped) or re.match(r"^\s*\d+\.\s+", stripped):
            flush_para()
            ordered = bool(re.match(r"^\s*\d+\.\s+", stripped))
            tag = "ol" if ordered else "ul"
            out.append(f"<{tag}>")
            while i < len(lines) and (
                re.match(r"^\s*[-*]\s+", lines[i])
                or re.match(r"^\s*\d+\.\s+", lines[i])
            ):
                item = re.sub(r"^\s*(?:[-*]|\d+\.)\s+", "", lines[i])
                out.append(f"<li>{_inline(item)}</li>")
                i += 1
            out.append(f"</{tag}>")
            continue

        # --- blank line ends paragraph
        if not stripped:
            flush_para()
            i += 1
            continue

        # --- paragraph accumulation
        in_para.append(stripped)
        i += 1

    flush_para()
    return "\n".join(out)

# ---------- report assembly ------------------------------------------------

CSS = """
:root {
  --bg:#0f1419; --bg-elev:#161b22; --bg-tbl:#11161d;
  --fg:#d4d4d8; --fg-dim:#8b949e; --accent:#79c0ff; --border:#30363d;
  --ok:#3fb950; --partial:#d29922; --planned:#8b949e; --tentative:#f0b1ff;
  --question:#ff7b72; --bg-q:#3d1e22;
}
* { box-sizing: border-box; }
body {
  background: var(--bg); color: var(--fg);
  font: 14.5px/1.55 -apple-system, "Segoe UI", system-ui, sans-serif;
  margin: 0; padding: 0;
}
.layout { display: grid; grid-template-columns: 260px 1fr; min-height: 100vh; }
nav.toc {
  background: var(--bg-elev); border-right: 1px solid var(--border);
  padding: 24px 18px; position: sticky; top: 0; align-self: start;
  height: 100vh; overflow-y: auto;
}
nav.toc h2 { font-size: 11px; text-transform: uppercase; letter-spacing: 1.5px; color: var(--fg-dim); margin: 18px 0 8px; }
nav.toc h2:first-child { margin-top: 0; }
nav.toc a { display: block; color: var(--fg); text-decoration: none; padding: 4px 8px; border-radius: 4px; font-size: 13px; }
nav.toc a:hover { background: rgba(255,255,255,0.06); color: var(--accent); }
nav.toc ul { list-style: none; padding-left: 12px; margin: 4px 0; }

main { padding: 32px 48px 80px; max-width: 1200px; }
h1 { font-size: 28px; border-bottom: 1px solid var(--border); padding-bottom: 12px; margin: 56px 0 16px; }
h1:first-of-type { margin-top: 0; }
h2 { font-size: 22px; margin: 40px 0 12px; border-bottom: 1px solid var(--border); padding-bottom: 6px; }
h3 { font-size: 18px; margin: 28px 0 10px; color: var(--accent); }
h4 { font-size: 15px; margin: 20px 0 8px; color: var(--fg-dim); text-transform: uppercase; letter-spacing: 1px; }
a { color: var(--accent); }
a.anchor { color: var(--border); text-decoration: none; margin-left: 8px; font-weight: normal; opacity: 0; transition: opacity 0.1s; }
h1:hover a.anchor, h2:hover a.anchor, h3:hover a.anchor, h4:hover a.anchor { opacity: 1; }

code {
  background: rgba(110, 118, 129, 0.18); padding: 1px 6px; border-radius: 4px;
  font-family: "JetBrains Mono", "SF Mono", Menlo, Consolas, monospace; font-size: 12.5px;
}
pre {
  background: var(--bg-tbl); border: 1px solid var(--border);
  border-radius: 6px; padding: 14px 18px; overflow-x: auto;
}
pre code { background: transparent; padding: 0; }

blockquote {
  background: rgba(121, 192, 255, 0.05); border-left: 3px solid var(--accent);
  padding: 4px 16px; margin: 14px 0; border-radius: 0 6px 6px 0;
}

.tbl-wrap { overflow-x: auto; margin: 14px 0; border: 1px solid var(--border); border-radius: 6px; }
table { border-collapse: collapse; width: 100%; background: var(--bg-tbl); }
th, td { padding: 8px 12px; text-align: left; vertical-align: top; border-bottom: 1px solid var(--border); font-size: 13.5px; }
th { background: var(--bg-elev); color: var(--accent); position: sticky; top: 0; font-weight: 600; }
tr:last-child td { border-bottom: none; }
tr:hover td { background: rgba(255, 255, 255, 0.025); }

.status-ok       { color: var(--ok);        font-weight: bold; }
.status-partial  { color: var(--partial);   font-weight: bold; }
.status-planned  { color: var(--planned);   font-weight: bold; }
.status-tentative{ color: var(--tentative); font-weight: bold; }

.cover {
  background: linear-gradient(180deg, rgba(121,192,255,0.12), transparent);
  border: 1px solid var(--border); border-radius: 10px;
  padding: 28px 32px; margin-bottom: 32px;
}
.cover h1 { border: none; margin: 0 0 8px; font-size: 32px; }
.cover .meta { color: var(--fg-dim); font-size: 13px; }

.legend {
  display: flex; gap: 22px; flex-wrap: wrap;
  background: var(--bg-elev); border: 1px solid var(--border);
  border-radius: 6px; padding: 12px 18px; margin: 14px 0 28px;
  font-size: 13px;
}
.legend .item { display: flex; align-items: center; gap: 6px; }

.question {
  background: rgba(255, 123, 114, 0.05);
  border-left: 3px solid var(--question);
  padding: 14px 18px; border-radius: 0 6px 6px 0; margin: 14px 0;
}
.question .q-status {
  display: inline-block;
  background: var(--bg-q); color: var(--question);
  padding: 2px 10px; border-radius: 11px; font-size: 11px;
  font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px;
  margin-bottom: 8px;
}
.question .q-topic { font-weight: 600; font-size: 15px; color: var(--fg); margin-bottom: 6px; }
.question .q-detail { color: var(--fg); }

.filemap {
  background: var(--bg-elev); border: 1px solid var(--border);
  border-radius: 6px; padding: 16px 20px; font-family: "JetBrains Mono", monospace;
  font-size: 12.5px; line-height: 1.7;
}
.filemap .header { color: var(--accent); margin-top: 10px; }
.filemap .header:first-child { margin-top: 0; }
.filemap .path { color: var(--fg); }
.filemap .note { color: var(--fg-dim); font-style: italic; }
"""

def build_questions_html() -> str:
    parts = ['<section id="questions">']
    parts.append('<h1>Open Questions / Decisions Pending</h1>')
    parts.append('<p>Each item below is a deliberate question mark — '
                 'something that needs an engineering decision before the '
                 'relevant slice can be fully resolved. Discuss; record '
                 'the answer either back into <code>ROADMAP.md</code> or as '
                 'an issue in the project tracker.</p>')
    for q in OPEN_QUESTIONS:
        parts.append('<div class="question">')
        parts.append(f'<div class="q-status">{html.escape(q["status"])}</div>')
        parts.append(f'<div class="q-topic">{html.escape(q["topic"])}</div>')
        parts.append(f'<div class="q-detail">{q["detail"]}</div>')
        parts.append('</div>')
    parts.append('</section>')
    return "\n".join(parts)


def build_filemap_html() -> str:
    return f"""
<section id="filemap">
<h1>Where the project files live</h1>
<div class="filemap">
<div class="header">~/workspace/RUST_CAD/  <span class="note">— source + reference docs (git repo, branch <code>main</code>)</span></div>
<div class="path">  Cargo.toml, Cargo.lock     <span class="note">workspace manifest</span></div>
<div class="path">  ROADMAP.md                 <span class="note">what we&#39;re doing / objectives / slice plan</span></div>
<div class="path">  Variables.md               <span class="note">user-settable SYSVARS catalog</span></div>
<div class="path">  Dobject_DXF.md             <span class="note">DXF I/O group-code dictionary</span></div>
<div class="path">  Dobject_Properties.md      <span class="note">in-memory property model per Dobject type</span></div>
<div class="path">  Plan_Report.html           <span class="note">THIS document (regenerated by build_report.py)</span></div>
<div class="path">  cad_kernel/                <span class="note">geometry + intersection + snap + spatial + property model</span></div>
<div class="path">  cad_app/                   <span class="note">egui front-end</span></div>
<div class="path">  cad_snap/                  <span class="note">distributable snap-engine facade</span></div>
<div class="path">  cad_cli/                   <span class="note">headless REPL for math verification</span></div>
<div class="path">  research/                  <span class="note">design-research notes</span></div>
<div class="path">  target/                    <span class="note">cargo build artifacts (.gitignored)</span></div>

<div class="header">~/.claude/projects/-home-HSI-workspace-qlcplus-master/memory/  <span class="note">— Claude&#39;s persistent memory across sessions</span></div>
<div class="path">  MEMORY.md                  <span class="note">index of remembered notes</span></div>
<div class="path">  project_rust_cad_*.md      <span class="note">RUST_CAD project memos (architecture, deferred cleanups, slice notes)</span></div>
<div class="path">  feedback_rust_cad_*.md     <span class="note">user-stated conventions (DObject naming, settings naming, etc.)</span></div>
<div class="path">  reference_rust_cad_*.md    <span class="note">pointers to the three reference docs at workspace root</span></div>

<div class="header">~/.config/rust_cad/  <span class="note">— per-user runtime config</span></div>
<div class="path">  user_env.txt               <span class="note">persisted SYSVAR values (auto-saved on settings change)</span></div>

<div class="header">~/.claude/projects/-home-HSI-workspace-qlcplus-master/*.jsonl  <span class="note">— conversation transcripts</span></div>
<div class="path">  &lt;session-id&gt;.jsonl      <span class="note">full chat history (sources for every decision in the three MD docs)</span></div>
</div>
</section>
"""


def build_cover_html(when: str) -> str:
    return f"""
<section class="cover">
<h1>RUST_CAD — Engineering Discussion Pack</h1>
<div class="meta">
Generated <strong>{when}</strong> &nbsp;•&nbsp;
Workspace: <code>~/workspace/RUST_CAD/</code> &nbsp;•&nbsp;
Branch: <code>main</code> &nbsp;•&nbsp;
Current slice: <strong>A — Property foundation (done)</strong>
</div>
<p style="margin-top:14px">
Single-file engineer-shareable snapshot of: <strong>(1)</strong> where the project
files live, <strong>(2)</strong> the complete catalog of user SYSVARS,
<strong>(3)</strong> the complete DXF I/O group-code dictionary,
<strong>(4)</strong> the in-memory property model for every Dobject type today
and planned, <strong>(5)</strong> the slice-by-slice implementation plan,
and <strong>(6)</strong> every open question / decision pending.
</p>
<div class="legend">
<div class="item"><span class="status-ok">●</span> Wired in code</div>
<div class="item"><span class="status-partial">◐</span> Partial (stored, not fully used)</div>
<div class="item"><span class="status-planned">○</span> Planned</div>
<div class="item"><span class="status-tentative">◌</span> Tentative</div>
<div class="item">? <span style="color:var(--question)">Open question</span></div>
</div>
</section>
"""


def build_toc_html() -> str:
    return """
<nav class="toc">
<h2>Top of report</h2>
<a href="#filemap">Where files live</a>
<a href="#questions">Open questions</a>

<h2>Roadmap</h2>
<a href="#roadmap-rust_cad-project-roadmap">RUST_CAD Project Roadmap</a>
<ul>
<li><a href="#roadmap-where-we-are-now-2026-06-01">Where we are now</a></li>
<li><a href="#roadmap-north-star-objectives">North-star objectives</a></li>
<li><a href="#roadmap-how-we-implement-foundation-first-slice-by-slice">How we implement</a></li>
<li><a href="#roadmap-crate-layout">Crate layout</a></li>
<li><a href="#roadmap-naming-conventions">Naming conventions</a></li>
</ul>

<h2>Variables</h2>
<a href="#variables-rust_cad-user-environment-variables-master-reference">SYSVAR master reference</a>

<h2>DXF dictionary</h2>
<a href="#dxf-dobject-dxf-group-code-dictionary">Group-code dictionary</a>

<h2>Property model</h2>
<a href="#properties-dobject-property-model-per-type-field-catalog">Per-Dobject in-memory model</a>
</nav>
"""


def main():
    when = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    body_sections = []

    body_sections.append(build_cover_html(when))
    body_sections.append(build_filemap_html())
    body_sections.append(build_questions_html())

    for sid, fname, title in SOURCES:
        md = (ROOT / fname).read_text()
        body_sections.append(f'<section id="{sid}">')
        body_sections.append(md_to_html(md, sid))
        body_sections.append('</section>')

    html_doc = f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>RUST_CAD — Engineering Discussion Pack</title>
<style>{CSS}</style>
</head>
<body>
<div class="layout">
{build_toc_html()}
<main>
{''.join(body_sections)}
</main>
</div>
</body>
</html>
"""
    out = ROOT / "Plan_Report.html"
    out.write_text(html_doc)
    print(f"wrote {out}  ({len(html_doc):,} bytes)")


if __name__ == "__main__":
    main()
