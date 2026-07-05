# Dobject ↔ DXF Group-Code Dictionary

This is the **file-format I/O contract** for reading and writing Dobjects to/from
DXF (R12 through R2018+). It is intentionally **separate** from `Variables.md`,
which catalogues *user-settable SYSVARS*. Different domain, different audience.

- **`Variables.md`** — what the user *configures* (SYSVAR-style).
- **`Dobject_DXF.md`** — what we *serialize* to disk for interop.

> Nothing here is wired yet. The `cad_io` (or `cad_dxf`) crate does not exist.
> Every row starts at status `○ Planned`. We will flip to `◐ Partial` /
> `● Wired` as the importer/exporter lands, slice by slice.

## Status legend

| Status | Meaning |
|--------|---------|
| `○` | **Planned** — known field, not yet implemented |
| `◐` | **Partial** — read or write only, or only some entity types |
| `●` | **Wired** — round-trips for all supported entity types |

## Prefix convention

| Prefix | Meaning |
|--------|---------|
| `dxf…` | DXF *structural* group codes (handles, subclass markers, etc.) — independent of entity type |
| `dob…` | **Dobject** common properties (layer, color, points, angles, …) |
| `xd…`  | Extended data (1000–1071 range) — application-attached metadata |

> Earlier external references used `ent…` for entity. In RUST_CAD the
> drafting primitive is **Dobject** (see `feedback_rust_cad_dobject_naming`),
> so the prefix is `dob…`. `dxf…` and `xd…` are unchanged because they refer
> to DXF *structure*, not the Dobject itself.

## Context-collisions (same code, different meanings)

DXF reuses three group codes depending on entity context. These are **not**
typos in the tables below — the dispatcher decides which meaning applies based
on the parent entity type.

| Code | First meaning | Second meaning |
|------|---------------|----------------|
| `38` | `dobOtherPointZ8` (Z of 8th point) | `dobElevation` (entity elevation if nonzero) |
| `92` | `dobProxyByteCount` (proxy entities) | `dxfInt92` (generic 32-bit int) |
| `310` | `dobProxyData` (proxy graphics) | `dxfBinary310` (generic binary chunk) |

## Entity-specific overrides

Codes like `41`, `42`, `70`, `71` carry **generic** meanings in the tables
below ("second double", "first 16-bit int", …), but specific entity types
redefine them:

- `INSERT` → `41/42/43` = X/Y/Z scale, `50` = rotation, `70/71` = column/row count
- `HATCH` → `41` = pattern scale, `52` = pattern angle, `75` = hatch style, `91` = boundary path count
- `DIMENSION` → `1` = override text, `41` = leader length, `70` = dimension type
- `LWPOLYLINE` → `38` = elevation, `39` = thickness, `43` = constant width, `70` = flags, `90` = vertex count

These per-entity meanings are **not** repeated here — they belong in the
per-entity reader/writer module when each entity type lands. This document is
the **common-code dictionary** only.

---

## 🧩 1. Core Dobject Identifiers & Structure

| Variable Name | DXF Code | Description | Status |
|---------------|----------|-------------|--------|
| `dxfDobjectName` | –1 | Dobject name (changes each drawing open; never saved) | ○ |
| `dxfType` | 0 | Text string indicating the dobject type (e.g., `LINE`, `CIRCLE`) | ○ |
| `dxfHandle` | 5 | Dobject handle (hexadecimal, persistent unique ID) | ○ |
| `dxfSubclass` | 100 | Subclass marker (e.g., `AcDbEntity`, `AcDbCircle`) | ○ |

---

## 🧱 2. Common Dobject Properties (Layer, Color, Linetype, Visibility)

| Variable Name | DXF Code | Description | Status |
|---------------|----------|-------------|--------|
| `dobLayer` | 8 | Layer name (e.g., `"0"`, `"Walls"`) | ○ |
| `dobLinetype` | 6 | Linetype name (`"BYLAYER"`, `"Continuous"`, `"Dashed"`) | ○ |
| `dobColor` | 62 | Color index (0 = ByBlock, 256 = ByLayer, 1‑255 = ACI) | ○ |
| `dobTrueColor` | 420 | 24‑bit TrueColor value (RGB); when used, `dobColor` is often 256 or 0 | ○ |
| `dobColorName` | 430 | Color name from a color book (e.g., `"RAL 9010"`) | ○ |
| `dobTransparency` | 440 | Transparency value (0…100%; 0 = opaque) | ○ |
| `dobLineweight` | 370 | Lineweight enum (–1 = ByLayer, –2 = ByBlock, –3 = Default, 0…211) | ○ |
| `dobLtScale` | 48 | Linetype scale factor (global for this dobject, >0) | ○ |
| `dobVisibility` | 60 | Visibility (0 = visible, 1 = invisible) | ○ |
| `dobMaterial` | 347 | Hard‑pointer handle to material object (if not BYLAYER) | ○ |
| `dobPlotStyle` | 390 | Hard‑pointer handle to the plot style object | ○ |
| `dobShadow` | 284 | Shadow mode (0 = casts & receives, 1 = casts only, 2 = receives only, 3 = ignores); obsolete from AutoCAD 2016 | ○ |

