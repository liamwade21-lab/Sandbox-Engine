// Soft eclipse shadows as SE's eclipse_common.glh casts them: each caster hides `EclipseValue` of the
// light's disc (the covered share, its penumbra shaped by a smoothstep), the light left is the
// product of what each leaves, raised to SE's shadow gamma 1 / 1.6.

const PI: f32 = 3.14159265359;

// SE's ECLIPSE_SHADOW_GAMMA (and ECLIPSE_SHADOW_GAMMA_RINGS3D).
const ECLIPSE_SHADOW_GAMMA: f32 = 1.0 / 1.6;

// One light's casters (eclipse.rs `EclipseLight`): xyz centre relative to the lit body (render
// orientation, lit-body radii), w radius (0 ends the list); info.x the light's angular radius.
struct EclipseLight {
    casters: array<vec4<f32>, 8>,
    info: vec4<f32>,
};

// SE's EclipseValue: the share of a light of angular radius `rl` hidden by a caster of angular radius
// `rc` whose centre is `d` away.
fn eclipse_value(rl: f32, rc: f32, d: f32) -> f32 {
    let sum = rl + rc;
    if d >= sum {
        return 0.0;
    }
    var min_r = rc;
    var max_phase = 1.0;
    if rl < rc {
        min_r = rl;
    } else if rl < 0.001 {
        max_phase = (rc * rc) / (rl * rl);
    } else {
        max_phase = (1.0 - cos(rc)) / (1.0 - cos(rl));
    }
    if d <= max(rl, rc) - min_r {
        return max_phase;
    }
    let diff = abs(rl - rc);
    return max_phase * smoothstep(1.0, 0.0, clamp((d - diff) / (sum - diff), 0.0, 1.0));
}

// What a caster at `c` (radius `r`) leaves of a light from `l` at `p`: SE's EclipseShadowAuto angle
// (acos far from the axis, asin of the cross product near it).
fn caster_light(c: vec3<f32>, r: f32, p: vec3<f32>, l: vec3<f32>, rl: f32) -> f32 {
    let v = c - p;
    // Behind the point, or wide of the light by more than both radii (sin(rc + rl) <= sin rc + sin rl):
    // nothing hidden, as EclipseValue gives, without the trigonometry.
    let along = dot(v, l);
    let vv = dot(v, v);
    let reach = r + sqrt(vv) * sin(rl);
    if along <= 0.0 || vv - along * along >= reach * reach {
        return 1.0;
    }
    let inv = inverseSqrt(max(vv, 1e-30));
    let rc = asin(clamp(r * inv, 0.0, 1.0));
    let dir = v * inv;
    let cosa = dot(l, dir);
    var a = asin(clamp(length(cross(l, dir)), 0.0, 1.0));
    if cosa < 0.95 {
        a = acos(clamp(cosa, 0.0, 1.0));
    }
    return clamp(1.0 - eclipse_value(rl, rc, a), 0.0, 1.0);
}

// One caster slot (radius 0: unused).
fn caster_slot(c: vec4<f32>, p: vec3<f32>, l: vec3<f32>, rl: f32) -> f32 {
    if c.w == 0.0 {
        return 1.0;
    }
    return caster_light(c.xyz, abs(c.w), p, l, rl);
}

// SE's EclipseShadowAuto: the share of the light from `l` hidden at `p` (lit-body radii from its
// centre), gamma corrected; 0 unshadowed. The slots are read by constant index (a loop over a local
// copy of the array is spilled to memory by the shader compilers).
fn eclipse_shadow(e: EclipseLight, p: vec3<f32>, l: vec3<f32>) -> f32 {
    let rl = max(e.info.x, 1e-7);
    var lit = caster_slot(e.casters[0], p, l, rl) * caster_slot(e.casters[1], p, l, rl);
    lit *= caster_slot(e.casters[2], p, l, rl) * caster_slot(e.casters[3], p, l, rl);
    if e.info.y > 4.5 {
        lit *= caster_slot(e.casters[4], p, l, rl) * caster_slot(e.casters[5], p, l, rl);
        lit *= caster_slot(e.casters[6], p, l, rl) * caster_slot(e.casters[7], p, l, rl);
    }
    return 1.0 - pow(lit, ECLIPSE_SHADOW_GAMMA);
}

// Starlight reaching `p` (lit-body radii, relative to its centre) from direction `l` through one
// light's casters: SE's EclipseShadowFar on the surface and clouds, Shadow *= 1 - eclipse.
fn eclipse_light(e: EclipseLight, p: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(1.0 - eclipse_shadow(e, p, l));
}
