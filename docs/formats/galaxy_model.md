# Galaxy models (`.pst`, `GalaxiesTex.pak`, `seSpriteModelMaker`)

How SpaceEngine 0.991 (Windows x64) picks and builds the 3D sprite model of a catalogue galaxy.
Derived from `SpaceEngine.exe` (Ghidra, no symbols; names below come from RTTI and log strings)
and from the shipped data. The Rust implementation is `crates/se_core/src/galaxy_model.rs`.

Legend: **[V]** verified in decompiled code or data, **[I]** inferred (plausible, not proven),
**[?]** unknown. Shader code (`Shaders.pak`) is encrypted and was not read, so everything that
happens on the GPU is at best **[I]**.

## 1. Pipeline overview

1. `seGalModelCache::Init` (`FUN_140265710`) reads every `*.pst` in `data/models/galaxies/` (the pak
   holds `Default.pst` and `Kexitt.pst`) and keeps all `GalaxyModel` blocks. [V]
2. `FUN_140266150` assigns a model index to every catalogue galaxy: `UseForObject` first, then a
   position hash over the pool of models sharing the galaxy's Hubble type. Types E0..E7 get no
   model (index -2) and are drawn procedurally. [V]
3. A model is generated lazily when its galaxy first becomes visible (`CheckGlowModel::Create`,
   `FUN_14049cdb0` -> `FUN_140266940` -> `FUN_14029b5f0` -> `FUN_1402db910`). [V]
4. `FUN_1402db910` seeds the RNG from the model name and dispatches on `Method` to
   `createGalaxy / createCylindrical / createSpherical / createFlat / createCubeMap /
   createDiffuse / createNebula`. Each builds 3D voxel maps, then turns the surviving voxels into
   one sprite each (`createModelCylindrical` / `createModelSpherical`), uploads a VBO and builds a
   `BBoxRes^3` culling grid (`seSpriteVBO::BuildTree`, `FUN_140334480`). [V]
5. Per frame `FUN_140299fe0` computes fade factors and `FUN_14029bdf0` draws the VBO with
   `galaxy_model.glsl` (uniform block `GalaxyModelUniforms { ModelViewMatrix; ProjectionMatrix;
   BrightZoom; Reddening; Clip }`, blend `glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA)`). [V]

## 2. Which galaxies get which model

### 2.1 Hubble type index (`FUN_14016a7a0`, CSV reader) [V]

| Catalogue `Type` | index | rule |
|---|---|---|
| `E0`..`E7` | 1..8 | `'0'..'7'` -> `c - 0x2f` |
| `S0` | 9 | |
| `Sa` `Sb` `Sc` `Sd` | 10 11 12 13 | `c - 0x57`, second char `a..d` |
| `SBa` `SBb` `SBc` `SBd` | 13 14 15 16 | `c - 0x54`, third char `a..d` (`SBab` -> 13) |
| `Irr` (exactly 3 chars) | 16 | |
| anything else (`E`, `Ir`, `Sm`, ...) | 0 | |

`Sd` collides with `SBa`, and `SBd` with `Irr`, exactly as in the exe. Only indices 9..16 ever
receive a sprite model. Indices 0..8 are rendered by `galaxy_bulge.glsl` (section 8). [V]
The catalogs contain `E0..E6, S0, Sa, Sb, Sc, SBa, SBb, SBc, Irr` (+ one `SBab`).

### 2.2 Assignment (`FUN_140266150`) [V]

1. For every model, `UseForObject` is split on `/` (`"NGC 4608/NGC 1211"`); each name is looked up in
   the global object-name table. A found galaxy gets that model. If two models name the same
   galaxy the one with the higher file priority (field at model+0x7fa0, equal priority = later
   wins with a log message) wins. Unknown names are logged.
2. A galaxy still without a model and with type index >= 9 gets
   `pool[type][ h % pool[type].len ]`, `h = (u32)(ix*0x11adc + iy*0xd191 - iz*0x5754)` with
   `i* = (int)(pos* * 0.01)` (coordinates wrapped into +-2^31 first). `pool[type]` contains **every**
   model whose `UseForType` has that index, including models that also have a `UseForObject`
   (so a random Sb galaxy can look like M 31). Pool sizes in the shipped data: S0 10, Sa 13,
   Sb 26, Sc 11, SBa 4, SBb 25, SBc 42, Irr 11.
3. Types 0..8 -> model index `0xfffe` (= -2, "no model").

[I] `pos*` is the float position stored in the exe's galaxy record (offsets +0x24/+0x28/+0x2c).
The unit is most likely parsecs relative to the catalog origin; I did not verify it, so the
*exact* random pick for a given galaxy is only reproducible once that is checked.
[I] `UseForType` is converted to the same 1..16 index as the catalogue type (the conversion
function was not located; the 8 type names in the pst files map one-to-one onto 9..16 and the
pool sizes match).

### 2.3 Name -> texture rules

The model, not the galaxy name, decides the textures; names in the pst are patterns:
`FrontTexture "NGC4038.*"`, `SysTexture "NGC4038_sys.*"`, `FrontImage "NGC4038_small.*"`,
`SideImage "Side1.*"`; `.*` means any extension. Lookup is **case-insensitive** (the pst says
`NGC1566.*`, the pak has `ngc1566.png`; `NGC2997_sys.*` -> `ngc2997_sys.jpg`). [V]

`GalaxiesTex.pak` (505 files, plain zip, 767 MB):

