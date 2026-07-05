;; RUST_CAD starter hatch patterns — standard straight-line families.
;; These cover the line-based swatches on a typical AutoCAD hatch sheet
;; (parallel lines, crosshatch, grids, brick). Ornamental/curved patterns
;; (florals, hexagons, circles, symbols) are NOT representable as .pat — they
;; are blocks. Feed a real acad.pat / acadiso.pat here for the full set.
;;
;; Format per line family: angle, x-origin, y-origin, delta-x, delta-y [, dashes]

*LINE, Parallel horizontal lines
0, 0,0, 0,0.125

*LINE45, Parallel lines at 45 degrees
45, 0,0, 0,0.125

*ANSI31, Crosshatch-style single 45-degree lines
45, 0,0, 0,0.125

*ANSI37, Crosshatch at 45 and 135 degrees
45, 0,0, 0,0.125
135, 0,0, 0,0.125

*NET, Horizontal / vertical grid
0, 0,0, 0,0.125
90, 0,0, 0,0.125

*NET45, Diagonal grid (45 / 135)
45, 0,0, 0,0.125
135, 0,0, 0,0.125

*GRID, Square grid (coarse)
0, 0,0, 0,0.25
90, 0,0, 0,0.25

*DASH, Dashed horizontal lines
0, 0,0, 0,0.125, 0.125,-0.0625

*BRICK, Brick (running bond)
0, 0,0, 0,0.25
90, 0,0, 0.25,0.25, 0.25,-0.25

*VERTICAL, Parallel vertical lines
90, 0,0, 0,0.125