---

## 📐 3. Geometry & Space (Points, Extrusion, Layout)

| Variable Name | DXF Code | Description | Status |
|---------------|----------|-------------|--------|
| `dobSpace` | 67 | Space (0 = Model space, 1 = Paper space / layout) | ○ |
| `dobLayoutTab` | 410 | Layout tab name (`"Model"` or layout name like `"Layout1"`) | ○ |
| `dobPrimaryPointX` | 10 | X value of primary point (start point, circle center, etc.) | ○ |
| `dobPrimaryPointY` | 20 | Y value of primary point | ○ |
| `dobPrimaryPointZ` | 30 | Z value of primary point | ○ |
| `dobOtherPointX1` | 11 | X value of first other point | ○ |
| `dobOtherPointY1` | 21 | Y value of first other point | ○ |
| `dobOtherPointZ1` | 31 | Z value of first other point | ○ |
| `dobOtherPointX2` | 12 | X value of second other point | ○ |
| `dobOtherPointY2` | 22 | Y value of second other point | ○ |
| `dobOtherPointZ2` | 32 | Z value of second other point | ○ |
| … `13/23/33` | … | Up to `18/28/38` for 8 points | ○ |
| `dobElevation` | 38 | Dobject's elevation if nonzero (collides with `dobOtherPointZ8`) | ○ |
| `dobThickness` | 39 | Dobject's thickness (3D objects) | ○ |
| `dobExtrusionX` | 210 | Extrusion direction X value | ○ |
| `dobExtrusionY` | 220 | Extrusion direction Y value | ○ |
| `dobExtrusionZ` | 230 | Extrusion direction Z value | ○ |

---

## 📦 4. Numerical, Angle & Text Values

| Variable Name | DXF Code | Description | Status |
|---------------|----------|-------------|--------|
| `dobReal40` | 40 | Double‑precision value (text height, radius, etc.) | ○ |
| `dobReal41` | 41 | Second double‑precision value | ○ |
| `dobReal42` | 42 | Third double‑precision value | ○ |
| … `dobReal48` | 48 | Already covered as `dobLtScale` | ○ |
| `dobReal49` | 49 | Repeated double‑precision value (dash lengths in LTYPE) | ○ |
| `dobAngle50` | 50 | Angle in degrees (start angle, rotation, etc.) | ○ |
| `dobAngle51` | 51 | Second angle | ○ |
| `dobAngle52` | 52 | Third angle | ○ |
| `dobAngle53` | 53 | Fourth angle | ○ |
| `dobAngle54` | 54 | Fifth angle | ○ |
| `dobAngle55` | 55 | Sixth angle | ○ |
| `dobAngle56` | 56 | Seventh angle | ○ |
| `dobAngle57` | 57 | Eighth angle | ○ |
| `dobAngle58` | 58 | Ninth angle | ○ |
| `dobTextString1` | 1 | Primary text value for a dobject (e.g., text string, attribute value) | ○ |
| `dobTextString2` | 2 | Name (attribute tag, block name, etc.) | ○ |
| `dobTextString3` | 3 | Other text or name values | ○ |
| `dobTextString4` | 4 | Other text or name values | ○ |

---

## 🔗 5. Ownership & Pointers (Reactors, Dictionaries, XDict)

| Variable Name | DXF Code | Description | Status |
|---------------|----------|-------------|--------|
| `dobSoftOwner` | 330 | Soft‑pointer handle to owner (BLOCK_RECORD, dictionary) | ○ |
| `dobHardOwner` | 360 | Hard‑owner handle to owner dictionary | ○ |
| `dobSoftPointer` | 331 | Soft‑pointer to another dobject (e.g., MTEXT background mask) | ○ |
| `dobHardPointer` | 350 | Hard‑pointer to another dobject | ○ |
| `dobReactorsStart` | 102 | Start of persistent reactor group (`"{ACAD_REACTORS"`) | ○ |
| `dobXdictionaryStart` | 102 | Start of extension dictionary group (`"{ACAD_XDICTIONARY"`) | ○ |
| `dobGroupEnd` | 102 | End of any 102 group (`"}"`) | ○ |

---

## 🧪 6. Proxy Graphics & Binary Data

| Variable Name | DXF Code | Description | Status |
|---------------|----------|-------------|--------|
| `dobProxyByteCount` | 92 | Number of bytes in the proxy dobject graphics (collides with `dxfInt92`) | ○ |
| `dobProxyData` | 310 | Proxy dobject graphics data (hexadecimal, 256 chars max per line; collides with `dxfBinary310`) | ○ |