| files | format | role |
|---|---|---|
| `<name>.png` (139 x 2048^2, 2 x 4096^2, 2 x 1024^2, a few odd sizes) | RGBA8 | `FrontTexture`: face-on image, full resolution |
| `<name>_small.png` (154, 512^2) | RGBA8 | `FrontImage`: same image downsampled; the generator input [V] |
| `<name>_sys.jpg` (same size as the front texture) | RGB8 | `SysTexture` [I, see below] |
| `Side1.png` 512x252, `Side2.png`, `S05_side.png`, `Coll_side.png` (128x63) | RGBA8 | `SideImage`: edge-on profile |
| `<name>_posx/_posy/_posz.png` (+ `_negx..` if present) | RGBA8 | cube-map faces (`CubeMap` models) |

Generic models (no catalogue name) use `S01..S06` (S0), `Sa1..Sa4`, `Sb2 Sb3`, `SBb1 SBb2`, `SBc`,
`Irr`-type models have no image (procedural noise). 169 models in total: 9 in `Default.pst`
(NGC 4038, NGC 4039, Hoag, Irr1..3, Arp 148, Arp 148 Disk, Centaurus A) and 160 in `Kexitt.pst`
(151 `SpiralGalaxy`, 9 `CubeMap`). 150 models carry `UseForObject` (6 + 144); 123 of them also carry `UseForType`.

### 2.4 Texture channels [V from data and generator code]

* **RGB** = stellar emission colour/brightness of the galaxy seen face-on, **straight (not
  premultiplied)**: the PNGs keep full-white RGB under `alpha == 0` (e.g. the centre of
  `MilkyWay.png` is `(255,255,255,0)`). Load them without any alpha premultiplication.
* **A** = dust/absorption density (high on the lanes along the spiral arms, 0 in the bulge). It is
  *not* a coverage mask. In the generator it feeds the absorption layer only.
* `_small.png` has statistics identical to the large file (a 4x box downsample).
* `_sys.jpg`: RGB only, blue-tinted, same footprint. Its consumer was not traced. The strings
  `seGalSysModel::CreateClusterSystem(): Galaxy "%s" model has no emMap` show that the galaxy
  interior (stars/clusters/nebulae) is generated from the model's *emission voxel map*; the sys
  texture is probably the colour/star-population map for that. [I]
* Side images are left/right mirror symmetric (error < 0.4 %). Their horizontal axis is radius^2
  (see 4.1) and the vertical axis is height above the disk; the bright part is a thin horizontal
  band around the middle row (thin disk), `Side2` is a variant used by the LMC. [V]
* `sprites_gal.png` (`Common.pak`, 2048^2, 8-bit grey): a 4x4 atlas of the sprite shapes. Tile 0
  (top-left) is a smooth round blob, tiles 1..15 are cloud shapes. The vertex "type" byte indexes
  it. [V for layout, I for the tile ordering row-major]

## 3. `.pst` syntax

Plain ASCII, CRLF, `//` comments, `Key value` lines, one block per model:

```
GalaxyModel "NGC4038"
{
    UseForObject "NGC 4038"          // slash separated list; and/or:
    UseForType   "SBc"
    Method "SpiralGalaxy"
    Position (0, 0, 0)
    Radius   (1.1, 0.1, 0.04)        // (disk radius, bulge radius, bulge thickness)
    ...
}
```

Values: numbers, quoted strings, tuples `(a, b, c)`. A missing key keeps the default and only
logs `Missing parameter` (`FUN_1402c4fd0`). Blocks need `UseForObject` or `UseForType`
(`seSpriteModelMaker::ReadFromScript` error otherwise). Parser: `FUN_1402dc0c0`. [V]

### 3.1 Top-level keys (struct offsets in the maker, exe default in brackets)

| key | off | meaning |
|---|---|---|
| `UseForObject` | 0x20 | catalogue names, `/` separated |
| `UseForType` | 0x40 | Hubble type name |
| `GroupModel` | 0x60 | unused by the galaxies |
| `Method` | 0x460 | `SpiralGalaxy`=0 `EllipticGalaxy`=1 `Cylindrical`=2 `Spherical`=3 (default) `Flat`=4 `CubeMap`=5 `Diffuse`=6 `Planetary`=7 `SNR`=8 |
| `Position` | 0x80 | model offset (always 0 in shipped data) |
| `Radius` | 0x8c | (disk, bulge, bulge thickness) in *model radii*; see 7.1 |
| `BBoxRes` | 0x98 | culling grid resolution (8,8,8) (4,4,4 for cube maps, one 10,10,10) |
| `Yaw` `Pitch` `Roll` | 0xa4 0xa8 0xac | extra rotation (not used in the shipped files) |
| `SpriteOblZ` | 0xd8 | [1] z oversampling; also multiplies the model z scale |
| `floatLOD` | 0x464 bit0 | [0] continuous LOD (1 only for cube maps) |
| `LODbase` | 0xd4 | [100] LOD distance base (1 for spirals, 50 for cube maps) |
| `useFBO` | | 1 on 10 cube-map models (render the model to an FBO) [I] |
| `FrontImage` `SideImage` | 0xf8 0x118 | generator input images |
| `CubePosX/NegX/PosY/NegY/PosZ/NegZ Image` | 0x138 0x158 .. 0x1d8 | cube faces; a missing `Neg*` copies the `Pos*` name |
| `FrontTexture` `SysTexture` | 0x1f8 0x218 | GPU textures |
| `Brightness` `Transparency` | 0x238 0x23c | [1, 1] multipliers, see 6.3 |
| `BulgeRadius` | 0x438 | [(1,1,1)] bulge gaussian axis exponents |
| `sumBright` `maxBright` `clipRadius` | 0x444.. | [1000, 0.2, 1.1] statistics that the generator overwrites |

