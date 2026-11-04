> Research notes made on 2026-11-04 for `docs/spec-vegetation-civilizations.md`: what Megaton Rainfall
> does, read from its shaders, data and file lists (the GOG copy in `C:\Users\liamw\Downloads\Megaton Rainfall`).
> Its text files and shaders are unpacked in `re/megaton_rainfall/extract` (out of git); the scripts that
> read its formats are in `re/megaton_rainfall/tools`. The port reads the game's assets at run time from
> that install and never copies them into the project.

# Megaton Rainfall (2017, D3D11): how vegetation is drawn and tied to the terrain

Method notes only. Evidence tags: [V: file + identifier] = read in the data/shaders; [I: why] = inferred.
Paths are relative to `re/megaton_rainfall/extract` (Effects/ = .txt effect sources, Data/ = text configs).
Extracted samples for the asset section are in a scratch folder (not kept) (Models/, Textures/). My scripts: `re/megaton_rainfall/tools\{bin.py,dds.py,ex.py}`.
The shader code is HLSL SM5 (compute, geometry, hull/domain shaders); comments are in Spanish.

## 0. Big picture
- The "ground" is not a sphere mesh near the camera. A small top-down orthographic render (`TerrainTopDown`) is made around the camera into 4 nested "cascade" texture-array layers: height, grass/dirt mask, baked sun light, flags. The tessellated ground (`TerrainQuad`), the grass (`TerrainGrass`) and the tree placer (`TerrainComputeObjsNearCameraCS`) all read those same cascades. [V: TerrainTopDown.txt "vColor = float4(vDiffuse.rg,fLights,iFlags*(1.0f/255))", "vColor1 = fHeight"; TerrainQuad.txt `g_texarrSampler0/1`, `GetCascade`; TerrainGrass.txt; TerrainComputeObjs...txt]
- So vegetation is "painted by the terrain": the top-down pass decides per texel where grass can grow (mask .r), the grass and tree shaders only look the mask up. No separate vegetation map exists. [V: comment in TerrainTopDown.txt "TerrainGrass y TerrainComputeObjsNearCamera usan .r (grass)"]
- From far away vegetation does not exist at all: the whole planet is a coloured sphere (`Planet`), coloured from a latitude/height gradient texture. Vegetation only exists inside the cascade area (hundreds of metres to a few km). [V: Planet.txt `vTexStyle = tex2D(g_texSampler2,float2(vTexCoords0.y,fHeightColor*0.5+0.5))`; no tree/grass code in Planet.txt] [I: nothing else in Effects/ writes trees]

## 1. Grass (`TerrainGrass`, `BillboardGrass`, `BillboardXZ`, `BillboardOmni`)
### 1a. Terrain grass = a fixed cloud of 50,000 points wrapped round the camera
- Input mesh `Models/TerrainGrass.bin` is 50,000 POINT vertices (no indices), y=0, x,z uniformly random in [-75,75] m; each point's UV picks one of 4 cells of the grass atlas (counts 12,598 / 12,498 / 12,476 / 12,428 for (0,.5)(.5,0)(.5,.5)(0,0)). Mean ~2.2 points per m^2 over the 150x150 m square. [V: I parsed the file: n=50000, bbox x[-75,75] z[-74.99,75], 4 UV values]
- Vertex shader keeps the points inside a 2*75 m torus-wrapped square centred on `g_vCenter` (camera-ish): `fmod(vDot, 2*c_fRadius)` with `c_fRadius = 75.0f // [grass_radius]`, applied along the world matrix's X and Z axes. The same 50k points therefore always surround the camera; no streaming, no per-point state. [V: TerrainGrass.txt `vs`]
- Size fade with distance: `fSize = saturate((2*c_fRadius - distance(camera.xz, pos.xz))/10)` -> full size to 140 m, gone at 150 m. The blade quad is scaled by fSize (shrink-to-nothing instead of alpha fade). [V: TerrainGrass.txt `vs`]
- Ground height: the VS samples cascade 0 height (nearest cascade only): `vPos += texSampleLevel(g_texarrSampler0,float3(...,0),0).r * g_texInstances[0].m_vRow1`. [V]
- Where it grows (geometry shader, point -> quad, `[maxvertexcount(4)]`): the GS reads mask texture (`g_texarrSampler1`, cascade 0) and discards the point if `vTex.r<0.1` (no grass mask), flag bit 0 clear (hole by tunnel or similar), world y outside `g_vIntervalHeight` (e.g. below sea), or `fSize<=0`. So density is binary-per-point against a threshold, with the 2.2/m^2 point density constant; the mask only thins by killing points where r<0.1. [V: TerrainGrass.txt `gsmain`]
- Quad shape: `vUp = (0, 0.66*fSize, 0)`, `vRight = 0.33*fSize` perpendicular to the view direction (cylindrical billboard, always faces the camera). So a blade tuft is about 0.66 m high, 0.66 m wide at fSize=1. Sway: `vUp += 0.2*cos(g_fTime*2 + pos.x) * vRight` (phase = world x). [V]
- Lighting/colour: terrain normal rebuilt from 4 height taps (`fDist = 5 + dist*0.02` m); colour = `g_vDiffuse.rgb * (g_vAmbient + vTex.b * sunColor)` where `vTex.b` is the sun-lighting term baked in the top-down pass ("[top_down_light]"), so grass is lit like the ground under it without its own lighting. Translucency (back-lit blades) = `saturate(dot(viewDir,sunDir)/0.8 - 0.25) * saturate(1-3.5*|dot(viewDir,normal)|)` on the top vertices only, added in the PS as `vColor.rgb += vTexCoords1.y*g_vGrassTranslucency`. Alpha test `clip(a-0.5)`; opaque (no blending), so it needs no sorting. [V: TerrainGrass.txt `gsmain`, `ps`]
- Atlas: `Billboard_Grass.dds` 2048x2048, 2x2 cells = 4 grass types (global.txt `billboard_grass` material tints `g_vDiffuse` 0.45/0.50/0.29). `g_vBillboardTexSize` = one cell's UV size. [V: Data/global.txt:168-175; Models/billboard_grass_N.bin tex_coords {0..0.5}/{0.5..1}]
- Colour does not come from the terrain colour texture; it is a constant tint (`g_vDiffuse`) times the baked light. [V] (Unknown whether C++ changes `g_vDiffuse` per planet/style.)

