# Regenerate the GENERATED TABLES section of SETTINGS.md from cad_app/src/varreg.rs.
# Everything above the "GENERATED TABLES BELOW" marker is hand-maintained prose
# and is preserved; everything below it is rebuilt. Idempotent.
#
# Run:  powershell -ExecutionPolicy Bypass -File tools\gen-settings.ps1
# (also runs automatically via the pre-commit hook when varreg.rs is staged)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$md     = Join-Path $root 'SETTINGS.md'
$reg    = Join-Path $root 'cad_app\src\varreg.rs'
$marker = '<!-- ===== GENERATED TABLES BELOW'

if (-not (Test-Path $md))  { Write-Error "SETTINGS.md not found"; exit 1 }
if (-not (Test-Path $reg)) { Write-Error "varreg.rs not found"; exit 1 }

# ---- keep prose up to and including the marker line ----
# Read/write as UTF-8 so multibyte chars (em-dash, ·, →) survive round-trips.
$utf8 = [System.IO.File]::ReadAllText($md, [System.Text.Encoding]::UTF8)
$content = $utf8 -split "`r?`n"
$markerLine = $content | Select-String -SimpleMatch $marker | Select-Object -First 1
if (-not $markerLine) { Write-Error "marker not found in SETTINGS.md"; exit 1 }
$head = $content[0..($markerLine.LineNumber - 1)]

# ---- refresh the "Last updated:" line in the head, if present ----
$today = (Get-Date).ToString('yyyy-MM-dd')
for ($i = 0; $i -lt $head.Count; $i++) {
  if ($head[$i] -match '^Last updated:') {
    $head[$i] = "Last updated: $today (branch windows-ui-session-2026-06-20)"
  }
}

# ---- parse varreg.rs Var rows ----
$regText = [System.IO.File]::ReadAllText($reg, [System.Text.Encoding]::UTF8)
$lines = ($regText -split "`r?`n") | Where-Object { $_ -match '^\s*Var \{ name:' }
$vars = foreach ($l in $lines) {
  $name  = if ($l -match 'name:\s*"([^"]*)"')      { $matches[1] } else { '' }
  $sec   = if ($l -match 'section:\s*"([^"]*)"')   { $matches[1] } else { '' }
  $desc  = if ($l -match 'desc:\s*"([^"]*)"')      { $matches[1] } else { '' }
  $stat  = if ($l -match 'status:\s*Status::(\w+)'){ $matches[1] } else { '' }
  $def   = if ($l -match 'default:\s*"([^"]*)"')   { $matches[1] } else { '' }
  $wired = if ($l -match 'wired:\s*(true|false)')  { $matches[1] } else { 'false' }
  $kraw  = if ($l -match 'kind:\s*(.*),\s*status:'){ $matches[1].Trim() } else { '' }
  $type = switch -Regex ($kraw) {
    '^Kind::Bool'  { 'Bool'; break }
    '^Kind::Color' { 'Color'; break }
    '^Kind::Text'  { 'Text'; break }
    '^Kind::U8\s*\{\s*min:\s*(\d+),\s*max:\s*(\d+)'        { "U8 $($matches[1])-$($matches[2])"; break }
    '^Kind::Int\s*\{\s*min:\s*([\d_]+),\s*max:\s*([\d_]+)' { "Int $($matches[1])-$($matches[2])"; break }
    '^Kind::Float' { 'Float'; break }
    '^Kind::Choice\(&\[(.*)\]\)' { 'Choice(' + (($matches[1] -replace '"','' -replace ',\s*','/')) + ')'; break }
    default { $kraw }
  }
  [pscustomobject]@{
    Name=$name; Section=$sec; Desc=($desc -replace '\|','\|'); Type=$type
    Default=$def; Status=$stat; Wired=($wired -eq 'true')
  }
}

# ---- build the generated section ----
$sb = New-Object System.Text.StringBuilder
$wiredVars = $vars | Where-Object { $_.Wired }
[void]$sb.AppendLine("")
[void]$sb.AppendLine("## Editable variables (wired = " + $wiredVars.Count + ")")
[void]$sb.AppendLine("")
[void]$sb.AppendLine("| Name | Section | Type | Default | Status | Description |")
[void]$sb.AppendLine("|---|---|---|---|---|---|")
foreach ($v in $wiredVars) {
  [void]$sb.AppendLine("| ``$($v.Name)`` | $($v.Section) | $($v.Type) | ``$($v.Default)`` | $($v.Status) | $($v.Desc) |")
}
[void]$sb.AppendLine("")
[void]$sb.AppendLine("## Full catalog (" + $vars.Count + " variables, by section)")
foreach ($grp in ($vars | Group-Object Section)) {
  [void]$sb.AppendLine("")
  [void]$sb.AppendLine("### $($grp.Name)  (" + $grp.Count + ")")
  [void]$sb.AppendLine("")
  [void]$sb.AppendLine("| Name | Type | Default | Status | Edit | Description |")
  [void]$sb.AppendLine("|---|---|---|---|---|---|")
  foreach ($v in $grp.Group) {
    $edit = if ($v.Wired) { 'yes' } else { '' }
    [void]$sb.AppendLine("| ``$($v.Name)`` | $($v.Type) | ``$($v.Default)`` | $($v.Status) | $edit | $($v.Desc) |")
  }
}

# ---- write head + generated section (UTF-8, no BOM, LF line endings) ----
$out = ($head -join "`n") + "`n" + ($sb.ToString() -replace "`r`n", "`n")
$noBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($md, $out, $noBom)
"SETTINGS.md regenerated: $($vars.Count) vars, $($wiredVars.Count) wired, $(($vars | Group-Object Section).Count) sections"