`Yaw/Pitch/Roll` key names were read from the exe strings at 0x14067e124/130 and "Pitch". [V]

### 3.2 Particle layers

Three layers with identical key sets and prefix `em` (emission), `abs` (absorption), `b` (bulge,
not for cube/diffuse); block offsets in the maker: em 0x240, abs 0x2e8, bulge 0x390 (stride 0xa8).
Defaults come from `FUN_140049670`; the bulge colour default (1, 0.8, 0.5) from `FUN_1402dbda0`.

| key suffix | default | meaning |
|---|---|---|
| `DetailR` `DetailZ` | 1 1 | voxel grid resolution multipliers (x/y, z); see section 4 |
| `ParticleDispR` `ParticleDispZ` | 0 0 | random jitter of a sprite inside its voxel, in voxel sizes (x/y, z) |
| `ParticleSizeCenter` `ParticleSizeEdge` | 1 1 | sprite size in voxel sizes, at r=0 / r=1 |
| `ParticleMinBrightCenter` `Edge` | 1 1 | voxel is dropped if `r+g+b` is below this (r=0 / r=1) |
| `ParticleBrightnessCenter` `Edge` | 1 1 | sprite intensity multiplier (r=0 / r=1) |
| `ParticleColor` | (1,1,1) | tint, the voxel colour is normalised to this |
| `ParticleColor2` | (1,1,1) | nebula models only |
| `NoiseFreq` `NoiseDistort` `NoiseH` `NoiseLacunarity` `NoiseOffset` `NoiseOctaves` `NoiseRandomize` | 1, 0.7, 0.6, 2.2183, 0.8, 8, (0,0,0) | `Diffuse` only |

Interpolation `lerp(Center, Edge, t)` is `Center` for t<0, `Edge` for t>1 (`t` = radius squared or
voxel value, see 5). `NoiseOffset` is parsed but no traced noise routine reads it. [V]

## 4. Voxel map construction

Common: grid cell centres of an axis with `n` cells: `step = 2/n`, `c_0 = step/2 - 1`,
`c_{i+1} = c_i + step` (float accumulation), i.e. `[-1, 1]` for every axis. Loop order: x
outermost, y, z innermost. Memory layout `((i*ny + j)*nz + k)*3` floats. [V]

Image sampling `FUN_1402e4320` (RGB) / `FUN_1402e44d0` (one channel, alpha = last channel):
coordinates `u,v` in `[-1,1]`, pixel position `(u+1)*0.5*size` (**no half-pixel offset**), integer
part truncated, bilinear between `(ix,iy) (ix+1,iy) (ix,iy+1) (ix+1,iy+1)`, texel indices clamped to
the image, channels `/255`. Row 0 of the file is `v = -1`. [V]

Before any layer is built, `createGalaxy` rescales the brightness pairs:
`BrightnessC,E *= 4 / ((SizeC+SizeE) * (DetailR+DetailZ))` for em, abs and bulge separately
(`createDiffuse`: `/ ((SizeC+SizeE)*(DetailR+DetailZ)*0.25)`, same value;
`createCubeMap`: `/ ((SizeC+SizeE)*DetailR*0.5)`). [V]

### 4.1 `SpiralGalaxy` (method 0; `FUN_1402d85c0`)

`side_aspect = side.height * 128 / side.width`.

**Emission map**: `nx = ny = (int)(emDetailR*128)` (0 -> 128), `nz = (int)(side_aspect / SpriteOblZ *
emDetailZ)` (0 -> 15). For every voxel with `r2 = x^2+y^2 <= 1`:

```
f  = front.rgb(x, y)
A  = side.rgb(+r2, z) ; B = side.rgb(-r2, z) ; t = x*0.5 + 0.5
s  = t<0 ? A : t>1 ? B : lerp(A, B, t)        // left/right mix (the images are symmetric)
v  = s * f                                      // per channel
v  = 0 if (v.r+v.g+v.b) < lerp(emMinBrightC, emMinBrightE, r2)
```

Outside the disk (`r2 > 1`) the voxel is 0. **Absorption map**: `nx=ny=(int)(absDetailR*128)`
(0 -> 128), `nz` as above with `absDetailZ` (0 -> 31); `fa = front.alpha(x,y)`,
`v = lerp(side.alpha(+r2,z), side.alpha(-r2,z), t) * fa`, dropped if `3*v < lerp(absMinBrightC,
absMinBrightE, r2)`, stored as `(v,v,v)`. **Bulge map** (also for method 1):
`nx=ny=(int)(bDetailR*15)` (0 -> 15), `nz=(int)(bDetailZ*15/SpriteOblZ)` (0 -> 15), with
`r3 = x^2+y^2+z^2`:

```
v = exp(-2 x^2 BulgeRadius.x) * exp(-2 y^2 BulgeRadius.y) * exp(-2 z^2 BulgeRadius.z)   (0 if r3 > 1)
v = 0 if 3*v < lerp(bMinBrightC, bMinBrightE, r3)       stored as (v,v,v)
```

The emission map is kept by the model (`+0x470`) for the galaxy interior generator. [V] The three
maps are built first, then the VBO; the sprite pass runs em, abs, bulge in that order.

Disk model scale factors passed to the sprite pass: x,y = 1, z = `nz/nx`, i.e. voxels are cubes of
side `2/nx` and the disk half-thickness is `nz/nx` disk radii. Bulge: x,y = `Radius.y/Radius.x`,
z = `(Radius.z/Radius.x) * nz/nx`. [V]

### 4.2 `CubeMap` (method 5; `FUN_1402d4c70`)

