# Surface biome presets and soils

Implemented in `crates/se_core/src/biomes.rs`. Both formats use the SE script syntax
(`crate::sc`); no parser changes were needed (`Layer "Large" { 64 1 1 "X" }` parses as a
block holding one anonymous row of values).

## Biome presets: `textures/terrain/samples/*.cfg`

225 files (SamplesPresets.pak). Top-level keys:

| Key | Meaning |
|---|---|
| `UseFor "Parent\|Body"` / `"Body"` | Pins the preset to that body (only 5 files: Earth, Moon, Mars, Eris, Sedna). Pinned presets never enter the weighted choice. Not inherited through `Default`. |
| `Default "file.cfg"` | Inherit everything not specified from that preset (chains up to 16 deep). |
| `Weight w` | Weight in the random choice (1e-5 .. 1). |
| `BulkClass` | Terra, Ferria, Carbonia, Aquaria, Asteroid, BrownDwarf, Jupiter, Neptune. |
| `TempClass` | Any, Frigid, Cold, Cool, Temperate, Warm, Hot, Torrid. |
| `AtmoClass` | Any, Airless, NonAirless. |
| `VolqClass` | Any, Airless, Arid, NonArid. |
| `SunClass "Any"`, `SunTemp (lo hi)`, `SpecClass` | Host star constraints (`SunTemp` in K; `SpecClass` L/T/Y/C for brown dwarfs). |
| `LifeOrganic`, `LifeExotic` | DontMatter in all shipped files; the exe also knows UniOrMulti, Unicellular, Multicellular. |
| `ColorVary (r g b)` | Per-pixel colour variation. |
| `Materials "Coll" { "soil name" ... }` | Soil names usable by the preset. Collections: Mineral, LifeOrganicUni/Multi, LifeExoticUni/Multi. Overrides per collection under `Default`. |
| `Biome "Name" { Color (r g b a)  Layer "Large" {..}  Layer "Small" {..} }` | One biome. Under `Default`, `Color` and each layer override individually. |
| `OceanPalette { Deep (..) Shelf (..) Absorption (..) }` | Ocean colours. |
| `CloudsPalette { Layer0 (..) ... Layer7 (..) }` | Cloud layer colours (gas giants). |

Biome names, in order: Bottom, Shelf, Beach, Lowland, Upland, Rock, Snow, Ice, Lava,
SteppeOrgU, ForestOrgU, GrassOrgU, SteppeOrgM, ForestOrgM, GrassOrgM, SteppeExoU,
ForestExoU, GrassExoU, SteppeExoM, ForestExoM, GrassExoM (Org/Exo = organic/exotic life,
U/M = unicellular/multicellular).

A layer is `{ freq a b "Texture" }` (`{% 4g %5.3f %5.3f "%s" }` in the exe); the texture is
a sample name such as `Desert003_COL` (18 distinct names in 0.991). Each sample in
`textures/terrain/samples/SamplesTex.pak` is three 512 px PNGs: `_COL` (RGB), `_DISP` (8-bit
height; one is spelled `_Disp`) and `_ROUGH` (8-bit roughness). SE's tile generator samples them
(`SampleMapArray`); the port keeps `_COL` as the colour layer and turns `_DISP` into a normal map
(relief 0.04 of the layer's period) with `_ROUGH` beside it, which light the ground between the
soils and orbit (`surface_tex.rs` `relief_layer`, `planet.wesl` `triplanar_relief`). Colours may use commas
or spaces. `custom_Mars.cfg` has no `Biome` blocks at all (only soils); gas giant, brown
dwarf and asteroid presets have palettes instead of, or in addition to, biomes.

## Soils: `textures/terrain/materials/*.pml`

6 files (aquaria, grass, mars, selena, snow, terra), 77 entries; names repeat across
entries and the first one wins in `SurfacePresets::soil`.

```
Soil "Mars red soil 1" {
    Color (r g b a)  Slope s  ModColor ModRough ModNorm ModHeight  BiasRough ScaleRough
    BiasHeight ScaleHeight
    Layer "Detail" { freq rot "Texture" }     // 2 numbers, not 3; rot: SE's anti-tiling rotation flag
    Layer "Modul"  { freq weight "Texture" }
}
```

Each layer texture `<base>_COL` (DXT1, 1024 px, 11 mips) comes with `<base>_NORM` (ATI2/BC5 normal
xy) and `<base>_RHAO` (DXT1: R roughness, G height, B occlusion) in `MaterialsTex.pak`. G is height,
not depth: it rises with the occlusion (r = 0.92 on GroundKX3) and the brightness. The normals' x
and y point up the height map's slope (x with +dh/dcolumn, y with +dh/drow, r = 0.5-0.86 on three
materials), so the tangent-plane normal is minus the stored xy. The port uploads all three as the
files' own BC blocks (`surface_tex.rs`) and `planet.wesl` (`soil_look`) uses them as SE's
`DetailTextureMulti` and `BlendModulate` do: the detail layer's colour and roughness times
`mix(1, luma(Modul), ModColor / ModRough)`, normals and occlusion mixed towards the Modul's by
`ModNorm` / `ModHeight` times the layers' scale ratio `min(a/b, b/a)`, roughness then
`Scale r + Bias`, then the HSV recolour below. SE reads the height channel only to displace the ground
near the camera (`DisplaceDetailTextures`, planet.glsl DET + DISP: no parallax): the detail layer's
height mixed towards the Modul's by `ModHeight` times the scale ratio, then `ScaleHeight h + BiasHeight`.
The port moves each vertex within the soils' 2 km by that height around the texture's mean, times
`DisplaceDetailHeight` (0.1) times the detail layer's period (`planet.wesl` `soil_disp_v`).

