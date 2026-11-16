// Elite's lit maths for the Sidewinder (ARCHITECTURE.md A6), lighting what a surface function of
// sidewinder_surface_hull.wesl (and later sidewinder_surface_cockpit.wesl, sidewinder_glass.wesl) worked out. Elite writes
// its surfaces to a G-buffer and lights them afterwards; this game lights forward, so `shade` is what Elite's lighting does
// to one G-buffer texel.
//
// Ported from (re/elite/frea_export/sidewinder/decoded/shaders/ and the deferred resolve the exe embeds):
// - the directional light and the clustered point / spot loops of the reference lit shader
//   `ps_Metalness_Roughness_OpaqueDeferred_EmissivePulse_TransparentBox_Forward_1_1_0_0` (FWD; light_cb_layout.json):
//   Lambert diffuse albedo / pi x N.L, GGX specular with alpha = a^2 (a = the roughness, at least 0.05), Schlick's Fresnel
//   on |L.H| to the fifth power, Smith-Schlick visibility 1 / ((N.L (2 - a^2) + a^2) (N.V (2 - a^2) + a^2)) (FWD 599-632);
//   point and spot lights with the window max(1 - (max(d^2, s^2) / R^2)^2, 0)^2 / max(d^2, s^2) (s the source size, R the
//   range), the specular scale on the specular term only (FWD 830-892), the spot's cone
//   pow(saturate((cos - lo) / (hi - lo)), exponent) and its IES profile on both terms (FWD 1620-1744);
// - the deferred resolve `cs_fFragmentDeferredShadingTiled_ShadingUAV0_1_0_0_0_1_1_1_1` (in the exe; work/paint/exeblobs.py):
//   the roughness clamped to [0.05, 1], the G-buffer's flags byte (pDeferredFlags x 255: 4 no light from the star where no
//   shadow map says otherwise, 32 lit from behind as from the front, 64 the specular colour grey from its red, 128 no
//   occlusion), and the occlusion: the diffuse terms times 1 - (1 - ao) g_diffuseAOControl, the specular ones times
//   1 - (1 - ao) g_specularAOControl, the star's, the lamps' and the ambient alike (both controls are the engine's globals,
//   not found: taken as 1, an open item).
//
// What this game feeds it instead of Elite's scene: the star and the sky are the sandbox's light record at the player's
// place (sandbox/light.rs `PropLight`, read as sandbox_prop.wesl reads it: the star's light with its terrain, cloud and
// eclipse shadow already in it, the sky light, the flat ambient, the street lamps by night, the haze). Elite's ambient is
// image-based (its light probes); nothing here holds SpaceEngine's surroundings as probes, so the diffuse ambient is the
// light record's sky taken as the ground takes it (lit_surface), and the specular ambient is OURS, as plain as it can be:
// the same sky irradiance seen along the reflected ray, over pi, times the usual approximation of the GGX model's
// environment BRDF (`env_brdf`), so it falls with roughness as the direct light's highlight does. The ship's own lamps come
// from the lights buffer (A6's contract: `ShipLights`, in the ship's space), each lamp's colour already carrying its own
// exposure factor (worked out on the CPU, as Elite does).

// ---- The known gaps ----