### 1b. Other grass things
- `BillboardGrass` / `BillboardXZ`: plain mesh billboards used by the `bunch_N` / `billboard_grass_N` models (a mesh = 2 crossed quads "BUNCH" with 8 verts/12 indices, or a single quad). VS rebuilds the matrix so the quad faces the camera around its own up axis; BillboardGrass adds `0.1*scale*cos(time*3 + matWorld._m30) * vPosOrig.y * right` sway (stronger at the top). Alpha-test 0.5. Used as pots/plants on props (`pot_0..3`), and as instanced objects [I]. [V: BillboardGrass.txt, BillboardXZ.txt, global.txt:1198-1275, 2239-2246]
- `BillboardOmni`: not grass. It is the "any-angle impostor" renderer for animated models (people/crowds): GS expands a point to a camera-facing quad, picking the atlas cell by (model, animation frame, phi, theta) from the view direction in the object's frame; distance fade `saturate((dist - g_fDistMin)/15)` with dithered clip. [V: BillboardOmni.txt `vs`: `iData = iModel*N.x + iFrame*N.y + iPhi*N.z + iTheta`, `GetDither`]
- Wind: none global. Per-object sines (grass) and 3D-noise wobble (leaves, section 2). [V]

## 2. Trees and bushes
- No bushes exist as separate assets: no model/material with bush/shrub/palm/fern in Models or global.txt. Only 16 tree models + grass. [V: Models.fcm.list names; grep global.txt]
- 16 tree models `Models/tree_0..15.bin`, each imported from the stock pack "Realtime 3D Trees XL - 16 Pack" (file `TREExxA/Treexxa_lod1.dae`), scaled x6, root removed (`REMOVE_ROOT {width}`), cylindrical normals on the leaf material. So only the pack's LOD1 mesh is used (753..1361 triangles each). [V: global.txt:2222-2238]
- Two materials per tree: bark `D_near` shader with colour + normal textures (`Trees\BarkNA`, `BarkNA_N`), and leaves `DA_alpha_test_leaves` with `Trees\BranchNA` (leaf cards, DXT5 alpha), `NoiseTileableR8G8`, `Sphere` (a sphere normal map). Leaf tint `g_vDiffuse = (1.3,1.3,0.56)` "[tree_color]" (strong yellow-green). [V: global.txt:2203-2220]
- Leaf shader (`DA_alpha_test_leaves`): 
  - wobble: `vPos += tex3Dlod(noise3D, vPosOrig*0.005 + time*0.008).rgb * 0.5` (0.5 m amplitude, per-vertex, no animation data in the mesh). [V]
  - distance fade: `fAlpha = g_fAlpha*clamp((g_fDistMax - dist)/10, 0, g_fAlphaMax)` and near fade `saturate((dot(vPos-cam,camDir)-2)/2)`; implemented as dither clip `clip(fAlpha - GetDither(vPosProj))` (screen-door cross-fade, no blending, so no sorting). So the mesh version vanishes between `g_fDistMax-10` and `g_fDistMax`. [V]
  - leaf normals are faked as a sphere around the crown centre: tex coords from `vFromCenter = vPos - (origin + (0,16*scale,0))` index the `Sphere` normal map (`0.5+0.07/scale*dot(vFromCenter,right)`, `0.5+0.03/scale*dot(vFromCenter,up)`), so the whole crown shades like a ball (soft, impostor-matching). [V]
  - translucency: sun behind leaf -> emissive `fTraslucency*saturate(0.75-0.5*n.z)*saturate(n.y*4)*(2-alpha)*g_vGrassTranslucency`. [V]
- Trees also cast/receive via the shared pass; shadow map is not used by terrain lighting (comment "FXESC_SHADOW_MAP disableado por [ShadoMapError]" in TerrainTopDown.txt). [V]
- LOD ladder (what the shaders prove): 
  1. close: the full tree mesh (bark + leaves), dither-faded out at `g_fDistMax`. 
  2. far: `Impostor` shader draws 1 quad per instance (4 vertices from `vertexId>>2`, no vertex buffer; `#define SHADER_QUADS`) from a runtime-baked atlas of the model seen from many angles: the cell is chosen per instance by `iModel = ins.iData % g_nTotal` (g_nCols cells per row); the quad corners come from `g_arr16LRUD[iModel]` (left/right/up/down extents per model, up to 16 models); the quad is rotated to face the camera around the world Y axis (`vX = normalize(cross(0,1,0), view)`), i.e. one view per model, not per angle (the angular version is BillboardOmni / street-furniture configs with `thetas`/`phis`). The impostor fades in with `clamp(min((g_vMaxDist.x - d)*g_vMaxDist.y, (d - g_fDistMin)/15), g_fAlphaMin, 1)` = a 15 m dither cross-fade starting at g_fDistMin, and optionally out at g_vMaxDist. Fog colour is looked up from a fog texture, sun term `0.5+0.5*sunShadow` (fake). `if (!g_bRenderingShadowMap) vPos += vToCamera*min(1, d*0.15)/d` nudges the quad up to 1 m toward the camera so it doesn't intersect the ground. [V: Impostor.txt]
  3. A comment says the 15 m is "acoplado a ModelInstanced::Render, CDynamicLodManager::UpdateVariables, Impostor": the C++ has a `CDynamicLodManager` and `ModelInstanced` that set `g_fDistMin`, `g_fDistMax` per model and keep the mesh/impostor ranges consistent. [V: BillboardOmni.txt/Impostor.txt comments; exe strings `dynamic_lod_manager`, `CDynamicLodManager: %s`, `geometry_instance_lod`, `Ctree_impostor`, `impostor_manager`, `ComputeBillboardTexture1/2`]
  4. The impostor atlases are not in the archives (no tree impostor texture in Textures list); the exe bakes them at load (`ComputeBillboardTexture`, `Impostor_%s`, `%s_impostor`, `impostors_%s`). [I: only strings present]
