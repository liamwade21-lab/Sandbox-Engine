# Vegetation and civilizations: plan

Made on 2026-11-04 from the user's brief and interview. Another session builds it step by step
(the Steps table below), in order, each step measured and checked in pictures before the next.

The goal is believable, detailed planets at a low cost:
- grass, bushes and forests on every planet whose life and climate allow them, from orbit down
  to the ground;
- a new life class, **multicellular intelligent life**, whose rare planets carry civilizations at
  one of four levels (primitive, modern, futuristic, interstellar), Earth among them as level 2.

The user named Megaton Rainfall (MR) as the technical reference. What it really does was read
from its files (its shaders are readable source and its data is plain text). See
`docs/megaton-rainfall-vegetation.md` and `docs/megaton-rainfall-cities.md`: every claim there is
tagged verified or inferred. Its files are unpacked in `re/megaton_rainfall/extract` (out of git),
and the scripts that read its formats are in `re/megaton_rainfall/tools`.

## Decisions (the user, 2026-11-04)

| Question | Decision |
|---|---|
| Assets | Reuse MR's own assets where they fit: its 16 trees, grass, building facade kit, window textures, roads, roof clutter, street furniture and cars. They are read at run time from the user's install (`C:\Users\liamw\Downloads\Megaton Rainfall`), never copied into the project, as with SE's files. |
| How rare | Rare but findable: about **1 in 20** planets that have multicellular land life carry a civilization. |
| Level mix | Science-shaped but playable: about **45% primitive, 15% modern, 20% futuristic, 20% interstellar**. |
| Which worlds | Only generated planets, plus **Earth (level 2)**. Catalog exoplanets keep their data. |
| Which life | Only **organic life on planets with dry land** (and enough oxygen for level 2 and up). No exotic or ocean-only civilizations. |
| Plants for other worlds | A **tree generator**: trees grown from rules, different on every planet, dressed in MR's bark and leaf textures and tinted to the planet's colours. Earth also uses MR's 16 trees. |
| Level 1 and 3/4 buildings | **Built from shapes in code, with textures made by the `codex-image` skill**, plus a few hero models from the **Blender tool's generator** (it may use the user's Blender-service credits: keep them to the few that matter). |
| Earth's cities | **Procedural**, placed and sized by SE's own Earth night-lights map. No MR landmarks and no MR OpenStreetMap cities. |
| Budget | About **3 ms** for vegetation and cities together, at Ultra 3440x1440 near the ground (RTX 5080). High is about half that. |
| Level 4 | **Orbital stations, colonized moons and planets, space traffic.** No orbital ring. |
| Street life | **Lights and traffic**: lit windows, street lamps, and cars driving the roads with their headlights. No pedestrians. |
| First step | **Vegetation first.** It builds the GPU placement system the cities reuse. Then the life class, then Earth's cities, then levels 1, 3 and 4. |

## What Megaton Rainfall does (verified in its files)

- **No vegetation or city geometry from far away.** From space its planet is a coloured sphere.
  Near the ground a camera-centred terrain takes over, the two blended by complementary dither
  clips.
- **Near-ground data.** A top-down pass renders four nested "cascade" textures around the camera:
  height, a grass mask, ground-layer weights, baked sun light and flags. The ground, the grass and
  the tree placer all read the same textures, so they agree.
- **Grass.** A fixed field of 50,000 random points over 150 m, wrapped round the camera
  (`fmod`), so it needs no streaming and keeps no state. Each point is expanded into a
  camera-facing quad about 0.66 m across. Points are killed where the grass mask is under 0.1. The
  blades shrink to nothing between 140 and 150 m: no alpha blending, no sorting. Their colour is a
  constant tint times the ground's baked light.
- **Trees.** A compute pass wraps a fixed template of tree positions round the camera, lifts them
  onto the ground, tests the masks and the view frustum, and appends them to lists. It adds to
  indirect-draw arguments atomically, so the CPU never touches an instance.
  - Up close the tree is a mesh (about 1,000 triangles); farther out it is an impostor quad from
    an atlas baked at load. The two cross-fade by screen-door dithering over 10-15 m.
  - Leaves shade as a ball: the normals come from a sphere normal map around the crown. They wobble
    with a 3D noise, and the sun shines through them.
  - Where grass grows: no grass on rock steeper than about 14 degrees or within 10 m (height) of the
    shore, and patches come from a banded noise. Trees need a stronger mask than grass, which gives
    "grass without trees" areas.
- **Cities.** Its famous cities are OpenStreetMap data baked offline (13 cities, 527 MB). The code
  that builds its other towns is in the exe (not read). So MR gives the drawing methods, not a
  generator, and the port needs its own (below). The draw chain by distance:
  - night lights painted into a low-resolution planet map (from 800 km);
  - textured district patches on the ground, a top-down picture of streets and blocks with
    street-lamp glow (to 900 km);
  - a cube-map impostor of the whole city on the ground plane;
  - real meshes in LOD rings cross-faded by dither;
  - GPU-culled props that become multi-angle impostors beyond about 150 m.
- **Buildings.** Facades are made of 5 or 10 m wide, 3 m high wall modules. An 18-layer window
  texture array is indexed by a vertex-colour byte.
  - At night a window is lit when its own noise texel is over 0.6, in warm tungsten colour, so no
    per-building state is stored.
  - Interior mapping (floor and ceiling planes every 3 m) shows rooms within about 20 m.
- **Roads.** Strips carry their lane and crosswalk counts in vertex attributes; the pixel shader
  draws the markings.
- **Terrain under cities.** A 16 m-per-texel "city texture" blends the terrain towards the city's
  height, and the ground is not drawn where the city covers it.
- **Formats.** All of these were decoded and checked on the files: plain ZIP archives; DDS
  (BC1/BC2/BC3, and a DX10 texture array); the model `.bin` (magic 205, counted vertex streams,
  u32 indices, a sub-mesh table and the material name); and the `global.txt` material language.

## What the port already has to build on

- **Life.**
  - `se_core/src/procgen/se_life.rs` is SE's life generator. It already draws a hidden third stage,
    "complex life" (organic `N(4, 0.5)` Gyr; Earth's Cambrian is at 4 Gyr), next to unicellular and
    multicellular.
  - `procgen/mod.rs` is the port's older generator, which also writes `Life` blocks.
  - `body_params.rs` `LifeParams` reads them, `hud.rs` shows them, and `star_search.rs` filters by
    them.
- **Where plants are, per pixel.**
  - `planet.wesl` works out the steppe, grass and forest weights (slots 9-11) from climate, height,
    slope and moisture: `se_biome_weights` for SE-generated worlds, `biome_weights` for the rest.
  - `surface_tex.rs` picks the organic or exotic, multicellular or unicellular palettes and soils.
  - From orbit, vegetation therefore already shows as colour.
- **Earth's maps.** SE's Earth colour tiles, water mask and night-lights tiles (`TerrainLayers`
  `lights`, drawn by `planet.wesl` `city_lights_on`).
- **Terrain.**
  - Quadtree nodes of 33x33 vertices with their heights kept per node (`NodeBuild::grid_heights`).
  - Generated heights come from the GPU (`terrain_gen.rs`, `tg_height.wesl`); catalog bodies use
    SE's height tiles.
  - Nodes are placed relative to the camera (`update_node_transforms`), which keeps them precise
    near the ground.
  - Lighting comes from storage-buffer light pages (`PatchLight`: several stars, eclipses).
  - The ground's haze and cloud shadows (`cloud_shadow`, bindings 27-33) are already there for the
    new objects to match.
- **Models.** `.sm` models (`models.rs`, `body_models.rs`) and the ship shader `HullMaterial`
  (`ship.wesl`), lit by the body's stars: the base for the stations.
- **Tools.** The speed logger (`SE_PERF`, `perf NAME`), the session tests (`session_rs.sh`),
  pipeline prewarming (`pipeline_watch.rs` `WARM_ORDER`), and the weather's wind field (for
  swaying plants).