`n = (int)(DetailR*64)` (0 -> 64) for em (`emDetailR`) and abs (`absDetailR`), cubic grid, scale
(1,1,1), sprite pass spherical. With `tx = x*0.5+0.5` etc.:

```
cx = lerp_clamped(PosX.rgb(y,z), NegX.rgb(y,z), tx)
cy = lerp_clamped(PosY.rgb(x,z), NegY.rgb(x,z), ty)
cz = lerp_clamped(PosZ.rgb(x,y), NegZ.rgb(x,y), tz)
em  = cx*cy*cz  (per channel)          abs = ax*ay*az (alpha channels, stored as grey)
```

(`lerp(first, second, t)`: the Pos image is the `t=0` end, as decompiled; shipped Neg faces are
identical copies of Pos, so the choice is visually moot.) Thresholds as in 4.1 with `r2 = x^2+y^2+z^2`;
abs uses `3*v < minBright`. No bulge layer. [V]

### 4.3 `Diffuse` (method 6; `FUN_1402d3f10`; irregular galaxies `Irr1..3`)

`nx=ny=(int)(DetailR*64)`, `nz=(int)(DetailZ*64)` (0 -> 64), no images. For em and abs separately
with their own noise parameters:

```
p = (x*freq + randomize.x, y*freq + randomize.y, z*freq + randomize.z)
v = (1 - |(x,y,z)|) * clamp(fbm(p, distort), 0, 1)       stored as (v,v,v)
```

Voxel kept in the sprite pass if `lerp(minBrightC,E,r3) <= 3v`. `fbm` (`FUN_140048be0`):
`q = p + distort * warp(p)`, then for `i < octaves`: `sum += lacunarity^(-H*i) * n1(q)`,
`q *= lacunarity`. `warp` (`FUN_1400490a0`): `w = 0; amp = 1; for each octave { w += amp *
(n1(q), n1(q+(3.33,5.71,1.96)), n1(q+(7.77,2.65,4.37))); amp *= H; q *= lacunarity }`.
`n1` (`FUN_140049390`) is value noise on Ken Perlin's reference permutation (table at
0x14069b2c0, doubled to 512): lattice value `g[i] = 2*perm[i]/255 - 1`, corner hash
`perm[perm[perm[X]+Y]+Z]` with the two Z corners taken as `g[idx]` and `g[idx+1]`, fade `3t^2-2t^3`,
trilinear. The implementation embeds the same permutation, so Diffuse output should match
bit for bit up to float rounding. [V]

### 4.4 Other methods

`Cylindrical/Spherical/Flat` build generic maps from images (`FUN_1402d75c0 / d6920 / d5cf0`);
`Planetary/SNR` are procedural nebula methods (`FUN_1402d3360`). None is used by a shipped galaxy model.
`Cylindrical`, `Spherical`, `Planetary` and `SNR` are implemented for nebulae (see
`docs/formats/nebula.md`); `Flat` is not (no shipped model uses it).

## 5. Sprite pass (`FUN_1402da540` cylindrical, `FUN_1402d9a10` spherical)

Per voxel in the same loop order, with `(r,g,b)` the map value, `sum = g+r+b`:

1. `r2 = x^2+y^2` (cylindrical, disk layers) or `x^2+y^2+z^2` (spherical: bulge, cube, diffuse),
   from the voxel centre.
2. Skip unless `lerp(MinBrightC, MinBrightE, r2) <= sum`.
3. `bright = lerp(BrightnessC, BrightnessE, r2)` (after the normalisation in 4);
   `intensity = sum * (bright / 3)`.
4. `maxc = max(r,g,b)`; if `maxc != 0`: colour `= (r,g,b) * ParticleColor / maxc` (per channel).
5. Statistics: `sumBright += r'+g'+b'`; `maxBright = max(intensity)`; `sumColor += c'/intensity`.
6. Size: cylindrical `lerp(SizeC, SizeE, r2) * step_x`; spherical `scale_x * step_x *
   lerp(SizeC, SizeE, maxc)` (the bulge/cube/diffuse size is interpolated by the *voxel value*,
   not by radius; as decompiled).
7. RNG: three draws `u1,u2,u3` (`rand = (float)state * 2^-32`):
   `pos.x = ((1-2u1) * DispR * step_x + x) * scale_x`, `pos.y = ((1-2u2) * DispR * step_y + y) *
   scale_y`, `pos.z = ((1-2u3) * DispZ * step_z + z) * scale_z`. A fourth draw gives the LOD random
   byte `(u4*255)`. A fifth draw (`state % 15 + 1`) gives the atlas tile **unless** the model is a
   spiral/elliptic one and the layer is emission or bulge (then tile 0). Absorption sprites of
   spiral galaxies therefore use random cloud tiles 1..15; every layer of Diffuse/CubeMap models
   does.
8. `clipRadius = max(|pos| + |size|)`.
9. Output 16-byte vertex x4 (one per quad corner, corner id from table 0x14067794c): half-float
   `pos.xyz`, half-float `size`, `rgb` bytes (`clamp(c')*255`), intensity byte (`min(1,intensity)*255`),
   atlas tile byte, corner byte, LOD random byte, layer byte (255 for emission and bulge, 0 for
   absorption). [V]

**RNG** [V]: LCG `s = s*1664525 + 1013904223` (u32) held in TLS (`tls+0x24`), seeded in
`FUN_1402db910` with `h = c + 2*h` over the bytes of the model name (signed char, 32-bit wrap), so
the output is a pure function of (model name, inputs). The sequence is consumed only in the sprite
pass: em, abs, bulge.