// Elite's scene exposure inputs (ARCHITECTURE.md A6, the known gap), none of whose writers was found
// (re/elite/notes/agent_paint.md, "sceneValue"): one small buffer bound to every ship material, written in place when a value
// changes (material.rs `ShipScene`; lights.rs works each out from this game's exposure and gives the rule). Each shader
// reads exactly the global its disassembly reads, where it reads it.
struct ShipScene {
    // The lighting manager's scene value (the lights task uses it on the CPU for the lamps' factors; no shader here reads
    // it yet).
    scene_ev: f32,
    // gGlobalSelfIllumScale (cb1[1].w): the emissive families' glow.
    self_illum_scale: f32,
    // gEmissiveMultiplier (cb1[61].z): the emissive rescaling's scene value, pEmissiveRescalingType 0.
    emissive_multiplier: f32,
    // The auto-exposure texel gAverageSceneLuminanceTexture holds: the rescaling's scene value x 0.125, types 1 and 2 (which
    // read its texels 0 and 1: one value stands in for both).
    luminance: f32,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(15) var<storage, read> ship_scene: ShipScene;

// What Elite's heat effect (ShipHeatEffectActivity) writes over its material element's attributes every frame: rgb
// pEmissiveColour and gStandardMapScales, each lerp(cold, hot, glow) (material.rs `HeatGlow`, written by lights.rs).
struct ShipGlow {
    emissive: vec4<f32>,
    map_scales: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(16) var<storage, read> ship_glow: ShipGlow;

// Elite's renderer globals g_diffuseAOControl and g_specularAOControl (the resolve's cb1[24].z / .w) scale how much of the
// G-buffer's occlusion reaches the lighting. The engine's globals, not found (an open item): 1.0, the occlusion as authored.
const AO_DIFFUSE_CONTROL: f32 = 1.0;
const AO_SPECULAR_CONTROL: f32 = 1.0;

const PI: f32 = 3.141592;
const INV_PI: f32 = 0.318310;

// ---- The ship's lights (A6's contract; material.rs `ShipLights` writes it) ----

// All in the ship's space, metres, linear colour.
struct ShipLight {
    // xyz position, w range (m).
    pos_range: vec4<f32>,
    // rgb colour x level x the exposure factor, w source size (m).
    colour: vec4<f32>,
    // xyz the spot's axis, w the cosine of its half cone (-2: a point light).
    dir_cos: vec4<f32>,
    // x the cosine of the inner half angle, y K = pi / cone for the IES lookup, z 1 when the IES profile applies, w the
    // specular scale.
    spot: vec4<f32>,
};

struct ShipLights {
    count: u32,
    _pad: vec3<u32>,
    lights: array<ShipLight, 96>,
};

// The ship's space from the render frame's: p_ship = (dot(rows[0], p), dot(rows[1], p), dot(rows[2], p)) with p = (x, y,
// z, 1); written each frame only while a light is on (material.rs `ShipLights::write_pose`).
struct ShipPose {
    rows: array<vec4<f32>, 3>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(10) var<storage, read> ship_lights: ShipLights;
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var<storage, read> ship_pose: ShipPose;
// The spot lights' IES profile (tex/ies_emergency_elmre_lp220l_fxo.dds), level 0 only, as Elite's atlas is sampled.
@group(constants::MATERIAL_BIND_GROUP) @binding(12) var ies_tex: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(13) var ies_smp: sampler;

// ---- What a surface function gives: one texel of Elite's G-buffer ----

struct Surface {
    // RT1.rgb: the diffuse albedo (the base colour x (1 - metalness) x the diffuse scale, the dirt mixed in).
    diffuse: vec3<f32>,
    // RT2.rgb: the specular colour (F0: lerp(0.04, base, metalness) x the specular scale, the dirt mixed in).
    specular: vec3<f32>,
    // RT0.x.
    roughness: f32,
    // The shading normal, world space.
    normal: vec3<f32>,
    // RT3.rgb: the emitted light, its scales and rescaling applied.
    emissive: vec3<f32>,
    // RT2.w.
    ao: f32,
    // The overlay families' coverage (1 for the opaque ones).
    alpha: f32,
    // RT1.w: pDeferredFlags as stored (0..1; the resolve reads it x 255).
    flags: f32,
};

// The sandbox's light at the player's place (sandbox_prop.wesl's reading of `PropLight`).
struct Scene {
    sun_dir: vec3<f32>,
    sun: vec3<f32>,
    sky: vec3<f32>,
    flat_e: vec3<f32>,
    haze_t: vec3<f32>,
    haze_a: vec3<f32>,
    up: vec3<f32>,
    // The street lamps' light at the place, by night only (0 by day).
    town: vec3<f32>,
};

// pEmissiveRescaling* (FWD 2915-2969, the same in every emissive shader): r = (Enable, Type, EVOffset, EnableClamping),
// clamps = EVClamps. The scene value s is gEmissiveMultiplier for type 0, the luminance texture's texel x 0.125 for types 1
// and 2, 1 for any other type (`ShipScene`).
fn emissive_rescale(r: vec4<f32>, clamps: vec2<f32>) -> f32 {
    if r.x == 0.0 {
        return 1.0;
    }
    let kind = i32(round(r.y));
    var s = 1.0;
    if kind == 0 {
        s = ship_scene.emissive_multiplier;
    } else if kind == 1 || kind == 2 {
        s = ship_scene.luminance * 0.125;
    }
    if r.w != 0.0 {
        return exp2(min(max(log2(s) + r.z, clamps.x), clamps.y));
    }
    return s * exp2(r.z);
}

// Elite's specular term for one light, without N.L or the light's colour: F D Vis (FWD 606-626). `a2` is a^2, `nv`
// saturate(N.V).
fn specular_brdf(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, f0: vec3<f32>, a2: f32, nv: f32) -> vec3<f32> {
    let h = normalize(v + l);
    let nl = saturate(dot(n, l));
    let nh = saturate(dot(n, h));
    let a4 = a2 * a2;
    var d = (nh * a4 - nh) * nh + 1.000001;
    d = min(d, 1.0);
    let dist = a4 / (d * d * PI);
    let x = 1.0 - abs(dot(l, h));
    let x2 = x * x;
    let f = x * x2 * x2;
    let fresnel = (1.0 - f0) * f + f0;
    let vis = 1.0 / ((nl * (2.0 - a2) + a2) * (nv * (2.0 - a2) + a2));
    return fresnel * (dist * vis);
}

// The ship's lamps' diffuse and specular sums (before the occlusion).
struct Lit {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
};

// The resolve's two-sided rule (flag 32): a light behind the surface sees the normal turned round.
fn facing(n: vec3<f32>, l: vec3<f32>, two_sided: bool) -> vec3<f32> {
    return select(n, -n, two_sided && dot(l, n) < 0.0);
}

// The ship's lamps in [first, first + count) of the buffer, in the ship's space (FWD's point and spot loops, one list).
fn ship_lamps(s: Surface, world_pos: vec3<f32>, cam_pos: vec3<f32>, f0: vec3<f32>, a2: f32, first: u32, count: u32, two_sided: bool) -> Lit {
    var out: Lit;
    out.diffuse = vec3<f32>(0.0);
    out.specular = vec3<f32>(0.0);
    let end = min(first + count, min(ship_lights.count, 96u));
    if first >= end {
        return out;
    }
    let r = ship_pose.rows;
    let p = vec3<f32>(dot(r[0], vec4<f32>(world_pos, 1.0)), dot(r[1], vec4<f32>(world_pos, 1.0)), dot(r[2], vec4<f32>(world_pos, 1.0)));
    let c = vec3<f32>(dot(r[0], vec4<f32>(cam_pos, 1.0)), dot(r[1], vec4<f32>(cam_pos, 1.0)), dot(r[2], vec4<f32>(cam_pos, 1.0)));
    let n0 = normalize(vec3<f32>(dot(r[0].xyz, s.normal), dot(r[1].xyz, s.normal), dot(r[2].xyz, s.normal)));
    let v = normalize(c - p);
    for (var i = first; i < end; i++) {
        let lamp = ship_lights.lights[i];
        if all(lamp.colour.rgb == vec3<f32>(0.0)) {
            continue;
        }
        let to = lamp.pos_range.xyz - p;
        let d2 = dot(to, to);
        let range = lamp.pos_range.w;
        if d2 >= range * range || d2 <= 0.0 {
            continue;
        }
        let l = to * inverseSqrt(d2);
        // (FWD 1625-1671: the IES profile, x = 1 - K (1 - cos theta) across the profile's texel centres; under 0.001 the
        // lamp is skipped.)
        var shape = 1.0;
        if lamp.dir_cos.w > -1.5 {
            let axis = lamp.dir_cos.xyz;
            let cos_t = dot(axis, -l);
            if lamp.spot.z > 0.5 {
                let size = f32(textureDimensions(ies_tex).x);
                let u0 = 0.5 / size;
                let u1 = (size - 0.5) / size;
                let u = u1 - (u1 - u0) * lamp.spot.y * (1.0 - saturate(cos_t));
                let ies = textureSampleLevel(ies_tex, ies_smp, vec2<f32>(u, 0.5), 0.0).r;
                if ies < 0.001 {
                    continue;
                }
                shape = ies;
            }
            // (The cone's exponent is light +0x120, 1.0 on every spot of the ship; A6's record holds none.)
            let lo = lamp.dir_cos.w;
            let hi = lamp.spot.x;
            shape *= saturate((cos_t - lo) / (hi - lo));
        }
        let n = facing(n0, l, two_sided);
        let nv = saturate(dot(n, v));
        let nl = saturate(dot(n, l));
        let s2 = lamp.colour.w * lamp.colour.w;
        let m = max(d2, s2);
        let w = max(1.0 - (m / (range * range)) * (m / (range * range)), 0.0);
        let att = w * w / m * shape;
        // (The colour carries the lamp's level and its exposure factor: A6's contract.)
        let e = lamp.colour.rgb;
        out.diffuse += s.diffuse * (nl * INV_PI) * att * e;
        out.specular += specular_brdf(n, v, l, f0, a2, nv) * nl * att * lamp.spot.w * e;
    }
    return out;
}

// One texel of Elite's G-buffer lit: the star, the sky, the street lamps, the ship's lamps in [first, first + count), the
// emitted light, through the haze. `cam_pos` and `world_pos` are in the render frame.
fn shade(s: Surface, world_pos: vec3<f32>, cam_pos: vec3<f32>, sc: Scene, first: u32, count: u32) -> vec3<f32> {
    let flags = u32(s.flags * 255.0 + 0.5);
    var f0 = s.specular;
    var ao_d = 1.0;
    var ao_s = 1.0;
    if (flags & 128u) == 0u {
        if (flags & 64u) != 0u {
            f0 = vec3<f32>(f0.r);
        }
        ao_d = 1.0 - (1.0 - s.ao) * AO_DIFFUSE_CONTROL;
        ao_s = 1.0 - (1.0 - s.ao) * AO_SPECULAR_CONTROL;
    }
    let two_sided = (flags & 32u) != 0u;
    let a = min(max(s.roughness, 0.05), 1.0);
    let a2 = a * a;
    let v = normalize(cam_pos - world_pos);
    var diffuse = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    // The star (Elite's directional light; flag 4: unlit by it where no shadow map says otherwise, and the ship has none).
    if (flags & 4u) == 0u {
        let l = sc.sun_dir;
        let n = facing(s.normal, l, two_sided);
        let nl = saturate(dot(n, l));
        let nv = saturate(dot(n, v));
        diffuse += s.diffuse * (nl * INV_PI) * sc.sun;
        specular += specular_brdf(n, v, l, f0, a2, nv) * nl * sc.sun;
    }
    // The sky (ours, as the ground takes it).
    let n = s.normal;
    let sky_e = sc.sky * (0.5 + 0.5 * dot(n, sc.up)) + sc.flat_e;
    diffuse += s.diffuse * INV_PI * sky_e;
    let rv = reflect(-v, n);
    specular += env_brdf(f0, a, saturate(dot(n, v))) * INV_PI * (sc.sky * (0.5 + 0.5 * dot(rv, sc.up)) + sc.flat_e);
    // The ship's lamps.
    let lamps = ship_lamps(s, world_pos, cam_pos, f0, a2, first, count, two_sided);
    let c = (diffuse + lamps.diffuse) * ao_d + (specular + lamps.specular) * ao_s + s.emissive;
    // The street lamps from above by night, after the haze as the props take them (sandbox_prop.wesl).
    let town = s.diffuse * sc.town * (0.25 + 0.2 * max(dot(n, sc.up), 0.0)) * ao_d;
    return c * sc.haze_t + sc.haze_a + town;
}

// OURS (the sky's reflection, `shade`): the GGX model's environment BRDF in its usual analytic approximation (Karis,
// "Physically Based Shading on Mobile", 2014: a fit of the split-sum's scale and bias over roughness and N.V), the
// roughness `a` taken as the direct light's (alpha = a^2). Rough surfaces lose the grazing sheen a bare Schlick term gives.
fn env_brdf(f0: vec3<f32>, a: f32, nv: f32) -> vec3<f32> {
    let r = a * vec4<f32>(-1.0, -0.0275, -0.572, 0.022) + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

// The detail stamp's forward stand-in (sidewinder_surface_hull.wesl's header): how much the stamped normal `stamped` and
// the occlusion `ao` change the diffuse light on the surface under it, whose normal is `under`: (the star's N.L and the
// sky's light with the stamp, times the occlusion) over (the same without it). The pixel under it is multiplied by this.
fn stamp_factor(under: vec3<f32>, stamped: vec3<f32>, ao: f32, sc: Scene) -> f32 {
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    let ao_d = 1.0 - (1.0 - ao) * AO_DIFFUSE_CONTROL;
    let with_stamp = dot(sc.sun * saturate(dot(stamped, sc.sun_dir)) + sc.sky * (0.5 + 0.5 * dot(stamped, sc.up)) + sc.flat_e, lum) * ao_d;
    let without = dot(sc.sun * saturate(dot(under, sc.sun_dir)) + sc.sky * (0.5 + 0.5 * dot(under, sc.up)) + sc.flat_e, lum);
    return select(1.0, with_stamp / without, without > 1.0e-6);
}