- **Indirect draws.** Bevy 0.20's `TrackedRenderPass` has `draw_indexed_indirect` and
  `multi_draw_indexed_indirect(_count)`: GPU-filled draws work from a custom draw function.

## The design

### 0. Foundations (shared by vegetation and cities)

**MR asset reader** (`mr_assets.rs`).
- A setting holds the MR install folder, next to SE's (default
  `C:\Users\liamw\Downloads\Megaton Rainfall`).
- The `.fcm` archives (plain ZIP) are opened read-only. It decodes:
  - DDS: legacy BC1/BC2/BC3 and the DX10 array header, which `windows` and `windows_NG` use (18
    layers);
  - the model `.bin`;
  - `global.txt` `MATERIAL` entries, from a material name to its effect and textures.
- Colour textures are sRGB; normal maps are linear (BC2/BC3 RGB, decoded `*2-1`).
- If the folder is missing, everything still works: generated trees and code-built buildings
  only, no cars, and a line in se.log. Nothing from MR is copied into the project or the
  repository.

**One "what is here" function** (`surface_class.wesl`, new; the cloud cover generators set the
pattern).
- `planet.wesl`'s biome weights (`biome_weights`, `se_biome_weights`, their moisture and climate
  noise) move into it unchanged. Check the ground stays pixel-identical (a before/after screenshot
  diff, as when `gas_giant.wesl` was split out).
- It gains vegetation densities per kind: grass, shrubs, broadleaf, conifer, and the generator's
  species bands. Later it also gains the settlement density.
- The ground, the placement passes and the orbit view all call it, so a forest never stands where
  the ground is drawn as desert.
- Earth and other textured bodies get their own version, from their maps:
  - a vegetation index from the colour tile (green and dark vegetation against bright sand, rock
    and ice);
  - the water mask;
  - slope and height;
  - latitude and height for the species (conifers north and high, palms in the tropics).

**GPU scatter system** (`scatter.rs` + `scatter.wesl`): MR's methods 3-5 on our quadtree, built
once and used by grass, trees, buildings, props and traffic.
- *Cells.* Objects belong to fixed cells of the cube-sphere quadtree (trees: the nodes split until
  their real edge is at most 800 m, so 400-800 m everywhere; buildings by city sector). (Step 2: one
  fixed level gave cells nine times apart in area; see the build log.) A cell's objects come from a hash of its
  key, so the same tree always stands in the same place.
  - Positions are stored in the cell's 2D frame. Every frame the cull pass lifts them onto the
    ground drawn there: the height of the resident terrain node that covers them, read from a GPU
    height atlas of the resident nodes' grids.
  - Trees therefore sit exactly on whatever level of ground is drawn, and re-sit when a node
    splits (MR re-lifts every frame from its cascades for the same reason).
  - Instances are stored relative to their cell's centre (f32), never in planet coordinates.
- *Pooled GPU memory.* One big instance buffer with fixed-size slots, one big indirect-args
  buffer, and a height atlas with fixed pages, all with hard limits. No per-cell buffers or bind
  groups: wgpu's per-frame cost grows with the GPU objects ever alive at once (the 2026-11-04
  session slowdown).
- *Placement pass* (compute), when a cell comes into range. One thread per candidate:
  - the hash gives its position, species and size;
  - `surface_class` gives the density at that point, and the candidate is kept where its hash is
    under the density;
  - the rules below reject it on slope, shore, snow, roads and buildings;
  - kept candidates are written to the cell's slot.
- *Cull and LOD pass* (compute), every frame, over the resident slots:
  - lift each instance onto the ground and test it against the view frustum (with a margin);
  - its distance picks the LOD (mesh, impostor, or gone) with a dither band;
  - instances are appended per (model, LOD) up to a per-model cap, and the indirect args are
    bumped. The cap is the performance guarantee.
  - No CPU readback.
