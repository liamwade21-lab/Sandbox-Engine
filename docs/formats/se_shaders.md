# SE's own shaders (decrypted 2026-10-28)

`data/shaders/Shaders.pak` is a ZIP with traditional PKWARE encryption (ZipCrypto), files stored
uncompressed. The password is a literal passed to `unzOpenCurrentFilePassword` in `FUN_14028a350`
(the pak reader). The 141 GLSL files were extracted, with the owner's permission, to
`re/se_shaders/` as a reference; nothing from them ships with the port. Line references below are
to those files.

## Terrain LOD and far detail
- No vertex geomorph (`planet.glsl:337,363`): vertices sit on the sphere at their height; skirts
  drop by `z * 2/nTiles` (`:344,363`). The port's geomorph goes beyond SE and stays.
- Mesh `PlanetCubeMeshLOD 5` = 33x33 vertices per node; tiles `PlanetTextureRes 256`
  (`main-def.cfg`, `TILE_RES` `planet.glsl:47`). Normals are made per node on the GPU from the
  256-texel bump tile (`bump_to_norm_planet.glsl:61-85`, central differences times the
  cube-to-sphere Jacobian): about 8 normal texels per mesh cell, which is why SE's splits don't
  show and why distant terrain keeps its relief. Tiles are mipmapped (`MipmapsLand`),
  `AnisotropyLevel 16`.
- Normal and colour lookups are jittered by `Fbm2(uv, 4) * HALF_TEXEL` (`planet.glsl:577`,
  `terrain_noise.glh:89-91`) to hide texel grids. Magnified height tiles are read with a 16-tap
  Catmull-Rom filter (`planet.glsl:232-241, 281-290`).
- Generator octave counts are fixed per feature, not cut by tile level (`tg_common.glh:1233`), so
  a child tile resamples the parent's surface and coastlines don't move as nodes split.
- CPU split test `FUN_1404b4fa0` (distance to the node's box in node-size units, against
  `K = 2^LandLOD * max(1, A/B)` set in `FUN_1404acdd0`; A/B probably resolution, uncertain).
  `LandTargetRes 1080`: larger screens are treated as 1080 (a cap, not a penalty).
  `MaxTilesPerFrame 4`.

## Sea and shore
- The sea is painted by the terrain pass with a hard per-pixel mask,
  `waterMask = step(heightN, WaterParams.x)` (`planet.glsl:816`), flat normal; a sea mesh
  (`water.glsl`, sphere at `Radiuses.w`) is drawn only near the camera and cross-faded with
  `waterFade3D = clamp(d * WaterParams.z - 1, 0, 1)` (`planet.glsl:820`, `water.glsl:348-355`).
- Opacity grows from the depth taken from the height map, `exp(-(dist + depth) * a * fog)`
  (`planet.glsl:851-866`); Fresnel `0.02 + w * 0.98 * (1 - |n.v|)^4` (`:1259`). No foam, no
  refraction, no Snell's window. Waves: 4 scrolling taps of `NoiseMap` at scales 1 and 4,
  full to 10 km and gone at 20 km (`water.glsl:357, 406-419`).

## Ground shading and soils
- Biome sample textures are baked into the colour tile (`tg_common.glh:680-733`): biome by height
  band `(h - sea)/(1 - sea)*4 + BEACH`, Large and Small layers grey-mixed by weight and combined
  with Overlay, then HSL-shifted so the texture's mean equals the biome `Color`. They carry no
  normals or roughness.
- Soils are baked per near node (`tg_detail_maps.glsl`). Slope `1 - nz^6` (`:548`); per soil
  `d = 4 (slope - Slope)^2 + sum |dHSL|` against the colour map, hue wrapped (`:491-520`,
  `COLOR_METRICS 1`), weight `d^-0.5`; mineral and organic soils all compete. The chosen soil is
  recoloured to the map by an HSL shift (`:459-469`; the port keeps the map's mean colour instead,
  see biome_presets.md). Modul layer per `texture_mix.glh:120-155`.
  Height (Bias/ScaleHeight) is never used for parallax.
- `detailFade = clamp(2 - d_km, 0, 1)` (`planet.glsl:731`): soils full within 1 km, gone at 2 km.
- Lighting `terrain_pbr.glh`: GGX F0 0.04, `(1-F)` diffuse, Hapke factor
  `mix(1, s/(NL+NV), SurfParams1.z)`; Cook-Torrance only with water or ice.