After all layers `FUN_1402db910` normalises `sumColor` to max channel 1 (-> bytes at model+0x7df8)
and `FUN_14029b5f0` stores `sqrt(sumBright)` (model+0xcc) and `clipRadius` (model+0xc8). The model
transform (`FUN_140299130`): position = `Position`; scale = `(Radius.x, Radius.x, Radius.x *
SpriteOblZ)`; for `EllipticGalaxy` the scale is `BulgeRadius`-based (`normalize(BulgeRadius) *
Radius.x * 3`); Yaw/Pitch/Roll as a rotation matrix. [V]

Zero-valued voxels pass step 2 when `MinBrightEdge == 0` (bulge, r3 > 1 corners) and become
invisible sprites in SE. The Rust generator consumes their random numbers but drops them.

## 6. Rendering parameters

### 6.1 Configuration (`config/main-def.cfg`) [V text]

`GalaxyMagnLimit 7`, `GlowMagnLimit 7` (galaxy/nebula *models* limiting magnitude),
`GalaxyBrightMin 0.03`, `GlowBrightMin 0.006` (sprite cut-off pixel brightness),
`AutoGalaxyMagnitude true` / `AutoGalaxyMagnSpeed 10` (raise the magnitude limit in intergalactic
space), `hdrGalaxySpriteBright 0.03`, `realGalaxySpriteBright 0.01`, `hdrGalaxyRaymarchBright 0.03`,
`realGalaxyRaymarchBright 0.01`, `EnableGalaxyFBO/Skybox/Impostors true`, `Memory`: `MaxSizeGalModels 64`
models cached, `MinGalModelsVRAM 10` MB. `GalaxyGenerator` block: `GalaxyMaxAppMagn 12.1`,
`GalaxyEllipticalScale 2.4`, `GalaxyEllipticalColor (1.0 0.9 0.7 0.0)`, `GalaxyEllipticalBright 1.0`,
`GalaxyEllipticalGamma 0.7`, `GalaxyFrontTexBright 1.0` (spiral front texture brightness).

### 6.2 Fade factors (`FUN_140299fe0`, per galaxy per frame)

With `d` the camera-galaxy distance, `R = galaxy.radius` (galaxy+0x20), `L = galaxy+0x1c`:

* `px = f(R/d) * K_px` (apparent radius in pixels; `f` = `FUN_1405e9de0`, probably atan/asin, `K_px` a
  runtime global). [V structure, I for f]
* `bright_term = clamp(K*L/d^2 - 0.05, 0, 1)` (`K` runtime). [V structure]
* `fade_in = clamp((px - 2) / 3, 0, 1)`: models appear between 2 and 5 px radius. [V]
* `p1 = fade_in * bright_term * galaxy[+0xa4]`; `p3 = clamp(1 - px*1e-7, 0, 1)` (fade-out when the
  camera is deep inside); `p2 = p1 * cfg35ac * cfg35ec * p3 * calib * galaxy[+0x74]`.
* distance ramps `1 - d/D`, `1 - 2d/D`, `1 - 4d/D` (`D` runtime global) stored for the shader. [V
  structure, ? meaning]

### 6.3 Draw call (`FUN_14029bdf0`)

`BrightZoom = (calib * galaxy[+0x74] * model.Brightness [* p3*cfg35b8*cfg35ac*cfg35ec*galaxy[+0xa4]],
model.Transparency [* p1], 2*DAT_140829598, sqrt(lodFloat) * galaxy.radius)`. `calib` is one of
`hdrGalaxySpriteBright` / `realGalaxySpriteBright` depending on the photo mode. [V structure, I
mapping of `calib`] Blend `ONE, ONE_MINUS_SRC_ALPHA` (premultiplied). [V]
GPU-side meaning (additive emission, absorption sprites darkening via alpha, `Transparency` scaling
dust, `Reddening`, `Clip`) is [I].

### 6.4 LOD

* `BBoxRes`: the sprites are sorted into a `BBoxRes^3` grid over `[-1,1]^3`; empty cells are
  removed (`seSpriteVBO::BuildTree`). Used for per-cell culling/LOD ranges. [V]
* The per-sprite LOD random byte is uploaded with every vertex; a shader-side threshold on it
  thins the cloud at distance. [I]
* `LODbase` and `floatLOD` are stored in the model (+0x48, +0x44) and `sqrt(clipRadius /
  (px * 4 / cfg * LODbase))` selects the impostor/skybox level in the draw call. [V formula, I
  semantics]
* Models are rendered into impostors/skyboxes when `EnableGalaxyImpostors/FBO/Skybox` are on
  (`useFBO 1` on cube-map models). [I]

## 7. Coordinates

### 7.1 Model space

`x,y in [-1,1]` is the disk plane, `z` is the disk axis; image column -> +x, image row (from the top)
-> +y (v = -1 is row 0). Positions are in units of the disk radius before the model scale
`(Radius.x, Radius.x, Radius.x*SpriteOblZ)` is applied; because the z grid count contains
`1/SpriteOblZ`, the physical thickness is independent of `SpriteOblZ` (it only changes the
number of z layers). Disk half-thickness in disk radii = `(sideH/sideW) * emDetailZ / emDetailR`
(e.g. 0.49 * 0.3 / 0.7 = 0.21 for `Side1`). The
catalogue `Quat.w/x/y/z` (not traced here) orients the model into the universe frame. [V grid
math, I orientation]

### 7.2 Port convention