- Actual distances (g_fDistMax, g_fDistMin, g_vMaxDist) for trees are not in any data file: only defaults (`g_fDistMax` 1e9, `g_fDistMin` 0). Unknown, set in C++. [V: effect property defaults]

## 3. GPU object placement / culling pipeline (the heart)
Instance record shared by all compute shaders (64 bytes): `float3 vPos, m_vVel, m_vRow0, m_vRow1, m_vRow2; uint iData`. `m_vRow0..2` = rotation*scale rows (render builds `float4x4(row0,row1,row2,pos)`); `m_vVel.x` = baked terrain light (written by the placer, read by `terrain_light` context so trees get the ground's light); `m_vVel.z==1` = "fading in" flag (render multiplies alpha by `g_fDetailsFadeIn`); `iData` = model index (`%nModels`) plus 6 random bits (`&0x3f`, indexes the 64-entry `g_fRandom` table for per-instance spin/colour). [V: _Instance struct in all CS .txt; Impostor.txt; BillboardOmni.txt; shader_contexts.txt "terrain_light"]

### 3a. `TerrainComputeObjsNearCameraCS` (+ `_deff84900e6` = `#define ONLY_ON_GRASS`)
One thread per source instance (`numthreads(64,1,1)`, count `g_iArgs.z`). Source buffer = a fixed random scatter of positions (a "template" field), `RWStructuredBuffer g_texInstances`.
1. `p.vPos += g_vPos` (world offset of the field). 
2. Torus wrap round the camera: `vCenter = camPos + camDir*g_fRadius`; `p.vPos.x = vCenter.x + fmod(p.vPos.x - vCenter.x, 2R) + (p.vPos.x>vCenter.x ? -R : R)` (same for z). So a field of size 2R x 2R slides with the camera; each template object always lands somewhere in the square around the point ahead of the camera. The whole forest is therefore "infinite" and stateless.
3. Cascade by horizontal distance to `g_vCameraRound` (rounded camera, so objects don't pop when the camera moves a little): `iCascade = d<limits.y ? (d<limits.x ? 0 : 1) : (d<limits.z ? 2 : 3)`. 
4. Height from the cascade's height texture, minus 1 m to avoid floating (tessellation displacement difference). 
5. Masks read from cascade 3 (the most stable one: "la cascade 3 es la más estable") so trees don't flicker when cascades change: flags bit0 must be set (not under sea/tunnel hole), y in `g_vIntervalHeight`, and with `ONLY_ON_GRASS`, mask `.r >= 0.5`. Otherwise `return` (discard).
6. Frustum test: planes 2..5 (left/right/top/bottom) with margin `g_vDistMinMaxNearRadius.w` (default 25 m).
7. `p.m_vVel.x = vTex.b` (baked sun light), then `AppendStructuredBuffer.Append` into BOTH `ArrayInFrustum` and `ArrayNearCamera`, and `InterlockedAdd(IndirectArgs[g_iArgs.y], g_iArgs.x)` on both indirect-arg buffers (x = vertices/indices per instance step, y = which draw slot).
No distance tests here. Distance LOD is left to the render shaders (section 2). Two variants exist: trees only on grass (`ONLY_ON_GRASS`) and objects anywhere. [V: TerrainComputeObjsNearCameraCS.txt full body; `_def` header `#define ONLY_ON_GRASS`]

### 3b. `InstancesFrustumLodCS` (+ `_def00000001/2`, which only add blank lines where `/*DEFORMER_CODE*/` is replaced: the variants are deformer code inserts: the data file `cs_deformer_tex.txt` / `cs_deformer_wrap.txt` give the city-deform texture and the world-wrap deformer)
Same record, for non-terrain placed objects (city props, street furniture, cars).
1. Optional `Deform(p)` (inject point; e.g. sampling `g_texDeformer` at `p.vPos.xz`: `tex2Dlod(g_texDeformer, p.vPos.xz*scale+offset)`). [V: cs_deformer_tex.txt line 1]
2. Test: sphere-ish frustum test against 4 side planes with radius margin `.w`, and `distance(pos, cam)` within `[g_vDistMinMaxNearRadius.x, .y]` (defaults 0..1e9, near radius default 1e9).
3. If inside: Append to `ArrayInFrustum` and add to its indirect args; if also `dist < .z` (near radius) Append to `ArrayNearCamera` and its args. The two lists = "all visible" (impostor/far LOD) and "near" (full model LOD). Written as two separate `if`s because of an Xbox driver bug (comment). [V: InstancesFrustumLodCS.txt]

### 3c. `InstancesFilterCS` (multi-model bucket sort by counting)
Input: instances in `g_texInstances`; `g_nIns.x` count. Per instance: reject beyond `g_fDistFar` (default 150 m); `iModel = iData % g_iArgs.w` (number of models); `InterlockedAdd(g_Counts[iModel],1,iWhere)`; if `iWhere < g_iArgs.z` (per-model capacity) copy to `ArrayOut[iModel*g_iArgs.z + iWhere]` and bump the model's draw args: `g_IndirectArgs[iModel*5*g_iArgs.y + 1] += g_iArgs.x` (that is the instanceCount field of an indexed indirect draw with 5 uints per draw) and, when y>=1, also `[... + 6]` (second LOD draw of the same model). So one indirect draw per model with a capacity cap = the budget. [V: InstancesFilterCS.txt]

### 3d. `InstanceCountShiftCS`
1 thread: multiplies the instance count in the indirect args by `1<<shift` and clamps to a maximum (`g_iShiftParams = (shift, max, quads, -)`); for quads it writes `[0] = 6*min(max, count<<shift)` (index count for a quad list). Used for VR/multi-view duplication (`g_vVPM`, "viewport multiplier" shift in the generated VS: `instanceId >>= g_vVPM.x; iVPI = instanceId & g_vVPM.y`). [V: InstanceCountShiftCS.txt; exe string `instanceId >>= g_vVPM.x`] [I: purpose = stereo]

### 3e. Sort family (`SortCS`, `SortInnerCS`, `SortStepCS2`, `SortInitArgsCS`)
Bitonic sort of instance structs by squared distance to camera, far to near (`if (GetOrder(a)<GetOrder(b)) swap`, "ordeno de mayor a menor"), 512 items per group in groupshared memory (`SORT_SIZE 512`), multi-pass for bigger arrays (`SortStepCS2` has `g_iParam0/1/2` per pass), `SortInitArgsCS` writes the indirect dispatch args `ceil(n / 2^shift)` from the append counter. Purpose: sorted back-to-front drawing of alpha-blended instances (impostor `ALPHA_BLEND` path, particles); it is not used by the alpha-tested/dithered tree path. [V: sort shaders] [I: no tree consumer found]

### 3f. How the pieces connect [I from the buffer names/registers]
`TerrainComputeObjsNearCameraCS` fills Append lists + indirect args each frame -> mesh draw of the trees (`DrawIndexedInstancedIndirect`, `D_near` + leaves materials, `ArrayNearCamera`) and impostor draw (`Impostor`, `ArrayInFrustum`), both distance-faded by dither. `InstancesFilterCS` partitions by model when several models share one buffer. Verified: the shaders' resources and the exe string `IndirectBuffer.Render.m_dwInstancesShift>2`. Not verified: the C++ that binds them.

## 4. Terrain integration
- Data hierarchy:
  1. `Planet` shader: a (tessellated, `fractional_odd`, max 7) sphere; height = `HeightMapEarth.dds` (33 MB) + two noise octaves `0.04*(noise.a*0.5-0.1)` at frequency 1e-5 and 0.0025 at 2e-4 (formula "acoplada a CPlanetDesc::GetHeight"); colour from the style gradient indexed by latitude (x) and height (y); clouds are a second shell (variant `CLOUDS`). Variants: `EXOPLANET`, `TEXTURED` (Moon/Mars/etc. coloured from a diffuse texture). [V: Planet.txt; Planet_def* headers]
  2. `TerrainQuad`: the near ground. VS: per-patch tessellation factor `0.5 + g_vTessFactor.x*saturate(...)` from the patch edge length over the distance (default `g_vTessFactor = (6.5, 0.154, -0.077, 0.1)`, `[maxtessfactor(21)]`, quad domain, integer partition), patches fully outside the frustum or over water/holes get factor -1 (culled: "+20% fps"); the domain shader displaces by the cascade height. Variants: `AROUND_CITY`, `OTHER_PLANET` (colour = layer textures modulated by the planet's own colour texture, `g_vLayer0/1/2` = what to multiply to make the average texture white). [V: TerrainQuad.txt; _def headers] Is it a quadtree? The mesh/patch layout is created in C++ (`TERRAIN_0x%08x_Plane` string); the shaders only show a flat patch grid displaced by cascades, not a tree. [I: no quadtree code in shaders; geometry may be a clipmap-like grid]
  3. `TerrainTopDown` writes the cascades: for every texel (4 cascades of `TEX_SIZE` 1024, 512 in LOW) it recomputes the planet height at that spot (same formula as Planet + "extra mountainousness" `saturate(over/0.05) * (noise*0.5+0.5) * g_fHeightMountains`, sea floor squashed), adds near objects' height (craters, Stonehenge `TerrainHM` height textures), city height textures (`CITY_SEA_OVER`, tiled noise, hole under cities), tunnel holes, then derives the normal (4 taps of 5 m) and the masks. [V: TerrainTopDown.txt]
- Mask rules written by `TerrainTopDown` (the biome logic): 
  - `fRocks = fNoBeach * saturate((0.97 - n.y)/0.03)` -> rock where slope normal.y < 0.97 (about >14 deg) ramping to full rock at n.y 0.94; `fNoBeach = saturate(|h - sea|/10 m)` removes grass within 10 m (height units) of the shore. 
  - grass = `(1 - fRocks) * patternFactor * fNoBeach` where `fGrassAux = saturate(|0.75 - frac((fHFNoise*20000 + n.z*40)/(g_fHeightMountains*250000))| / 0.25)`, used as `1-fGrassAux` or `fGrassAux` if `g_bMoreGrass` (style has `more_grass 1`). I.e. a banded noise (from the high-frequency height noise and the slope's z component, repeating) cuts the grass into patches/patchy meadows; no real climate. 
  - output RGBA = (grass, 1-grass-rocks [= second ground texture, dirt/sand], baked sun light = `saturate(dot(sunDir, normal))`, flags/255). Flags: 1 draw land, 2 draw sea, 4 "ground may not clip (coast shape)". Near cities: grass reduced by slope, "césped sin árboles" 0.4/0.6 to keep trees out of parks; Stonehenge: `fGrass = lerp(1,0.4,detailsAlpha)` "[grass_without_trees]" -> TerrainQuad uses `saturate(r*4)` for the texture blend, and the tree placer needs `r>=0.5`, so r in [0.1,0.5) means grass blades but no trees. [V: TerrainTopDown.txt, TerrainQuad.txt `vDiffuse.r = saturate(vDiffuse.r*4.0f); // [grass_without_trees]`, ComputeObjs `ONLY_ON_GRASS`]
  - Ground colour = r*tex_0(grass_01) + g*tex_1(sand/dirt) + rocks*tex_2(rock), tiled at 0.125 m^-1 (`vPos.xz*0.125`), times `g_vDiffuse` and `ambient + baked light`; bluish near the sea. The texture trio comes from `TERRAIN_STYLE {tex_0..tex_3, height_map_amplitude, color_factor, more_grass}` entries in global.txt (field, rocks, mountains, snow_mountains, desert, grass, desert_sand ... alien variants); the planet picks a style per lat/long cell via `m_arrStyles` (Earth: 64 rows x 16 cols, 7 styles; deserts/snow rows hand-painted, comment "giza aquí"). [V: global.txt:4885-4913; scene_universe.txt:41-70] Index to style order is [I].
- Transition from space to ground: dither cross-fade. `Planet` PS: `clip(max(g_fAlpha - dither, dist*g_vDistMax.y - dither + g_vDistMax.x))`; `TerrainQuad` PS: `clip(dist*g_vDistMax.y + dither + g_vDistMax.x)`: complementary so the planet shell disappears as the quad terrain appears; `Planet` also snaps the pixel to the terrain plane (`PlaneIntersectSegment(g_vPlaneTerrain,...)`) and uses the same `g_matRound100000` / `g_matWorldToPlanetCoords` so they agree; `TerrainQuad.vNorFar` blends in the planet's normal map with distance. [V: Planet.txt, TerrainQuad.txt]
- Floating origin: noises indexed through `g_vRound100000 / g_vRound40000` (rounded offsets) to keep floats exact near the camera. [V]
- Where vegetation exists: Earth for sure (cascade grass from `grass_01`, `more_grass` style). On other planets the `OTHER_PLANET` quad variant exists and exoplanets use `TERRAIN_STYLE` entries with `grass_01` (global.txt:4897-4904) so grass/trees may appear there too, but the shaders can't tell: the `Planet` body table gives style 6 (`desert_sand`) to `interlude1_planet`. [V: scene_universe.txt:338-343] Whether trees use ONLY_ON_GRASS on exoplanets: unknown (C++).
- Water: `Ocean`, `OceanUnderSea`, `TerrainSeaEffects` strings in the exe; flags bit 2 select water; trees die below `g_vIntervalHeight.x`. [V strings/shader]

## 5. Budgets, distances, densities in the data
- Grass: 50,000 points, 150x150 m wrap square, fade 140-150 m, blade 0.66 x 0.66 m. [V]
- Tree placer: radius `g_fRadius` and the instance count `g_iArgs.z` come from C++ (not in data). Frustum margin 25 m (default). [V defaults]
- Tree filter: capacity per model `g_iArgs.z`, default far distance 150 m, models `g_iArgs.w`. [V]
- Cascades: 4 layers, 1024^2 (512 LOW) x 4 channels (RGBA8; height in a second target, 32-bit float R: `FMT_32_R`). The limits `g_vTexLimits` are C++. [V: TerrainTopDown.txt `#pragma PSSL_target_output_format (target 1 FMT_32_R)`; TEX_SIZE]
- Tessellation up to 21; planet sphere tessellation up to 7. [V]
- Leaves: 0.5 m noise wobble; 10 m fade in the mesh path, 15 m in the impostor path. [V]
- Impostor atlas parameters in data (for street furniture, same system): `Data/street_furniture_normal.txt`: models 11, phis 16, thetas 32, `size_factor {35.5 35.4375}`, `angles {5.093 10.186 0}` (= 16/pi and 32/pi: steps per radian); `antenas.txt`: models 5, phis 8, thetas 16, size 6.625x6.75. `BillboardOmni` defaults: 32 models x 32 frames, 4 phis, `g_vAngles` 8/pi. [V]
- Tree `.bin` triangle budgets: 753-1361 tris/tree (LOD1 of the pack). [V]
- Impostors `Stone.bin`: a single POINT vertex (1 vertex, no UV); a "Stone" material uses Impostor with a 4x4 grid of 16 stone cells (`g_nTotal 16 g_nCols 4`, texture `stones.dds` 512^2 DXT5). Instances are expanded to quads in the VS (`vertexId>>2`), i.e. 4 vertices per instance generated with no vertex buffer. [V]

## 6. Exe strings relating to this (C++ side hints; exe not decompiled)
`Ctree_impostor`, `impostor_manager`, `ComputeBillboardTexture1/2`, `Impostor_%s`, `%s_impostor`, `%s_impostorRotating`, `ImpostorRotating_%s`, `impostors_%s`, `gpu_models_impostors`, `city_impostor`, `CCityImpostor`, `dynamic_lod_manager`, `CDynamicLodManager: %s`, `geometry_instancer`, `CGeometryInstancer`, `CGeometryInstance`, `CGeometryInstanceLOD`, `geometry_instance_lod`, `BatchRenderer@CGeometryInstancer`, `video_auto_lod`, `MENU_OPTIONS_AUTO_LOD`, `TerrainGrass`, `TerrainComputeObjsNearCameraCS`, `CS_%s_%s_%.02f` (generated compute-shader names), `more_grass`, `ONLY_ON_GRASS`, `terrain_style`, `CTerrainStyle`, `CTerrain`, `CTerrainObject`, `TerrainTopDown`, `TerrainTopDown_height`, `TerrainQuad`, `TerrainQuadStar`, `TerrainShadows`, `TerrainAroundCity`, `TerrainCityHole`, `TerrainUnderCity`, `g_iGrass` (shader param name), `g_bMoreGrass`, `g_vGrassTranslucency`, `g_fDetailsFadeIn`, `g_vCamPosDetails`, `PushTrees` (Lua call: trees can be knocked by the UFO), `IndirectBuffer.Render.m_dwInstancesShift>2`, `Models\%s.bin`. [V: offsets 0x601xxx-0x61xxxx in MegatonRainfall.exe via string scan]

## 7. Reusable vegetation assets
### 7.1 Inventory
Archive paths inside the ZIPs (`Assets\Textures.fcm`, `Assets\Models.fcm`), sizes in bytes. [V: *.fcm.list + the extracted files]

**Textures/Trees/ (35 files, all DDS)**
| Species | Bark colour (DXT3, 512x1024, 11 mips, 699,216 B) | Bark normal (`_N`, same format) | `_S` (same format; no material uses it) | Leaf card (DXT5, 512x512, 10 mips, 349,680 B) |
|---|---|---|---|---|
| pack tree 01/08 | Bark1A | Bark1A_N | Bark1A_S | Branch1A (tree_0, tree_12) |
| pack tree 02 | Bark2A | Bark2A_N | Bark2A_S | Branch2A (tree_2) |
| 10 (thin trunk) / 02 | Bark2A (tree_3 bark) | | | Branch10 (tree_3; material name `RT_Tree076767ff6-effect`) |
| 03 | Bark3A | Bark3A_N | Bark3A_S | Branch3A (tree_4, tree_7) |
| 04 | Bark4A | Bark4A_N | Bark4A_S | Branch4A (tree_1, tree_8, tree_14) |
| 05 | Bark5A | Bark5A_N | Bark5A_S | Branch5A (tree_5, tree_9) |
| 06 | Bark6A | Bark6A_N | Bark6A_S | Branch6A (tree_10) |
| 07 | Bark7A | Bark7A_N | Bark7A_S | Branch7A (tree_6, 11, 13, 15) |
| 09 | Bark9A | Bark9A_N | Bark9A_S | none of its own |
| 10 | Bark10A | Bark10A_N | Bark10A_S | (see Branch10) |
There are 9 bark sets (1A-7A, 9A, 10A) x 3 files = 27 files, plus 8 leaf cards (Branch1A-7A, Branch10) = 35. Bark8A and Branch8/9 do not exist. [V: Textures list]

**Other vegetation-related textures (Textures/)**
| File | Format | Use |
|---|---|---|
| `Billboard_Grass.dds` | 2048x2048, 12 mips, DXT5, 5,592,560 B | 2x2 atlas of grass tufts, alpha cut-out (materials `billboard_grass`, `billboard_grass_xz`, shader TerrainGrass `g_texSampler2`) |
| `grass_01.dds` | 2048x2048, 12 mips, DXT5, 5,592,560 B | ground grass layer `tex_0` of most TERRAIN_STYLEs; also city-park lawns |
| `test_dae/grass_01.dds` | 2048x2048 DXT1 | test copy, unused |
| `Estadio/grass_04.dds` (699,192 B) | stadium pitch | not vegetation system |
| `rock_01.dds`, `rock_02.dds`, `stones_01.dds`, `Stonehenge/stones.dds` | 2048^2 DXT1 | ground rock layers |
| `stones.dds` | 512x512 DXT5 | the "Stone" impostor 4x4 atlas |
| `sand_01..03`, `dirt_00..03`, `snow_ice_01..04` | ground layers (names from global.txt:4885-4913) | other `TERRAIN_STYLE` textures (I did not read their headers) |
| `NoiseTileableR8G8.dds` | 2097300 B | 3D/2D noise used for leaf wobble |
Sphere: the `Sphere` texture used as the leaf normal map is a texture named "Sphere" (not looked up in the list here). [V: global.txt:2203 for the name; I did not check the file]

**Models/ (Models.fcm; `.bin`, bytes)**
- `tree_0` 103,784 | `tree_1` 114,186 | `tree_2` 125,324 | `tree_3` 111,710 | `tree_4` 131,221 | `tree_5` 128,240 | `tree_6` 141,732 | `tree_7` 130,504 | `tree_8` 153,704 | `tree_9` 145,180 | `tree_10` 81,464 | `tree_11` 116,178 | `tree_12` 120,784 | `tree_13` 106,170 | `tree_14` 124,785 | `tree_15` 119,958.
- `TerrainGrass` 800,664 (50,000 grass points); `bunch_0..3` 777 each (crossed-quad tuft, 8 verts / 4 atlas cells); `billboard_grass_0..3` 582 each (single quad); `Stone` 599 (one point sprite for the Impostor).
- There are NO separate LOD models, bushes, palms or low-poly distant-tree models. The only "LOD" of a tree is the pack's LOD1 mesh in `tree_N.bin` + the runtime-baked impostor. [V: list of Models.fcm; grep]

**Per tree (parsed from the files): vertices / triangles / height / width / bark / leaf material / source pack tree** [V]
| model | pack tree | verts | tris | height m | width m | bark | leaf material (card) |
|---|---|---|---|---|---|---|---|
| tree_0 | 01A | 2437 | 813 | 25.3 | 16.5 | Bark1A | Tree01A_FOL (Branch1A) |
| tree_1 | 16A | 2926 | 980 | 24.1 | 14.3 | Bark5A | Tree04A_FOL (Branch4A) |
| tree_2 | 02A | 3139 | 1047 | 22.1 | 14.7 | Bark2A | Tree02A_FOL (Branch2A) |
| tree_3 | 10A | 2485 | 829 | 20.1 | 10.8 | Bark2A | Tree076767ff6 (Branch10) |
| tree_4 | 11A | 3286 | 1096 | 16.6 | 18.4 | Bark10A | Tree03A_FOL (Branch3A) |
| tree_5 | 14A | 3117 | 1039 | 27.5 | 17.6 | Bark9A | Tree05A_FOL (Branch5A) |
| tree_6 | 12A | 3365 | 1125 | 17.5 | 13.1 | Bark3A | Tree07A_FOL (Branch7A) |
| tree_7 | 03A | 3275 | 1093 | 19.9 | 21.1 | Bark3A | Tree03A_FOL (Branch3A) |
| tree_8 | 04A | 4075 | 1361 | 24.8 | 18.7 | Bark4A | Tree04A_FOL (Branch4A) |
| tree_9 | 05A | 3641 | 1215 | 16.1 | 22.4 | Bark5A | Tree05A_FOL (Branch5A) |
| tree_10 | 06A | 2255 | 753 | 28.4 | 19.1 | Bark6A | Tree06A_FOL (Branch6A) |
| tree_11 | 07A | 2776 | 928 | 17.0 | 19.5 | Bark7A | Tree07A_FOL (Branch7A) |
| tree_12 | 08A | 3071 | 1027 | 30.3 | 16.8 | Bark9A | Tree01A_FOL (Branch1A) |
| tree_13 | 09A | 2628 | 878 | 28.4 | 13.6 | Bark7A | Tree07A_FOL (Branch7A) |
| tree_14 | 13A | 3046 | 1018 | 18.9 | 22.2 | Bark10A | Tree04A_FOL (Branch4A) |
| tree_15 | 15A | 3004 | 1002 | 12.3 | 19.0 | Bark7A | Tree07A_FOL (Branch7A) |
Bark and leaf choices are read from the materials embedded in each `.bin` (bark list by regex) and agree with global.txt:2222-2238 for the leaf. Heights/widths are the bounding box of the baked positions (units are metres; scale x6 already applied; y up, origin at the trunk base, y min about 0). `tree_0..3` are annotated "los que quedan bien en ciudad [tronco_delgado]" (thin trunk, used for city streets). The two bark materials are `RT_Bark?A-effect` = shader `D_near` with textures {Bark, Bark_N}. [V: global.txt:2211-2238 and the `.bin` string tables]

### 7.2 Origin: stock vs made for the game
- Trees: a stock asset-store pack. The import path in global.txt is `...\models\Realtime 3D Trees XL - 16 Pack\3DS\TREExxA\Treexxa_lod1.dae`, material names `RT_Tree01A_FOL`, `RT_Bark1A` (RT = "Realtime Trees"), textures `BarkNA/_N/_S`, `BranchNA` follow that pack's naming. Likely a commercial pack bought for the project (licence terms unknown; the user owns the game, but check the licence for anything beyond reading the files at run time, which is the plan). [V: file_name strings; I: "RT" = pack prefix, store origin]
- Grass: made for the game (the `Billboard_Grass` atlas and the 50k-point `TerrainGrass` mesh are generated by the engine's own "BUNCH"/point generators; `grass_01` is a generic texture, `test_dae/grass_01` suggests an internal test). [I]
- Ground layer textures (`rock_01`, `sand_01`, etc.) look like generic texture-library pieces. [I]
- Leaf colour: the leaf cards are painted for a green tint multiplied by (1.3, 1.3, 0.56) in the material, so reuse needs that tint to look the same. [V: global.txt:2203 "[tree_color]"]

### 7.3 Decoded file formats (for a loader)
**DDS** [V: headers read for all 35 Trees textures, grass and rock]: classic 128-byte DDS (magic `DDS `, no DX10 header), legacy FourCC: bark `DXT3` (BC2), leaf cards and grass atlas `DXT5` (BC3), rock/ground `DXT1` (BC1) with `mipmaps` full chain (11 for 512x1024, 10 for 512x512, 12 for 2048x2048). Sizes in the table above match `header + mip chain` (e.g. 699,216 = 128 + 699,088 for DXT3 512x1024 with 11 mips). No sRGB flag exists in DDS; assume colour textures are sRGB, `_N` linear. The `_N` bark normal maps are DXT3 (BC2), so the three channels are plain RGB (not BC5): shader decode `tex*2-1` with the leaf `Sphere` normal map decode `xyz*2-1`; `-vNorMap.z*camDir` in the leaf shader. [V shaders; I: sRGB assumption]
**Models `.bin`** (all little-endian) [V = parsed and cross-checked against the shaders and against index ranges; G = guessed]:
```
u32 magic = 0xCD (205)                        [V, all files]
u32 version = 1                               [V]
u8  flags = 0x10                              [V constant in the 8 files read; meaning G (stream mask?)]
then a series of arrays, each = u32 count + count*elem bytes, in this order:
  positions   float3 (12 B)                   [V]
  normals     snorm8 x3 (3 B, no padding; 127 = 1.0)   [V: tree_0 normal (33,7,-122), tangent (122,5,33): orthogonal]
  tangents    snorm8 x3 (3 B)                 [V]  (grass quads: tangent 0 or (126,0,0))
  colours     RGBA8 (4 B) - count 0 for trees; 8 for bunch (all 0xFF) [V]
  texcoords   half2 (float16 x2, 4 B)         [V: grass atlas 0/0.5; tree u,v in about [-1,2.7] i.e. tiled]
  3 more arrays of 4-byte elements, always empty in the files read (2nd UV set, bone ids, weights?)  [V empty; role G]
  indices     u32 (4 B) triangle list         [V: max index = vertices-1 in every file]
u32 numSubMeshes
per submesh: u32 firstIndex, u32 firstVertex, u32 indexCount, u32 vertexCount   [V: tree_0 (0,0,1683,1683) + (1683,1683,756,754); sums = totals for all 16 trees]
             then a material record: u8 1, u32 byteLength, payload   [V framing]
               payload: u32 flags(0x0f), u32 0, then length-prefixed material name ("RT_Bark1A-effect"), effect name ("D_near"), then property records (type byte + 4-byte hash of the name + value) and texture names ("Trees\Bark1A", "Trees\Bark1A_N")   [V strings; hash/property layout G]
trailing data: bounding box / vertex remap (a long list of u32 after the last submesh in tree_0)   [G; not needed to render]
```
Per-vertex stride a loader would build: pos(12)+normal(3)+tangent(3)+uv(4) (+colour 4 when present); arrays are optional by count (Stone.bin has 1 position, normal, colour and no UVs/tangents). Positions are in model space, metres, tree base at y=0, no skinning in the trees. There are no LODs inside a file: one geometry per file, split into 2 submeshes (bark/leaves; the order can be swapped, e.g. tree_3 has leaves first), so match submesh to texture by the material name inside the record. Since the engine re-imports the DAE through `MESH_IMPORT`, the `.bin` is the baked result: normals of the leaf submesh are already "cylindric" (REMOVE_ROOT/CYLINDRIC_NORMALS applied). [V: tools/bin.py, run over bunch_0, billboard_grass_0, Stone, tree_0..15, TerrainGrass]
Unresolved: the exact meaning of the 0x10 flag, the three empty arrays, the property-record hashes, the trailing block. None is needed to draw: use pos/normal/tangent/uv/indices + submesh table + texture names.

## 8. Methods worth adopting
1. **Camera-centred top-down "terrain cascades" as the single source of truth.** Render height, a grass mask, ground-layer weights, baked sun light and flags for a few nested squares around the camera (here 4 x 1024^2, 1 channel float height + RGBA8). Ground, grass and trees all sample it. Cheap because it is one small render per frame (or on movement) and every later pass is a texture fetch; the grass/tree decision is a texture lookup, not noise evaluation. Needs from the terrain system: a function height(x,z)/normal/biome you can run in a fragment shader for the area, plus a recentring policy (the game uses rounded camera positions to avoid popping; the finest cascade is not used for trees: use the coarsest, steadiest one).
2. **Stateless torus-wrapped scatter.** Keep one fixed set of N random positions in a square of side 2R; every frame, `pos = center + fmod(pos - center, 2R) +- R` puts each point inside the square around the camera. No streaming, no allocation, deterministic; works for grass points in the vertex shader and tree templates in a compute shader. Needs: a stable random template (the game uses 50,000 grass points / a fixed tree buffer) and a height+mask lookup. Caveat: density can't vary per area except by killing points via the mask (so ask the mask to carry density).
3. **Compute cull + append + indirect args.** One thread per template instance: wrap, fetch cascade height/mask, frustum test (4 side planes, 25 m margin), append, atomic-add into the indirect draw args. Output goes straight to `DrawIndexedIndirect` with no CPU readback. Needs from Bevy/wgpu: storage buffers, atomic counters, `draw_indexed_indirect` (supported), a pass that writes the indirect buffer before the main pass.
4. **Per-model bucket counting.** `iModel = iData % nModels`, atomic count per model, capacity cap, write to `out[model*cap + slot]` and bump that model's instance count in a 5-u32 indirect-args block: one indirect draw per tree species with a hard per-species budget. Cheap and simple; the cap is the performance guarantee.
5. **Mesh <-> impostor cross-fade with dithering, no sorting.** Mesh alpha = `(DistMax - d)/10` and impostor alpha = `(d - DistMin)/15` with `clip(alpha - dither)` (screen-door), both alpha-tested and written to depth: no OIT, no sorting, works with deferred/MSAA. Needs a Bayer/dither function and TWO draws of the same instance list. Quads for impostors can be generated from `vertex_index>>2` with no vertex buffer.
6. **Impostor atlas baked at load, indexed per (model, angle).** The engine renders each model from `thetas x phis` directions into an atlas (config `models/phis/thetas/size_factor/angles`) and picks a cell from the view direction in the object frame; trees use a camera-facing quad with a per-model LRUD rectangle. Cheap: 1 quad per far tree. Needs: a bake pass at load (render the 16 models from a few angles) and extents per model.
7. **Grass as camera-facing alpha-tested quads from a geometry/mesh-shader stage; grass shrinks to zero size at the edge** (`fSize = saturate((2R - d)/10)`), so no alpha, no pop, no sort. In wgpu without geometry shaders: expand points to quads from `vertex_index` (6 verts per instance, instance buffer = the 50k points) or compute the visible list first (method 3). Colour = constant tint * baked ground light from the cascade, so the grass matches the ground.
8. **Light baked in the cascade and carried on the instance.** The ground sun term goes into the cascade's B channel; the placer copies it into `m_vVel.x`; trees/grass use `ambient + baked*sun` (no shadow map, no per-instance lighting), plus a fake translucency term from `dot(view, sun)`. Very cheap and visually continuous with the ground. Needs: a baked N.L in the cascade; works only for a single sun (for multiple stars compute it per star).
9. **Foliage shading tricks:** leaf normals from a sphere-normal-map indexed by the vertex's offset from the crown centre (soft clump shading from flat alpha cards), 3D-noise vertex wobble (0.5 m), alpha-test 0.5. No skinned wind.
10. **Vegetation rules as three smooth masks, not a biome map:** rocks by slope (`saturate((0.97 - n.y)/0.03)`), shore exclusion by height (`saturate(|h - sea|/10m)`), patchiness from a repeating band of high-frequency height noise and slope; a "no trees but grass" band by thresholding the same mask (grass r>=0.1, trees r>=0.5). Easy to feed from the port's own planet generator.
11. **Space-to-ground hand-off by complementary dither clips:** far sphere clips with `max(alpha - dither, dist*s - dither + o)` while near terrain clips with `dist*s + dither + o`; same noise functions and the same rounded origin on both sides. Vegetation simply starts inside the near terrain's range.

## 9. Unknowns (could not be verified from the data/shaders)
- Every distance and count that C++ sets: `g_fRadius`, tree instance count (`g_iArgs.z`), per-model caps, `g_vTexLimits` cascade radii, `g_fDistMax`/`g_fDistMin` of trees and impostors, `g_vCenter` for the grass (camera or hand-set), `g_vIntervalHeight` per planet.
- How the source tree positions are generated (random field? density from style? which model per instance) and whether the same tree buffer is used on exoplanets; whether non-Earth planets get trees/grass at all.
- Where the impostor atlas for trees is generated and its resolution/angles (exe strings only, no tree impostor config file; street furniture has configs).
- TerrainQuad mesh topology (quadtree vs fixed grid), the order/meaning of the Earth `m_arrStyles` indices.
- The role of `Sphere` and the NoiseTileableR8G8 files (not opened), the exact property-record format and trailing block of the `.bin`, and whether the `*_S.dds` textures are used anywhere (no material references them).
- Whether grass lives on tessellated terrain only (it samples cascade 0 only; behaviour farther than the nearest cascade is just the 150 m limit).
