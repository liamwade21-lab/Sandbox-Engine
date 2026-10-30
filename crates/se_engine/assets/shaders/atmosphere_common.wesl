// Shared atmospheric scattering functions (Bruneton & Neyret 2008 tables as shipped by SpaceEngine).
//
// UNITS AND CONVENTIONS
//   * All lengths are in TABLE UNITS = kilometres of the `.atm` model, and all positions are
//     PLANET-CENTRED: a point on the ground has length ~ `radii.x` (Rg), the top of the atmosphere is
//     at `radii.y` (Rt). Convert render space to table space with
//         x_table = (x_render - planet_center_render) * (Rt / top_radius_render)
//     (uniform scale; the body is scaled so its atmosphere top maps onto the table's Rt).
//   * Directions (`v` view, `s` sun) are unit vectors in the same (planet-centred) axes.
//   * r = |x|, mu = dot(x, v) / r (view zenith cosine), mu_s = dot(x, s) / r (sun zenith cosine),
//     nu = dot(v, s).
//   * Radiance returned by the *_radiance / inscatter functions is PER UNIT SUN IRRADIANCE (the
//     tables were computed for a sun of irradiance 1); multiply by the sun's irradiance (rgb) in your
//     scene's radiance units. `atm_irradiance` is also per unit sun irradiance.
//   * Tables (verified against numerical integration, see docs/atmosphere.md):
//       transmittance  2D 256 (mu) x 64 (r):     u = atan((mu + 0.15) / 1.15 * tan(1.5)) / 1.5,
//                                                v = sqrt((r - Rg) / (Rt - Rg))
//       irradiance     2D 64 (mu_s) x 16 (r):    u = (mu_s + 0.2) / 1.2, v = (r - Rg) / (Rt - Rg)
//       inscatter      3D (16 nu blocks * 32 mu_s) x 64 mu x 32 r, rgb = Rayleigh + multiple scattering
//                      without phase function, a = single Mie scattering (red channel).
//   * All textures are sampled with `textureSampleLevel` (safe in non-uniform control flow) and must be
//     filterable (Rgba16Float) with a linear clamp-to-edge sampler.

const ATM_PI: f32 = 3.14159265358979;
const ATM_RES_R: f32 = 32.0;
const ATM_RES_MU: f32 = 64.0;
const ATM_RES_MU_S: f32 = 32.0;
const ATM_RES_NU: f32 = 16.0;

// Layout mirrors `AtmosphereParamsUniform` in src/atmosphere.rs.
struct AtmParams {
    // x: Rg (ground radius, km), y: Rt (top radius, km), z: Rayleigh scale height (km), w: Mie scale height (km)
    radii: vec4<f32>,
    // x: Mie asymmetry g, y: Mie twilight fade (mu_s range over which Mie fades in), zw: unused
    mie: vec4<f32>,
    // Rayleigh scattering coefficient (1/km), rgb
    beta_r: vec4<f32>,
    // Mie scattering coefficient (1/km), rgb
    beta_m_sca: vec4<f32>,
    // Mie extinction coefficient (1/km), rgb
    beta_m_ext: vec4<f32>,
    // The body's catalog look: x Opacity (SE's density: every transmittance mixed towards 1 by it),
    // y Hue (turns) and z Saturation (factor) of the scattered light, w SkyLight.
    look: vec4<f32>,
};

// SE lights the ground's ambient (sky irradiance, day ambient) with the star's own light
// (hdrAtmoIllumBright 1) and its sunlit side with five times it (hdrPlanetSurfaceBright 5, the
// port's sun irradiance scale): the ambient's share of the latter.
const ATM_AMBIENT_SCALE: f32 = 0.2;

struct PathScatter {
    // Radiance scattered towards the viewer along the path (per unit sun irradiance), phase functions applied.
    inscatter: vec3<f32>,
    // Transmittance of the path itself.
    transmittance: vec3<f32>,
};