The port uses "disk normal = model -Z", which is also SE's convention (catalog `Quat` maps universe to
model space, disk normal model -Z, towards the observer side). The sampling code maps image column to
+x and image row (down) to +y, which is the *unmirrored* picture for a viewer on the -Z side looking along
+Z. The generator output therefore needs **no conversion** (`se_to_port_model` is the identity). An earlier
draft rotated by 180 degrees about X (`(x,y,z) -> (x,-y,-z)`); rendering M 101 and NGC 1365 from the Sun
direction and registering them against their `_small` textures showed that variant is mirrored (correlation
0.51 mirrored vs 0.37 for the 180-degree rotation, and the reverse for the identity), so the in-plane
handedness is [I, supported by image registration of two galaxies]. The in-plane rotation angle of the
pattern could not be confirmed (registration was noisy).

## 8. Galaxies without sprite models

Types 0..8 (all `E*`) are rendered procedurally by `galaxy_bulge.glsl` (uniforms `Mvp, EyePos,
ColorEffects, Clip, Color`) using a 512x64 float lookup (`galaxy_bulge.tif`, `Common.pak`, single
channel, `PrecomputedDensityFunc`; loaded by `seGalaxyCatalog::LoadBulgeLookupTexture`
`FUN_14016a1d0`). The table is a pre-integrated radial density profile: along x it falls from 1 to
~1e-4 (about 0.9994, 0.30, 0.089, 0.027, 0.0085, 0.0026, 0.0007, 0.0002 at x = 0, 64, ... 448), the
upper half of the rows (0..31) are nearly constant in y (inside the ellipsoid), the lower rows drop
off quickly (outside). Colour/brightness/scale come from the `GalaxyGenerator` settings in 6.1.
The shader (`galaxy_bulge.glsl`, decrypted) and its draw (`seGalaxySpriteModel::DrawBulge`, FUN_14029ce50) are
known [V]: the galaxy's ellipsoid (radius `GalaxyEllipticalScale` x the catalog radius, x and z shortened to
`1 - (index - 1) / 10`, FUN_140168210) is drawn with its back faces; per pixel `depth` = the ray's half chord
(half of what lies ahead from inside), `r = sqrt(1 - depth^2)` its offset from the centre, and the table is read
at `((0.5 r)^0.25, depth)`, its rows bottom-up as OpenGL receives them (depth 1 reads the saturated first file
rows). `Color` = `GalaxyEllipticalColor` x `GalaxyEllipticalBright` (square-rooted in HDR) x the galaxy draw factor
(fade-in `(px - 2) / 3` x brightness term, hdrGalaxySpriteBright x GlowBrightMin coef(GlowMagnLimit), ModelBright,
`1 - 1e-7 px`) x the distance reddening `(1 - k, 1 - 2k, 1 - 4k)`, `k = d / 4.227 Gpc`; the pixel is
`ColorCorrentionHDR(Color x density x 12, (DSOBrightness - 1, DSOContrast, DSOSaturation - 1,
1 / GalaxyEllipticalGamma)) x 10`, added. The port (`ellipticals.rs`, `galaxy_elliptical.wesl`) does this for the
16 largest ellipticals on screen, the table as R16Float, the ellipsoid projected to infinity like the other galaxy
layers; the far sprite fades out as it fades in (`resident`). Procedural ellipticals are promoted to the catalog like the
other types, so they can be picked and filled with stars.
Nebula models (`Nebulae-sprite.pak`) share the same maker; their extra keys are listed in
`config_locale.md`.

## 9. Ghidra function map (address -> role)

| address | role |
|---|---|
| `0x1402dc0c0` | `seSpriteModelMaker::ReadFromScript` (.pst block parser) |
| `0x1402dbb60` / `0x1402dbda0` | maker constructor / defaults (`Method`=3, `SpriteOblZ`=1, `LODbase`=100, bulge colour) |
| `0x140049670` | particle-layer defaults (size 1, minBright 1, noise defaults) |
| `0x1402db910` | build dispatcher: RNG seed from name, method switch, `sumColor` normalisation |
| `0x1402d85c0` | `createGalaxy` (Spiral/Elliptic) |
| `0x1402d4c70` / `0x1402d3f10` / `0x1402d3360` | `createCubeMap` / `createDiffuse` / `createNebula` |
| `0x1402d75c0` `0x1402d6920` `0x1402d5cf0` | `createCylindrical` / `createSpherical` / `createFlat` |
| `0x1402da540` / `0x1402d9a10` | `createModelCylindrical` / `createModelSpherical` (sprite pass) |
| `0x1402e4320` `0x1402e44d0` `0x1402dea10` `0x1402dee10` | bilinear RGB / 1-channel sampler, texel fetch |
| `0x140048be0` `0x1400490a0` `0x140049390` `0x140048fc0` `0x140069040` | fbm, warp, 1-octave noise, noise TLS params, noise table init |
| `0x140265710` | `seGalModelCache::Init` (loads all pst, builds type pools) |
| `0x140266150` | assigns models to galaxies (UseForObject, type hash) |
| `0x14016a7a0` | galaxy CSV reader, Hubble type index |
| `0x14049cdb0` / `0x140266940` / `0x14029b5f0` | `CheckGlowModel::Create`, model creation, per-model post-processing |
| `0x140299130` | maker -> model copy (transform, texture names, LOD fields) |
| `0x140299fe0` | per-frame fade/brightness factors |
| `0x14029bdf0` | draw (uniform fill, blend) |
| `0x140334480` `0x140334850` | `seSpriteVBO::BuildTree`, VBO allocation |
| `0x14049cf40` `0x14049def0` | `seGalSysModel` cluster/nebula system creation (uses the emission map) |
| `0x140168bd0` | shader/texture init (`galaxy_model.glsl`, `galaxy_bulge.glsl`, `sprites_gal.*`, `galaxy_bulge.tif`) |
| `0x14027a6a0` / `0x140280ec0` | `GalaxyGenerator` / `PointRendering` config read / write |

