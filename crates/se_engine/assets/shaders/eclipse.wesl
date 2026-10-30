// Soft eclipse shadows: how much of the star's disc is hidden by up to four
// occluding bodies, seen from a point on (or above) the lit body.

const PI: f32 = 3.14159265359;

struct Eclipse {
    // xyz: occluder centre relative to the lit body's centre (render orientation,
    // in lit-body radii); w: occluder radius in the same units, negative when the
    // occluder has an atmosphere (its umbra is lit by red refracted light).
    occ: array<vec4<f32>, 4>,
    // x: star angular radius (radians), y: number of occluders.
    info: vec4<f32>,
};

// Fraction of a disc of angular radius `rs` covered by a disc of radius `ro`
// whose centre is `d` away.
fn disc_overlap(rs: f32, ro: f32, d: f32) -> f32 {
    if d >= rs + ro {
        return 0.0;
    }
    if d <= abs(ro - rs) {
        if ro >= rs {
            return 1.0;
        }
        return (ro * ro) / (rs * rs);
    }
    let a1 = rs * rs * acos(clamp((d * d + rs * rs - ro * ro) / (2.0 * d * rs), -1.0, 1.0));
    let a2 = ro * ro * acos(clamp((d * d + ro * ro - rs * rs) / (2.0 * d * ro), -1.0, 1.0));
    let k = sqrt(max((-d + rs + ro) * (d + rs - ro) * (d - rs + ro) * (d + rs + ro), 0.0));
    return clamp((a1 + a2 - 0.5 * k) / (PI * rs * rs), 0.0, 1.0);
}

// Starlight reaching `p` (lit-body radii, relative to its centre) from direction `l`.
fn eclipse_light(e: Eclipse, p: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(1.0);
    var occ = e.occ;
    let n = i32(e.info.y);
    let rs = max(e.info.x, 1e-7);
    for (var i = 0; i < 4; i++) {
        if i >= n {
            break;
        }
        let o = occ[i];
        let v = o.xyz - p;
        let dist = length(v);
        let r = abs(o.w);
        if dist <= r || dot(v, l) <= 0.0 {
            continue;
        }
        let ro = asin(min(r / dist, 1.0));
        let dir = v / dist;
        let sep = atan2(length(cross(dir, l)), dot(dir, l));
        let f = disc_overlap(rs, ro, sep);
        var residual = vec3<f32>(0.0);
        if o.w < 0.0 {
            residual = vec3<f32>(0.1, 0.03, 0.006);
        }
        light *= vec3<f32>(1.0) - f * (vec3<f32>(1.0) - residual);
    }
    return light;
}