struct SkyRadiance {
    // Sky radiance along the ray (per unit sun irradiance). For rays that hit the ground this is the
    // grazing-horizon radiance (the surface shader owns those pixels, see `atm_ground_atmosphere`).
    radiance: vec3<f32>,
    // Transmittance from the camera to space along the ray (for attenuating the sun disc / stars).
    transmittance: vec3<f32>,
    // Distance (km) to the ground if the ray hits the table's ground sphere, else -1.
    ground_dist: f32,
};

// ---------------------------------------------------------------------------------------------
// Phase functions
// ---------------------------------------------------------------------------------------------

fn atm_phase_rayleigh(nu: f32) -> f32 {
    return 3.0 / (16.0 * ATM_PI) * (1.0 + nu * nu);
}

fn atm_phase_mie(g: f32, nu: f32) -> f32 {
    // Cornette-Shanks style phase function used by the original precomputation.
    return 1.5 / (4.0 * ATM_PI) * (1.0 - g * g) * pow(max(1.0 + g * g - 2.0 * g * nu, 1e-4), -1.5) * (1.0 + nu * nu) / (2.0 + g * g);
}

// ---------------------------------------------------------------------------------------------
// Transmittance and irradiance
// ---------------------------------------------------------------------------------------------

fn atm_transmittance_uv(p: AtmParams, r: f32, mu: f32) -> vec2<f32> {
    let u_r = sqrt(clamp((r - p.radii.x) / (p.radii.y - p.radii.x), 0.0, 1.0));
    let u_mu = atan((mu + 0.15) / 1.15 * tan(1.5)) / 1.5;
    return vec2<f32>(u_mu, u_r);
}

// Transmittance from altitude r along direction mu up to the top of the atmosphere, assuming the ray
// does not hit the ground (rays below the horizon return ~0).
fn atm_transmittance(p: AtmParams, tex_t: texture_2d<f32>, smp: sampler, r: f32, mu: f32) -> vec3<f32> {
    return mix(vec3<f32>(1.0), textureSampleLevel(tex_t, smp, atm_transmittance_uv(p, r, mu), 0.0).rgb, p.look.x);
}

// Transmittance from point (r) towards the sun; zero when the sun is below the geometric horizon
// (softened over the sun's angular size).
fn atm_transmittance_sun(p: AtmParams, tex_t: texture_2d<f32>, smp: sampler, r_in: f32, mu_s: f32) -> vec3<f32> {
    let rg = p.radii.x;
    let r = max(r_in, rg);
    let mu_h = -sqrt(max((r - rg) * (r + rg), 0.0)) / r;
    let vis = smoothstep(mu_h - 0.005, mu_h + 0.005, mu_s);
    return atm_transmittance(p, tex_t, smp, r, mu_s) * vis;
}

// Sky irradiance on a horizontal surface at radius r (per unit sun irradiance); excludes the direct sun.
fn atm_irradiance(p: AtmParams, tex_e: texture_2d<f32>, smp: sampler, r: f32, mu_s: f32) -> vec3<f32> {
    let uv = vec2<f32>((mu_s + 0.2) / 1.2, (r - p.radii.x) / (p.radii.y - p.radii.x));
    return textureSampleLevel(tex_e, smp, uv, 0.0).rgb;
}

// Closed-form optical depth of an exponential atmosphere (scale height h) along a straight path of
// length d starting at radius r with direction cosine mu. Ignores the ground.
fn atm_optical_depth(rg: f32, h: f32, r: f32, mu: f32, d: f32) -> f32 {
    let a = sqrt((0.5 / h) * r);
    let a0 = a * mu;
    let a1 = a * (mu + d / r);
    let s0 = select(-1.0, 1.0, a0 >= 0.0);
    let s1 = select(-1.0, 1.0, a1 >= 0.0);
    var x = 0.0;
    if s1 > s0 {
        x = exp(a0 * a0);
    }
    let y0 = s0 / (2.3193 * abs(a0) + sqrt(1.52 * a0 * a0 + 4.0));
    let y1 = s1 / (2.3193 * abs(a1) + sqrt(1.52 * a1 * a1 + 4.0)) * exp(-d / h * (d / (2.0 * r) + mu));
    return sqrt(6.2831 * h * r) * exp((rg - r) / h) * (x + y0 - y1);
}