- *Drawing.* Custom phase items in Bevy's opaque and alpha-mask phases, so they land in the right
  target, viewport (render_scale's solids corner) and MSAA, and before the volumetric clouds read
  the depth buffer. One indirect draw per (model, LOD).
  - Alpha-tested with dithered LOD cross-fades (`clip(fade - dither)`): no blending, no sorting.
  - Distances are measured from the camera position, not view depth, so turning never fades
    anything (MR's note).
- *Impostors.* Each model is baked at load from 8-16 azimuths (and 2-4 elevations for trees seen
  from above) into an atlas, with per-model quad extents. A far tree is one quad with its 4
  vertices made from `vertex_index`.
- *Lighting and haze.* The ground's lighting, haze and cloud-shadow code moves into a shared
  module used by `planet.wesl` and the new shaders. A tree, a building and the ground under them
  must take the same stars, planet shine, eclipses, cloud shadows, ambient light and atmosphere,
  or forests and towns show as wrongly lit patches from altitude.
- New pipelines are queued at `RenderStartup` (the loading screen waits for them); `WARM_ORDER` is
  only for the ground's Material pipelines (step 2).

### 1. Vegetation

**Which planets.**
- Land plants grow only where the body's `Life` is multicellular with a terrestrial biome
  (`Marine/Terrestrial` or `Terrestrial`).
- Unicellular life keeps today's ground colours only (its "U" palettes): no 3D plants.
- Ocean-only life has no land plants.
- Exotic life gets the same system with the exotic palettes and the generator's exotic rules.
- No air or no liquid water at the surface means no plants, whatever the catalog says.

**Where on a planet (per point, in `surface_class`).**
- **Temperature.** The ground's own band (today 250-330 K with soft edges), lowered by height
  through the climate's lapse.
- **Water.** The moisture term: forest needs it wet, grass needs some, steppe tolerates dry.
- **Slope.** MR's rule: no plants on rock steeper than about 14 degrees; trees thin out before
  grass does.
- **Shore.** No plants within a few metres of height above sea level (beaches).
- **Snow, ice and lava.** None.
- **Cities and fields.** None, except parks: grass without trees.
- **Patchiness.** A banded noise breaks meadows and forest edges into patches. A "trees need more
  than grass" threshold makes clearings with grass.

**Species.**
- **Earth.** MR's 16 trees (broadleaf temperate; the pack's LOD1 meshes, 750-1,360 triangles,
  12-30 m tall), plus the generator's conifers, palms and shrubs for the climates the pack lacks.
  Species by climate and latitude: boreal conifers, temperate broadleaf, tropical palms and
  broadleaf, savanna shrubs and lone trees, desert edges with sparse shrubs.
- **Generated planets.** A flora of 6-10 species from the tree generator, seeded by the planet,
  each with a climate band.
  - Gravity sets height and girth (taller and thinner when low, squat when high).
  - The atmosphere's density sets the crown's spread.
  - The star's colour and SE's biome palette set the leaf colour: purple, red and dark leaves come
    from the palettes the ground already uses.
  - Other worlds therefore look related to their ground colours, not like Earth's forests.
- **Grass.** MR's 2x2 grass-tuft atlas, tinted by the biome's grass colour.

**Tree generator** (`trees.rs`).
- A species is a recipe: trunk height and taper, branch levels, angles, count and droop, and crown
  shape (cone, column, sphere, umbrella, palm fan, shrub).
- The recipe is grown into a mesh: tapered tubes for trunk and branches, leaf cards (MR's leaf
  textures) at the branch tips.
- It makes a near mesh (about 1,000-2,000 triangles), a simplified one, and the impostor bake.
- Bark and leaf textures come from MR's 9 bark and 8 leaf sets. They are tinted so that the
  species' mean leaf colour, measured from the texture at load, equals the biome colour it stands
  in.
- Generated once per planet on a worker thread, cached and capped like the other caches.

**From orbit to the ground** (Ultra distances; tuned to the budget; High about 60% of them).

| Camera distance to the plants | What shows | Notes |
|---|---|---|
| Beyond ~40 km | The biome colours the ground already has | No new cost from orbit. |
| 2-40 km | **Canopy shading in the ground shader**: where forest density is high, crown-sized blobs with self-shadow and a matching normal, in the trees' mean colour | New, cheap: MR has none, and from 1-20 km up a forest must look like trees, not paint. Fades out as the impostors take over. |
| 0.25-3 km | **Impostors**, thinned beyond 1 km (a hash keeps 1 in 4, scaled up to keep the cover) | One quad per tree; dithered hand-over to the canopy. |
| 0-250 m | **Tree meshes** (near mesh to 100 m, simplified beyond) | Dither cross-fades of 10-15 m between rungs. |
| 0-100 m | **Grass**: MR's method, a fixed set of points over the finest nodes, blades shrinking to nothing at the edge | Stateless; tinted by the ground's grass colour; lit by the ground's light. |

**Matching and motion.**
- The canopy's colour, the impostors' and the meshes' leaves all take the same mean colour, which
  is the forest biome colour. Then a flight from orbit to the ground never changes the forest's
  colour, only its detail.
- Ground under trees darkens with the forest density (a cheap stand-in for shadows; MR had none
  either).
- Leaves sway with the weather's wind (`weather_sim`), grass with a per-blade phase.
- Snow on plants and seasons (autumn colours, bare winters) come later, in step 11.

### 2. Multicellular intelligent life

**Data.**
- A new life type, `Type "Intelligent"` (multicellular, intelligent), and a new key
  `Civilization 1..4`, in the body's `Life` block.
- Every place that checks for multicellular life treats Intelligent as multicellular:
  `surface_tex.rs` `body_info` (today `contains("multi")`), `star_search.rs`, `se_life.rs` and
  `procgen` writers, and the catalog export.
- Catalog files may set it themselves (an add-on planet with `Type "Intelligent" Civilization 3`
  is honoured).
- The info panel (`hud.rs`): "Organic multicellular intelligent, Level 2 civilization (modern)".
- The star browser: life type "Intelligent", and a civilization level filter.

**Generation** (`se_core/src/procgen/civilization.rs`, run after the life generator, for generated
bodies only).
- It draws from **its own hash** of the body's seed and a salt, never from SE's streams: every
  existing system stays exactly as it is, apart from the new tag.
- **Eligible** when all of these hold:
  - organic multicellular life with a terrestrial biome;
  - SE's complex stage reached: `complex < age`. Keep the value `stage()` already draws: no new
    draw from SE's stream.
  - mean surface temperature between 260 and 310 K;
  - surface pressure between 0.3 and 5 atm;
  - dry land: the volatiles class is not a global ocean.
- **Emergence.** Intelligence appears a lognormal time after complex life: median 0.5 Gyr, wide
  spread. On Earth complex life came at 4.0 Gyr and humans at 4.6. The body's age must exceed
  that. Under it is a base chance per eligible world (the "hard steps": Carter 1983, restated by
  Snyder-Beattie et al. 2021 as evidence that intelligence is rare even where life is common). The
  base chance is calibrated so that **1 in 20 land-life planets** end up with a civilization.
- **Level.** Drawn from the target mix (45 / 15 / 20 / 20), shifted by how long ago intelligence
  appeared (more time, higher odds of the later levels), then capped by the planet:
  - **Oxygen** (the completed atmosphere's O2 share) under about 12% means **at most level 1**. No
    open fire, so no metallurgy.
  - **Surface gravity** above about 2 g means **at most level 3**. Chemical rockets struggle to
    reach orbit from such super-Earths (Hippke 2018).
  - Calibrate after the caps so the final mix is the target.
- **Culture seed.** Each civilization gets its own seed that sets its style, so no two look alike
  and none copies Earth:
  - road pattern (grid, radial, organic, canal);
  - building proportions and roof shapes;
  - facade palette and night-light colour (warm sodium, cool white, alien hues to suit the star's
    light);
  - where it likes to settle (coasts, rivers, highlands);
  - the size spread of its cities.
- **Earth.** Built in: Sol's Earth is Intelligent, level 2. Its catalog is left unchanged.
- **Calibration test.** An ignored test (`cargo test -p se_core print_civ_stats -- --ignored
  --nocapture`) generates a large sample of systems and prints:
  - land-life planets per 1,000 systems;
  - intelligent planets per 1,000 systems;
  - the level mix.
  Tune the base chance and the shift to the decisions, then record the numbers in PROGRESS.md.

### 3. Settlements (all levels)

**Settlement map** (in `surface_class`).
- **Earth.** SE's night-lights tiles give the urban density (brightness to density), restricted
  to land by the water mask and to gentle slopes.
- **Generated planets.** A procedural field that depends on the level:
  - it prefers flat, low land near coasts and rivers, mild climate, and fertile biomes (grass and
    forest);
  - clustered: a few big cities, many towns, a long tail of villages;
  - the culture seed sets the preferences;
  - urban share of the land: level 1 under 0.5% (villages and fields), level 2 about 3%, level 3
    about 10% with megacities, level 4 about 20% with megalopolises.

**From orbit.**
- **Night side.** The settlement density lights it through the ground's existing city-light path
  (Earth already does this from its tiles; generated planets get the same from the field), in the
  culture's light colour, under the clouds' cover.
- **Day side.** An urban colour blends into the ground's biome colour (grey-beige concrete, the
  culture's palette for level 3+). Around towns, farmland draws as a patchwork of field colours in
  the ground shader. Both are visible from low orbit, cost nothing beyond the ground's pixel, and
  make cities findable from space.

**City sectors.**
- Cities are built in fixed 5 km sectors (MR's size) on the cube-sphere.
- A sector's layout comes from a hash of its key, the settlement density and the culture seed:
  1. a road graph (arterials first, then streets, in the culture's pattern, bent around steep
     ground and water);
  2. blocks between the roads, cut into lots;
  3. building footprints and heights from the density: a downtown core of towers, then
     mid-rises, then houses, then fields.
- Roads crossing a sector edge meet at points hashed from that edge, so sectors join.
- Built on a worker thread when a sector comes within range, cached and capped, deterministic.

**Roads in the ground shader.**
- Each sector keeps its road segments in one storage buffer, with a coarse grid of segment lists.
- The ground's pixel shader finds the nearest segments and draws asphalt, lane lines, crossings
  and sidewalks with MR's road textures, as MR's `Road` shader does from lane and crosswalk counts.
- Roads are then exactly on the ground at every distance (no z-fighting, no floating strips), and
  from altitude the same roads draw thinner.
- Roads flatten the ground across their width only where the slope needs it (a later refinement).

**Buildings, by distance** (Ultra; tuned to the budget).

| Distance | What shows |
|---|---|
| Beyond ~15 km | The ground's urban colour and roads; lights at night |
| 0.6-15 km | Each building is one **instanced box** (per-instance size and roof type), its facade drawn by the shader: MR's 18-layer window array, windows lit at night by their own hash (MR's rule), the culture's tints. About 12 triangles a building, a few draws a city. |
| 0-600 m | **Modular facades**: MR's 5 and 10 m wall modules stacked per 3 m storey, roof props (HVAC, chimneys, tanks, antennas) on roofs, interior mapping within 20 m. Cross-faded by dither from the boxes. |
| 0-150 m | Street furniture (MR's lamps, signals, benches, bins) and parked cars, as meshes; impostors beyond |

- **On the ground.** A building stands on the lowest ground point under its footprint, with a
  plinth down to it; lots steeper than a limit stay empty. The terrain is not flattened, so the
  ground's height code stays as it is. MR flattened because its cities were flat models.
- **Night.** Lit windows, street-lamp glows (instanced constant-size sprites, MR's method), and
  the ground's light from the street-lamp channel.
- **Traffic.**
  - MR's 25 cars drive along the road graph's segments, moved on the GPU by a hash and the time,
    with no per-car state. The cull pass places them.
  - Headlights and tail lights are glow sprites. From altitude, roads at night show streams of
    lights.
  - No pedestrians.

### 4. The four levels

| Level | Settlements | Assets |
|---|---|---|
| 1 Primitive | Villages and small towns: huts, longhouses, mud-brick and stone houses around a centre; dirt paths in the ground shader; field patchworks; fire and lamp glows at night (dim, sparse) | Code-built shapes (round huts with cone roofs, gable-roofed longhouses, walls) with `codex-image` textures (thatch, mud brick, timber, stone); a few Blender hero models (temple, watchtower) |
| 2 Modern | Cities as Earth's: downtown towers, suburbs, roads, street lamps, traffic | MR's facade kit, window array, roads, roof clutter, furniture, cars |
| 3 Futuristic | Denser and taller: megatowers of 300-1,500 m in the cores, skybridges, elevated roads, neon signs, flying-car light streams in 3D lanes, arcologies | The same city framework; code-built towers (tapering, stepped, twisted); `codex-image` facade panels and signs; Blender hero models |
| 4 Interstellar | Level 3 cities over a fifth of the land, plus the system: **orbital stations** (3-12 round the home world, big ones at moons), **colonized moons and rocky planets** (domed cities where there is no air, level 3 cities where there is), **space traffic** (streams of ship lights along transfer arcs between the planet, stations and colonies) | Stations: Blender hero models plus code-built modules (rings, trusses, panels), drawn with the ship shader (`HullMaterial`) and lit by the system's stars; points of light from afar |

Level 4's system features belong to the system, not the planet. A system with a level 4 planet
marks which of its moons and planets are colonized, chosen by the civilization's hash and their
size and distance. The star browser can find them.

## Budget

Ultra at 3440x1440 on the user's RTX 5080, measured with `SE_PERF` segments as for the clouds,
added on top of today's frame:

| View | Target |
|---|---|
| From orbit (any planet, Earth's night side) | under 0.3 ms |
| 3 km up over a forest | 1.5 ms |
| On the ground in a forest | 2 ms |
| Downtown of a level 2 city, street level and 300 m up | 2.5 ms |
| A forested town at the ground | 3 ms (vegetation and city together) |

- High is about half (shorter distances, sparser grass); lower presets are less again, or off.
- New graphics settings: **Vegetation** and **Cities** (Off / Low / High / Ultra), part of every
  preset and of Custom. The graphics menu shows them.
- The caps in the cull pass hold the budget at the densest views; the distances are tuned to them.
- The ground already costs 11-16 ms near a surface. The new passes must not add to its pixel cost
  beyond the canopy shading, the urban colour and the road look-up, and those run only where they
  apply (measure them as the ground's parts were measured, `CLAUDE.md` "What the ground costs").

## Steps

Each step builds (`cargo build --release`), passes the tests, is checked in pictures (the test
views below, shaders fully loaded) and measured at Ultra 3440x1440 before the next. Each ends with
PROGRESS.md and CLAUDE.md updated.

| # | Step | Done when |
|---|---|---|
| 1 | **MR asset reader**: the setting, archive access, DDS (BC1/2/3, DX10 array), model `.bin`, `global.txt` materials; tests on the user's files; the fallback when the folder is missing | Every tree, grass, facade, road, prop and car file named in the notes loads; unit tests decode known counts (for example `tree_0`: 2,437 vertices, 2 sub-meshes); with the folder missing the port runs and logs it |
| 2 | **Scatter system**: cells, pooled buffers, height atlas, placement and cull passes, indirect draws in the phases, dither LOD, impostor bake, shared lighting and haze module, prewarmed pipelines | MR's `tree_0` scattered evenly on a test planet: no floating or sunk trees from orbit to the ground, while nodes split, on a flattened planet (Earth) too; no pops; GPU objects flat over a session (`session_rs.sh`) |
| 3 | **Vegetation on Earth and SE's worlds**: `surface_class.wesl` (biome weights moved, ground pixel-identical), Earth's version from its tiles, MR's 16 trees, grass, canopy shading, colour matching, wind | Flying from orbit to the ground over Earth's forests (test views) and a generated life planet: forests look like forests at every height and never change colour; deserts and ice stay bare; budget met |
| 4 | **Tree generator**: recipes, meshes, LODs, impostor bake, tints, climate bands, gravity and atmosphere | Five different generated life planets have visibly different floras that suit their colours and gravity; Earth has conifers and palms where they belong; budget met |
| 5 | **Intelligent life class**: data, generation, Earth's override, info panel, star browser, calibration test | The test's numbers match the decisions (1 in 20 land-life planets; 45/15/20/20); every existing system unchanged apart from the tag (a before/after diff of generated systems); one test planet of each level found with the star browser and recorded in PROGRESS.md |
| 6 | **Settlement map and the view from orbit**: the field, night lights on generated planets, urban and farmland colours; Earth's from its lights map | From orbit, each test planet's civilization shows by night and by day as its level suggests; Earth's night side unchanged where it was right |
| 7 | **Level 2 cities**: sectors, road graph, lots, buildings (boxes and modular facades), roads in the ground shader, night windows, street lamps, furniture, traffic; Earth through its lights map | A flight from orbit into a city (Earth: Tokyo's and New York's areas; a level 2 test planet) without pops or floating parts; roads lie on the ground; budget met |
| 8 | **Level 1**: villages, huts, fields, paths, fire glows; the `codex-image` textures; the Blender hero models | The level 1 test planet's villages look lived-in from 1 km and the ground; cheap |
| 9 | **Level 3**: megatowers, skybridges, neon, flying traffic; textures and hero models | The level 3 test planet reads as a future city from orbit (lights), from 10 km and in its streets; budget met |
| 10 | **Level 4**: stations, colonized moons and planets, space traffic | The level 4 test system shows stations orbiting, domed colonies on airless moons, and traffic between them; no cost when far |
| 11 | **Polish**: graphics settings and presets, seasons and snow on plants, roads flattening steep ground, final measurements, README / PROGRESS / CLAUDE | All test views pass; the user's custom settings meet the budget; docs updated |

## Test views

Fixed scripted views (time paused), screenshots after the shaders are fully loaded:

1. **Earth's Black Forest** (48.0 N, 8.2 E): from 400 km, 30 km, 3 km and the ground, at midday
   and at a low sun.
2. **Earth's boreal forest** (60 N, 100 W) and **the Amazon** (3 S, 60 W): from 3 km and the
   ground (conifers, broadleaf, palms).
3. **The Sahel** (14 N, 0 E): the sparse edge between savanna and desert.
4. **Tokyo** (35.7 N, 139.7 E) and **New York** (40.7 N, 74.0 W): from orbit at night, from 30 km,
   300 m and the street, by day and night.
5. **A generated planet with multicellular land life but no civilization**: flora only.
6. **One test planet of each civilization level** (found in step 5), from orbit, from 10 km and on
   the ground, by day and night.
7. **The level 4 test system**: a station close up, a domed moon colony, traffic from 50,000 km.
8. **A flight from orbit to the ground over a forest and into a city**, as a video: no pops, no
   colour changes, no floating objects.

## Build log (what each step learned)

### Step 1: MR asset reader (done 2026-11-04)

- Code: `se_core/src/mr.rs` (archives, models, material records, global.txt), `se_core/src/dds.rs`
  `parse_dds` / `decode_level` (layouts with every mip and layer), `se_engine/src/mr_assets.rs`
  (`MrAssets` resource, `dds_image` to a Bevy image, `submesh_mesh` to a Bevy mesh). The folder is
  `EngineConfig::mr_path` (`--mr-path`, `MR_PATH`, default the user's copy). Opening takes 12 ms.
- The `.fcm` archives mount with the port's own `Vfs::mount_pak` (their entries already start with
  `Models/`, `Textures/`, `Data/`).
- **Model materials are a tree, not a flat list** (the notes left it undecoded): each node is
  `u8 child count, u8 payload size, u32 key hash, payload`, then its children. The root has 15
  children: the name (key `0xde08cb97`), the effect (`0x722146f4`, its children are the effect's
  parameters; `g_vDiffuse` is `0xb0256f2a` with x, y, z, w children), the texture list
  (`0xe7595b09`, one string child per texture) and render flags. The hash function is unknown
  (not FNV, CRC32, djb2, sdbm or murmur3); the keys were identified on the files. The tree parse
  consumes every record of all 549 readable models exactly.
- **Most props' materials are not in global.txt** (231 names such as `bench_0_Vertex`): the record
  embedded in the model is the source for those. `MrInstall::resolve` takes global.txt's entry of
  the same name where one exists, else the embedded one.
- 549 of 557 models parse. Not read: the skinned people, hands, `_anim_test` (bone streams with
  larger elements) and `white_house_inner` (version flags `0x20000001`). The three extra 4-byte
  streams are filled on a few models (galaxies, giza, tunnels): they are skipped.
- All 587 DDS files parse. Formats: DXT1/3/5, 32-bit BGRA (with and without alpha), FourCC 114
  (R32F, the height maps) and 116 (RGBA32F, the impostor index sheets), DX10 with DXGI 31
  (RGBA8 snorm), 51 (**`NoiseTileableR8G8` is R8G8_SNORM**, not unorm), 61 (R8) and 77 (BC3 arrays:
  `windows`, `windows_NG`), and one cube map (`NoiseCube`).
- Textures named by models but **not shipped with the game**: `brick_1`..`brick_6` (+ `_ng`) of the
  window modules' embedded materials and `AC_Hvac_Units\20_Dish_512`. The window modules' walls
  are drawn with the `windows` material in MR's cities anyway; the port draws missing textures
  with a neutral one.
- The notes' example "UV half 0x33ae = 0.2257" is 0.24 (a misreading; the decoder is right).
- Tests: `cargo test -p se_core --release mr::` (the 16 trees' counts and heights, every named
  model, material and texture, the whole archive) and `cargo test -p se_engine --release mr_assets`
  (images and meshes). They pass (and say "skipping") without the install.

### Step 2: GPU scatter system (done 2026-11-04)

- Code: `se_engine/src/scatter.rs` (main world: cells, slots, height pages, each cell's tree of drawn
  nodes), `scatter/gpu.rs` (render world: pooled buffers, the compute passes, the phase item and its
  draw command), `scatter/library.rs` (models packed into one vertex and one index buffer, buckets,
  draws), `scatter/impostor.rs` (the impostor bake), shaders `scatter.wesl` (compute: `place`, `cull`,
  `finish`), `scatter_draw.wesl` (meshes and impostors) and `ground_light.wesl` (the lighting shared
  with the ground). The test: `SE_SCATTER_TEST=1` or the dev command `scatter test|off` scatters
  `tree_0` evenly (one per 900 m^2) over the land of the planet the camera is at; `SE_SCATTER_DEBUG=1`
  logs the resident cells, pages and each bucket's instance count (read back from the GPU) every
  120 frames.
- **What changed from the design, and why:**
  - **Cells are the quadtree's nodes split until their real edge is at most 800 m** (`CELL_EDGE_M`),
    not one fixed level per class. The cube's gnomonic mapping makes one level's cells differ about
    5x in area across a face, and the level's rounding 2x more: with a fixed cap per cell the small
    cells filled every slot (256 of 256 on the user's planet) and the big ones could not hold a
    dense forest. Cells are now 400-800 m everywhere (a fixed rule of position, so still repeatable).
  - **The drawn ground is geomorphed** (planet.wesl's vertex shader moves vertices towards the
    parent's surface by distance), which the plan missed. The cull pass rebuilds it: a page holds the
    node's own 33 x 33 positions (and heights), and each vertex's morph target is the mean of two
    neighbours by parity, exactly as `build_node` makes `morph`. So the trees follow the ground as it
    morphs and splits.
  - **A page holds positions, not heights**: the node's vertices relative to its centre (f64 maths,
    then f32), remade from the height grid by `terrain::node_vertices` as `build_node` makes them.
    The page's per-frame record is the node's centre relative to the camera and the camera in the
    node's frame; the body's matrix (orientation x ellipsoid x the space's scale) is shared. A tree's
    base is then the drawn triangle in the node's own frame: exact on flattened planets, with no f32
    planet-scale positions anywhere.
  - **Instances are 4 bytes**: the candidate's number in its cell (and model). The cull pass
    re-derives its place, turn and size from the cell's seed and that number (`candidate`), so 384
    slots of 8,192 cost 12.6 MB.
  - **Placement cannot be lost**: the main world sends every resident cell's job each frame; the
    render world places the slots it has not placed yet, a few a frame, and marks a job done only when
    the passes are ready. Sent once, jobs issued while the pipelines were still compiling were lost
    and those cells stayed empty.
  - **WARM_ORDER is only for the ground's Material pipelines.** Pipelines queued directly at
    `RenderStartup` start building at once and the loading screen waits for them; the scatter's
    (compute, and the draws for no MSAA and 4x) are queued there.
  - **Bevy 0.20 has no render graph**: `Core3d` is a schedule. The compute passes are a system in
    `Core3dSystems::Prepass` before `main_opaque_pass_3d`; the draws are one non-mesh item in
    `AlphaMask3d` (drawn after the ground in the main pass, in render_scale's corner, with the view's
    MSAA, before the clouds read the depth).
  - **The light is worked out per vertex**: the normal-free part (the star through the atmosphere,
    its cloud and eclipse shadow, the sky light, day ambient, the further stars on level ground,
    AmbientLighting, the haze's transmittance and inscatter) in the vertex stage (impostors at the
    tree's centre), the normal's part in the pixel. Per pixel the same light cost 1.5 ms on the
    ground at Ultra; per vertex 0.6 ms.
- **Shared lighting (`ground_light.wesl`)**: `PatchLight`, `CloudShadow`, `cloud_shadow_at`,
  `horizon_shadow_at` and `ground_haze` moved out of planet.wesl, which now calls them. Checked
  against the old shader with the same binary: with the 3D clouds off, the ground at 300 m and 3 km
  is identical (no pixel differs); from 400 km 7 % of pixels differ by one level (the haze's
  rounding now that it is a shared function); with the 3D clouds on the differences are their
  frame-phase noise (two runs of the old shader differ as much).
- **Review (a four-angle review with every finding checked):** fixed: height pages lost in a frame
  without the main view (they are uploaded first now); a cell with no drawn ground under it (past the
  horizon) built its node tree down to level 20 and filled the tree buffer (the tree now descends only
  into ancestors of drawn nodes); impostors drawn twice too bright (their colour was divided by an
  alpha it was not multiplied by); impostors 5 % too large (pulled towards the camera, not shrunk);
  the trees' cloud shadow read at the sphere frame's height instead of the ground's (they differ on
  flattened planets); trees on giants in the test.
- **Still to do in later steps:** HD screenshots, skybox export and the projection modes do not draw
  the scatter (as the volumetric clouds; its passes run for the main view only); the rings' shadow on the trees (the ground's `ring_shadow` uses
  `fwidth`; a shared version needs the width passed in); the soils' "3D stones" move the near ground
  by decimetres that the pages do not hold, so trees stand 0.4 m into the drawn ground (`SINK_M`);
  from 3 km up no tree is within the 3 km range (the canopy shading of step 3 covers it).
- **Measured** (Ultra 3440x1440, Ultra clouds, the test cover of one tree per 900 m^2,
  `.optimize/bench/scatter_ultra.sh`): the compute passes under 0.05 ms; the draws add 0.1 ms to the
  main pass at 300 m (1,600 impostors) and 0.6 ms on the ground (40 meshes, 2,700 impostors). The
  whole frame on the ground was faster with the trees (16.2 against 16.8 ms): they hide sky the
  clouds would trace. Over a session (`.optimize/bench/session_scatter.sh`, 6 rounds of new places)
  bind groups and buffers stay flat (the scatter adds a fixed 23 bind groups and 41 buffers); the
  textures grow as much without the trees (Earth's tile cache filling, 4,900 to 7,300). At that
  session's fixed view (300 m) the trees cost 0.4 ms of the frame.
- Checked in pictures: Earth's Black Forest from 3 km down to 2 m (trunks on the ground at every
  height while the nodes split), the user's planet RS 8514-3620-1-7-69 1 from the ground and 300 m,
  the clouds hiding the trees (depth), Ultra 3440x1440.

### Step 3: vegetation on Earth and SE's worlds (done 2026-11-04)

What was built:
- **Which bodies.** Those whose catalog Life has multicellular (or intelligent) organic or exotic life
  on land (`Terrestrial`, `Marine/Terrestrial`; `scatter::land_plants`, kept in `Terrain::land_plants`).
  Unicellular or sea-only life keeps the ground's colours alone. On by default; `SE_SCATTER=0` or the
  dev command `scatter off` turns it off, `scatter test` is step 2's even test cover.
- **`surface_class.wesl`** (done by a helper agent, checked pixel-identical on 11 views and no slower):
  the ground's biome weights moved out of planet.wesl into a module without bindings, shared with the
  cull pass. The scatter works out SE's slow noises once per tree at placement (f16, the `noises`
  buffer) and the rest per frame from the drawn ground's height and slope.
- **Earth (any body with a colour map): a vegetation patch from its map.** 512 x 512 texels, 600 km
  across, round the camera, made on a worker thread from the colour map's deepest tiles (Earth's level
  5, 1.2 km a texel; 12 tiles, 50-60 ms) and the water mask, again once the camera is a quarter of its
  half-width from its centre. Each texel is classified from its colour (`scatter/earth.rs` `classify`,
  `vegetation.wesl` `veg_classes`), with thresholds from Earth's map sampled at the test places:
  forest is dark and strongly green (excess green 0.56-0.66 under 45-75 brightness: Black Forest,
  Congo, the taiga), farmland green but redder (0.39-0.43), tundra 0.24, dry savanna and desert about
  0 (deserts bright), ice white. The woods' shapes come from a noise of 100 m - 1.5 km over the map's
  forest class (`map_noise`, `map_forest`), so the 1.2 km texels become woods with edges and
  clearings; copses and hedgerows stand in grassland, lone trees on the steppe; none on steep ground.
- **MR's 16 trees** (all broadleaf: conifers and palms come with step 4's generator), one picked per
  tree by its candidate number; 1 tree per 90 m^2 in a full forest.
- **Colour matching.** Each tree takes the ground's colour where it stands (Earth: the map's colour;
  SE: its forest slot colour) over the model's mean colour (measured from its impostor views at the
  bake) times a gain (`BodyFrame::tint_gain`) matched by pixel means to the ground with the trees off,
  0.7-3 km away: Earth (2.6, 2.3, 3.5), SE's terra worlds (1.7, 1.8, 1.8). The crowns' ball shading is
  about 1.5 times darker than level ground; Earth's ground also brightens its map by its detail
  textures' brightness (about 1.6), SE's draws its slot colours. Copses in grassland are darker than
  the fields round them (Earth's forest over its farmland), lone steppe trees olive, not the dry
  ground's beige. The bark takes the square root of the tint.
- **The ground's part** (planet.wesl `lit_ground`, told the body's vegetation by its light record,
  `PatchLight::flags` z and w): the forest floor darkens by 0.45 under the trees; past them the ground
  draws the crowns (`canopy_shade`: crowns 14 m apart lit on the star's side, dark gaps, groups of
  trees lighter and darker, the mean kept at 1 so the forest keeps its colour), faded out as a crown
  shrinks under 0.7 pixel footprints (about 15 km straight down). The hand-over takes the last quarter
  of the trees' range: the trees shrink and thin out as the canopy comes in (a sharp end showed as a
  circle round the camera from above).
- **Grass:** the same scatter in cells of at most 40 m within 120 m of the camera (MR: a fixed cloud of
  50,000 points wrapped round the camera; cells reuse the placement, the drawn-ground lift and the
  culling), 2.5 tufts a square metre (MR 2.2), shrinking to nothing over the last 10 m. Four tufts are
  built in code (three quads crossed at 60 degrees, 0.66 m, MR's `bunch_N` are made by its code, not
  stored) with MR's `Billboard_Grass` atlas and tint. Grassland and steppe grow it, the woods a little
  undergrowth; none on rock over about 14 degrees or within 8 m of the sea. Tinted as the trees at half
  the gain, shaded under the woods like the floor.
- **Wind:** the derived weather's wind at the camera (`volumetric_clouds::wind_at`, worked out every
  second): trees lean and sway about their base, more towards the top, each at its own phase; leaves
  and grass flutter.
