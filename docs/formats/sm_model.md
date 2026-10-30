# SpaceEngine `.sm` mesh format (SM01 / SM02) and model `.cfg` / `.sss` files

Status: reverse engineered from `SpaceEngine.exe` (Ghidra project `se`) and verified against all
52 `.sm` files in `planets/Asteroids/*.pak` (2014MU69, 67P, Eros, Itokawa, Oumuamua),
`spacecraft/ShipsModels.pak` (45) and `common/Common.pak` (2): every file parses to the last byte,
all floats are finite, all indices are `< vertex count`, normals are unit length, and the decoded AABB
centre equals the header centre. Two independent parsers (Python reference, Rust
`crates/se_core/src/model_sm.rs`) agree.

All values are **little endian**. There is **no compression** and no LOD data: a `.sm` is a flat list of
sub-meshes ("objects"). The format contains **no texture names**; objects only carry a material index
(see below).

## 1. File layout

```
SM01:  magic(4) | header(20) | object* 
SM02:  magic(4) | header(24) | object*
object = obj_header (12 bytes SM01 / 40 bytes SM02) | vertex data | index data
```

The file ends exactly after the last object (no trailer, no padding).

### 1.1 Header

| Offset | Type    | Field        | Notes |
|-------:|---------|--------------|-------|
| 0x00   | char[4] | magic        | `"SM01"` or `"SM02"` (strings at 0x140694750 / 0x140694758) |
| 0x04   | u32     | object_count | 1 for asteroids, 6 for DragonFly, 12 for Shuttle |
| 0x08   | f32[3]  | center       | centre of the AABB of all decoded vertices (exact match in all files) |
| 0x14   | f32     | radius       | bounding radius; SM01: half AABB diagonal; SM02 writers: half diagonal x 0.866 (DragonFly/Skylone: x 1.0). Writer dependent, treat as a hint |
| 0x18   | u32     | vertex_fmt   | **SM02 only**, 0..5, see 1.3. SM01 has no such field and is always format 0 |

Header size: 0x18 (SM01) / 0x1c (SM02). The first object header starts there.

### 1.2 Object header

| Offset | Type   | Field       | Notes |
|-------:|--------|-------------|-------|
| +0x00  | u32    | vertex_count| |
| +0x04  | u32    | index_count | **u32 indices**, always |
| +0x08  | u16    | material    | index into the model `.cfg` `Material "..."` list (see 4). DragonFly objects 0..5 map to its 6 materials; ApolloEagle uses 1 |
| +0x0a  | u16    | primitive   | file key, see table below. Every shipped file uses 0 (triangle list) |
| +0x0c  | f32[3] | pos_offset  | **SM02 only.** Used by vertex formats 4/5 (default 0,0,0) |
| +0x18  | f32    | pos_scale   | SM02 only (default 1) |
| +0x1c  | f32[2] | uv_offset   | SM02 only, format 4 (default 0,0) |
| +0x24  | f32    | uv_scale    | SM02 only, format 4 (default 1) |

SM01 object headers are 12 bytes and the transform is implicitly `(0,0,0) 1 (0,0) 1`.
All shipped SM02 files with vertex format < 4 have exactly this identity transform.

Followed by `vertex_count * stride(vertex_fmt)` bytes of vertices and `index_count * 4` bytes of indices.
Objects are stored back to back (header, vertices, indices, next header...).

Primitive key to GL primitive (table at 0x140677640, 9 pairs `(file_key, gl_enum)`; unmatched keys decode
to 0 = `GL_POINTS`): 0 triangles, 1 triangle strip, 2 triangle fan, 3 quads, 4 quad strip, 5 lines,
6 line strip, 7 line loop, 8 points. Only 0 occurs in the game data; the others are untested.

### 1.3 Vertex formats

Stride table at 0x1406784b0 = `[48, 40, 24, 20, 20, 16]`. Layouts, derived from the converter
(FUN_14032f190, which decodes every format into a 48-byte canonical float vertex
`pos3 normal3 tangent4 uv2`):

| fmt | stride | Layout (byte offsets) |
|----:|-------:|-----------------------|
| 0 | 48 | f32 pos[3] @0, normal[3] @12, tangent[4] @24 (xyz + w = handedness +-1), uv[2] @40 |
| 1 | 40 | as 0 without uv |
| 2 | 24 | IEEE half pos[3] @0, normal[3] @6, tangent[4] @12, uv[2] @20 |
| 3 | 20 | as 2 without uv |
| 4 | 20 | i16 pos[3] @0, 2 bytes padding @6, u32 packed normal @8, u32 packed tangent @12, i16 uv[2] @16 |
| 5 | 16 | as 4 without uv |

