> Research notes made on 2026-11-04 for `docs/spec-vegetation-civilizations.md`: what Megaton Rainfall
> does, read from its shaders, data and file lists (the GOG copy in `C:\Users\liamw\Downloads\Megaton Rainfall`).
> Its text files and shaders are unpacked in `re/megaton_rainfall/extract` (out of git); the scripts that
> read its formats are in `re/megaton_rainfall/tools`. The port reads the game's assets at run time from
> that install and never copies them into the project.

# Megaton Rainfall (MR): how cities are generated and drawn (methods + reusable assets)

Sources: shaders in `re/megaton_rainfall/extract/Effects` (comments are Spanish), text data in `re/megaton_rainfall/extract/Data`, exe strings
(`MegatonRainfall.exe`, strings only), small extractions in `a scratch folder (not kept)` (headers of DDS/bin, ~25 model
files, 2 city textures), scripts in `re/megaton_rainfall/tools`. All of it is read-only research.
Tags: [V] verified from a quoted file/identifier/bytes, [I] inferred (reason given), [?] unknown.
Shader units are metres (the Planet shader's night-light ramp uses 800,000..10,000,000 for 800 km..10,000 km) [I].

## 0. Headline findings
1. MR does NOT invent city layouts procedurally for the famous cities: the layouts come from OpenStreetMap (OSM) XML
   files read offline (`CCityModel::Compute xml_document::load_file`, `CCityModel::InitDistricts OSM`) and baked into one
   big `Data\city_0x<hash>_<name>.bin` per city (20-70 MB each, 13 files) plus a `Districts\District_0x<hash>` file [V].
   Cities WITHOUT an osm entry (cities_params_no_osm.txt: Washington, Rome, Paris, Miami...) use another path, called
   `CCityModel::Set no file` [V name], which is presumably a procedural generator (the Style enum `oneoldstyle..zerooldstyle` hints at
   10 procedural styles) [I]. The exe holds a `CCityModel` with `Node`, `Edge`, `Block`, `Piece`, `District`, `Style`
   (RTTI `.?AUBlock@CCityModel@@`, `Edge`, `Node`, `Piece`, `.?AV...Style@CCityModel`) = a road graph (nodes/edges) with
   blocks between roads, cut into pieces [V names, structure I].
2. Draw chain by distance (all verified in shaders): night-side city lights painted into the planet's own cloud/cities texture
   (orbit) -> `CityVeryFar` textured district quads (to 900 km) -> `CityFar` / `CityFarWrap` cube-map impostor of the whole
   city projected on the ground plane -> real meshes in 3 LOD material sets (`windows`, `_med`, `_near`, `_fromMed`) + GPU
   instanced props with billboard impostors -> interior-mapped windows at street level.
3. The "CityDeform*" shaders are about DESTRUCTION (shock-wave deform of buildings/roads in a 512^2 deformer texture), NOT
   terrain flattening. Terrain flattening is a separate "city texture" (section 4).
4. The game's buildings are NOT boxes assembled in global.txt. global.txt has no building generator: it only has materials
   and imported meshes (222 MESH_IMPORT, 91 `mesh_gen BOX`, all for props/UFOs). Building facade geometry comes from C++
   (`CCityObject::ComputeModel`) using ready-made wall modules `Models\window_0..9.bin` (5 or 10 m bays, 3 m storeys)
   and the `windows` texture array; see section 3.

## 1. Where cities go (cities.csv, cities_params, Districts, bins)
- `Data\cities.csv`: 17,300 lines, columns `country,key,Display name,admin/region,population,lat,lon` [V: `ar,cullen,Cullen,23,,-52.88,-68.45`;
  `jp,tokyo,Tokyo,40,31480498,35.685,139.751389`]. Population is often empty (small hamlets). `cities_short.csv` is a 12-city subset
  (Sydney, Rio, Johannesburg, Manila, Miami, Dubai, LA, Tokyo, NYC, Madrid...) [V]. The exe error string: "Ciudad '%s' referenciada por scripts y no encontrada en cities.csv" [V].
  Use: a city object (`CCity`, `CCitySector`) is made at the lat/lon of a listed town; only towns named in `cities_params*.txt`
  get a detailed model, the 17k others are presumably just markers/lights [I].
- `cities_params.txt` (identical to `_osm.txt`): per-city options [V all keys below]:
  `osm "<file>.osm"` (OSM export, author's own disk path, NOT shipped), `osm_radius 12500.0` (m; Dubai 17500), `osm_center {lon lat}`,
  `osm_coast_radius 7500.0` (NY, Sydney: a smaller disk where coast lines are read), `density 0.0`, `coast yes`, `river yes`,
  `city_landmark { name "empire_state" lon lat }` (several for Dubai), `skyscraper_areas { {-976 -137} }` (NY: a point in local metres
  where skyscrapers cluster).
  `density 0.0` is set for every OSM city: exe has `density`/`osm_all`; 0 most likely = "do not add procedural filler buildings, trust OSM" [I].
  `cities_params_no_osm.txt`: same list with osm lines commented out + extra names (miami, paris, johannesburg, istanbul, honolulu) [V].
  `_demo`: maidstone (a "parking" landmark = test lot, exe `Parking Lot`), London, Dubai. `_pegi`: no-OSM names only.
- Radius 12.5 km => a city = a 25 x 25 km square [V District header: corners +-12500 m]. That is exactly 5 x 5 sectors of 5000 m [V next point].
- `Data\Districts\District_0x<hash>[_<osm name>]` (33 files, 387 KB-1.1 MB; 13 carry the OSM file name, 20 only the hash) [V list].
  FORMAT (binary, little endian, decoded from London `District_0xcb1822ac_london_england`, 613,696 bytes) [V]:
  - 12 floats: 4 corner x,z pairs (+-12500) then an enlarged box (+-12832, y from -49.7 to 232.1) and a third (+-12822) = bounds of the area.
  - u32 `0x18` (24) and u32 1, then a table of 25 fixed 100-byte records, each a 4x3 matrix + translation (x,0,z) with
    x,z in {-10000,-5000,0,5000,10000} (a 5x5 grid of 5 km sectors; positions found at byte offsets 112,212,312,...2312).
  - after the table: variable lists. Seen: `count=0x11f` then (x,z) float pairs such as (7805.1,477.5),(7681.4,432.0),(7614.8,412.5)
    = a 2D polyline/polygon in city metres (287 points). So the file holds district outlines (the shape of each district, which
    is what the `CityFar` mesh is built from: shader comment "ahora pinto CityFar con malla con la forma de los distritos") [V comment, I link].
  Not decoded: the per-record fields, how polygons group into districts. [?]
- Exe strings tie the files: `Data\Districts\District_0x%08x%s`, `Data\city_0x%08x%s%s.bin`, `Textures\city_0x%08x.dds` and `_Normal.dds`,
  `Textures\district_%d.dds`, `district_%d_Night.dds`, `district_%d_Normal.dds`, `city_0x%08x_night`, `_in` [V]. The hash is
  probably a hash of the city key (the named files keep a readable suffix, e.g. `_tokyo_japan` = the OSM file name stem) [I].
- Big `.bin` (Data.fcm, only names/sizes/first bytes read): 13 `city_0x*.bin`: tokyo 69.9 MB, dubai 63.0, NY 50.4, SF 46.9, sydney 44.4, 0x3b0d9a19 40.1 (no name),
  0xecef1a41 40.1 (no name), rio 39.6, rome 37.1, london 26.5, agra 24.5, madrid 21.8, LA 20.6 (total ~527 MB with the small ones) [V sizes].
  Zip ratio ~0.43 (so mostly float data, not text). Header: 1 flag byte 0, then floats: bounds (-12826,-49.4,-10964), (813,228.8,12150)...,
  then count `0x3d` (61), `0x3a`, then x,z pairs of a polyline: (-12237.8,5533.7),(-12289.2,5588.6)... spaced ~75 m along a line = a
  coast/river/outline polyline [V bytes; reading I]. The body (buildings/roads) is NOT decoded [?]. Content is expected to be the baked
  `CCityModel` after OSM processing (nodes, edges, blocks, building footprints+heights) because an OSM parse at game start would need the .osm files, which are not shipped [I].
- Small bins: `city_size.bin` 180 bytes = 45 u32 (e.g. 0x162eb8=1,454,776; 0x045da0; 0x102ee4...) with a repeating 3-value pattern: very likely vertex/index/instance counts used to pre-size buffers [I,?].
  `windows_colors.bin` 292,644 bytes: u32 18 then float RGB-like triples: the 18 window-type colour/tint table (18 = the `windows` array depth) [V count, I use].
  `morpher.bin` 87 bytes (6 float3 + ...): a tiny shape-morph table, not city [?]. `clouds_EarthCloudsCities.bin` 274 KB: the planet's cloud+city-light map (below).
  `stadium_crowd.bin` 600 KB: crowd instances for the stadium [I from name].
- Per-city textures (Textures.fcm): `city_0x<hash>.dds` and `_Normal.dds`, each 2048x2048 DXT5 with 12 mips (5,592,560 B) [V: `city_0xb1943604`],
  one pair per city (13 pairs + a 1024 pair `0xe81e2854`). I decoded Madrid's: an ATLAS of ~60 irregular district patches packed into the square,
  each a top-down painting of its streets (grey roads/sidewalks, dark green blocks), transparent between patches [V by viewing].
  Normal map: RG = normal, B = mostly white with black blobs inside blocks (looks like building footprints dark / open ground white),
  A = white blobs on streets = street-light glow positions [V by viewing; B meaning I]. The shader comment confirms: "normal en .rg, calles en .b, glows farolas en .a" [V CityVeryFar.txt].
  The exe has separate `Build\Textures1024` and `Textures2048` builds (Nintendo port) [V string].

## 2. How a city is built (what the data says)
- Road graph: OSM `highway`, `lanes`, `footway`, `sidewalk`, `river`, `riverbank`, `coast`/`coastline`, `building`, `building:levels`, `height` are string
  literals in the exe [V]. The road mesh has per-vertex lane count and crosswalk count: `Road.txt` reads `vTan.y*127` = lanes, `vTan.z*127` = crosswalks, `vTan.x` = lateral coordinate
  across the road; lane lines are chosen in the shader (centre line where `vTan.x` is near `fLanesLeft/fLanes`, `RoadLines` texture) [V].
  So the road mesh is a strip per edge, UV.x = across (0..1), UV.y = along in metres, UV.x*UV.y is distance from the start in metres (crosswalk if < 6 m from either end) [V `vTexCoords0.x*vTexCoords0.y<6.0f`].
  Intersections: a different UV mode (`vTexCoords0.x` outside 0..1 => plain `Asphalt` at `vPosOrig.xz*0.25`) [V].
- Buildings: `building:levels` and `height` attributes feed storeys; storey height is 3 m (interior floor planes are at `floor(y/3)*3`
  in the window shaders) [V]. Facade modules `window_0..9.bin` (below) are 3 m tall and 5 or 10 m wide [V from bounding boxes]. The window type per
  module vertex comes from the vertex colour: `iData = uint(vDiffuse.b*255) % 18` (the "window_texs" index into an 18-layer texture array) [V DS_NG_Window.txt].
  The wall material is `DS_NG_Auto` (tri-planar brick `brick_0` + normal) [V window_0.bin material names: `window` (glass, effect `Cube`), `brick_0`/`brick_6` (`DS_NG_Auto`)].
  Roof: material `DS_Auto_XZ_RoofTop_fromMed` with `RoofTop` texture [V]. Roof props are separate meshes: `air_condition_0`, `air_condition_window_0..3`, `chimney_0..6`,
  `chimney_facade_0/1`, `hvac_big_0..2`, `hvac_small_0..2`, `antena_small_0..4`, `roof_fan`, `roof_sideboard`, `water_deposit`, `bricks_heap_0`, `cabin_prop_0/1`, `awning_0`, `awning_low_0`, `pot_0..3` [V names].
  Landmarks: `city_landmark { name lon lat }` places a hand-made model (`empire_state`, `tokyo_tower`...) with an `_inner` companion (probably the
  inside/destroyed shell or collision) [V file names; I meaning]. `skyscraper_areas` marks points for tall buildings [V]; exe `CUfoChameleon::States haciendo SetWantSkyscraper por no encontrar edificios cerca` and `InitSkyscraperDomino`, `padel_skyscrapers.txt` (camera paths for a skyscraper test) [V].
- Variation: textures index by vertex colour, per-window noise texel (`g_texSamplerNoise` at `int2(vTexCoords0)*size`: ONE noise texel per window cell) controls brightness/lit state (section 3).
  Tri-planar world-space brick means no UV authoring [V]. Wall tint via `g_vDiffuse` per material.
- Street furniture and traffic: `Data\street_furniture_normal.txt` (11 models, 16 azimuth x 32 elevation views), `_1theta` (16 models, 8 azimuths, 1 elevation),
  `_antenas` (21 models, 16 az x 1 elev), `vehicle_omni.txt` (16 x 32), `people_{Walk,Run,Stop}_omni.txt` (4 models, 3 x 6 views, Walk 35 frames, Run 15 frames) [V].
  Each describes an IMPOSTOR ATLAS: a baked sheet of views of the object from a grid of angles (`phis` x `thetas`), and a one-row `*_indices.dds` (1 x N, 128-bit texels) that gives per-model
  atlas rectangles [V DDS: `street_furniture_normal.dds` 4544x4536 DXT5, `_indices` 5632x1 128-bit; `people_Walk_omni.dds` 3108x3612]. `Impostor.fsh5` draws camera-facing quads whose
  texture rectangle = `(iModel % cols) * billboardTexSize` and uses `g_arr16LRUD[iModel]` for per-model quad extents [V].
  Instances come from a GPU buffer `_Instance { float3 vPos, m_vVel, m_vRow0, m_vRow1, m_vRow2; uint iData }` = 60 bytes, with `m_vVel.x/y/z` reused as alpha-fade fields [V InstancesFilterCS.txt, D_StreetLightGlow.txt, CityVeryFar.txt].
  Cars: 25 vehicle models + `vehicle_damaged_*`, palette texture `vehicles_colors`, `Impostor_Car` for far cars; lights are glow SPRITES baked into the mesh (`mesh_modify SPRITE { material "streetlight_glow_night_small" center {-2.2 0.7 0.8} color {0.8 0.8 0.6 0.4} }` = headlights; red tail lights 2.5, 0.7, +-0.8) [V global.txt:1342-1345].
  Pedestrians: `people_0..3` rigged meshes (copy bones from `jugador_0`), `pedestrian_0` = a 1.8 m cylinder (radius 0.25) placeholder/collision [V global.txt:2123]. Exe `spawn_surprised_pedestrians`, `p_floor_vehicle` [V].
  Street furniture models are imported from a store pack ("101 Street Furniture": seats, bins, hydrants, street-lights, telephones, traffic signals, bus stop) [V file paths in global.txt].

## 3. How a city is drawn across distance (all [V] unless marked)
LOD ladder (outer to inner). Distances measured in metres from the shader formulas.
1. ORBIT (> ~800 km): no city geometry. `Planet.txt` (CLOUDS variant) reads `EarthCloudsCities` texture: `.a` = cloud cover, `.gb` = cloud normal, `.r` = city-light intensity. Night lights:
   `fNightLights = (1-vColor.a) * saturate(r + noise.a*0.5) * saturate(-dot(sun)*10) * saturate(dist/800000) * saturate(1 - dist/10000000)`, colour `(0.6*0.41, 0.6*0.34, 0.6*0.18)` (warm amber), drawn only where clouds are absent
   (`1-vColor.a`), only on the night side, fading in from 800 km and out beyond 10,000 km (comment `[city_night_bloom]`). A 274 KB map, so city lights = a low-res painted map, not per-city data.
2. FAR (<= 900 km): `CityVeryFar` - instanced quads (one per district patch, instance buffer `g_texInstances`) textured with the per-city atlas:
   `vDiffuse.a = 0.05+0.95*saturate(900000/160000 - d/160000)` = opaque to 740 km, gone at 900 km (comment `[cities_around]`). Colour per instance from `g_arr16Colors[p.iData]` (16-entry palette, blended by `g_fLerpColors`, 1 high / 0 low);
   lighting changes from HALF_LAMBERT (low) to plain N.L (high) so it matches the planet shader. Normal map .rg normal, `.b` streets, `.a` street-lamp glow; emission
   `(b + a*0.3) * noise(uv*0.1).a * g_vFloorLightVeryFar` = night streets glow, lamps 30%, modulated by a noise map; a `g_bIsBloom` variant outputs flat `g_vFloorLightVeryFar` for the bloom pass.
3. MID-FAR: `CityFar` (and `CityFarWrap` where the city "wraps"): a mesh in the shape of the districts, coloured from a CUBE MAP (`g_texSampler1`, `_City_Cube_Map.dds`, rendered at run time from `g_vCubeMapCenter`).
   Per pixel: intersect the view ray with the city's ground plane `g_vPlane` and sample the cube map in that direction => a parallax-correct impostor of the whole city as seen from its centre
   (`PlaneIntersectSegment(g_vPlane, cam, cam+10*(pos-cam))`, `texCUBE(..., normalize(mul(inters-center, matCubeMap)))`). Old/new cube map pair cross-fades (`g_fLerp`, `g_matCubeMapOld`) when it is re-rendered.
   `CityFar.vs` pushes the mesh toward the camera: `vPos += saturate(0.03 - 30/dist) * toCamera` (kills z-fighting with real geometry). Fade-out by dithered clip at distance `g_vDissolveFar.x` (dither = `GetDither(vPosProj)`), computed with `length(vPos-g_vCamPosDetails)` "to avoid fades when turning the camera".
   `CityFarWrap` adds a 2nd cube map and a "wrap plane" so the impostor can follow the curl of the deformed ("abducity"/bent) city; its `g_texSampler2` (real top texture) replaces the cube map in the fold band (`vTexCoords1.y>0`). The wrap is a gameplay effect (UFO bends the city), skip it [I].
   Exe: `city_far`, `city_far_old`, `CityFarWrap_0x%08x`, `Textures\_City_Cube_Map.dds`, `_City_Cube_Map_Reflections.dds` [V].
4. NEAR: real meshes, three material sets per type in global.txt: `windows`/`windows_med`/`windows_near`/`windows_fromMed` (effect `DS_NG_Window`), `windows_night`/`_far`/`_near` (`DS_NG_Night`),
   `bricks`/`_med`/`_near`/`_xz*` (`DS_Bricks`, `DS_Auto_XZ_Bricks`), `Road_med`. They share textures `windows` (1024^2 BC3 x18 array, 11 mips, 25 MB), `windows_NG` (same, normal+gloss),
   `sidewalk`, `Windows_ceiling`, `cracks`. The `_med`/`_near` variants differ only by `g_fDistMin`/`g_vDissolveFar` ring: `DS_NG_Window` clips with
   `min(dist - g_fDistMin, g_vDissolveFar.x - dist)` using a dither = a ring-shaped dissolve between LOD sets (cross-fade with NO extra passes; stochastic) [V]. LOWER define: same distance in a vertex clip-distance instead.
   Window detail: interior mapping. For a facade pixel on a window texel (`vTexDS.a>0`): intersect the eye ray with the floor plane `-floor(y/3)*3` or the ceiling plane and sample `sidewalk`/`Windows_ceiling`
   textures (`g_iWindowDetails`), fade by `saturate(1-0.05*dist)` (visible to ~20 m); plus a mirror reflection of the previous frame (`g_texSamplerFrameOld`, for 150 m) and a cube-map reflection (`g_texSamplerCube`). Specular fades in with `saturate(dist/500-0.5)` [V].
   NIGHT (`DS_NG_Night`): per window `vTexNoiseWindow = noise(int2(texcoord))`; lit if `noise.a > 0.6` (about 40% of windows if the noise is uniform) with emissive `0.5*reflection + (0.72, 0.55, 0.21)` (warm tungsten),
   amplitude `* noise.r * vTexDS.a`; lit rooms also show the interior floor/ceiling texture as extra emissive. [V] No per-building state: the same hash of window cell decides it, so lights are stable and free.
   `iData` (window type 0..17): the shader comment says larger iData are dirtier and were once made to get no lights (`0.6f+iData*0.04f`), now disabled [V].
   Roads: `Road.txt`: `g_texSampler1` asphalt-with-lanes strip by `(vTan.x*(lanes-0.05), along*0.125)`, lane markers `RoadLines`, crossings `RoadCrossWalk`, centre/edge `RoadLine`; darkens left-hand lanes x0.85; cracks from the deformer. Fresnel rim light `0.666*pow(saturate(N.y - dot(V,N)), 18) * sky` makes wet-looking roads at grazing angles [V].
5. PROPS: `InstancesFrustumLodCS` (compute, 64 threads) per instance: 4 side frustum planes with slack `g_vDistMinMaxNearRadius.w` (default 25 m), distance interval test, then APPENDs to `ArrayInFrustum` and, if closer than `.z`, also `ArrayNearCamera`;
   `InterlockedAdd` on indirect-draw argument buffers => the CPU never touches instances (GPU-driven instancing). `InstancesFilterCS`: groups instances by `iModel = iData % nModels`, per model slot `g_iArgs.z` max, rejects `> g_fDistFar` (default 150 m), writes indirect args per model (several LODs via `g_iArgs.y`) [V].
   So: instance cell list built by CPU once per sector (`InstancesNearCamera` per a comment), per frame GPU culls and bins by model.
   Far props (street furniture, cars, people, antennas) become `Impostor` billboards: `fAlphaDist = alpha * clamp(min((maxDist - d)*k, (d - distMin)/15), alphaMin, 1)` (15 m cross-fade in, linear fade out), a deterministic 64-entry random table gives each instance a rotation phase, dithered clip for opaque pass, `ALPHA_BLEND` variant for soft ones, and fog from the froxel/fog texture [V].
   Street-lamp glow `D_StreetLightGlow`: geometry-shader point sprite, size `d * g_fSize` (0.003 / 0.002 of distance = constant ~screen size), additive, motion-stretched by velocity (`fStretch`), reduced alpha with distance `-min(0.4, d/17000)` "so they do not burn when drawn into the cube map", fades by `g_vDissolveFar`; `vDiffuse.b==1` flags lamp (colour overridable) vs car lights [V].
   Cars carry baked glow sprites; the night city is mostly emissive windows + glow sprites + the street-glow channel of the far texture.
6. BUILDINGS BELOW: `building_inner` = a plain `DS` asphalt-textured material for the inside/cut faces (destruction slicing, `CityMaximizeAlpha`, `CityFloor`) [V name, I use].

Deformer (destruction, gameplay only; mentioned so nobody copies it for terrain): `cs_deformer_tex.txt` samples `g_texDeformer` (`CityDeformPreprocess` turns a height texture into normal.xyz + height.a from 4 neighbours, 1/512 pixel step; `CityDeformRemove` blend `max`, `CityDeformRemoveMaxZero` writes 0; `CityDeform` blend `revsubtract`) and displaces instances (`p.vPos.y += vTex.a`, rotate their basis by the normal) [V].
`cs_deformer_wrap.txt` rolls the city around a cylinder of radius `g_vWrapParams.x` ("abducity" fold) [V]. `DS_NG_Window ds()` tessellates broken buildings only (`fractional_odd`, factor 0.3*distance, max 7) [V].

## 4. Terrain integration (V from Planet, TerrainQuad, TerrainTopDown shaders)
- A "city texture" `g_matCityTexture` / `g_texSampler5`, written by `CCityNear::ComputeTexture` at 1 pixel per 16.38 m ("CityNear.ComputeTexture pinta la textura a 1 pixel cada 16.38 metros"), RGBA: `.r` height offset, `.b` sea/coast depth mask, `.a` city coverage.
  Terrain height near a city: `fHeightForSea = lerp(fHeight, (30 + 0*r - (30+50)*b)/20000, a)` (CITY_SEA_OVER 30 m, CITY_SEA_UNDER 50 m, height-map amplitude 20,000 m) in `Planet.txt` (planet shader, for sea) and `vTexCity.r = vPos.y + g_fTerrainToCity - 80*b` then `lerp(detailsHeight, vTexCity.rg, a)` in `TerrainTopDown.txt`. I.e. terrain is blended toward the city's own flat height by alpha, with a coast cut-out channel.
- Seams: vertices at `vPosOrig.y >= 100` are tagged as "adjacent to the city" in the terrain mesh `city_*_around_0/1.bin` (`TerrainAroundCity`); their y is shifted `g_fCityToTerrain - 100 + height` so the terrain ring meets the flat city without cracks (`[height_around_city]`; touching them caused holes far away) [V TerrainQuad.txt:56-58].
- Under the city: terrain is not drawn where the city covers it. `TerrainTopDown` samples the city alpha with `texGatherAlpha` at 3-texel offsets and drops land/sea rendering only when ALL alphas are 1 (`[inhibe_terrain_in_city]`, `TERRAINHM_MAX = 252/255` reserves the top values); not in the last cascades (1 px per 70 m). `TerrainUnderCity` / `city_*_under.bin` / `_hole.bin` (Tokyo, 348 KB) are meshes for the ground plane under and the hole [V names].
- Coast: `g_texSampler9`/`g_matCityCoastTexture` `.b` clips water/land: `clip(g_fUnderSea - coast.b)`. `coast yes` in params; `CCoastLines::Set`, file `_delete_me_coastline_%s.bin` [V].
- Terrain colour near a city: `g_vCityLight` adds emissive city-lit ground (`saturate(tex.a/0.1 - 0.9/0.1) * g_vCityLight.rgb`, only away from the sea) at night; rocks removed near cities and "grass without trees" nearer (`[grass_without_trees]`) [V].
- Seamless 5 km symmetric noise (`GetTiledNoise` in TerrainTopDown): the world is cut into 5000 m cells (`CITY_SECTOR_DIAMETER`); inside a cell the coordinate is folded by 180/90 degree rotations and a mirror about y=x into one eighth, then 4 octaves of table-lookup value noise (1024 entries, hashes 485789, 34893483, amplitude 20 halving) => noise that is identical across sector edges and has 8-fold symmetry, added to the city height `vTexCity.r += GetTiledNoise(...)`. [V] Cheap way to give flat city ground a little undulation that lines up with neighbouring sectors.
- Roads follow the city's own flat model (the city is modelled in a local flat frame, `Models\city_*_around_*.bin` are the terrain meshes around, with TerrainTopDown/HeightMapEarth/NoiseTileableMountains/grass/rock/sand materials) [V material names in `city_0xa216752f_around_0.bin`].

## 5. Budgets and distances found
- City area 25 x 25 km (radius 12.5 km, Dubai 35 km), 5 x 5 sectors of 5 km; city texture 2048^2 per city (1024 build for consoles); city-height texture 16.38 m/pixel (so ~1526 px for 25 km = 2048 px at 12.2 m? the stated 16.38 m * 1526 = 25 km; matches ~1536..2048 px) [V 16.38, I the size].
- Planet night lights 800 km-10,000 km; CityVeryFar 740-900 km fade; props max 150 m (`g_fDistFar` default); frustum slack 25 m; window interior detail ~20 m; reflections 150 m; impostor fade-in 15 m; specular on windows from 250 m [V].
- Data size: 13 baked city bins 527 MB raw/230 MB zipped; district files ~0.4-1.1 MB; city textures 5.6 MB x 2 each; windows array 25 MB x 2; street furniture atlas 27.5 MB (4544x4536) ; people Walk atlas 15 MB, Run 29.5 MB [V sizes].
- Vehicle mesh sizes 67-120 KB each (1-2 K vertices; vehicle_6: 1,710 vertices, 1,941 indices -> 647 triangles) [V].
- Exe: `dynamic_lod_manager`, `geometry_instance_lod`, `CGeometryInstanceLOD`, `CImpostorManager`, `MENU_OPTIONS_AUTO_LOD`, `video_auto_lod` = automatic LOD scaling to the frame rate in the menu [V strings].

## 6. Exe strings relevant to the C++ side [V; selection]
Classes: `CCity`, `CCityModel` (`Block`, `Edge`, `Node`, `Piece`, `District`, `Style`), `CCityObject` (`ComputeModel`, `Slice`, `RemoveCityPiece`), `CCitySector` (`BSector::m_arrbitDistricts`), `CCityImpostor`, `CCityExplosion`, `CCityNear`, `CCoastLines`, `CBuildingFlashes`, `CComponentBuildingCutter`, `CImpostorManager`, `CGeometryInstanceLOD`.
Functions: `CCityModel::Compute` (loads OSM xml), `InitDistricts`, `InitDistricts OSM`, `DegenerateTrisById`, `RegenerateTrisById`, `ComputeTerrainAroundCity`, `ComputeUnderCity`, `GenerateTexture iDistrict %d`, `CCityNear::Init ... va a calcular/ha calculado` (bakes a city on first use and logs seconds), `AddObbCityCenter`, `GetBuildingsNear`, `IntersectsCity`, `SetInsideCity`.
Keys: `city_landmark`, `city_params`, `city_name_param`, `city_style`, `terrain_style`, `create_style`, `m_arrStyles`, `m_bLearntBuildings`, `m_colStreetLights`, `osm_all`, `osm_center`, `osm_radius`, `osm_coast_radius`, `skyscraper_areas`, `model_height_map`, `model_unholed`, `height_min`, `height_map_amplitude`, `landmarks`, `far_from_city`.
OSM keys: `highway`, `lanes`, `footway`, `sidewalk`, `river`, `riverbank`, `coast`, `coastline`, `building`, `building:levels`, `height`, `increase-x-height`.
Source path strings show a monolithic City.cpp/City.h (>30,000 lines: errors at `City.cpp:32706`, `City.h:28315`) [V].

---------------------------------------------------------------------------------------------------------------
## REUSABLE CITY ASSETS (the project will read these from the install at run time)
Licence note: many meshes were bought/imported from third-party store packs (paths in global.txt: "101 Street Furniture", "25 - City cars models - max7 gmax 3ds",
"AC Hvac Units", "AntenasPack", "awning", "People\man_1_3ds"). Reading them at run time from the user's own copy is the stated plan; do not redistribute. [V paths]

### A. Generic, reusable (Earth or any modern-civilisation planet)
Models (name: bytes; source/material evidence). `.bin` files in Models.fcm (format in B).
- Facade/wall modules (made by the game, not in global.txt): `window_0.bin` 1,728 B (20 verts; bbox x -0.1..0, y 0..3, z 0..5 = a 5 m x 3 m bay with 0.1 m frame depth; materials `window`(effect `Cube`, glass) + `brick_0`(`DS_NG_Auto`)),
  `window_1` 1,740, `window_2` 3,305, `window_3` 3,383, `window_4` 3,608, `window_5` 2,009, `window_6` 2,889, `window_7`/`_8` 2,310, `window_9` 7,043 (105 verts, 2 sub-meshes, 10 m wide: `window` + `brick_6`) [V window_0, window_9; others I].
  These ten are the "pieces" of `CCityObject` facades [I]. The wall material the game really draws in the city is `windows*` (`DS_NG_Window`, 18-layer array) [V global.txt:54-105].
- Roof/rooftop clutter: `air_condition_0` 13,046 (225 verts, material `AirCondition`, `acon_4`), `air_condition_window_0..3` ~5.6-6.9 KB, `awning_0` 43,633, `awning_low_0` 1,617 (texture `Awning\Awning.dds`),
  `chimney_0..6` 6-124 KB, `chimney_facade_0/1` 175,759/160,697 (taken from the Empire State model!, material `metal_2`), `hvac_big_0..2` 214/509/711 KB, `hvac_small_0..2` 59/46/986 KB (`AC_Hvac_Units` textures 1K BC1 `19_Metal_H_1K` etc.),
  `antena_small_0..4`, `antena`/`antena_0..3` (0.26-2.2 MB, `AntenasPack` communications towers), `roof_fan` 2,434 (texture `roof_fan` 174,904 B), `roof_sideboard` 1,400 (also from Empire State), `spotlight_0` 16,774, `water_deposit` 43,054 (rooftop water tank),
  `bricks_heap_0` 35,917, `cabin_prop_0/1` 114/39 KB, `pot_0..3` 2.3-2.5 KB (`pot_0`: 24 verts: planter + `billboard_grass`, a 0.8 m pot), `fence_0` 6,437 (6 m of fence, `fence_metal`), `railing` 766, `pole_0` 6,030 (bollard: 5 stacked cylinders, `Vertex` material via `UNION_FAST` of `CYLINDER`s, scale 0.6) [V recipe in global.txt:955-995].
- Street furniture (store pack "101 Street Furniture", all `MESH_IMPORT COLLADA` + RECOMPUTE_NORMALS + REMOVE_REPEATED_VERTICES): `streetlight_0..8` (stlght15,16,2,3,4,5,6,13,14; some scaled 2x/6x), `streetlight2_0..5` (stlght10,11,12,7,8,9),
  `semaphore_0..4` 24-51 KB (traffic lights trfsig1-5), `hydrant_0/1` 20/18 KB, `bench_0..4` 9-20 KB (bench_0: 236 verts, 1.1 x 1.2 x 3.2 m), `dustbin_0..6` 5-13 KB (dustbin_0: 74 verts, 0.48 x 0.96 m), `dustbin_big_0..2` 9-14 KB, `telephone_0..2` 34-42 KB, `bus_stop_0` 16 KB, `newspaper_vending_machine_0/1` 5.7/5.1 KB, `mailbox` 5.9 KB,
  `furniture_0..2` 5.4/3.5/4.0 KB (chair, table, tall wardrobe: interior props, comments in Spanish "silla/mesa/armario") [V names and global.txt:3782-3848].
  Materials: `streetlight_metal` (`Constant`, grey 0.5, spec 1.5), lamp glow materials `streetlight_glow_night` (size 0.003), `_far`, `_small` (0.002; `D_StreetLightGlow`, texture `circle`) [V].
  Impostor sheets: `street_furniture_normal.dds` 4544x4536 BC3 (11 models, 16x32 views), `street_furniture_1theta.dds` 568x656 (16 models, 8 views), `street_furniture_antenas.dds` (21 models, 16 views) plus `_indices.dds` [V].
- Decals (flat QUAD meshes made by `mesh_gen QUAD`, y-up, `DA_ZBias`/`Decal` effect): `Drain.bin` 572 (0.75 x 0.6 m, `StreetFurniture\Drain.dds` 156x132 BC1), `Sewer.bin` 573 (1.66 m, `Sewer.dds` 332x332 BC3), `RoadArrow.bin` 607 (6 x 1.5 m, `RoadArrow.dds` 512^2 BC3) [V decoded], `Trash` 664, `TrashGroup` 646, `RoadCrack` 638 [V sizes], `Stone` 599 [?].
- Roads/ground textures (Textures.fcm DDS): `Asphalt` 2048^2 BC1 12 mips; `AsphaltLanes` 2048^2 BC3 (alpha = specular); `RoadLines` 256^2 BC3; `RoadLine` 64x64 BC3 (procedurally defined: constant 0.9 grey + noise); `RoadCrossWalk` 256^2; `RoadIntersection` 256^2; `RoadArrow` 512^2;
  `road_cracks` 1024^2 BC3; `cracks` 1024^2 BC3; `sidewalk` 2048^2 BC1; `sidewalk_border` 256x36 BC3; `RoofTop` 1024^2 BC1; `brick_0` 1024^2 BC1 (+`brick_0_ng` 2x2 flat); `bricks` 1024^2 uncompressed 32-bit BGRA 11 mips (5.6 MB);
  `Windows_ceiling` 512^2 BC1; **`windows`** 1024^2 x 18 array, BC3, 11 mips (DX10 header, DXGI 77 = BC3_UNORM, arraySize 18; 25,166,452 B) [V]; **`windows_NG`** same format (normal in .agb, gloss in .r, as the shader reads `vTexNG.agb*2-1`, `fGloss*vTexNG.r`) [V shader];
  `noise.dds` 256^2 32-bit (per-window noise), `noise_signed` 256^2 DX10 DXGI 31 (R8G8B8A8_SNORM? 31 = R8G8B8A8_SNORM) [V header; format reading I], `circle` 256^2 32-bit (glow), `AC_Hvac_Units/*_1K` BC1.
- Vehicles (25): `vehicle_0..24.bin` 67-120 KB, `vehicle_damaged_0..24.bin` 42-285 KB. Material per vehicle: effect `Vehicle`, textures `Vehicles\<name>` (512^2 BC3, 22+ files, e.g. `LexusLS.dds` 349,680 B) + `vehicles_colors` (paint palette). Order [V global.txt:1307-1331]: 0 Blazer, 1 Chevrolet Express, 2 coupe B, 3 coupe Muscle, 4 police (LA/NY), 5 taxi (LA/NY), 6 Lexus LS, 7 Mercedes CLS, 8 Mercedes E, 9 Mercedes S, 10 Mercedes CL, 11 Mercedes M, 12 Mitsubishi X, 13 police, 14 Pontiac G6, 15 roadster, 16 Range Rover Sport, 17 sedan, 18 sedan3, 19 sedan 80, 20 SUV01 X5, 21 SUV02 90, 22 Toyota Prius, 23 van02, 24 van. Rotated -90 deg (`ROTATE_MODEL yaw -1.5707963`), nose along +X (bbox x -2.7..2.6, y 0..1.8, z +-1.1 for vehicle_6). Head/tail light sprites baked in as `SPRITE` meshes. `Impostor_Car` + `vehicle_omni.txt` = far version. Damaged variants use `*_Damaged` texture folders (BMW_6, Cadillac_XLR, Dodge_Challenger, Ford_Crown, Subaru...) so they are a different (not identical) set of car models [V folder names].
- People: `people_0..3.bin` (152-162 KB each; textures `man_1_3ds`..`man_5_3ds`, `woman_1..5_3ds`, `man_N_co.dds` 1024^2 BC1), skeleton copied from `jugador_0`; animated frames only as impostor sheets `people_Walk_omni.dds` (35 frames x 4 models x 3x6 views, 3108x3612 BC3), `people_Run_omni.dds`, `people_Stop_omni.dds`; `pedestrian_0` cylinder. [V]
- Bridges/tunnels/infra: `bridge.bin` 1.19 MB, `tunnel_segment` 1,966, `tunnel_cap` 3,020, `windmill` 56 KB, `helicopter_0` 130 KB (+`helicopter_rotor_0`), `stadium` 1.04 MB (`Estadio\texture_estadio` 1024^2 BC1, grass, seats) with `stadium_crowd.bin`, `sign`/`sign_piece*`/`micro_sign_piece*` (road signs, 3 KB each; coloured variants and `_transparent`), `sign_rock`. [V names]
- Near/far terrain wrappers (city-specific bakes): `city_0x<hash>_around_0/1.bin` 94 KB-1 MB (terrain ring around each city), `_hole`, `_under` (Tokyo) [V names].

### B. Real-world landmarks (only suit Earth; each has an `_inner` companion)
`empire_state` 2.25 MB (+`_inner` 1.57 MB; textures `Empire_State\*` 2048^2 BC3), `burj_khalifa` 1.42 MB (+`_inner` 1.43 MB, `texture_Burj_Khalifa`), `burj_al_arab` 0.93 MB (+inner; `Cristales` glass materials use `DS_NG_Window`),
`four_towers` (Madrid CTBA; `four_towers_A..D` + `_inner`s, 21 textures), `coliseo` 8.4 MB, `cristo_redentor` 0.44 MB, `eiffel` 0.83 MB, `westminster_palace` 1.4 MB, `white_house` 0.47 MB (+inner 2.0 MB), `taj_mahal` 3.1 MB (atlas 4096^2 BC3?, 16.8 MB file), `tokyo_tower` 2.2 MB, `sydney_opera` 1.4 MB, `giza` 0.56 MB, `stonehenge` 1.27 MB, `Anubis_Totem.fom`. Landmark materials re-use the generic window shaders (`DS_NG_Window` with `g_fTexCoordsScale 1.0`, vertex_color `{0 0 0 0}` "required by DS_NG_Window" => window type 0) [V global.txt:3909-4237].
Authors of several: file path `grafistas\VictorTDiaz\Empire State` (hired artist) [V]. Skip for generic planets.

### C. Game-specific (skip)
`abducity*` (alien city-eater, 28 files), UFOs (`ufo*`), bombs (`bomb`, `gravity_bomb_*`, `drill_bomb`, `incandescent_bomb`), `disintegrator*`, `quark_box*`, `whirl_*`, `wrecking_ball*`, `hand*`, `foot_*`, `xp_up_*`, `sierpinski_pyramid*`, `runes*`, `kaka`, `icoshield*`, `fish_*`, `seagull`, `northern_light`, `galaxy_*`, `Planet*`, `asteroid_field`, `erta_ale`, `debris*`, `vr_blind*`, `enemy_test`, `pirindolo_*`, `Walle`, `Indy.fom`, `Jugador_0.fom` (player rig).

### D. Which look stock/store vs made for the game
- Stock/store (third party, per the import paths in global.txt): all vehicles (`25 - City cars models - max7 gmax 3ds`, named real car models), street furniture (`101 Street Furniture`), HVAC/chimney/cabin props (`AC Hvac Units`), antennas (`AntenasPack`), awning, `people_*` (`People\man_1_3ds`...), plus textures `AC_Hvac_Units`, `Vehicles\*`, `man_N_3ds`. [V]
- Made by the game's authors: facade modules `window_N`, bollard/fence/pot/etc. (code-built `UNION_FAST` of BOX/CYLINDER in global.txt), road/sidewalk/asphalt textures (`RoadLine` is described procedurally in `Textures\RoadLine.txt`), `city_0x*.dds` atlases, impostor sheets, the 10 landmark models (some by a hired grafista), cube map and noise textures. [I from paths: they have `MR\models\...` author-owned paths but the Textures folders (`Taj_Mahal\Marmol Blanco`, Spanish names) suggest in-house/commissioned art].

### E. FORMATS (what a loader needs)
**DDS** [V headers read from 33 files]: standard `DDS ` + 124-byte header (+20-byte DX10 header when fourCC = `DX10`).
 - `windows.dds`, `windows_NG.dds`: 1024x1024, 11 mips, DX10 header, DXGI 77 (BC3_UNORM), array size 18 (data = for each array slice, its mip chain: DX10 layout) [V].
 - Most textures: legacy fourCC `DXT1` (BC1: Asphalt, sidewalk, RoofTop, brick_0, Windows_ceiling, AC_Hvac 1K, Drain) or `DXT5` (BC3: AsphaltLanes, RoadLines, RoadLine, RoadCrossWalk, RoadIntersection, RoadArrow, cracks, road_cracks, city_*, vehicles, Empire State, impostor sheets).
 - Uncompressed: `bricks.dds` (pf flags 65, 32 bpp, 11 mips), `noise.dds`, `circle.dds` (32 bpp, no mips); `brick_0_ng.dds` is a 2x2 32 bpp placeholder (flat normal) [V].
 - `*_indices.dds`: fourCC value 0x74 = D3DFMT_A32B32G32R32F (128-bit float RGBA), width N, height 1, no mips: one float4 per model (the atlas rectangle) [V fourCC 116, bpp 128].
 - `noise_signed.dds`: DX10, DXGI 31 [V].
 Everything loads in Bevy via `image` crate DDS support or a 50-line custom BC1/BC3 upload; the array just needs `Texture2dArray`.
**Models `.bin`** (header little-endian; all offsets verified on RoadArrow, window_0/9, pole_0, pot_0, bench_0, dustbin_0, chimney_0, fence_0, air_condition_0, vehicle_6) [V unless marked]:
 - 0x00 u32 = 205 (magic/version); 0x04 u32 = 1 (second version, other values: 0x80000001 Planet/IncandescentSphere, 0x20000001 white_house_inner = flags in the high bits); 0x08 u8 flag (16 for 508 files, 0 for 21, 80 for 22 [skinned/bones? `jugador_0`, `runes_runes` use 80], 17 for 3 [roof_fan]); meaning not decoded [?].
 - 0x09 u32 N = vertex count, then N x (f32 x,y,z) positions (12 B each, starts at 0x0D). Axes: Y up, metres, model origin at base (bbox y from 0), cars nose +X [V bboxes].
 - then 4 counted streams, each `u32 count` + `count x size` bytes, in this order [V on 10 files]:
   1) normals: 3 bytes/vertex, signed-normalised bytes (e.g. `00 7e 00` = (0, 126/127, 0) for an up normal) [V by value; snorm reading I],
   2) tangents: 3 bytes/vertex snorm (e.g. `7e 00 00` = +X) [V by value],
   3) vertex colours: 4 bytes/vertex (RGBA8; props with material `Vertex` use it, e.g. pole_0 `3d 4c 66 ff`; the window shader's `iData` = colour .b byte),
   4) UV0: 4 bytes/vertex = two IEEE half floats (u,v) (RoadArrow: 0x3c00 = 1.0, 0x33ae = 0.2257) [V values; half reading I].
   A stream may have fewer entries than N or zero (then that attribute is missing, e.g. `window_*` have no UVs, no colours: they are tri-planar `DS_NG_Auto`; in `pot_0` tangents/UV exist for only the first 8 of 24 vertices = the first sub-mesh's) [V counts]. Therefore: a loader must read `count` for each stream and map sub-mesh vertex ranges.
 - then 3 more u32 counts (always 0 in all files tested; presumably UV1 / bone indices / bone weights; 22 skinned files "flag 80" would use them) [V zeros, ? meaning].
 - then u32 index count `ni` + `ni x u32` indices (32-bit, triangle list, winding as stored; RoadArrow: 0,2,1, 1,2,3 for a quad) [V; every index < N checked on 11 models].
 - then u32 sub-mesh count `S` and per sub-mesh: 4 x u32 `(firstIndex, firstVertex, indexCount, vertexCount)` then 1 byte `01` + u32 blob size + material blob [V: vehicle_6 -> sub 0 = (0,0,1941,1706), sub 1 = (1941,1706,0,4) = the 4 light-sprite quads; window_9 -> (0,0,303,65),(303,65,99,40); counts sum to N].
   Indices in a sub-mesh are relative to its first vertex? In window_9 sub 1 starts at index 303 with firstVertex 65 and max index overall 104 = absolute (0..104) => indices are ABSOLUTE vertex ids [I from max index = N-1; not tested with base-vertex offset].
 - material blob: `0f 00 00 00 00 00 00 | len u8 | 4-byte hash 97 cb 08 de | name (len bytes incl. NUL)` e.g. `RoadArrow`, `window`, `brick_6`, `Vertex`, `Vehicle`/`vehicle_6`, `streetlight_glow_night_small`, followed by effect name records (`DA_ZBias`, `Cube`, `DS_NG_Auto`, `Vehicle`, `D_StreetLightGlow`) and constant records (`04 00 2a 6f 25 b0 00` then floats such as `00 00 80 3f` = 1.0 = g_vDiffuse etc.), texture names (`brick_0`, `brick_0_ng`, `vehicles_colors`, `circle`). The property hashes are not decoded; a loader can ignore the blob and instead look the material NAME up in global.txt (MATERIAL entries give effect + textures) [V strings present; ignoring I].
 - Trailing data after the sub-mesh list (2.9 KB in pole_0, 4.9 KB in air_condition_0, 14.4 KB in vehicle_6): float data that begins with the bounding values (pole_0: -0.15, 0, -0.15, 0.15, 0.84...) = most likely a collision hull / bounding volume / physics proxy; not decoded and not needed for rendering [?].
 - LODs: no LOD chain inside the files tested (one vertex set); LODs are separate files/materials (`bricks_med`, `windows_near`, impostor sheets) [V by structure; I that no hidden LOD].
 - Big files (landmarks, `city_*_around_*`) use the same header (`city_0xa216752f_around_0` strings: `TerrainTopDown_height`, `HeightMapEarth`, `TerrainAroundCity_defdd793e1a`) but I did not walk them [?].
**`.fom`** (4 files: `Anubis_Totem` 43.7 KB, `Flechacursor` 440 B, `Indy` 184.7 KB, `Jugador_0` 211.5 KB): magic `FM` 0x0007 (version 7) 0x0001, then a 32-byte name (`mod_flecha_cursor`), a second name (`FLECHACURSOR`), then sections with hashes; a legacy skeletal/model format from the engine's earlier days (player rig, a cursor arrow, an Indiana Jones head?). Not city assets; ignore [V magic/names; rest ?].
**Text scene language** (`global.txt`, `cities_params.txt`): brace-delimited `key value` / `key { ... }` / `array<type> key { ... }` / `//` comments, easy to parse: `MATERIAL { name effect "X" { params } textures {"a" "b"} flags8 ... }`, `MESH_IMPORT { name mesh_gen model <KIND> { ... mesh_modify <OP> {...} } }` (KINDs seen: COLLADA, BOX, CYLINDER, UNION, UNION_FAST, DIFF_EXPLODE, QUAD, FOM; mods: ROTATE_MODEL, SCALE_MODEL, RECOMPUTE_NORMALS, REMOVE_REPEATED_VERTICES, FLIP_TRIS, SPRITE, REMOVE_TRIS, CENTER_MODEL, BEND_ARM, COPY_BONES) [V]. The shipped Models `.bin` are the BAKED output of these `MESH_IMPORT`s (the DAE sources are not shipped) [I: names match 1:1].

---------------------------------------------------------------------------------------------------------------
## Methods worth adopting (for a Rust/Bevy planet engine; none needs MR's code)
1. Night side = painted light map by distance ramp (orbit): one low-res RGBA map (city light in R, clouds in A) times `sun-away * saturate(d/800 km) * (1 - d/10,000 km)` times `1 - cloud`, warm colour (0.25, 0.20, 0.11). Cost: one texture sample in the planet shader. Needs: a population/light map (we can bake from cities.csv populations; MR's cities.csv has 17,300 towns).
2. District atlas quads for the mid-far view (to ~900 km): one top-down texture per city (2048^2 atlas of district patches: roads grey, blocks green; normal map with street-glow in A and street mask in B), drawn as one instanced quad per district with a 16-colour palette and night emission `(streetMask + 0.3*lampGlow) * noise`. Cost: 1 draw per city. Needs: baking tool (rasterise road graph + blocks top-down) at city creation; or runtime generation on GPU into the atlas.
3. Cube-map impostor of a whole city projected on the ground plane for 1-50 km: render the city once from its centre into a cube map (low frequency refresh, old/new cross-fade `g_fLerp`), then per pixel intersect view ray with the ground plane and sample the cube in that direction (parallax-correct, no geometry). Cost: ~1 mesh + 1 cube sample. Needs: render-to-cubemap pass and a ground-plane equation; refresh when camera moves far or time-of-day changes.
4. Dithered ring LOD cross-fade: give each LOD material `distMin` and `dissolveFar` and `clip(min(d - distMin, far - d)*k - dither)`: LOD sets overlap with stochastic blending, no alpha, no extra pass. Needs: blue-noise/Bayer dither, LOD material variants (cheap). Compute distance with `length(pos - camPosDetails)`, not view depth, to avoid fades while rotating.
5. Window shading from a texture array of 18 window types chosen by a vertex-colour byte and one noise texel per window cell: lit if noise.a > 0.6 (warm 0.72, 0.55, 0.21), brightness by noise.r; interior mapping with floor/ceiling planes every 3 m for the first ~20 m and a cube-map reflection; the ray-plane intersection is 10 instructions. Needs: facade UVs in window-cell units; a hash/noise texture; no per-window data in memory.
6. Facade module kit: buildings as wall bays 5 or 10 m wide x 3 m high, stacked per storey (from OSM `building:levels`/`height`), tri-planar brick on walls, roof clutter prefabs (HVAC, chimney, tank, antenna) scattered on roofs. Needs: footprints; storeys from height/3. For generated (non-OSM) cities, density/height noise per block.
7. GPU-driven prop instancing: instance struct 60 B (pos, 3x3 basis, data), compute pass frustum+distance cull -> append buffers + indirect args, per-model binning (`iData % nModels`), max distance 150 m, plus camera-facing impostors with 16 azimuth x 32 elevation view atlases for far props and cars; instance cells built per 5 km sector. Needs: wgpu indirect draws (Bevy supports `draw_indirect` with custom pipelines) and an atlas baker.
8. Street-lamp glow as geometry-shader-style point sprites (in wgpu: instanced quads): size = distance * 0.003 (constant screen size), additive, alpha reduced `-min(0.4, d/17000)`; car lights are baked sprite quads in the car mesh. Cheap night street life with no real lights.
9. Terrain blend to a flat city height with an RGBA "city texture" (height offset, sea mask, city alpha) at ~16 m/texel: lerp(terrain, cityHeight, alpha); skip drawing terrain where all 4+ neighbour alphas = 1; mark ring vertices; 5 km seamless symmetric tiled noise (fold into one octant) to keep the flat ground from looking dead and to match across sectors.
10. Road strip mesh with lane count and crosswalk count in vertex attributes and the lane lines chosen in the pixel shader (`lanes`, `crosswalks` packed in tangent bytes), crossings in the first/last 6 m of every edge. Needs: a road graph (OSM or procedural); fine ground-texture work in shader instead of per-lane meshes.
11. Procedural fallback: MR's own non-OSM towns use a 10-style generator; for a whole-planet port we have no OSM, so use road graph + block subdivision + population/height field from a land-use texture and `cities.csv` populations; pick style from latitude/country.

## Unknowns
- Interior of `city_0x*.bin` (buildings, road graph, footprints): only the first ~60 floats seen. Needs a decoder pass with full extraction (not done: instructed to read only headers).
- District records beyond the sector table; precise district-polygon grouping.
- Flag byte values (0/16/17/80), the 3 trailing zero counts, trailing collision-like data, material-blob property hashes in `.bin` models; whether indices in later sub-meshes are absolute or base-vertex relative.
- Whether procedural (no-OSM) cities really use a block generator (only names and 10 "style" strings seen; no code read, no sample output).
- `density` semantics; `skyscraper_areas` units/coordinates (looks like metres east/north of city centre).
- `city_size.bin` meaning; `morpher.bin`; `clouds_EarthCloudsCities.bin` internal layout (Planet shader reads it as a texture, probably DDS-like or raw).
- Exact numbers: how many buildings per city, vertex/triangle budgets per LOD, memory; not in the data files read; no explicit budget values besides the distances above.
- Whether the `.bin` model sizes in vehicles include LODs: tested only vehicle_6 (1 LOD).