Useful constants (all float unless noted): 128 and 15 (disk / bulge grid scale), 64 (cube, diffuse),
2^-32 at `0x1406d3e00`, LCG multiplier `0x19660d`, increment `0x3c6ef35f`, `3.0` at `0x1406d5008`
(`3v < minBright` test, `/3` brightness), `-2.0` at `0x1406d5ed0` (bulge gaussian), `255`
at `0x1406d58f8`.

## 10. Verification status

| item | status |
|---|---|
| pst keys, defaults, offsets | V (decompile + all 169 shipped blocks parse) |
| texture channel meaning, sizes | V (Pillow inspection + generator code) |
| model assignment (type table, hash, pools) | V code; I for position units and `UseForType` conversion |
| voxel maps, sprite pass, RNG, brightness normalisation | V (line by line from the decompile) |
| noise (Diffuse) | V structure and tables; unverified against a runtime dump |
| fade-in at 2..5 px, `px*1e-7` fade-out | V formula; K/D runtime values [?] |
| shader behaviour, atlas tile order, `_sys` use, LOD semantics | I |
| elliptical rendering | ? (shader not readable) |

Smoke test on the real data (`Kexitt.pst`, M 31 / NGC 1300 / Milky Way, `Side1.png`, 95-180 ms in a
release build): M 31 -> 7.4k emission + 37k absorption + 64 bulge sprites, clip radius 0.91; the
projected result shows the spiral arms, dust lanes and the thin disk with its dust band edge-on.

## 11. Renderer integration (port, `se_engine/src/galaxies.rs`)

Implemented in `GalaxiesPlugin`; shaders `assets/shaders/galaxy_far.wesl` and `galaxy_model.wesl`.

* **Startup**: all `models/galaxies/*.pst` are parsed, `textures/galaxies/*` is listed, a
  `ModelCatalog` is built and each catalog galaxy is assigned a model once (`UseForObject` alias match,
  else the type pool by position hash; the port feeds `pos_pc` as the hash position). 9.3k of 11.1k
  galaxies get a sprite model, the rest (E types) use the analytic far sprite only.
* **Far field**: one mesh with one quad per galaxy (4 vertices: position pc from the Sun, rgb + absolute
  magnitude, disk normal + radius pc, corner + id + profile type). The vertex shader works at infinity
  (like the starfield): apparent magnitude from the distance, angular radius `focal * R / sqrt(d^2 - R^2)`
  in pixels, the quad is an ellipse (major axis R, minor axis `R*|cos i|`, minor axis along the projected
  disk normal), clamped to `min_px`. Intensity = `K * flux * (focal/focal_ref)^2 / (norm * a * b)` times a
  radial profile (de Vaucouleurs for E, exponential disc + bulge for S, flat for Irr), i.e. surface
  brightness independent of distance and FOV. The sprite is hidden inside the galaxy (`d < 1.02 R`) and
  fades out as the 3D model fades in.
* **3D models**: a model is built when the galaxy's angular radius exceeds `build_px` (4 px) or the camera
  is inside it; it fades in over 6..16 px (SE: 2..5 px, widened to hide the build latency). Build runs on
  `AsyncComputeTaskPool`: texture decode (`image`), `build_galaxy_model`, then conversion of SE model space
  to pc offsets: `local * Radius.x * R_pc` (z times `SpriteOblZ`), identity (`se_to_port_model`), rotate by the catalog `orientation`. At most 6 models are resident
  (LRU, 4 s grace, the Milky Way is pinned because its emission map feeds the density API).
* **Sprites**: camera-facing billboards at infinity with a random in-plane rotation; size in pixels is
  `size_pc / d * focal` (min 1.5 px, area-weighted flux below that). Sprites larger than 250 px fade out
  (gone at 875 px) so the part of the galaxy around the camera never covers the screen. Atlas =
  `sprites_gal.png` (box-filtered to 512^2, R8, 4x4 tiles). One mesh per model holds emission, dust and bulge,
  drawn in one call with SE's blend `ONE, ONE_MINUS_SRC_ALPHA` (`AlphaMode::Premultiplied`, FUN_140334e90): emission
  and bulge return `(colour * emBright * w, 0)` (additive), dust `(-colour * emBright * w, absBright * w)` with
  `w = I * tile * fade`, as galaxy_model.glsl: it darkens what was drawn before it by `absBright` and subtracts the
  dust colour (`absParticleColor`); Bevy's tonemapper clamps negative results to 0. Vertex colour and intensity are
  clamped to 0..1 like SE's bytes (`sprite_vertex`; the Milky Way's absorption intensities reach 5.5 unclamped).
  **Draw order**: SE sorts the sprites by distance from the camera, farthest first (FUN_140333a30, descending
  quicksort FUN_140334190) for models over 10 px; `SpriteOrder` does that on the async pool (`back_to_front`) and
  swaps the index buffer in when the camera has moved more than 2% of its distance to the model, so dust only
  darkens what lies behind it (no per-frame upload; frame times equal the baseline).
* **Reddening**: SE's `Reddening = 1 - d / 4.227e9 pc * (1, 2, 4)` (FUN_140299fe0) multiplies the far sprites,
  model sprites and front texture (clamped to 0..1).