- **The meshes' light once per instance:** a compute pass (`scatter_draw.wesl` `light`) after the cull
  works out the ground's light at each mesh instance's crown into an f16 record the vertex shader reads.
  Per vertex it was 0.9 ms of a forest floor's 2.3 ms.

What changed from the plan, and why:
- Grass is scatter cells rather than MR's wrapped point cloud: the cells already stand objects on the
  drawn ground exactly and cull them; a point cloud would need the ground's height under each point
  every frame.
- The trees' colour is the ground's colour where they stand (per tree), not one biome colour: Earth's
  forests differ (taiga darker, tropics bluer), and a tree then never changes the ground's colour.
- The canopy's mean is kept at the ground's colour, and the hand-over is a quarter of the range wide.

Measured (Ultra 3440x1440 with 4x MSAA, Ultra clouds, Black Forest, `.optimize/bench/scatter_ultra.sh`,
the main pass's GPU time with the vegetation against `SE_SCATTER=0`, means of 2-3 runs):
- 3 km up over the forest: +1.7 ms (target 1.5); 300 m up: +1.7 ms; on the forest floor: +1.4 ms
  (target 2). Before the light pass the floor was +2.3 ms. The compute passes take 0.06-0.10 ms. Runs
  vary by about 1 ms; the user's own settings (no MSAA) cost less. At 3 km most of the cost is the
  ground's canopy code (the woods' noise and the crowns: six value noises a forest pixel); a noise
  texture instead of hashed value noise is the next saving if it is needed.