---

## 📇 7. Complete DXF Group Codes by Number (Fixed Purpose)

This section repeats every numbered group code with `ent` → `dob` applied.
Some rows duplicate entries from Sections 1–6 by design — this is the
**number-indexed** view, useful when reading a raw DXF stream.

| Variable Name | DXF Code | Description | Status |
|---------------|----------|-------------|--------|
| `dxfPersReactor` | –5 | APP: persistent reactor chain | ○ |
| `dxfCondOperator` | –4 | APP: conditional operator (only with `ssget`) | ○ |
| `dxfXdataSentinel` | –3 | APP: extended data sentinel (fixed) | ○ |
| `dxfDobjectRef` | –2 | APP: dobject name reference (fixed) | ○ |
| `dxfDobjectName` | –1 | Dobject name (changes each drawing open; never saved) | ○ |
| `dxfType` | 0 | Dobject type string (fixed) | ○ |
| `dobTextString1` | 1 | Primary text value for a dobject | ○ |
| `dobTextString2` | 2 | Name (attribute tag, block name, etc.) | ○ |
| `dobTextString3` | 3 | Other text or name values | ○ |
| `dobTextString4` | 4 | Other text or name values | ○ |
| `dxfHandle` | 5 | Dobject handle (hexadecimal, fixed) | ○ |
| `dobLinetype` | 6 | Linetype name (fixed) | ○ |
| `dxfTextStyle` | 7 | Text style name (fixed) | ○ |
| `dobLayer` | 8 | Layer name (fixed) | ○ |
| `dxfHeaderVarId` | 9 | Variable name identifier (HEADER section only) | ○ |
| `dobPrimaryPointX` | 10 | X of primary point (DXF); 3D point (APP) | ○ |
| `dobOtherPointX1` | 11 | X of first other point | ○ |
| `dobOtherPointX2` | 12 | X of second other point | ○ |
| `dobOtherPointX3` | 13 | X of third other point | ○ |
| `dobOtherPointX4` | 14 | X of fourth other point | ○ |
| `dobOtherPointX5` | 15 | X of fifth other point | ○ |
| `dobOtherPointX6` | 16 | X of sixth other point | ○ |
| `dobOtherPointX7` | 17 | X of seventh other point | ○ |
| `dobOtherPointX8` | 18 | X of eighth other point | ○ |
| `dobPrimaryPointY` | 20 | Y of primary point | ○ |
| `dobOtherPointY1` | 21 | Y of first other point | ○ |
| `dobOtherPointY2` | 22 | Y of second other point | ○ |
| `dobOtherPointY3` | 23 | Y of third other point | ○ |
| `dobOtherPointY4` | 24 | Y of fourth other point | ○ |
| `dobOtherPointY5` | 25 | Y of fifth other point | ○ |
| `dobOtherPointY6` | 26 | Y of sixth other point | ○ |
| `dobOtherPointY7` | 27 | Y of seventh other point | ○ |
| `dobOtherPointY8` | 28 | Y of eighth other point | ○ |
| `dobPrimaryPointZ` | 30 | Z of primary point | ○ |
| `dobOtherPointZ1` | 31 | Z of first other point | ○ |
| `dobOtherPointZ2` | 32 | Z of second other point | ○ |
| `dobOtherPointZ3` | 33 | Z of third other point | ○ |
| `dobOtherPointZ4` | 34 | Z of fourth other point | ○ |
| `dobOtherPointZ5` | 35 | Z of fifth other point | ○ |
| `dobOtherPointZ6` | 36 | Z of sixth other point | ○ |
| `dobOtherPointZ7` | 37 | Z of seventh other point | ○ |
| `dobOtherPointZ8` | 38 | Z of eighth other point | ○ |
| `dobElevation` | 38 | Dobject's elevation if nonzero (collides with `dobOtherPointZ8`) | ○ |
| `dobThickness` | 39 | Dobject's thickness (fixed) | ○ |
| `dobReal40` | 40 | Double‑precision floating‑point value | ○ |
| `dobReal41` | 41 | Double‑precision floating‑point value | ○ |
| `dobReal42` | 42 | Double‑precision floating‑point value | ○ |
| `dobReal43` | 43 | Double‑precision floating‑point value | ○ |
| `dobReal44` | 44 | Double‑precision floating‑point value | ○ |
| `dobReal45` | 45 | Double‑precision floating‑point value | ○ |
| `dobReal46` | 46 | Double‑precision floating‑point value | ○ |
| `dobReal47` | 47 | Double‑precision floating‑point value | ○ |
| `dobLtScale` | 48 | Linetype scale (double) | ○ |
| `dobReal49` | 49 | Repeated double‑precision value (dash lengths) | ○ |
| `dobAngle50` | 50 | Angle in degrees | ○ |
| `dobAngle51` | 51 | Angle in degrees | ○ |
| `dobAngle52` | 52 | Angle in degrees | ○ |
| `dobAngle53` | 53 | Angle in degrees | ○ |
| `dobAngle54` | 54 | Angle in degrees | ○ |
| `dobAngle55` | 55 | Angle in degrees | ○ |
| `dobAngle56` | 56 | Angle in degrees | ○ |
| `dobAngle57` | 57 | Angle in degrees | ○ |
| `dobAngle58` | 58 | Angle in degrees | ○ |
| `dobVisibility` | 60 | Visibility (0 = visible, 1 = invisible) | ○ |
| `dobColor` | 62 | Color number (fixed) | ○ |
| `dxfEntitiesFollow` | 66 | "Dobjects follow" flag | ○ |
| `dobSpace` | 67 | Space (0 = model, 1 = paper) | ○ |
| `dxfViewportOff` | 68 | Viewport is off‑screen / not active | ○ |
| `dxfViewportId` | 69 | Viewport identification number | ○ |
| `dxfInt70` | 70 | 16‑bit integer (repeat count, flags, etc.) | ○ |
| `dxfInt71` | 71 | 16‑bit integer | ○ |
| `dxfInt72` | 72 | 16‑bit integer | ○ |
| `dxfInt73` | 73 | 16‑bit integer | ○ |
| `dxfInt74` | 74 | 16‑bit integer | ○ |
| `dxfInt75` | 75 | 16‑bit integer | ○ |
| `dxfInt76` | 76 | 16‑bit integer | ○ |
| `dxfInt77` | 77 | 16‑bit integer | ○ |
| `dxfInt78` | 78 | 16‑bit integer | ○ |
| `dxfInt90` | 90 | 32‑bit integer | ○ |
| `dxfInt91` | 91 | 32‑bit integer | ○ |
| `dxfInt92` | 92 | 32‑bit integer (collides with `dobProxyByteCount`) | ○ |
| `dxfInt93` | 93 | 32‑bit integer | ○ |
| `dxfInt94` | 94 | 32‑bit integer | ○ |
| `dxfInt95` | 95 | 32‑bit integer | ○ |
| `dxfInt96` | 96 | 32‑bit integer | ○ |
| `dxfInt97` | 97 | 32‑bit integer | ○ |
| `dxfInt98` | 98 | 32‑bit integer | ○ |
| `dxfInt99` | 99 | 32‑bit integer | ○ |
| `dxfSubclass` | 100 | Subclass marker (fixed) | ○ |
| `dxfControlString` | 102 | Control string (`"{...}"`) | ○ |
| `dxfDimvarHandle` | 105 | Object handle for DIMVAR symbol table entry | ○ |
| `dxfUcsOriginX` | 110 | UCS origin X (if code 72 set to 1) | ○ |
| `dxfUcsOriginY` | 120 | UCS origin Y | ○ |
| `dxfUcsOriginZ` | 130 | UCS origin Z | ○ |
| `dxfUcsXaxisX` | 111 | UCS X‑axis X | ○ |
| `dxfUcsXaxisY` | 121 | UCS X‑axis Y | ○ |
| `dxfUcsXaxisZ` | 131 | UCS X‑axis Z | ○ |
| `dxfUcsYaxisX` | 112 | UCS Y‑axis X | ○ |
| `dxfUcsYaxisY` | 122 | UCS Y‑axis Y | ○ |
| `dxfUcsYaxisZ` | 132 | UCS Y‑axis Z | ○ |
| `dxfReal140` | 140 | Double‑precision floating‑point (DIMSTYLE settings) | ○ |
| `dxfReal141` | 141 | Double‑precision floating‑point | ○ |
| `dxfReal142` | 142 | Double‑precision floating‑point | ○ |
| `dxfReal143` | 143 | Double‑precision floating‑point | ○ |
| `dxfReal144` | 144 | Double‑precision floating‑point | ○ |
| `dxfReal145` | 145 | Double‑precision floating‑point | ○ |
| `dxfReal146` | 146 | Double‑precision floating‑point | ○ |
| `dxfReal147` | 147 | Double‑precision floating‑point | ○ |
| `dxfReal148` | 148 | Double‑precision floating‑point | ○ |
| `dxfReal149` | 149 | Double‑precision floating‑point | ○ |
| `dxfShort170` | 170 | 16‑bit integer (DIMSTYLE flags) | ○ |
| `dxfShort171` | 171 | 16‑bit integer | ○ |
| `dxfShort172` | 172 | 16‑bit integer | ○ |
| `dxfShort173` | 173 | 16‑bit integer | ○ |
| `dxfShort174` | 174 | 16‑bit integer | ○ |
| `dxfShort175` | 175 | 16‑bit integer | ○ |
| `dxfShort176` | 176 | 16‑bit integer | ○ |
| `dxfShort177` | 177 | 16‑bit integer | ○ |
| `dxfShort178` | 178 | 16‑bit integer | ○ |
| `dxfShort179` | 179 | 16‑bit integer | ○ |
| `dobExtrusionX` | 210 | Extrusion direction X (fixed) | ○ |
| `dobExtrusionY` | 220 | Extrusion direction Y | ○ |
| `dobExtrusionZ` | 230 | Extrusion direction Z | ○ |
| `dxfShort270` | 270 | 16‑bit integer | ○ |
| `dxfShort271` | 271 | 16‑bit integer | ○ |
| `dxfShort272` | 272 | 16‑bit integer | ○ |
| `dxfShort273` | 273 | 16‑bit integer | ○ |
| `dxfShort274` | 274 | 16‑bit integer | ○ |
| `dxfShort275` | 275 | 16‑bit integer | ○ |
| `dxfShort276` | 276 | 16‑bit integer | ○ |
| `dxfShort277` | 277 | 16‑bit integer | ○ |
| `dxfShort278` | 278 | 16‑bit integer | ○ |
| `dxfShort279` | 279 | 16‑bit integer | ○ |
| `dxfShort280` | 280 | 16‑bit integer | ○ |
| `dxfShort281` | 281 | 16‑bit integer | ○ |
| `dxfShort282` | 282 | 16‑bit integer | ○ |
| `dxfShort283` | 283 | 16‑bit integer | ○ |
| `dxfShort284` | 284 | 16‑bit integer | ○ |
| `dxfShort285` | 285 | 16‑bit integer | ○ |
| `dxfShort286` | 286 | 16‑bit integer | ○ |
| `dxfShort287` | 287 | 16‑bit integer | ○ |
| `dxfShort288` | 288 | 16‑bit integer | ○ |
| `dxfShort289` | 289 | 16‑bit integer | ○ |
| `dxfBool290` | 290 | Boolean flag | ○ |
| `dxfBool291` | 291 | Boolean flag | ○ |
| `dxfBool292` | 292 | Boolean flag | ○ |
| `dxfBool293` | 293 | Boolean flag | ○ |
| `dxfBool294` | 294 | Boolean flag | ○ |
| `dxfBool295` | 295 | Boolean flag | ○ |
| `dxfBool296` | 296 | Boolean flag | ○ |
| `dxfBool297` | 297 | Boolean flag | ○ |
| `dxfBool298` | 298 | Boolean flag | ○ |
| `dxfBool299` | 299 | Boolean flag | ○ |
| `dxfString300` | 300 | Arbitrary text string | ○ |
| `dxfString301` | 301 | Arbitrary text string | ○ |
| `dxfString302` | 302 | Arbitrary text string | ○ |
| `dxfString303` | 303 | Arbitrary text string | ○ |
| `dxfString304` | 304 | Arbitrary text string | ○ |
| `dxfString305` | 305 | Arbitrary text string | ○ |
| `dxfString306` | 306 | Arbitrary text string | ○ |
| `dxfString307` | 307 | Arbitrary text string | ○ |
| `dxfString308` | 308 | Arbitrary text string | ○ |
| `dxfString309` | 309 | Arbitrary text string | ○ |
| `dxfBinary310` | 310 | Arbitrary binary chunk (hexadecimal, 254 chars max; collides with `dobProxyData`) | ○ |
| `dxfBinary311` | 311 | Arbitrary binary chunk | ○ |
| `dxfBinary312` | 312 | Arbitrary binary chunk | ○ |
| `dxfBinary313` | 313 | Arbitrary binary chunk | ○ |
| `dxfBinary314` | 314 | Arbitrary binary chunk | ○ |
| `dxfBinary315` | 315 | Arbitrary binary chunk | ○ |
| `dxfBinary316` | 316 | Arbitrary binary chunk | ○ |
| `dxfBinary317` | 317 | Arbitrary binary chunk | ○ |
| `dxfBinary318` | 318 | Arbitrary binary chunk | ○ |
| `dxfBinary319` | 319 | Arbitrary binary chunk | ○ |
| `dxfRawHandle320` | 320 | Arbitrary dobject handle (not translated) | ○ |
| `dxfRawHandle321` | 321 | Arbitrary dobject handle | ○ |
| `dxfRawHandle322` | 322 | Arbitrary dobject handle | ○ |
| `dxfRawHandle323` | 323 | Arbitrary dobject handle | ○ |
| `dxfRawHandle324` | 324 | Arbitrary dobject handle | ○ |
| `dxfRawHandle325` | 325 | Arbitrary dobject handle | ○ |
| `dxfRawHandle326` | 326 | Arbitrary dobject handle | ○ |
| `dxfRawHandle327` | 327 | Arbitrary dobject handle | ○ |
| `dxfRawHandle328` | 328 | Arbitrary dobject handle | ○ |
| `dxfRawHandle329` | 329 | Arbitrary dobject handle | ○ |
| `dobSoftOwner` | 330 | Soft‑owner handle | ○ |
| `dobSoftPointer` | 331 | Soft‑pointer handle | ○ |
| `dobSoftPointer2` | 332 | Soft‑pointer handle | ○ |
| `dobSoftPointer3` | 333 | Soft‑pointer handle | ○ |
| `dobSoftPointer4` | 334 | Soft‑pointer handle | ○ |
| `dobSoftPointer5` | 335 | Soft‑pointer handle | ○ |
| `dobSoftPointer6` | 336 | Soft‑pointer handle | ○ |
| `dobSoftPointer7` | 337 | Soft‑pointer handle | ○ |
| `dobSoftPointer8` | 338 | Soft‑pointer handle | ○ |
| `dobSoftPointer9` | 339 | Soft‑pointer handle | ○ |
| `dobHardPointer` | 340 | Hard‑pointer handle | ○ |
| `dobHardPointer2` | 341 | Hard‑pointer handle | ○ |
| `dobHardPointer3` | 342 | Hard‑pointer handle | ○ |
| `dobHardPointer4` | 343 | Hard‑pointer handle | ○ |
| `dobHardPointer5` | 344 | Hard‑pointer handle | ○ |
| `dobHardPointer6` | 345 | Hard‑pointer handle | ○ |
| `dobHardPointer7` | 346 | Hard‑pointer handle | ○ |
| `dobMaterial` | 347 | Hard‑pointer handle to material object | ○ |
| `dobHardPointer8` | 348 | Hard‑pointer handle | ○ |
| `dobHardPointer9` | 349 | Hard‑pointer handle | ○ |
| `dobSoftOwner2` | 350 | Soft‑owner handle | ○ |
| `dobSoftOwner3` | 351 | Soft‑owner handle | ○ |
| `dobSoftOwner4` | 352 | Soft‑owner handle | ○ |
| `dobSoftOwner5` | 353 | Soft‑owner handle | ○ |
| `dobSoftOwner6` | 354 | Soft‑owner handle | ○ |
| `dobSoftOwner7` | 355 | Soft‑owner handle | ○ |
| `dobSoftOwner8` | 356 | Soft‑owner handle | ○ |
| `dobSoftOwner9` | 357 | Soft‑owner handle | ○ |
| `dobSoftOwner10` | 358 | Soft‑owner handle | ○ |
| `dobSoftOwner11` | 359 | Soft‑owner handle | ○ |
| `dobHardOwner` | 360 | Hard‑owner handle | ○ |
| `dobHardOwner2` | 361 | Hard‑owner handle | ○ |
| `dobHardOwner3` | 362 | Hard‑owner handle | ○ |
| `dobHardOwner4` | 363 | Hard‑owner handle | ○ |
| `dobHardOwner5` | 364 | Hard‑owner handle | ○ |
| `dobHardOwner6` | 365 | Hard‑owner handle | ○ |
| `dobHardOwner7` | 366 | Hard‑owner handle | ○ |
| `dobHardOwner8` | 367 | Hard‑owner handle | ○ |
| `dobHardOwner9` | 368 | Hard‑owner handle | ○ |
| `dobHardOwner10` | 369 | Hard‑owner handle | ○ |
| `dobLineweight` | 370 | Lineweight enum (fixed) | ○ |
| `dxfLineweightCustom` | 371 | Custom dobject lineweight (if full range used) | ○ |
| `dxfLineweightCustom2` | 372 | Custom dobject lineweight | ○ |
| `dxfLineweightCustom3` | 373 | Custom dobject lineweight | ○ |
| `dxfLineweightCustom4` | 374 | Custom dobject lineweight | ○ |
| `dxfLineweightCustom5` | 375 | Custom dobject lineweight | ○ |
| `dxfLineweightCustom6` | 376 | Custom dobject lineweight | ○ |
| `dxfLineweightCustom7` | 377 | Custom dobject lineweight | ○ |
| `dxfLineweightCustom8` | 378 | Custom dobject lineweight | ○ |
| `dxfLineweightCustom9` | 379 | Custom dobject lineweight | ○ |
| `dxfPlotStyleNameType` | 380 | PlotStyleName type enum (fixed) | ○ |
| `dxfPlotStyleNameTypeC1` | 381 | Custom plot style name type | ○ |
| `dxfPlotStyleNameTypeC2` | 382 | Custom plot style name type | ○ |
| `dxfPlotStyleNameTypeC3` | 383 | Custom plot style name type | ○ |
| `dxfPlotStyleNameTypeC4` | 384 | Custom plot style name type | ○ |
| `dxfPlotStyleNameTypeC5` | 385 | Custom plot style name type | ○ |
| `dxfPlotStyleNameTypeC6` | 386 | Custom plot style name type | ○ |
| `dxfPlotStyleNameTypeC7` | 387 | Custom plot style name type | ○ |
| `dxfPlotStyleNameTypeC8` | 388 | Custom plot style name type | ○ |
| `dxfPlotStyleNameTypeC9` | 389 | Custom plot style name type | ○ |
| `dobPlotStyle` | 390 | Hard‑pointer handle to plot style object | ○ |
| `dxfPlotStyleHandleC1` | 391 | Custom plot style handle | ○ |
| `dxfPlotStyleHandleC2` | 392 | Custom plot style handle | ○ |
| `dxfPlotStyleHandleC3` | 393 | Custom plot style handle | ○ |
| `dxfPlotStyleHandleC4` | 394 | Custom plot style handle | ○ |
| `dxfPlotStyleHandleC5` | 395 | Custom plot style handle | ○ |
| `dxfPlotStyleHandleC6` | 396 | Custom plot style handle | ○ |
| `dxfPlotStyleHandleC7` | 397 | Custom plot style handle | ○ |
| `dxfPlotStyleHandleC8` | 398 | Custom plot style handle | ○ |
| `dxfPlotStyleHandleC9` | 399 | Custom plot style handle | ○ |
| `dxfShort400` | 400 | 16‑bit integer | ○ |
| `dxfShort401` | 401 | 16‑bit integer | ○ |
| `dxfShort402` | 402 | 16‑bit integer | ○ |
| `dxfShort403` | 403 | 16‑bit integer | ○ |
| `dxfShort404` | 404 | 16‑bit integer | ○ |
| `dxfShort405` | 405 | 16‑bit integer | ○ |
| `dxfShort406` | 406 | 16‑bit integer | ○ |
| `dxfShort407` | 407 | 16‑bit integer | ○ |
| `dxfShort408` | 408 | 16‑bit integer | ○ |
| `dxfShort409` | 409 | 16‑bit integer | ○ |
| `dobLayoutTab` | 410 | Layout tab name (string) | ○ |
| `dxfString411` | 411 | String | ○ |
| `dxfString412` | 412 | String | ○ |
| `dxfString413` | 413 | String | ○ |
| `dxfString414` | 414 | String | ○ |
| `dxfString415` | 415 | String | ○ |
| `dxfString416` | 416 | String | ○ |
| `dxfString417` | 417 | String | ○ |
| `dxfString418` | 418 | String | ○ |
| `dxfString419` | 419 | String | ○ |
| `dobTrueColor` | 420 | 32‑bit TrueColor value | ○ |
| `dxfTrueColor421` | 421 | 32‑bit TrueColor | ○ |
| `dxfTrueColor422` | 422 | 32‑bit TrueColor | ○ |
| `dxfTrueColor423` | 423 | 32‑bit TrueColor | ○ |
| `dxfTrueColor424` | 424 | 32‑bit TrueColor | ○ |
| `dxfTrueColor425` | 425 | 32‑bit TrueColor | ○ |
| `dxfTrueColor426` | 426 | 32‑bit TrueColor | ○ |
| `dxfTrueColor427` | 427 | 32‑bit TrueColor | ○ |
| `dobColorName` | 430 | Color name (string) | ○ |
| `dxfColorName431` | 431 | Color name | ○ |
| `dxfColorName432` | 432 | Color name | ○ |
| `dxfColorName433` | 433 | Color name | ○ |
| `dxfColorName434` | 434 | Color name | ○ |
| `dxfColorName435` | 435 | Color name | ○ |
| `dxfColorName436` | 436 | Color name | ○ |
| `dxfColorName437` | 437 | Color name | ○ |
| `dobTransparency` | 440 | Transparency value (32‑bit integer) | ○ |
| `dxfTransparency441` | 441 | Transparency value | ○ |
| `dxfTransparency442` | 442 | Transparency value | ○ |
| `dxfTransparency443` | 443 | Transparency value | ○ |
| `dxfTransparency444` | 444 | Transparency value | ○ |
| `dxfTransparency445` | 445 | Transparency value | ○ |
| `dxfTransparency446` | 446 | Transparency value | ○ |
| `dxfTransparency447` | 447 | Transparency value | ○ |
| `dxfLong450` | 450 | Long | ○ |
| `dxfLong451` | 451 | Long | ○ |
| `dxfLong452` | 452 | Long | ○ |
| `dxfLong453` | 453 | Long | ○ |
| `dxfLong454` | 454 | Long | ○ |
| `dxfLong455` | 455 | Long | ○ |
| `dxfLong456` | 456 | Long | ○ |
| `dxfLong457` | 457 | Long | ○ |
| `dxfLong458` | 458 | Long | ○ |
| `dxfLong459` | 459 | Long | ○ |
| `dxfDouble460` | 460 | Double‑precision floating‑point | ○ |
| `dxfDouble461` | 461 | Double‑precision floating‑point | ○ |
| `dxfDouble462` | 462 | Double‑precision floating‑point | ○ |
| `dxfDouble463` | 463 | Double‑precision floating‑point | ○ |
| `dxfDouble464` | 464 | Double‑precision floating‑point | ○ |
| `dxfDouble465` | 465 | Double‑precision floating‑point | ○ |
| `dxfDouble466` | 466 | Double‑precision floating‑point | ○ |
| `dxfDouble467` | 467 | Double‑precision floating‑point | ○ |
| `dxfDouble468` | 468 | Double‑precision floating‑point | ○ |
| `dxfDouble469` | 469 | Double‑precision floating‑point | ○ |
| `dxfString470` | 470 | String | ○ |
| `dxfString471` | 471 | String | ○ |
| `dxfString472` | 472 | String | ○ |
| `dxfString473` | 473 | String | ○ |
| `dxfString474` | 474 | String | ○ |
| `dxfString475` | 475 | String | ○ |
| `dxfString476` | 476 | String | ○ |
| `dxfString477` | 477 | String | ○ |
| `dxfString478` | 478 | String | ○ |
| `dxfString479` | 479 | String | ○ |
| `dxfHardPointer480` | 480 | Hard‑pointer handle | ○ |
| `dxfHardPointer481` | 481 | Hard‑pointer handle | ○ |
| `dxfComment` | 999 | Comment string (ignored by OPEN, not written by SAVEAS) | ○ |

