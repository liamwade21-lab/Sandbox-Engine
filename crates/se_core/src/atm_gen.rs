//! SpaceEngine's generator of atmosphere scattering tables (`seAtmosphereModelCache::RecreateModel`
//! FUN_14044a760, the GPU passes FUN_14044c8c0 with the `ag_*.glsl` shaders, which are Bruneton &
//! Neyret's 2008 precomputation at SE's resolutions), run on the CPU for an `atmospheres.cfg` model
//! that has no `.atm` file.
//!
//! - Model keys (FUN_140451710): `RadiusGround`, `Height` (the top is ground + height), `RayleighH`,
//!   `MieH`, `MieG`, `MieFade`, `RayleighBeta` x `RayleighScale`, `MieScaBeta` x `MieScaScale`,
//!   `MieExtBeta` x `MieExtScale` (the shipped `.atm` headers hold the products).
//! - Passes (algorithm 4.1 of the paper): transmittance (500 trapezoid samples), direct ground
//!   irradiance, single Rayleigh and Mie scattering (50 samples), then orders 2..4 of multiple
//!   scattering (16 x 32 directions for J, 32 x 16 for the sky irradiance, 50 samples along rays),
//!   with the ground's reflectance 0.1. Layer `L` of the inscatter table sits at
//!   `r = sqrt(Rg^2 + (Rt^2 - Rg^2) (L/31)^2)` (+0.01 km at the ground, -0.001 at the top, FUN_14044d920).
//! - Tables are read with bilinear / trilinear clamped filtering, as the GPU does (in `f32`, so the
//!   results differ from SE's in the last digits).
//! - `analytic_transm` is SE's `AtmoAnalyticTransm`: transmittance between two points from the analytic
//!   optical depth instead of the table.

use crate::atm::{AtmParams, AtmTables, RES_MU, RES_MU_S, RES_NU, RES_R, SKY_H, SKY_W, TRANSMITTANCE_H, TRANSMITTANCE_W};

const TRANSMITTANCE_SAMPLES: usize = 500;
const INSCATTER_SAMPLES: usize = 50;
const IRRADIANCE_SAMPLES: usize = 32;
const SPHERICAL_SAMPLES: usize = 16;
const MIN_MU_S: f32 = 0.2;
const GROUND_REFLECTANCE: f32 = 0.1;
const PI: f32 = 3.141_592_657;
/// The ray limit radius above the top (Bruneton's `RL = Rt + 1` km).
const LIMIT_ABOVE_KM: f32 = 1.0;
const INSCATTER_W: usize = RES_MU_S * RES_NU;

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// One model of `atmospheres.cfg`.
pub fn parse_models(text: &str) -> Vec<(String, AtmParams)> {
    let doc = crate::sc::parse(text);
    doc.items
        .iter()
        .filter(|i| i.key.eq_ignore_ascii_case("Atmosphere"))
        .filter_map(|i| {
            let b = i.block()?;
            let name = i.name()?.to_string();
            let f = |k: &str, d: f64| b.f64(k).unwrap_or(d) as f32;
            let v = |k: &str| b.vec3(k).map(|v| v.map(|x| x as f32)).unwrap_or([0.0; 3]);
            let rg = f("RadiusGround", 6360.0);
            let (rs, ms, me) = (f("RayleighScale", 1.0), f("MieScaScale", 1.0), f("MieExtScale", 1.0));
            Some((
                name,
                AtmParams {
                    radius_ground: rg,
                    radius_top: rg + f("Height", 60.0),
                    rayleigh_h: f("RayleighH", 8.0),
                    mie_h: f("MieH", 2.0),
                    rayleigh_beta: v("RayleighBeta").map(|x| x * rs),
                    mie_sca_beta: v("MieScaBeta").map(|x| x * ms),
                    mie_ext_beta: v("MieExtBeta").map(|x| x * me),
                    mie_g: f("MieG", 0.8),
                    mie_fade: f("MieFade", 0.02),
                },
            ))
        })
        .collect()
}