* **Brightness (SE's chain, FUN_14029bdf0 / FUN_140299fe0 / FUN_1404c4610 / FUN_1404c4ae0)**: `emBright =
  GlowBrightMin * exp((GlowMagnLimit + 0.2) / 1.0587) * hdrGalaxySpriteBright * Brightness * ModelBright` (0.162 x
  Brightness at SE's defaults), `absBright = Transparency * bright_term`, `bright_term = clamp(GalaxyBrightMin *
  exp((GalaxyMagnLimit - m) / 1.0587) * gain - GalaxyBrightMin, 0, 1)` (m the apparent magnitude, gain the
  intergalactic vision; 0 hides the model; FUN_1404c4610, FUN_1404c51a0), front texture `emBright / Brightness *
  GalaxyFrontTexBright * bright_term`. Everything is read from main-def.cfg; GlowMagnLimit follows `]` / `[`. The other
  factors of the exe's chain are 1 in a steady view from space: renderer +0x35ac (deep-sky extinction inside an
  atmosphere, FUN_14044fe70, not ported), +0x35b8 (reset to 1 each frame), galaxy +0xa4 (show/hide fader). No free
  factor. The model is no longer calibrated to the far sprite, so the hand-over at 6..16 px changes brightness (SE's
  points and models are not matched either). Far sprites: `K_far = 1.5`, `m_ref = GalaxyMagnLimit + 0.5 = 7.5` (Shift+`]`/`[` change the limit),
  `dust_strength = 1` (= SE). `AutoGalaxyMagnitude`, `AutoGalaxyMagnSpeed` and `ShowCatGalaxies` are read
  from main-def.cfg. Quasars are drawn as galaxies of their `Type` (SE only flags them, 0x800).
  Env overrides for tuning: `SE_GAL_FAR`, `SE_GAL_MODEL`, `SE_GAL_DUST`, `SE_GAL_BUILD`.
* **Sort order**: Bevy sorts transparent items by the view depth of the mesh AABB centre plus
  `depth_bias`. The atmosphere sky shell shader returns alpha 1 with premultiplied blending, i.e. it
  erases everything drawn before it, and its sort depth depends on the look direction (a shell below the
  camera sorts after a bias of any moderate size). Galaxy meshes therefore use a bias of 1e15..3e15
  (far, emission, dust) and are padded with two unreferenced vertices so their AABB centre is the origin.
  Depth test against opaque bodies still hides galaxies behind planets.
* **Density API**: `GalaxyDensity` resource (Milky Way). `stars_per_pc3(pos_pc)` (stars per cubic parsec,
  universe frame, parsecs from the Sun), `relative_emission(model_pos)` (dimensionless voxel brightness of
  the generator's emission map, SE model units, disk radius = 1), `analytic_stars_per_pc3` (exponential disc
  + bulge fallback, used until the map is built), `model_from_universe`. Normalised to 0.14 stars/pc^3 at the Sun.
* **Stars inside galaxies**: SE's star octree (`se_core::star_octree`, `procstars.rs`, docs/missing_features.md
  06-005): a galaxy with a sprite model accepts its candidate points with probability `0.15 x` the red channel of
  its emission voxel map at box coordinates (`DensityMap::sample_se`, FUN_140440840: the grid spans the galaxy's
  whole box along every axis, cell `(c + 1) n / 2` without a half-cell offset). The Milky Way's map (89 x 89 x 18
  over a cubic box of one radius) gives R 0.16 on the Sun's ring and a stellar disk about 2 kpc thick. Galaxies
  without a map (ellipticals) use SE's analytic exp(-r) young disk, bulge and halo. The field waits up to 5 s for
  a galaxy's model.
* **Daylight**: inside an atmosphere shell whose star is above -3 degrees elevation the whole galaxy
  (far sprites, emission, dust) is faded out (`1 - smoothstep(-0.2, -0.05, sin elevation)`), because the
  shell replaces the background and a multiplicative dust sprite would otherwise darken the blue sky. The
  fade reads `SkyShell` / `SkyMaterial` (sun direction, shell centre and radius) read-only. Residual: from
  orbit, dust still multiplies the lit atmosphere limb in directions where dust sprites exist.
* **Intergalactic vision** (SE `AutoGalaxyMagnitude`, FUN_140299fe0 / FUN_1404c51a0): every drawn galaxy adds
  the sky it covers, `0.01 R^2 / max(d^2 - R^2, 100)` (parsecs), the sum is eased at `AutoGalaxyMagnSpeed`
  (`1 - exp(-dt speed)`, starting from 1e6) and the galaxy brightness is multiplied by
  `max(1, 1 / (coverage + e^-14.356739 coef(GalaxyMagnLimit)))`: 1 inside a galaxy, up to the brightness of a
  magnitude-15 limit (1912 at the default 7) in empty space. The port sums the catalog galaxies every frame and the
  drawn procedural ones every 10 frames; SE sums the galaxies its draw loop visits (whether frustum-culled ones
  count was not traced). `SE_GAL_AUTO=0` disables it.
* **Handedness**: the SE-to-port model mapping is the identity (see 7.2); `SE_GAL_MAP=1..5` selects the
  alternative flips used for the calibration. `SE_GAL_NODEPTH=1` disables the depth test (debugging).
* **Known artifact**: at ground level a thin black depth sliver (not from the galaxy code; it disappears with
  `SE_GAL_NODEPTH`) appears in one test view near the horizon-side sky.

- **Galactic centre blobs.** The saturated white blobs seen from inside the Milky Way towards Sagittarius were not
  the galaxy model: they were procedural supergiants/OB stars generated in the dense bulge by
  `se_core::star_octree` (no age model). `DensityModel::young_fraction` now restricts stars brighter than M = -2 to
  the disk population, so the bulge keeps only old stars. Separately the model's bulge sprites (layer `Bulge`) are
  drawn at full strength as in SE (no per-layer gain; `SE_GAL_BULGE` scales them for comparisons), with SE's clamped
  vertex intensities.