---

## 📨 8. Extended Data (Xdata) – 1000 … 1071

Xdata is application-attached metadata. Codes do **not** carry the `dob` prefix
because they are payloads, not Dobject fields.

| Variable Name | DXF Code | Description | Status |
|---------------|----------|-------------|--------|
| `xdString` | 1000 | ASCII string up to 255 bytes | ○ |
| `xdAppName` | 1001 | Registered application name (up to 31 bytes) | ○ |
| `xdControl` | 1002 | Control string (`"{"` or `"}"`) | ○ |
| `xdLayer` | 1003 | Layer name associated with the xdata | ○ |
| `xdBinary` | 1004 | Binary chunk (hexadecimal, up to 254 chars) | ○ |
| `xdHandle` | 1005 | Dobject handle (16 hex digits) | ○ |
| `xdPntX` | 1010 | X of 3D point (followed by 1020, 1030) | ○ |
| `xdPntY` | 1020 | Y of 3D point | ○ |
| `xdPntZ` | 1030 | Z of 3D point | ○ |
| `xdWorldPosX` | 1011 | X of world space position (scaled, rotated, mirrored) | ○ |
| `xdWorldPosY` | 1021 | Y of world space position | ○ |
| `xdWorldPosZ` | 1031 | Z of world space position | ○ |
| `xdWorldDsplX` | 1012 | X of world space displacement (scaled, rotated, mirrored; not moved) | ○ |
| `xdWorldDsplY` | 1022 | Y of world space displacement | ○ |
| `xdWorldDsplZ` | 1032 | Z of world space displacement | ○ |
| `xdWorldDirX` | 1013 | X of world space direction (rotated, mirrored; not scaled) | ○ |
| `xdWorldDirY` | 1023 | Y of world space direction | ○ |
| `xdWorldDirZ` | 1033 | Z of world space direction | ○ |
| `xdReal` | 1040 | Double‑precision floating‑point value | ○ |
| `xdDistance` | 1041 | Double‑precision distance (scaled with dobject) | ○ |
| `xdScale` | 1042 | Double‑precision scale factor (scaled with dobject) | ○ |
| `xdInt16` | 1070 | 16‑bit signed integer | ○ |
| `xdLong32` | 1071 | 32‑bit signed long integer | ○ |

---

## When wiring begins

Future `cad_io` (or `cad_dxf`) crate will own the actual reader/writer.
When a code starts round-tripping for an entity type, flip its status here.
A reasonable first slice covers **LINE / CIRCLE / ARC / ELLIPSE / ELLIPSE_ARC**
with just these codes wired:

- `dxfType` (0), `dxfHandle` (5), `dxfSubclass` (100)
- `dobLayer` (8), `dobColor` (62), `dobLinetype` (6)
- `dobPrimaryPointX/Y/Z` (10/20/30), `dobOtherPointX1/Y1` (11/21)
- `dobReal40` (40 = radius), `dobAngle50/51` (50/51 = start/end angle)
- `dobExtrusionX/Y/Z` (210/220/230) — set to `(0,0,1)` for 2D

Everything else stays `○` until the corresponding entity type or feature lands.