// Transmittance along a finite straight path (length d km) from radius r, direction cosine mu.
// Accurate near the horizon, unlike texture ratios.
fn atm_path_transmittance(p: AtmParams, r: f32, mu: f32, d: f32) -> vec3<f32> {
    let rg = p.radii.x;
    let od_r = atm_optical_depth(rg, p.radii.z, r, mu, d);
    let od_m = atm_optical_depth(rg, p.radii.w, r, mu, d);
    return mix(vec3<f32>(1.0), exp(-(p.beta_r.rgb * od_r + p.beta_m_ext.rgb * od_m)), p.look.x);
}

// SE's AtmoColAdjust (atmo_common.glh, hsl.glh): the scattered light's hue turned by `look.y` and its
// saturation scaled by `look.z`, in SE's HSV.
fn atm_color_adjust(p: AtmParams, c: vec3<f32>) -> vec3<f32> {
    if p.look.y == 0.0 && p.look.z == 1.0 {
        return c;
    }
    let k = vec4<f32>(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    let a = mix(vec4<f32>(c.bg, k.wz), vec4<f32>(c.gb, k.xy), step(c.b, c.g));
    let q = mix(vec4<f32>(a.xyw, c.r), vec4<f32>(c.r, a.yzx), step(a.x, c.r));
    let d = q.x - min(q.w, q.y);
    let h = clamp(abs(q.z + (q.w - q.y) / (6.0 * d + 1.0e-10)), 0.0, 1.0) + p.look.y;
    let s = clamp(d / (q.x + 1.0e-10) * p.look.z, 0.0, 1.0);
    let k2 = vec4<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    let rgb = abs(fract(vec3<f32>(h) + k2.xyz) * 6.0 - k2.www);
    return q.x * mix(k2.xxx, clamp(rgb - k2.xxx, vec3<f32>(0.0), vec3<f32>(1.0)), s);
}

// ---------------------------------------------------------------------------------------------
// 4D inscatter table
// ---------------------------------------------------------------------------------------------

fn atm_inscatter_lookup(p: AtmParams, tex_s: texture_3d<f32>, smp: sampler,
                        r_in: f32, mu_in: f32, mu_s: f32, nu: f32) -> vec4<f32> {
    let rg = p.radii.x;
    let rt = p.radii.y;
    let r = clamp(r_in, rg + 0.005, rt);
    let mu = clamp(mu_in, -1.0, 1.0);
    let hh = sqrt((rt - rg) * (rt + rg));
    let rho = sqrt(max((r - rg) * (r + rg), 0.0));
    let rmu = r * mu;
    let delta = rmu * rmu - (r - rg) * (r + rg);
    var cst: vec4<f32>;
    if rmu < 0.0 && delta > 0.0 {
        // ray hits the ground: lower half of the mu axis
        cst = vec4<f32>(1.0, 0.0, 0.0, 0.5 - 0.5 / ATM_RES_MU);
    } else {
        cst = vec4<f32>(-1.0, hh * hh, hh, 0.5 + 0.5 / ATM_RES_MU);
    }
    let u_r = 0.5 / ATM_RES_R + rho / hh * (1.0 - 1.0 / ATM_RES_R);
    let u_mu = cst.w + (rmu * cst.x + sqrt(max(delta + cst.y, 0.0))) / max(rho + cst.z, 1e-6) * (0.5 - 1.0 / ATM_RES_MU);
    let u_mu_s = 0.5 / ATM_RES_MU_S
        + max((1.0 - exp(-3.0 * mu_s - 0.6)) / (1.0 - exp(-3.6)), 0.0) * (1.0 - 1.0 / ATM_RES_MU_S);

    // SE's texture4D (atmo_common.glh): 16 nu blocks from nu = -1 (block 0) to +1 (block 15), each
    // 32 mu_s wide, in 512-texel rows of 64 mu. (Read as 256 x 128 the rows looked like mirrored
    // pairs, and the old even-rows-with-minus-nu lookup returned S(-nu): toward a star behind the
    // planet it read the dark backward table, so no light came round the limb.)
    let lerp_nu = (nu + 1.0) * 0.5 * (ATM_RES_NU - 1.0);
    let u_nu = floor(lerp_nu);
    let f = lerp_nu - u_nu;
    let a = textureSampleLevel(tex_s, smp, vec3<f32>((u_nu + u_mu_s) / ATM_RES_NU, u_mu, u_r), 0.0);
    let b = textureSampleLevel(tex_s, smp, vec3<f32>((u_nu + u_mu_s + 1.0) / ATM_RES_NU, u_mu, u_r), 0.0);
    return a * (1.0 - f) + b * f;
}

// Reconstruct the Mie colour from the stored red-channel single-scattering term.
fn atm_get_mie(p: AtmParams, ray_mie: vec4<f32>) -> vec3<f32> {
    let bm = p.beta_m_sca.rgb / max(p.beta_m_sca.r, 1e-8);
    return ray_mie.rgb * ray_mie.w / max(ray_mie.r, 1e-4) * (p.beta_r.r / p.beta_r.rgb) * bm;
}

// ---------------------------------------------------------------------------------------------
// Path scattering
// ---------------------------------------------------------------------------------------------

// Light scattered into the viewer along the path starting at x (km, planet-centred; may be outside the
// atmosphere) in direction v (unit) for length t km; t < 0 means "to the top of the atmosphere".
// Per unit sun irradiance. Handles starting in space (moves to the atmosphere boundary).
fn atm_path(p: AtmParams, tex_s: texture_3d<f32>, tex_t: texture_2d<f32>, smp: sampler,
            x_in: vec3<f32>, v: vec3<f32>, s: vec3<f32>, t_in: f32) -> PathScatter {
    var res: PathScatter;
    res.inscatter = vec3<f32>(0.0);
    res.transmittance = vec3<f32>(1.0);
    let rg = p.radii.x;
    let rt = p.radii.y;
    var x = x_in;
    var t = t_in;
    var r = length(x);
    var mu = dot(x, v) / r;
    let disc = r * r * (mu * mu - 1.0) + rt * rt;
    if disc < 0.0 {
        return res; // ray misses the atmosphere
    }
    let d = -r * mu - sqrt(disc);
    if d > 0.0 {
        // in space and the ray enters the atmosphere: move to the entry point
        x = x + d * v;
        if t >= 0.0 {
            t = t - d;
            if t <= 0.0 {
                return res; // target lies before the atmosphere
            }
        }
        mu = (r * mu + d) / rt;
        r = rt;
    } else if r > rt {
        return res; // outside and moving away
    }

    let nu = dot(v, s);
    let mu_s = dot(x, s) / r;
    var ins = max(atm_inscatter_lookup(p, tex_s, smp, r, mu, mu_s, nu), vec4<f32>(0.0));
    if t > 0.0 {
        let x0 = x + t * v;
        let r0 = length(x0);
        let mu0 = (r * mu + t) / r0;
        let mu_s0 = dot(x0, s) / r0;
        let att = atm_path_transmittance(p, r, mu, t);
        res.transmittance = att;
        // S[L](x) - T(x, x0) S[L](x0), for a target on the ground too (Bruneton skips it there, the
        // table's ground-bound rays ending at the ground): the lookups clamp both ends just above
        // the ground, and from a camera a few metres up (on Jupiter, where f32 puts the ground a few
        // metres either side of it) the skip flipped per triangle between a little haze and the
        // haze out to the table's horizon.
        {
            let att4 = vec4<f32>(att, att.r);
            ins = max(ins - att4 * atm_inscatter_lookup(p, tex_s, smp, r0, mu0, mu_s0, nu), vec4<f32>(0.0));
            // Avoid the imprecision right at the horizon by blending two nearby directions (SE's
            // inscatterGroundFix). For a target above the ground (a cloud) the blend spans the gap up to
            // the tangent of the target's sphere: rays there pass the ground horizon behind the target,
            // and the two halves of the table no longer agree on the long grazing path.
            let mu_h = -sqrt(max(1.0 - (rg / r) * (rg / r), 0.0));
            let mu_c = -sqrt(max(1.0 - (r0 / r) * (r0 / r), 0.0));
            let eps = max(0.004, mu_c - mu_h);
            if abs(mu - mu_h) < eps {
                let a = ((mu - mu_h) + eps) / (2.0 * eps);
                var mm = mu_h - eps;
                var rr0 = sqrt(r * r + t * t + 2.0 * r * t * mm);
                var mm0 = (r * mm + t) / rr0;
                var i0 = atm_inscatter_lookup(p, tex_s, smp, r, mm, mu_s, nu);
                var i1 = atm_inscatter_lookup(p, tex_s, smp, rr0, mm0, mu_s0, nu);
                let ins_a = max(i0 - att4 * i1, vec4<f32>(0.0));
                mm = mu_h + eps;
                rr0 = sqrt(r * r + t * t + 2.0 * r * t * mm);
                mm0 = (r * mm + t) / rr0;
                i0 = atm_inscatter_lookup(p, tex_s, smp, r, mm, mu_s, nu);
                i1 = atm_inscatter_lookup(p, tex_s, smp, rr0, mm0, mu_s0, nu);
                let ins_b = max(i0 - att4 * i1, vec4<f32>(0.0));
                ins = mix(ins_a, ins_b, a);
            }
        }
    } else {
        res.transmittance = atm_transmittance(p, tex_t, smp, r, mu);
    }
    // Mie scattering fades out while the sun is below the horizon (avoids a bright Mie halo at night).
    ins.w = ins.w * smoothstep(0.0, max(p.mie.y, 1e-4), mu_s);
    res.inscatter = atm_color_adjust(p, max(ins.rgb * atm_phase_rayleigh(nu) + atm_get_mie(p, ins) * atm_phase_mie(p.mie.x, nu), vec3<f32>(0.0)));
    return res;
}

// Sky radiance for a camera at `cam` (km, planet-centred) looking along `v`; sun direction `s`.
fn atm_sky_radiance(p: AtmParams, tex_s: texture_3d<f32>, tex_t: texture_2d<f32>, smp: sampler,
                    cam: vec3<f32>, v: vec3<f32>, s: vec3<f32>) -> SkyRadiance {
    var out: SkyRadiance;
    out.radiance = vec3<f32>(0.0);
    out.transmittance = vec3<f32>(1.0);
    out.ground_dist = -1.0;
    let rg = p.radii.x;
    let r = length(cam);
    let mu = dot(cam, v) / r;
    let delta = r * r * (mu * mu - 1.0) + rg * rg;
    var dir = v;
    if mu < 0.0 && delta >= 0.0 {
        // The ray hits the ground. The surface itself is drawn by the planet shader (which applies aerial
        // perspective through `atm_ground_atmosphere`), and the sky shell is depth-tested behind it. So
        // that no gaps appear where the two independently evaluated horizons disagree by a sub-pixel
        // amount, return the radiance of the grazing direction just above the horizon instead of zero.
        out.ground_dist = -r * mu - sqrt(delta);
        let up = cam / r;
        let mu_h = -sqrt(max((r - rg) * (r + rg), 0.0)) / r;
        let h = v - mu * up;
        let hl = max(length(h), 1e-6);
        let mu2 = mu_h + 0.002;
        dir = h / hl * sqrt(max(1.0 - mu2 * mu2, 0.0)) + up * mu2;
    }
    let ps = atm_path(p, tex_s, tex_t, smp, cam, dir, s, -1.0);
    out.radiance = ps.inscatter;
    out.transmittance = ps.transmittance;
    return out;
}

// ---------------------------------------------------------------------------------------------
// Ground helpers (for terrain / planet surface shaders)
// ---------------------------------------------------------------------------------------------

// Irradiance (rgb, in the units of `sun_irradiance`) on a surface at `pos` with unit `normal`, for a
// radiance of albedo / pi times it: attenuated direct sun plus sky light. `sun_vis` in [0,1] scales
// the direct term (shadows). The sky light is SE's ambient term (planet.glsl): the table's irradiance
// times SkyLight, not divided by pi, at the ambient's share of the light (`ATM_AMBIENT_SCALE`).
fn atm_surface_irradiance(p: AtmParams, tex_t: texture_2d<f32>, tex_e: texture_2d<f32>, smp: sampler,
                          pos: vec3<f32>, normal: vec3<f32>, sun_dir: vec3<f32>,
                          sun_irradiance: vec3<f32>, sun_vis: f32) -> vec3<f32> {
    let r = length(pos);
    let up = pos / r;
    let mu_s = dot(up, sun_dir);
    let t_sun = atm_transmittance_sun(p, tex_t, smp, r, mu_s);
    let e_sky = atm_irradiance(p, tex_e, smp, r, mu_s) * (p.look.w * ATM_PI * ATM_AMBIENT_SCALE);
    let direct = t_sun * max(dot(normal, sun_dir), 0.0) * sun_vis;
    let sky = e_sky * (0.5 + 0.5 * dot(normal, up));
    return sun_irradiance * (direct + sky);
}

// Apply aerial perspective to the radiance `surface` leaving the point `pos` towards `cam`:
// result = surface * T(path) + inscatter(path) * sun_irradiance. Positions in km, planet-centred.
// Works from inside the atmosphere and from space (the path starts at the atmosphere boundary).
fn atm_apply_aerial(p: AtmParams, tex_s: texture_3d<f32>, tex_t: texture_2d<f32>, smp: sampler,
                    surface: vec3<f32>, cam: vec3<f32>, pos: vec3<f32>, sun_dir: vec3<f32>,
                    sun_irradiance: vec3<f32>) -> vec3<f32> {
    let dv = pos - cam;
    let t = length(dv);
    if t < 1e-5 {
        return surface;
    }
    let v = dv / t;
    let ps = atm_path(p, tex_s, tex_t, smp, cam, v, sun_dir, t);
    return surface * ps.transmittance + sun_irradiance * ps.inscatter;
}

// Full Lambertian surface lighting + aerial perspective: `albedo` (diffuse reflectance, rgb) at `pos`
// with `normal`, lit by the attenuated sun (irradiance `sun_irradiance`, scene radiance units) and
// sky light, seen from `cam`. Returns the radiance arriving at the camera.
// `sun_vis` in [0,1] scales the direct sun (cast shadows / eclipses); pass 1.0 for none.
fn atm_ground_atmosphere(p: AtmParams, tex_s: texture_3d<f32>, tex_t: texture_2d<f32>, tex_e: texture_2d<f32>,
                         smp: sampler, albedo: vec3<f32>, normal: vec3<f32>, sun_dir: vec3<f32>,
                         sun_irradiance: vec3<f32>, sun_vis: f32,
                         cam: vec3<f32>, pos: vec3<f32>) -> vec3<f32> {
    let e = atm_surface_irradiance(p, tex_t, tex_e, smp, pos, normal, sun_dir, sun_irradiance, sun_vis);
    let lit = albedo / ATM_PI * e;
    return atm_apply_aerial(p, tex_s, tex_t, smp, lit, cam, pos, sun_dir, sun_irradiance);
}