Which formats occur: format 0 in SM01 files, in Eros/Itokawa/2014MU69/ApolloEagle/radiators; format 4 in
67P, Oumuamua, AmS-*, DragonFly, Shuttle, Skylone, VR controllers. Formats 1, 2, 3, 5 never occur in
the shipped data (decoder derived from code only, untested on real files).

Decoding (format 4/5; the engine divides by 32767.0, constant at 0x1406d5bf8):

```
pos = (i16 / 32767.0) * pos_scale + pos_offset          (per axis)
uv  = (i16 / 32767.0) * uv_scale  + uv_offset
```

(The offset is the AABB centre of the object, `pos_scale` the largest half extent after centring; the
UV offset/scale are the centre/half range of the UV bounding box, typically `0.5, 0.5, 0.5`.)

Packed normal, `u32`, three 10-bit fields at bits 0, 10, 20 (FUN_140442460; divisor 511.0, constant
0x1406d5a18). Each field `v`: `v < 512 ? v : -(1023 - v)`, result `/ 511`.
Packed tangent (FUN_140442530): same three fields, plus bits 30..31 handedness `w`: `w < 2 ? w : -(3 - w)`
i.e. 0 -> 0, 1 -> +1, 2 -> -1, 3 -> -0. About 1500 of 378k vertices in AmS-B decode to a zero normal
(degenerate triangles); all others are unit length.

Half floats decode with the standard IEEE binary16 conversion (FUN_14003bce0).

### 1.4 Winding, UVs and handedness

* Triangles are counter-clockwise (face normals agree with vertex normals in 100% of Eros triangles).
* Geometry is stored flat/unwelded in many files (e.g. Eros: 4608 vertices, 1536 triangles, 770 unique
  positions, indices are simply `0..n-1`; Euler characteristic V-E+F = 2, closed mesh).
* UVs are used as stored (range 0..1, some ship modules go slightly outside, e.g. -0.98..0.98).
* Axes are the engine's own model space; coordinate conversion to Bevy is left to the caller.

### 1.5 Size / scale relationship with `.cfg`

Model units are scaled by the `.cfg` `Unit` (metres per model unit). Verified: Shuttle `Unit 0.0254`
(inches) with `Length 63.581` = `2 * radius * Unit`; Skylone `Unit 1`, `Length 40.27867` = `2 * radius`.
`seMesh::Load` (FUN_1402a3f30, after the loader at 0x1402a4dde) does: if `Unit == 0`: if `Length == 0`
then `Length = 2r * 0.001`, `Unit = 0.001`, else `Unit = Length / max(2r, 1e-6)`; if `Unit != 0` and
`Length == 0` then `Length = 2r * Unit`. (The Eros cfg has `Length` 10x larger than `2r*Unit`;
presumably handled by the `.sss` Module matrix scale 16 and its own `Length 35000`; not investigated.)

## 2. Ghidra evidence (SpaceEngine.exe, image base 0x140000000)

| Address | Role |
|---------|------|
| `FUN_1402a5c50` | `seMesh::LoadBinary`: reads magic (compares to `SM01`/`SM02`), 0x14 / 0x18 byte header, then per object the 0xc / 0x28 byte header, vertex block (`vertex_count * stride`), index block (`index_count * 4`). Error strings at 0x140694720..0x140694850 |
| `FUN_1402a5810` | `seMesh::SaveBinary`: writes `SM02`, count, centre, radius, vertex format; confirms field order and the primitive key mapping (default key 8) |
| `FUN_1402a3f30` | `seMesh::Load`: opens file (archive or loose), calls LoadBinary, resolves cfg material names (error "unknown material"), applies Unit/Length logic |
| `FUN_14032ee20` | `seMeshVBO::AllocateRAM(format, vcount, icount)`: `vbytes = vcount * stride[fmt]`, `ibytes = icount * 4` |
| `FUN_14032f190` | vertex format converter: per-format decoders (cases 0..5), computes centre/scale/UV centre/scale for formats 4/5 and stores them at object +0x2c..+0x44 |
| `FUN_140442460` / `FUN_140442530` | unpack 10:10:10 normal / 10:10:10:2 tangent; encoders `FUN_140442700` / `FUN_1404427f0` |
| `FUN_14003bce0` | half to float |
| `0x1406784b0` | stride table, `0x140677640` primitive key table, `0x1406d5bf8` = 32767.0, `0x1406d5a18` = 511.0 |