/// A texture coordinate as the GPU's sampler takes it: NaN (the square root of a slightly negative
/// number near the ground) reads as 0, infinities clamp.
fn gpu_coord(c: f32) -> f32 {
    if c.is_nan() { 0.0 } else { c.clamp(-1e6, 1e6) }
}

/// A 2D RGB table read like an OpenGL texture (texel centres at `(i + 0.5) / n`, bilinear, clamped).
struct Tex2 {
    w: usize,
    h: usize,
    d: Vec<V3>,
}

impl Tex2 {
    fn at(&self, x: i64, y: i64) -> V3 {
        let x = x.clamp(0, self.w as i64 - 1) as usize;
        let y = y.clamp(0, self.h as i64 - 1) as usize;
        self.d[y * self.w + x]
    }
    fn sample(&self, u: f32, v: f32) -> V3 {
        let (u, v) = (gpu_coord(u), gpu_coord(v));
        let fx = u * self.w as f32 - 0.5;
        let fy = v * self.h as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let a = add(scale(self.at(x0, y0), 1.0 - tx), scale(self.at(x0 + 1, y0), tx));
        let b = add(scale(self.at(x0, y0 + 1), 1.0 - tx), scale(self.at(x0 + 1, y0 + 1), tx));
        add(scale(a, 1.0 - ty), scale(b, ty))
    }
}

/// The 4D inscatter table as SE's 3D texture (x = nu block * 32 + mu_s, y = mu, z = r).
struct Tex3 {
    d: Vec<V3>,
}

impl Tex3 {
    fn new() -> Self {
        Self { d: vec![[0.0; 3]; INSCATTER_W * RES_MU * RES_R] }
    }
    fn at(&self, x: i64, y: i64, z: i64) -> V3 {
        let x = x.clamp(0, INSCATTER_W as i64 - 1) as usize;
        let y = y.clamp(0, RES_MU as i64 - 1) as usize;
        let z = z.clamp(0, RES_R as i64 - 1) as usize;
        self.d[(z * RES_MU + y) * INSCATTER_W + x]
    }
    fn sample(&self, u: f32, v: f32, w: f32) -> V3 {
        let f = |c: f32, n: usize| {
            let p = gpu_coord(c) * n as f32 - 0.5;
            let i = p.floor();
            (i as i64, p - i)
        };
        let ((x0, tx), (y0, ty), (z0, tz)) = (f(u, INSCATTER_W), f(v, RES_MU), f(w, RES_R));
        let mut out = [0.0; 3];
        for (dz, wz) in [(0, 1.0 - tz), (1, tz)] {
            for (dy, wy) in [(0, 1.0 - ty), (1, ty)] {
                for (dx, wx) in [(0, 1.0 - tx), (1, tx)] {
                    out = add(out, scale(self.at(x0 + dx, y0 + dy, z0 + dz), wx * wy * wz));
                }
            }
        }
        out
    }
}

struct Model {
    rg: f32,
    rt: f32,
    rl: f32,
    hr: f32,
    hm: f32,
    beta_r: V3,
    beta_msca: V3,
    beta_mex: V3,
    mie_g: f32,
    analytic: bool,
}

impl Model {
    fn limit(&self, r: f32, mu: f32) -> f32 {
        let mut dout = -r * mu + (r * r * (mu * mu - 1.0) + self.rl * self.rl).sqrt();
        let delta2 = r * r * (mu * mu - 1.0) + self.rg * self.rg;
        if delta2 >= 0.0 {
            let din = -r * mu - delta2.sqrt();
            if din >= 0.0 {
                dout = dout.min(din);
            }
        }
        dout
    }

    fn transmittance_uv(&self, r: f32, mu: f32) -> (f32, f32) {
        let ur = ((r - self.rg) / (self.rt - self.rg)).sqrt();
        let umu = ((mu + 0.15) / 1.15 * 1.5f32.tan()).atan() / 1.5;
        (umu, ur)
    }