A layer's `freq` is copied to the GPU as an integer (`FUN_140373330` fills the soil table from the
parser's `FUN_14015f8d0` table at `0x14080d54c`), so its scale is set in the shader. It is a
frequency, not a size: the finest photos have the largest values (sand grains 256, grass blades
96-128) and the coarsest the smallest (sand ripples and snow dunes 4-12, grass's patchy dirt 2-5),
whichever of Detail or Modul they sit in. The port takes the period as `128 / freq` metres (snapped
to divide the 4096 m wrap of the soil coordinates). SE seen straight down on Earth (2026-10-28):
from 1.5 m, grass blades 4-11 cm long with no visible repeat across 3 m ("Grass 3/4 green", Detail
96-128 -> 1.0-1.3 m); from 20 m over the Sahara, ripples 0.7-1.1 m apart ("Sand white 1", Detail 11
-> 11.6 m, the ripples being in its normal and occlusion maps, about 17 across the texture). An
earlier reading of `freq / 8` m, fitted to a single Mars view, made sand grains 32 m across.

**Which soil goes where** (SE's own rule, from its decrypted `tg_detail_maps.glsl`, see
`se_shaders.md`). Every soil of the preset (its `Mineral` list, then the life collection's) is
weighed at each point: `d = 4 (slope - Slope)^2 + |dH| + |dS| + |dV|` between the colour map and
the soil's `Color` in HSV (SE's `rgb2hsl` is HSV; stored sRGB values; the hue gap is looped only
when positive, as SE does), slope `1 - cos^6` of the ground's tilt from the node's normal map,
weight `d^-1/2`; the heaviest wins, blended with the next over a weight band of 0.05. The chosen
soil is recoloured: its texels move in HSV by the ground's colour minus the texture's mean, so the
map's colour shows through. Soils are full within 1 km and gone by 2 km. Earth's grassland
(HSV 0.19 0.52 0.25) gets "Grass 3/4 green", the Sahara at 23.5 N 10 E "Sand white 1" (with the
ground flat: steeper plains take "Blocks yellow", Slope 0.2).

SE's 256-texel planet tiles (bump, colour, clouds, lights; every pack checked) carry a 2-texel
border: neighbours repeat each other's 4 edge texels, and against the parent tile texel `i` sits
at `(i - 1.5) / 252`. The port samples the 252 texels between (`HeightTile::sample`,
`tile_uv`); sampling all 256 had heights disagree across tile edges, straight cliffs of up to a
few hundred metres on Mars.

## Choosing a preset (`SurfacePresets::find_for`)

1. `UseFor` match (case-insensitive; `Parent|Body` exact beats name-only, since the
   catalog parent may be a barycentre).
2. Unpinned presets whose classes match the body. `Any`/`DontMatter` match anything.
   Body temperature class: Frigid < 90 K, Cold < 170, Cool < 250, Temperate < 330,
   Warm < 500, Hot < 1000, else Torrid. Volatile class of the body: no atmosphere ->
   Airless, atmosphere and ocean -> NonArid, else Arid; it must equal the preset's
   `VolqClass`. `AtmoClass` Airless/NonAirless compares with `has_atmosphere`.
   Catalog class -> bulk class: Terra/Selena -> Terra, Ferria/Chthonia -> Ferria,
   Carbonia, Aquaria/Glacia -> Aquaria, Asteroid/Comet -> Asteroid, Jupiter -> Jupiter,
   Neptune -> Neptune, BrownDwarf.
3. Weighted choice in file-name order: walk cumulative weights to `surf_style * total`.
4. If nothing matches, constraints are dropped in order: `SunTemp`, temperature,
   volatiles and atmosphere, life, bulk class. `find_for_ex` reports the level reached
   and a `log::warn!` is emitted. `None` is only returned with no presets at all.

`surf_style(block)` reads `Surface/SurfStyle` from a body block.

**SE's own rule** (decoded, docs/formats/surface_params.md): the engine now fills `BodyInfo::se`
with SE's classes and `sun_temp` with SE's star temperature (5800 K unless the host star is a
component of a multiple system), and `surf_style` with the generated `SurfStyle`. With `se` set,
matching follows the exe: exact classes, `NonArid` = Lacustrine..Superoceanic, `NonAirless` = any
atmosphere, `None` bulk = any, `SunTemp` (lo, hi]. SE itself never relaxes constraints (it logs
"Can't find a suitable biome preset" and leaves the body without one); the relaxation above is
the port's fallback.