Review (three angles, each finding checked by a second agent), fixed: about 4 % of candidates came
through where the share was 0 (trees and grass in deserts and on ice: the threshold's soft edge was
centred on it); grass cells took the far tree cells' slots (slots 384 to 640); the ground drew its
forest floor and canopy before the trees could be drawn (no Megaton Rainfall, impostors baking); a
missing patch read as full forest (Bevy's white fallback, now its black one, and a new patch waits for
its image); after a jump the old patch folded the far side of the body into itself; the wind snapped
once an hour (Bevy's clock wraps at 3600 s; the scatter's at 200 pi s with whole-multiple
frequencies); the trees classified filtered classes and the ground a filtered colour, so woods' edges
differed (both classify the colour now; the patch holds only the colour, land in alpha).

Checked in pictures: the Black Forest from 4 km down to the forest floor, the Amazon, the Siberian
taiga, the Paris basin's fields and woods, the Sahel's lone trees and the Sahara bare, and SE's warm
arid life world RS 8514-928-8-5906585-158 3 from 600 m and the ground.

Left open:
- Earth's taiga and tropics have MR's broadleaf trees (step 4: conifers and palms where they belong).
- The canopy is faint from 3-5 km up on Earth: the port's haze there is strong (the ground's own look).
- From above the trees look a little brighter than the canopy beyond them (the gain was matched at
  grazing angles).