    fn transmittance(&self, t: &Tex2, r: f32, mu: f32) -> V3 {
        let (u, v) = self.transmittance_uv(r, mu);
        t.sample(u, v)
    }

    /// Transmittance between `r, mu` and the point `d` along the ray (no ground crossing).
    fn transmittance_d(&self, t: &Tex2, r: f32, mu: f32, d: f32) -> V3 {
        if self.analytic {
            return self.analytic_transmittance(r, mu, d);
        }
        let r1 = (r * r + d * d + 2.0 * r * mu * d).sqrt();
        let mu1 = (r * mu + d) / r1;
        let q = |a: V3, b: V3| [0, 1, 2].map(|i| (a[i] / b[i]).min(1.0));
        if mu > 0.0 { q(self.transmittance(t, r, mu), self.transmittance(t, r1, mu1)) } else { q(self.transmittance(t, r1, -mu1), self.transmittance(t, r, -mu)) }
    }

    fn optical_depth_analytic(&self, h: f32, r: f32, mu: f32, d: f32) -> f32 {
        let a = ((0.5 / h) * r).sqrt();
        let a01 = [a * mu, a * (mu + d / r)];
        let s = a01.map(|x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 });
        let sq = a01.map(|x| x * x);
        let x = if s[1] > s[0] { sq[0].exp() } else { 0.0 };
        let y0 = s[0] / (2.3193 * a01[0].abs() + (1.52 * sq[0] + 4.0).sqrt());
        let y1 = s[1] / (2.3193 * a01[1].abs() + (1.52 * sq[1] + 4.0).sqrt()) * (-d / h * (d / (2.0 * r) + mu)).exp();
        ((6.2831 * h) * r).sqrt() * ((self.rg - r) / h).exp() * (x + y0 - y1)
    }

    fn analytic_transmittance(&self, r: f32, mu: f32, d: f32) -> V3 {
        let (dr, dm) = (self.optical_depth_analytic(self.hr, r, mu, d), self.optical_depth_analytic(self.hm, r, mu, d));
        [0, 1, 2].map(|i| (-self.beta_r[i] * dr - self.beta_mex[i] * dm).exp())
    }

    fn irradiance(&self, e: &Tex2, r: f32, mu_s: f32) -> V3 {
        let ur = (r - self.rg) / (self.rt - self.rg);
        let ums = (mu_s + MIN_MU_S) / (1.0 + MIN_MU_S);
        e.sample(ums, ur)
    }

    fn texture4d(&self, t: &Tex3, r: f32, mu: f32, mu_s: f32, nu: f32) -> V3 {
        let h = (self.rt * self.rt - self.rg * self.rg).sqrt();
        let rho = (r * r - self.rg * self.rg).sqrt();
        let rmu = r * mu;
        let delta = rmu * rmu - r * r + self.rg * self.rg;
        let cst = if rmu < 0.0 && delta > 0.0 { [1.0, 0.0, 0.0, 0.5 - 0.5 / RES_MU as f32] } else { [-1.0, h * h, h, 0.5 + 0.5 / RES_MU as f32] };
        let ur = 0.5 / RES_R as f32 + rho / h * (1.0 - 1.0 / RES_R as f32);
        let umu = cst[3] + (rmu * cst[0] + (delta + cst[1]).sqrt()) / (rho + cst[2]) * (0.5 - 1.0 / RES_MU as f32);
        let ums = 0.5 / RES_MU_S as f32 + ((1.0 - (-3.0 * mu_s - 0.6).exp()) / (1.0 - (-3.6f32).exp())).max(0.0) * (1.0 - 1.0 / RES_MU_S as f32);
        let lerp = (nu + 1.0) / 2.0 * (RES_NU as f32 - 1.0);
        let unu = lerp.floor();
        let lerp = lerp - unu;
        let a = t.sample((unu + ums) / RES_NU as f32, umu, ur);
        let b = t.sample((unu + ums + 1.0) / RES_NU as f32, umu, ur);
        add(scale(a, 1.0 - lerp), scale(b, lerp))
    }

    fn phase_r(mu: f32) -> f32 {
        (3.0 / (16.0 * PI)) * (1.0 + mu * mu)
    }

    fn phase_m(&self, mu: f32) -> f32 {
        let g = self.mie_g;
        1.5 / (4.0 * PI) * (1.0 - g * g) * (1.0 + g * g - 2.0 * g * mu).powf(-1.5) * (1.0 + mu * mu) / (2.0 + g * g)
    }

    /// The layer's radius and its `dhdH` (FUN_14044d920).
    fn layer(&self, l: usize) -> (f32, [f32; 4]) {
        let (rg2, rt2) = (self.rg * self.rg, self.rt * self.rt);
        let t = l as f32 / (RES_R as f32 - 1.0);
        let off = if l == 0 { 0.01 } else if l == RES_R - 1 { -0.001 } else { 0.0 };
        let r = ((rt2 - rg2) * t * t + rg2).sqrt() + off;
        let rho = (r * r - rg2).sqrt();
        let hh = (rt2 - rg2).sqrt();
        (r, [self.rt - r, hh + rho, r - self.rg, rho])
    }

    /// (mu, mu_s, nu) of inscatter texel (x, y) of a layer (`getMuMuSNu`).
    fn mu_mus_nu(&self, r: f32, dhdh: [f32; 4], x: usize, y: usize) -> (f32, f32, f32) {
        let (x, y) = (x as f32, y as f32);
        let half = RES_MU as f32 / 2.0;
        let mu = if y < half {
            let d = 1.0 - y / (half - 1.0);
            let d = (d * dhdh[3]).max(dhdh[2]).min(dhdh[3] * 0.999);
            let mu = (self.rg * self.rg - r * r - d * d) / (2.0 * r * d);
            mu.min(-(1.0 - (self.rg / r) * (self.rg / r)).sqrt() - 0.001)
        } else {
            let d = (y - half) / (half - 1.0);
            let d = (d * dhdh[1]).max(dhdh[0]).min(dhdh[1] * 0.999);
            (self.rt * self.rt - r * r - d * d) / (2.0 * r * d)
        };
        let mus = (x % RES_MU_S as f32) / (RES_MU_S as f32 - 1.0);
        let mus = -(0.6 + (1.0 - mus * (1.0 - (-3.6f32).exp())).ln()) / 3.0;
        let nu = -1.0 + (x / RES_MU_S as f32).floor() / (RES_NU as f32 - 1.0) * 2.0;
        (mu, mus, nu)
    }
}