## Terrain generator
`tg_terra_height.glsl`, `tg_selena_height.glsl`, `tg_common.glh`: hash Perlin (period 69 cells),
unnormalised Fbm, de Carpentier eroded ridged noise, Worley continents with a shore mask, landform
chosen per Voronoi cell (dunes, hills, canyons, mountains with terraces), maria, ice-cap cracks,
replace-style craters in every cell, pseudo-rivers, rifts, shield volcanoes, `softPolyMin`. Cell
noise reads `Noise3DSampler`, a 64^3 RGBA texture the exe makes at startup (FUN_1402f0880): generator A
(`u = u * 0x19660d + 0x3c6ef35f`) seeded with 0x7178d fills a 512^2 RGBA map, a 512^2 R map, a
64^2 R map, a 256^2 and a 16^2 map (two draws per texel), then this texture, each byte `u / 255 + u`
(low 8 bits); a second 64^3 texture follows. The height pass (FUN_1402fab70) binds the first to
unit 0, nearest. Uniform block (FUN_1402fab70, from the body record): Randomize, faceParams,
then mainParams (mainFreq, terraceProb, oceanType 1 for Superoceanic / 0.5 for the other wet
classes / 0, tidalLock +1 or -1 from +0x165c), colorParams, climateParams, plantsParams1/2,
mareParams (sqrt of MareDensity), montesParams.. canyonsParams (the cumulative landform
thresholds of FUN_1402f9fa0, record +0x1bc..0x1cc), riversParams, riftsParams, eqridgeParams,
cracksParams, craterParams (sqrt of CraterDensity), volcanoParams1/2 (volcanoTemp x 0.001),
lavaParams, textureParams. Generator per body (FUN_1402ef870): bulk 1-4 `tg_terra`, or
`tg_selena` when the atmosphere class is below 2; asteroids `tg_asteroid` (ported too); giants,
brown dwarfs and stars their own. Heights are 0..1 of `BumpHeight` above `R - BumpOffset`
(`FUN_1404ae0d0`: `h * BumpHeight - BumpOffset` km; `FUN_1404ae4a0` sets the surface and
atmosphere-bottom radius to `R - BumpOffset`, body record +0x1110 minus +0x1118). `BumpOffset` is
the ocean's height (SE warns when `Ocean.Height` differs from it) and 0 without an ocean
(FUN_1405057e0), so dry worlds and asteroids rise from R; an asteroid's default bump is up to 0.9 R,
hence its irregular shape. Ported: `se_core::tg` and `tg_height.wesl` (a compute pass per terrain node, 145^2 grid
reaching 8 steps past the node, read back for the mesh; nodes finer than 2 m steps interpolate
the deepest grid, as SE's f32 point cannot resolve finer).

## HDR
Default `PhotoMode 0` (HDR) and `ToneMappingShader "AgX"` (`main-def.cfg:198,221`): SE's own
polynomial AgX (`hdrt_AgX_tone.glh`), saturation 1.5 (`hsl.glh`), bloom 6 mips weighted
0.50/0.60/0.60/0.45/0.35/0.23 at `BloomBright 0.5` (`hdr_combine.glsl:166-179`). The
histogram auto exposure (`hdr_histogram.glsl`, `AutoExp*` keys) looks like mode 2 only (inferred).

## What the port took over (2026-10-28)
- Coast per pixel from the ground's height (normal map alpha, `tanh(h / 8 m)`), as `step(height,
  seaLevel)`; no water painted on land.
- Heights the same at every LOD level (smallest relief 20 m; the fractal detail below the data's
  resolution is small, 0.02 of its scale per octave, so plains stay flat enough for SE's sand);
  normal maps 129 texels a side, mipmapped, anisotropic 16; Catmull-Rom height tiles; half-texel
  jitter; `LandTargetRes` as a cap.
- Soils: SE's weight and blend, HSV recolour (exact sRGB curve, as the stored colours), Modul
  blend, 1-2 km fade, no parallax. A layer's second number is the rotation flag, not a weight.
- `ColorVary` (biome presets, `colorParams2.xyz`): `color *= mix(colorVary, 1, albedoVary)`, a
  cubed planet-scale noise (`tg_terra_color.glsl:82-158`, `tg_selena_color.glsl:108-138`); the
  asteroid colour shader leaves it out.
- Anti-tiling (TILING_FIX_MODE 4) on the biome sample layers and the soils: 8 zones picked by a
  noise of one value a repeat; samples shifted 0.5 repeats and turned by the zone in radians;
  soils shifted 1.5 repeats, quarter turns and flips only with the rotation flag (sent in
  `soil_bias.zw`). The soils' noise repeats with their 4096 m wrap.
- Limb glow ("aureole") with the star behind a planet: no special code in SE, just the sky's
  inscatter table toward the star (`sky.glsl`, Mie cut by `MieHorFade` at the ray's entry point), and
  (by the look of the video) the pseudo lens flare copying the bright limb into arcs. The port reads the table at SE's 16 nu x
  64 mu (docs/atmosphere.md). SE has no atmospheric refraction (no shader, setting or string).
- Star sprites (`star_sprite.glsl` MODULATE, `atmo_transm.glsl`): the glare is drawn after the
  atmospheres and scaled by a 1x1 texel of the atmospheres' transmittance and the star's cover, so a
  star behind a limb is hidden whole and a setting one is reddened. Ported as `SunSpriteMaterial`
  (after the skies) with `sun_sprite_tint`: the unhidden part of the disc times SE's analytic
  transmittance (`transmittance_to_space`); SE's exact texel content is not known, the port keeps the
  transmittance's brightness as well as its colour.
- Tone: Bevy's AgX with exposure log2(0.75) and post-saturation 1.35 (SE's vibrance at saturation
  0.3); `SE_TONEMAP=aces` for the earlier look.