- MR's grass atlas has leafy plants as well as grass: a meadow reads as scattered tufts.
- From step 2: the rings' shadow on the trees, the capture views, the 0.4 m sink.
- No Vegetation graphics setting yet (Off / Low / High / Ultra, the plan's budget section).

### Step 4: tree generator (done 2026-11-04)

What was built:
- **`scatter/trees.rs`:** a species is a recipe (crown shape: broad, cone, column, umbrella, palm,
  shrub; height, trunk radius, where the crown starts, its spread, branches with their tilt and
  droop, leaf cards and their size, the trunk's lean, bark set and leaf card) grown into one mesh:
  tapered tubes (7 sides for the trunk, 3-4 for branches) dressed in MR's bark (`Trees/Bark1A`..10A)
  and leaf cards along the branches' outer parts (two crossed per spot): MR's 8 broadleaf branch
  cards (`Trees/Branch1A`..), the port's conifer spray and palm frond (below). Palms are a bare
  curved trunk and a fan of fronds (strips of 4 pieces bending down, folded into a shallow V); shrubs
  a clump of cards on three short stems. 700-1,500 triangles each (tested: 100-2,600 for every
  species and seed).
- **Earth's species** (fixed recipes, beside MR's 16 broadleaf): spruce, pine, fir; coconut palm
  (21 m) and date palm (13 m); an acacia; two shrubs. The cull picks per tree (scatter.wesl
  `earth_species`): coldness = latitude + 9 degrees a km of height; conifers from 42 to 58 (so the
  boreal forests, and the Black Forest at its height, are mostly spruce and fir), palms in the
  tropics (12 % inland, up to 70 % within 60 m of sea level), the steppe's lone trees acacias where
  it is warm and shrubs where cooler, MR's broadleaf elsewhere.
- **Floras:** every generated planet with land plants gets 8 species from its seed (`trees::flora`):
  three forest trees, two highland trees (the upper part of the relief), two of the dry land (the
  steppe's lone trees, which SE's terra worlds now get too) and one of the shore (within 30 m of
  the sea). Gravity sets height and girth (height x (g / 9.81)^-0.5, girth x its inverse, both
  clamped); the air's pressure sets the crowns' spread (x p^0.2, 0.7-1.5). The pressure is the one
  SE's generator works with (`Terrain::pressure_atm`; generated planets carry no `Atmosphere`
  block when SE completes them). The leaves' colour is still the ground's (the tint), so a red or
  yellow world gets red or yellow forests.
