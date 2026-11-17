// The Sidewinder's shadow (sidewinder/shadow.rs): its record and the map's frame, which its map passes (sidewinder_shadow.wesl)
// and the ship's own lighting (sidewinder_lit.wesl `star_shadow`) share. No bindings here: each caller binds the record and the
// maps. The filter itself and the pick of a cascade are the engine's (moment_shadow.wesl), which the world's receivers of the
// ship's shadow read the same maps through (ground_light.wesl `ship_shade_at`).
//
// The maps: orthographic along the star's direction, in the ship's space, over a sphere round every caster in every pose of
// its moving parts (shadow.rs `sphere`, ours; the record's `sphere`), so only the light's direction changes them. Their axes come from the star's
// direction alone (`light_axes`), worked out the same way by every shader that reads them (shadow.rs `light_axes` for the
// world's record). Their depth `d` runs from 0 on the star's side of the sphere to 1 on the far side: the quantity Elite's
// lookup compares (FWD 352: 1 - the map's z).
//
// One map of the ship for each of Elite's cascades the ship can be seen in (the user's High `Profile_Space_Cockpit`, ends 5,
// 80.8 and 739.2 m; re/elite/notes/sidewinder_shadow.md), each with Elite's moment texel for that cascade in the view that draws
// (shadow.rs `cascades`, `Cascade::texel`), so its size in texels is the sphere's width over that texel; and the seat's own map of the
// cockpit set at the first two cascades' texels. Each map a square region of a moments texture (the record's `maps`, shadow.rs `layout`), drawn and
// read only inside its region: the outside set's three in the engine's texture (volumetric_clouds.rs
// `SHIP_SHADE_IMAGE`, which the world's receivers bind too), the seat's two in the mod's own (shadow.rs `ShipShadow::seat_moments`);
// a ship material binds the one its set reads.
//
// The pick's boxes are Elite's: each cascade's box is centred on its bounding sphere, `ahead` m along the view from the camera,
// `half` m wide either side across the light (shadow.rs `cascades`); its axes about the light are `light_axes` (E: Elite's light
// view's own orientation about the light was not read). The maps stay fitted to the ship: only which map a point reads, and
// the blend, are Elite's. A cascade without a map of the ship (the fourth, whose moment texel, about 32 m, is wider than the
// ship; the seat's third) reads lit.

// One storage buffer (shadow.rs `ShadowRecord`).
import super::moment_shadow::moment_map_visibility;

struct ShipShadow {
    // x 1 once this frame's maps are drawn (the map pass sets it after drawing them; the render world writes it 0 first each
    // frame), so nothing reads a map that is not this frame's.
    live: vec4<u32>,
    // The ship's space from the render frame's: p_ship = (dot(rows[0], p), dot(rows[1], p), dot(rows[2], p)), p = (x, y, z, 1).
    to_ship: array<vec4<f32>, 3>,
    // xyz the maps' middle in the ship's space, w their radius (m).
    sphere: vec4<f32>,
    // x Elite's quantisation offset, y the moment bias, z the light-bleeding cutoff, w the resolve depth bias (map depth).
    msm: vec4<f32>,
    // Elite's moments from the quantised ones (FWD 346-349), row by row; and the write's, its inverse.
    read: array<vec4<f32>, 4>,
    write: array<vec4<f32>, 4>,
    // The moments textures (both the same size): their width and height in texels, and 1 / those.
    atlas: vec4<f32>,
    // The maps: 0, 1, 2 the outside set's at Elite's cascades 0, 1, 2 (in the engine's texture); 3, 4 the seat's (the cockpit
    // set) at cascades 0, 1 (in the mod's own). xy the region's corner in its texture (texels), z its size (moment texels a
    // side), w its moment texel (m).
    maps: array<vec4<f32>, 5>,
    // Elite's cascades' boxes, cascade k in component k: how far ahead of the camera along the view each box's middle lies
    // (m), and its half-width across the light (m).
    ahead: vec4<f32>,
    half: vec4<f32>,
    // Elite's fade thresholds (cb1[326]): x the cascades' count, y the floor (4 / 1536: a cascade is not read within it of its
    // box's edge), z an inner cascade's fade top (0.15), w the last's (0.2).
    fade: vec4<f32>,
    // The lookup's offset along the normal at grazing light for map m in offsets[m / 4][m % 4] (m; ours: the filter's reach,
    // shadow.rs `normal_reach`), scaled by the sine of the light's angle to the normal.
    offsets: array<vec4<f32>, 2>,
};

fn to_ship_point(s: ShipShadow, p: vec3<f32>) -> vec3<f32> {
    let h = vec4<f32>(p, 1.0);
    return vec3<f32>(dot(s.to_ship[0], h), dot(s.to_ship[1], h), dot(s.to_ship[2], h));
}

fn to_ship_dir(s: ShipShadow, v: vec3<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(dot(s.to_ship[0].xyz, v), dot(s.to_ship[1].xyz, v), dot(s.to_ship[2].xyz, v)));
}

// The map's axes for the star's direction `l` (ship space, towards the star): columns u, v (the map's x and y) and l. Ours.
fn light_axes(l: vec3<f32>) -> mat3x3<f32> {
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(l.y) < 0.99);
    let u = normalize(cross(helper, l));
    return mat3x3<f32>(u, cross(l, u), l);
}

// A point of the ship's space on the map: x, y in -1..1 across the sphere, z the depth d (0 the star's side).
fn map_point(s: ShipShadow, p: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let a = light_axes(l);
    let q = (p - s.sphere.xyz) / s.sphere.w;
    return vec3<f32>(dot(q, a[0]), dot(q, a[1]), 0.5 - 0.5 * dot(q, a[2]));
}

// The quantised moments of depth `d` (ours: the inverse of Elite's read, so its lookup gets back d, d^2, d^3, d^4).
fn quantised_moments(s: ShipShadow, d: f32) -> vec4<f32> {
    let d2 = d * d;
    let b = vec4<f32>(d, d2, d2 * d, d2 * d2);
    return vec4<f32>(dot(s.write[0], b), dot(s.write[1], b), dot(s.write[2], b), dot(s.write[3], b)) + vec4<f32>(s.msm.x, 0.0, 0.0, 0.0);
}

// Map `m`'s visibility (1 lit) at the map point `mp` (x, y across the map, z the depth); 1 off the map (moment_shadow.wesl, with
// this record's values).
fn map_visibility(t: texture_2d<f32>, smp: sampler, s: ShipShadow, m: u32, mp: vec3<f32>) -> f32 {
    return moment_map_visibility(t, smp, s.atlas.zw, s.maps[m], s.msm, s.read[0], s.read[1], s.read[2], s.read[3], mp);
}

// Is there any light from the star to take away (`HeroLight::sun`: the ship's own copy, without its shadow on the world)?
fn star_lit(sun: vec3<f32>) -> bool {
    return max(sun.r, max(sun.g, sun.b)) > 0.0;
}