In-memory mesh object (size 0x58, array at `seMesh+0x48`, count at `seMesh+0x44`): +0x00 vertex
format, +0x10 vertex count, +0x14 index count, +0x18 vertex bytes, +0x1c index bytes, +0x24 material
(u16), +0x28 GL primitive, +0x2c pos offset[3], +0x38 pos scale, +0x3c uv offset[2], +0x44 uv scale,
+0x48 vertex pointer, +0x50 index pointer. `seMesh`: +0x80 centre[3], +0x98 radius, +0x9c Length,
+0xa0 Unit.

## 3. Verification summary

* 52 / 52 `.sm` files parse (Python reference and Rust). Totals include 67P (12.3 M vertices, 295 MB) and
  Oumuamua (2.36 M vertices).
* SM01: 36 files (ship modules), vertex format 0. SM02: 16 files. Objects per file: 1 (all but DragonFly
  6 and Shuttle 12).
* Eros exported to OBJ: 4608 vertices / 1536 triangles, matching header counts, positive signed volume,
  Euler characteristic 2. Output at
  `.../scratchpad/sm/Eros.obj` (session scratchpad).

## 4. Companion files

### 4.1 Model `.cfg` (text, 54 files)

Same `Key value` / `Key { ... }` syntax as other SE scripts (`//` comments). Keys seen:

| Key | Meaning |
|-----|---------|
| `Unit` | metres per model unit (1, 0.0254, 0.0645161) |
| `Length` | nominal length in metres (see 1.5) |
| `Mass` | kg |
| `Mesh "path.sm"` | mesh path relative to `data/models/`, case-insensitive (`spacecraft/Modules/x.sm`) |
| `Material "name"` | repeated; the n-th entry is material index n for object `material == n`. Names are material presets (e.g. `Asteroid_Eros`, `Shuttle`, `SHW_Default`) defined elsewhere (`data/.../MaterialsPresets`), which is where the textures are referenced |
| `Type` | at top level: module type `"Engine"`, `"Shield"` etc. (inside blocks: effect type) |
| `ThrustMain`, `ThrustCorr`, `ThrustRetro` | engine thrust (modules) |
| `EngineEffect "name" { Group Type Pos Rot Size Color Sound Volume [BrightnessHDR BrightnessReal SoundVolume] }` | engine flame/sound placement (`Type` `"Fusion"`, `"Ion"`, `"Aerospike"`); Pos/Rot(deg)/Size/Color are `(x y z)` tuples |
| `DockingPort "name" { CorrPos CorrRot CorrSize ShipPos ShipRot }` | docking geometry (27 blocks) |

Counts over the archives: Type 223, EngineEffect/Group/Pos/Rot/Size/Color/Sound 156 each, Volume 152,
Material 70, Unit/Length 54, Mass 48, DockingPort and its 5 sub keys 27, Mesh 19, Brightness* 4,
ThrustMain 2, ThrustCorr 2, ThrustRetro 1. Asteroid cfgs have only `Unit Length Mesh Material`.

### 4.2 `.sss` (text, 36 files; no binary content found)

Plain text script describing the placed object, **not** mesh data. Keys: `Name`, `Class`
(`"Starship"`, `"Probe"`...), `Pack`, `Faction`, `Type` (`"BodyModel"` for asteroids), `Length`,
`Offset (x y z)`, `Quat (w x y z)`, `Mass`, `Albedo`, `Exposure`, `Color (r g b)`, `RadTemp`,
`MainEngines`, `RetroEngines`, `HoverEngines`, `CorrEngines`, `TurnEngines`, `WarpBoostLog`,
`Warpdrive`/`Hyperdrive`, `Aerodynamics`, `AeroModel`, `Camera { "Name" (x y z) (quat) }` and
`Module "id" { "path.cfg" (4x4 column-major matrix, 16 numbers) }` which instantiates a `.cfg`
(25050 `Module` lines in total across the 36 files; a few ship files are very large). Asteroid example:
`Module "Main" { "asteroids/Eros.cfg" (16 0 0 0 0 16 0 0 0 0 16 0 0 0 0 1) }`.
Ship modules compose ships from `.cfg` files (`Modules/cargo.cfg`, ...), each referencing one `.sm`.

## 5. Uncertain / not verified

* Meaning of `material` (u16 at +0x08) is inferred from data (DragonFly 0..5, Apollo 1), not from a
  decompiled consumer.
* Exact definition of header `radius` for SM02 (observed 0.866 x half-diagonal, or 1.0 x).
* Formats 1, 2, 3, 5 and primitive types 1..8 are decoded from code only; no shipped asset uses them.
* Primitive restart / strip handling for non-triangle-list primitives is not known.
* Eros `Length` vs `2*radius*Unit` mismatch (factor 10) is unexplained.