- **The library** (`library.rs`, rewritten): model sources packed by `assemble`; a fixed material set
  (MR's trees' own, its 9 barks and 8 branch cards, the port's 2 cards, the grass atlas); 36 models
  (16 MR, 8 Earth, 8 flora, 4 grass; 864 impostor layers, baked in 3.6 s at start). Arriving at a
  generated life planet grows its flora and bakes only those 8 models' impostors on a worker thread
  (about 1 s); the library switches to it when done (meshes, layers and mean colours together), and
  until then the planet shows no trees. Earth's and the flora's species get larger buckets (4,096
  meshes, 40,000 impostors): a taiga is three species.
- A log line names a generated system's bodies with land plants ("generated bodies with land
  plants: ..."), for finding test planets.

Textures made for it (`crates/se_engine/assets/textures/vegetation/`, the port's own):
- `conifer_branch.png` and `palm_frond.png`: made with the `codex-image` skill (2026-11-04; prompts:
  a flat spruce/fir branch spray, and a single coconut palm frond, seen from above, filling the
  square, on a flat magenta background, even light), then keyed (magenta to alpha, the spill taken
  out of the edges, the leaves' colour bled into the clear texels so filtering never shows pink) and
  scaled to 512 px (`scratchpad/key_cards.py` of that session; its method is in this note). Their
  mips are made at load with the alpha kept to the top level's coverage, as the impostors'.

Checked in pictures: Earth's taiga from 300 m and 60 m (a conifer forest), the Sahel (acacias and
shrubs), the Amazon delta's coast (palms through the broadleaf canopy), the test cover on the ground
(every species); five generated life planets, each with a flora of its own: RS 8514-928-8-5906585-158
3 (g 6.2, 0.07 atm: yellow spires on sand), RS 8514-928-8-5794236-229 4 (g 14.9, 223 atm: squat dark
trees on red ground), RS 8514-928-8-5794238-114 B3 (g 11.1, 147 atm: a dense dark red forest in thick
haze), RS 8514-928-8-5906584-114 A2 (g 3.4, 0.01 atm: tall thin spikes) and RS 8514-928-8-5906586-28
B1 (g 3.3, 2.07 atm: tall rust conifers).

Measured (Ultra 3440x1440 with 4x MSAA, Black Forest, now mostly the generated conifers, against the
vegetation off): +0.9 ms 3 km up (target 1.5), +1.1 ms at 300 m, +0.6 ms on the forest floor (target
2): less than step 3 with MR's broadleaf meshes everywhere, the generated trees having fewer vertices.

Left open:
- The generated trees' crowns are thin from close up (cards along few branches): a denser crown
  (more cards, a second card size) would read better under the camera.
- A flora is 8 species; the plan said 6-10. Nothing yet ties a species to SE's own biome slots
  beyond forest / steppe / height / shore.
- Arriving at a life planet re-uploads the whole impostor atlas (about 90 MB with its normals), a
  short hitch once per arrival; the atlas copy kept for the splicing holds the same on the CPU.

Review (two angles, each finding checked), fixed: the generated bark tubes were wound inside out
(back-face culling hid their outer walls); a flora's arrival copied and flattened the whole atlas
(about 150 MB) on the main thread (now on the worker, which also makes the images); a flora bake
that finished after the camera came back to the previous body replaced the right flora; the ground
drew its forest floor while a planet's flora was still baking; an exe copy with only its shaders
beside it got grey leaf cards (the cards now also come from the source tree); the highland species
were measured from the radius, so on worlds without a sea every forest tree was a highland one (now
from SE's sea level).

### Step 5: intelligent life class (done 2026-11-04)

What was built:
- **Data:** `Life { Type "Intelligent" Civilization 1..4 }` (1 primitive, 2 modern, 3 futuristic, 4
  interstellar; Intelligent without the key is level 2). `LifeParams::civilization`,
  `LifeParams::multicellular()` (intelligent counts as multicellular: the ground's life colours,
  the vegetation), `BodyRecord::civilization()` (Sol's Earth is level 2 built in; its catalog is left
  as it is). A catalog add-on may set its own.
- **Generation** (`se_core/src/procgen/civilization.rs`, run inside SE's life generator `se_life::apply`
  for generated bodies, after SE's draws): its own hash of the system's seed and the body's place,
  never SE's streams. Eligible: organic multicellular life on land, SE's complex stage reached (the
  value SE's generator drew, kept in `LifeOut::complex_gyr`: no new draw), 250-370 K, 0.1-30 atm.
  Intelligence a lognormal time after complex life (median 0.5 Gyr, sigma 0.8); then the base chance;
  the level from weights shifted towards the later levels the longer ago it appeared; then capped
  (oxygen under 12 %: level 1 at most; over 2 g: level 3 at most).
- **Info panel:** "Organic multicellular intelligent, Level 2 civilization (modern)".
- **Star browser:** life type "Intelligent" (and Multicellular includes it), a Civilization filter
  (any level, 1-4); the dev command `search LY civ` lists and logs the civilizations found.

What changed from the plan, and why:
- **The bands are wider:** 250-370 K and 0.1-30 atm, not 260-310 K and 0.3-5 atm. SE's planets with
  multicellular land life are mostly warm and dense-aired (of 1,672 in 20,000 generated systems: median
  354 K and 84 atm, a quarter over 445 K or 247 atm); Earth's bands kept 3 % of them, so the decided
  1 in 20 could not be reached at any base chance.
- **Oxygen from the biosphere:** SE's completed atmospheres hold no oxygen made by life (every
  eligible world's completion has under 12 %), so the cap would have made nearly every civilization
  primitive. A land biosphere's own oxygen counts: Earth's 21 % after the first 0.5 Gyr of complex
  life (photosynthesis), so only young biospheres stay at level 1.

Calibration (`cargo test -p se_core print_civ_stats -- --ignored --nocapture`, 20,000 systems round
stars of 0.25-1.45 solar masses and 1-11 Gyr): 83.6 planets with multicellular land life per 1,000
systems, 4.15 civilizations per 1,000 (1 in 20.1 land-life planets); eligible 311, intelligence reached
273, over 2 g 12. Base chance 0.30 and level weights 0.641 / 0.157 / 0.133 / 0.069 give the levels 45 /
15 / 19 / 20 % (the pool of worlds intelligence reached). Every generated system is otherwise
unchanged (`civilizations_only_add_tags`: 1,500 systems, SE's life identical, the Life text identical
apart from `Type "Intelligent"` and the `Civilization` line).

Test planets (`search 140 civ`, the star browser within 140 light-years: 8,109 systems, 14
civilizations besides Earth): level 1 WISE 19101+56342 3 (63.6 ly), level 2 2MASSI J1029216+162652 2
(62.4 ly), level 3 2MASSW J1338261+414034 4.3 (68.9 ly), level 4 SDSS J212413.88+010000.2 1 (58.7 ly;
its info panel reads "Organic multicellular intelligent, Level 4 civilization (interstellar)"). Most
hosts nearby are brown dwarfs' generated systems (the nearest catalog stars).

## Implementation notes (from the port's gotchas: keep these)

- **GPU objects.** Pooled buffers only, no per-cell or per-instance buffers, textures or bind
  groups; caches with limits (`PORT_LAND_NODES`, `TEXTURE_CACHE_MB` are the precedents). Watch
  `SE_PERF`'s wgpu counts over `session_rs.sh`.
- **Materials.** Never change a Bevy material asset every frame. Per-frame values go in storage
  buffers written in place.
- **Pipelines.** Queue every new pipeline variant at `RenderStartup` (custom pipelines) or add it
  to `pipeline_watch.rs` `WARM_ORDER` (Material pipelines). After a shader edit, wait 2,400+ frames
  before judging a picture, and after a camera jump let the ground load (1,200+ frames).
- **Ground pixel shader.** Never `discard` in the ground's `fragment` (it would turn off the early
  depth test). The new objects use their own pipelines for that.
- **Depth.** Vegetation and buildings must be in the depth buffer before the volumetric clouds'
  passes, which read it as true distances. The active body is drawn in linear space then.
- **Flattened planets.** Earth is an ellipsoid. Placement and lifting go through the terrain's own
  node frames and heights, never a sphere of one radius.
- **Precision.** Instances are relative to their cell, and cells to the camera, as the terrain
  nodes are.
- **Shared code.** Anything the ground and the new objects both need (biome weights, haze, cloud
  shadow, lighting, settlement density) lives in one shared WESL module. Copies drift apart: the
  cloud shadows' bright square happened three times.
- **Repeatability.** The scatter's hashes depend only on cell keys and seeds, never on frame
  order, so pictures repeat between runs (the benches need that).
- **Files.** Keep each file's line endings; patch with Python bytes.

## Licence and assets

- MR's assets stay in the user's install and are read at run time. They are never copied into the
  project or the repository (the same rule as SE's files). `re/megaton_rainfall/extract` holds
  only its text and shaders for study, out of git.
- Several MR assets come from third-party store packs: the trees ("Realtime 3D Trees XL"), the
  street furniture ("101 Street Furniture"), the cars, the roof units and the antennas. Reading
  the user's own copy for this private port is the plan. If the port were ever shared, those
  features fall back to the generator and code-built parts.
- Textures made with `codex-image` and models made with the Blender tool belong in the project:
  keep them in `crates/se_engine/assets/` (small: compressed, one size each), with a note in this
  file of how each was made. So far: `textures/vegetation/conifer_branch.png` and `palm_frond.png`
  (step 4's build log says how).

## Open questions

- MR's tree impostors are baked by its exe, with distances set in its C++. Ours are tuned by the
  budget, not copied.
- How far the canopy shading can go before it costs too much in the ground's pixel shader: measured
  in step 3 (its build log): it fades by the pixel's footprint, about 15 km straight down, and costs
  most of the 1.7 ms at 3 km up.
- Whether level 4's colonized moons need their own settlement maps or reuse the field with an
  "airless: domes" rule (decide in step 10).