/// Fill every inscatter texel with `f(r, dhdH, mu, mu_s, nu)` on all threads.
fn per_texel(m: &Model, f: &(dyn Fn(f32, f32, f32, f32) -> V3 + Sync)) -> Tex3 {
    let mut out = Tex3::new();
    let layer_len = INSCATTER_W * RES_MU;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 32);
    let per = RES_R.div_ceil(threads);
    std::thread::scope(|s| {
        for (chunk_i, chunk) in out.d.chunks_mut(layer_len * per).enumerate() {
            s.spawn(move || {
                for (k, layer) in chunk.chunks_mut(layer_len).enumerate() {
                    let (r, dhdh) = m.layer(chunk_i * per + k);
                    for y in 0..RES_MU {
                        for x in 0..INSCATTER_W {
                            let (mu, mus, nu) = m.mu_mus_nu(r, dhdh, x, y);
                            layer[y * INSCATTER_W + x] = f(r, mu, mus, nu);
                        }
                    }
                }
            });
        }
    });
    out
}

/// Generate the tables of an atmosphere model as SE does.
pub fn generate(p: &AtmParams, analytic_transm: bool) -> AtmTables {
    let m = Model {
        rg: p.radius_ground,
        rt: p.radius_top,
        rl: p.radius_top + LIMIT_ABOVE_KM,
        hr: p.rayleigh_h,
        hm: p.mie_h,
        beta_r: p.rayleigh_beta,
        beta_msca: p.mie_sca_beta,
        beta_mex: p.mie_ext_beta,
        mie_g: p.mie_g,
        analytic: analytic_transm,
    };
    // Transmittance (ag_transmittance).
    let mut tr = Tex2 { w: TRANSMITTANCE_W, h: TRANSMITTANCE_H, d: vec![[0.0; 3]; TRANSMITTANCE_W * TRANSMITTANCE_H] };
    for j in 0..TRANSMITTANCE_H {
        for i in 0..TRANSMITTANCE_W {
            let ry = (j as f32 + 0.5) / TRANSMITTANCE_H as f32;
            let r = m.rg + ry * ry * (m.rt - m.rg);
            let mu = -0.15 + (1.5 * (i as f32 + 0.5) / TRANSMITTANCE_W as f32).tan() / 1.5f32.tan() * 1.15;
            let depth = |h: f32| -> f32 {
                if mu < -(1.0 - (m.rg / r) * (m.rg / r)).sqrt() {
                    return 1e9;
                }
                let dx = m.limit(r, mu) / TRANSMITTANCE_SAMPLES as f32;
                let mut yi = (-(r - m.rg) / h).exp();
                let mut sum = 0.0;
                for k in 1..=TRANSMITTANCE_SAMPLES {
                    let xj = k as f32 * dx;
                    let yj = (-((r * r + xj * xj + 2.0 * xj * r * mu).sqrt() - m.rg) / h).exp();
                    sum += (yi + yj) / 2.0 * dx;
                    yi = yj;
                }
                sum
            };
            let (dr, dm) = (depth(m.hr), depth(m.hm));
            tr.d[j * TRANSMITTANCE_W + i] = [0, 1, 2].map(|c| (-(m.beta_r[c] * dr + m.beta_mex[c] * dm)).exp());
        }
    }
    let irr_rmus = |i: usize, j: usize| -> (f32, f32) {
        let r = m.rg + j as f32 / (SKY_H as f32 - 1.0) * (m.rt - m.rg);
        let mus = (-MIN_MU_S + i as f32 / (SKY_W as f32 - 1.0) * (1.0 + MIN_MU_S)).clamp(-MIN_MU_S, 1.0);
        (r, mus)
    };
    // Direct ground irradiance deltaE (ag_irradiance1).
    let mut delta_e = Tex2 { w: SKY_W, h: SKY_H, d: vec![[0.0; 3]; SKY_W * SKY_H] };
    for j in 0..SKY_H {
        for i in 0..SKY_W {
            let (r, mus) = irr_rmus(i, j);
            delta_e.d[j * SKY_W + i] = scale(m.transmittance(&tr, r, mus), mus.max(0.0));
        }
    }
    // Single scattering deltaSR, deltaSM (ag_inscatter1).
    let single = |r: f32, mu: f32, mus: f32, nu: f32, mie: bool| -> V3 {
        let integrand = |t: f32| -> V3 {
            let ri = (r * r + t * t + 2.0 * r * mu * t).sqrt();
            let musi = (nu * t + mus * r) / ri;
            let ri = ri.max(m.rg);
            if musi >= -(1.0 - m.rg * m.rg / (ri * ri)).sqrt() {
                let ti = mul(m.transmittance_d(&tr, r, mu, t), m.transmittance(&tr, ri, musi));
                scale(ti, (-(ri - m.rg) / if mie { m.hm } else { m.hr }).exp())
            } else {
                [0.0; 3]
            }
        };
        let dx = m.limit(r, mu) / INSCATTER_SAMPLES as f32;
        let mut yi = integrand(0.0);
        let mut sum = [0.0; 3];
        for k in 1..=INSCATTER_SAMPLES {
            let yj = integrand(k as f32 * dx);
            sum = add(sum, scale(add(yi, yj), dx / 2.0));
            yi = yj;
        }
        mul(sum, if mie { m.beta_msca } else { m.beta_r })
    };
    let mut delta_sr = per_texel(&m, &|r, mu, mus, nu| single(r, mu, mus, nu, false));
    let delta_sm = per_texel(&m, &|r, mu, mus, nu| single(r, mu, mus, nu, true));
    // S = (deltaSR.rgb, deltaSM.r) (ag_copyInscatter1); E starts at 0 x deltaE (ag_copyIrradiance, k 0).
    let mut inscatter: Vec<f32> = delta_sr.d.iter().zip(&delta_sm.d).flat_map(|(r, m)| [r[0], r[1], r[2], m[0]]).collect();
    let mut irradiance = vec![0.0f32; SKY_W * SKY_H * 4];
    for order in 2..=4 {
        let first = order == 2;
        // deltaJ (ag_inscatterS).
        let dphi = PI / SPHERICAL_SAMPLES as f32;
        let dtheta = PI / SPHERICAL_SAMPLES as f32;
        let delta_j = per_texel(&m, &|r, mu, mus, nu| {
            let r = r.clamp(m.rg, m.rt);
            let mu = mu.clamp(-1.0, 1.0);
            let mus = mus.clamp(-1.0, 1.0);
            let var = (1.0 - mu * mu).sqrt() * (1.0 - mus * mus).sqrt();
            let nu = nu.clamp(mus * mu - var, mus * mu + var);
            let cthetamin = -(1.0 - (m.rg / r) * (m.rg / r)).sqrt();
            let v = [(1.0 - mu * mu).sqrt(), 0.0, mu];
            let sx = if v[0] == 0.0 { 0.0 } else { (nu - mus * mu) / v[0] };
            let s = [sx, (1.0 - sx * sx - mus * mus).max(0.0).sqrt(), mus];
            let mut raymie = [0.0; 3];
            let dens = (m.beta_r.map(|b| b * (-(r - m.rg) / m.hr).exp()), m.beta_msca.map(|b| b * (-(r - m.rg) / m.hm).exp()));
            for itheta in 0..SPHERICAL_SAMPLES {
                let theta = (itheta as f32 + 0.5) * dtheta;
                let ctheta = theta.cos();
                let (mut gref, mut dground, mut gtransp) = (0.0, 0.0, [0.0; 3]);
                if ctheta < cthetamin {
                    gref = GROUND_REFLECTANCE / PI;
                    dground = -r * ctheta - (r * r * (ctheta * ctheta - 1.0) + m.rg * m.rg).sqrt();
                    gtransp = m.transmittance_d(&tr, m.rg, -(r * ctheta + dground) / m.rg, dground);
                }
                for iphi in 0..2 * SPHERICAL_SAMPLES {
                    let phi = (iphi as f32 + 0.5) * dphi;
                    let dw = dtheta * dphi * theta.sin();
                    let w = [phi.cos() * theta.sin(), phi.sin() * theta.sin(), ctheta];
                    let nu1 = dot(s, w);
                    let nu2 = dot(v, w);
                    let (pr2, pm2) = (Model::phase_r(nu2), m.phase_m(nu2));
                    let gnormal = scale(add([0.0, 0.0, r], scale(w, dground)), 1.0 / m.rg);
                    let girr = m.irradiance(&delta_e, m.rg, dot(gnormal, s));
                    let mut raymie1 = scale(mul(girr, gtransp), gref);
                    if first {
                        let ray1 = m.texture4d(&delta_sr, r, w[2], mus, nu1);
                        let mie1 = m.texture4d(&delta_sm, r, w[2], mus, nu1);
                        raymie1 = add(raymie1, add(scale(ray1, Model::phase_r(nu1)), scale(mie1, m.phase_m(nu1))));
                    } else {
                        raymie1 = add(raymie1, m.texture4d(&delta_sr, r, w[2], mus, nu1));
                    }
                    raymie = add(raymie, scale(mul(raymie1, add(scale(dens.0, pr2), scale(dens.1, pm2))), dw));
                }
            }
            raymie
        });
        // deltaE: sky irradiance (ag_irradianceN).
        let idphi = PI / IRRADIANCE_SAMPLES as f32;
        let idtheta = PI / IRRADIANCE_SAMPLES as f32;
        for j in 0..SKY_H {
            for i in 0..SKY_W {
                let (r, mus) = irr_rmus(i, j);
                let s = [(1.0 - mus * mus).sqrt(), 0.0, mus];
                let mut result = [0.0; 3];
                for iphi in 0..IRRADIANCE_SAMPLES * 2 {
                    let phi = (iphi as f32 + 0.5) * idphi;
                    for itheta in 0..IRRADIANCE_SAMPLES / 2 {
                        let theta = (itheta as f32 + 0.5) * idtheta;
                        let dw = idtheta * idphi * theta.sin();
                        let w = [phi.cos() * theta.sin(), phi.sin() * theta.sin(), theta.cos()];
                        let nu = dot(s, w);
                        let l = if first {
                            add(scale(m.texture4d(&delta_sr, r, w[2], mus, nu), Model::phase_r(nu)), scale(m.texture4d(&delta_sm, r, w[2], mus, nu), m.phase_m(nu)))
                        } else {
                            m.texture4d(&delta_sr, r, w[2], mus, nu)
                        };
                        result = add(result, scale(l, w[2] * dw));
                    }
                }
                delta_e.d[j * SKY_W + i] = result;
            }
        }
        // deltaSR: the order's inscatter (ag_inscatterN).
        delta_sr = per_texel(&m, &|r, mu, mus, nu| {
            let integrand = |t: f32| -> V3 {
                let ri = (r * r + t * t + 2.0 * r * mu * t).sqrt();
                let mui = (r * mu + t) / ri;
                let musi = (nu * t + mus * r) / ri;
                mul(m.texture4d(&delta_j, ri, mui, musi, nu), m.transmittance_d(&tr, r, mu, t))
            };
            let dx = m.limit(r, mu) / INSCATTER_SAMPLES as f32;
            let mut yi = integrand(0.0);
            let mut sum = [0.0; 3];
            for k in 1..=INSCATTER_SAMPLES {
                let yj = integrand(k as f32 * dx);
                sum = add(sum, scale(add(yi, yj), dx / 2.0));
                yi = yj;
            }
            sum
        });
        // E += deltaE (ag_copyIrradiance, k 1); S.rgb += deltaSR / phaseR(nu) (ag_copyInscatterN).
        for (k, e) in delta_e.d.iter().enumerate() {
            for c in 0..3 {
                irradiance[k * 4 + c] += e[c];
            }
        }
        for l in 0..RES_R {
            let (r, dhdh) = m.layer(l);
            for y in 0..RES_MU {
                for x in 0..INSCATTER_W {
                    let (_, _, nu) = m.mu_mus_nu(r, dhdh, x, y);
                    let k = (l * RES_MU + y) * INSCATTER_W + x;
                    let pr = Model::phase_r(nu);
                    for c in 0..3 {
                        inscatter[k * 4 + c] += delta_sr.d[k][c] / pr;
                    }
                }
            }
        }
    }
    let transmittance = tr.d.iter().flat_map(|t| [t[0], t[1], t[2], 0.0]).collect();
    AtmTables { params: *p, inscatter, transmittance, irradiance }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_parse_with_scales() {
        let text = "Atmosphere \"Earth\"\n{\n RadiusGround 6360.0\n Height 60.0\n RayleighH 8.0\n MieH 2.0\n MieG 0.8\n MieFade 0.02\n RayleighBeta (0.0058, 0.0135, 0.0331)\n MieScaBeta (0.004, 0.004, 0.004)\n MieExtBeta (0.004, 0.004, 0.004)\n RayleighScale 1.0\n MieScaScale 1.0\n MieExtScale 1.11111\n}\n";
        let m = parse_models(text);
        assert_eq!(m.len(), 1);
        let p = &m[0].1;
        assert_eq!((m[0].0.as_str(), p.radius_ground, p.radius_top), ("Earth", 6360.0, 6420.0));
        assert!((p.mie_ext_beta[0] - 0.004_444_44).abs() < 1e-7);
    }
}
