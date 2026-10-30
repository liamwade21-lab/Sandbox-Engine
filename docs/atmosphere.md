# Atmospheric scattering

Files: `crates/se_engine/src/atmosphere.rs` (Rust, depends only on bevy + se_core),
`assets/shaders/atmosphere_common.wesl` (shared functions), `assets/shaders/sky.wesl` (sky shell),
`src/bin/atmo_demo.rs` + `assets/shaders/atmo_demo_ground.wesl` (visual test, demo only).

## Table format (verified against the real files)

`models/atmospheres/<Model>.atm` = Bruneton & Neyret 2008 tables (`se_core::atm`). Checked in Python
against numerical integration (Earth, Mars, Titan):

| table | texture | coordinates (texel centres at `(i+0.5)/N`, bilinear) |
|---|---|---|
| transmittance 256x64 | 2D | `u = atan((mu+0.15)/1.15*tan(1.5))/1.5`, `v = sqrt((r-Rg)/(Rt-Rg))`; value = T from r along mu to the atmosphere top (rays that hit the ground read 0). Matches analytic optical depth to 4 digits. |
| irradiance 64x16 | 2D | `u = (mu_s+0.2)/1.2`, `v = (r-Rg)/(Rt-Rg)`; sky irradiance on a horizontal surface for a unit-irradiance sun, **no direct sun**. Equals the hemispherical integral of the inscatter radiance within 2-5 %. |
| inscatter 512x64x32 | 3D | `x = (nu_idx + uMuS)/16` (nu_idx 0..15 for nu -1..+1), `y = uMu`, `z = uR` with Bruneton's `texture4D` mapping: `uR = .5/32 + rho/H*(1-1/32)`, `uMuS = .5/32 + max((1-exp(-3 mu_s-0.6))/(1-exp(-3.6)),0)*(1-1/32)` (the *paper* mu_s formula, not the later atan one), split `uMu` at the horizon (rows 0-31 ground-hitting rays, 32-63 sky rays). rgb = Rayleigh + all multiple scattering with the Rayleigh phase divided out, a = single Mie scattering (red). Mie colour = `rgb * a/max(r,1e-4) * (betaR.r/betaR) * (betaMSca/betaMSca.r)`. Mie single scattering alpha matches numerical integration to ~3 %. |

Radiance is for a sun of irradiance 1: `L = E_sun * (rgb*phaseR(nu) + mie*phaseM(nu))`, with
`phaseR = 3/(16pi)(1+nu^2)`, `phaseM = 3/(8pi)(1-g^2)(1+nu^2)/((2+g^2)(1+g^2-2g nu)^1.5)`.
`mie_fade` is the twilight range of Bruneton's `smoothstep(0, fade, mu_s)` on the Mie term.

### SpaceEngine's resolutions: RES_MU 64, RES_NU 16

SE's precompute and runtime (`ag_common.glh`, `atmo_common.glh`) use `RES_MU 64` and `RES_NU 16`
instead of Bruneton's 128 and 8, so a row is 512 texels: 16 nu blocks of 32 mu_s, from nu = -1
(block 0) to +1 (block 15), looked up as SE's `texture4D` (`nu * 7.5 + 7.5`, two taps). Read as
256 x 128 (the port did until 2026-10-28), each row looks like a pair of rows with mirrored nu, and the
old even-rows-with-minus-nu lookup returned S(-nu): ordinary skies came out nearly the same, but
toward a star behind the planet it read the backward table and the limb stayed dark. The two
halves of a row differ about 4x more than neighbouring nu blocks (Earth.atm), so they are not
mirrored copies.

## Units and conventions

* Table units are km of the model. All shader functions take **planet-centred** positions in table km
  (ground radius ~ `Rg`, top `Rt`) and unit directions in the same axes.
* Render space -> table: `x_table = (x_render - centre) * (Rt / top_radius_render)`. The body is scaled
  uniformly so that its atmosphere top maps onto the table's `Rt`. Use
  `AtmosphereGpu::top_radius_m(planet_radius_m)` (= `planet_radius * Rt/Rg`) as the shell radius so the
  planet surface lands exactly on `Rg` (catalog `radius + height` differs by ~0.3 km for Earth).
* Radiance functions return values per unit sun irradiance; multiply by the sun irradiance (rgb, scene
  units). The sky and demo shaders do not multiply by `view.exposure`, like `stars.wesl`. A sun
  irradiance of ~10 gives a good daylight sky under the engine's HDR + AcesFitted camera.
* f32 precision: table-space positions near 6000 km have ~0.5 m resolution, fine for altitudes above
  ~50 m. Table lookups clamp `r` to `Rg + 5 m`.

## Rust API (`atmosphere.rs`)

```rust
pub struct AtmospherePlugin;                       // registers MaterialPlugin<SkyMaterial>
pub struct AtmosphereGpu { pub params: AtmParams, pub inscatter: Handle<Image>,
                           pub transmittance: Handle<Image>, pub irradiance: Handle<Image> } // Clone
impl AtmosphereGpu { pub fn uniform(&self) -> AtmosphereParamsUniform;
                     pub fn top_radius_m(&self, planet_radius_m: f64) -> f64; }
pub fn load_atmosphere(vfs: &Vfs, model: &str, images: &mut Assets<Image>) -> Option<AtmosphereGpu>;
pub struct SkyMaterial { pub uniform: SkyUniform, pub inscatter: Handle<Image>, pub transmittance: Handle<Image> }
impl SkyMaterial { pub fn new(gpu: &AtmosphereGpu, bright: f32) -> Self;
                   pub fn set_sun(&mut self, dir_to_star: Vec3, irradiance: Vec3, angular_radius: f32);
                   pub fn set_brightness(&mut self, bright: f32); }
#[derive(Component)] pub struct SkyShell { pub material: Handle<SkyMaterial> }
pub fn spawn_sky_shell(commands, meshes, materials, gpu, planet_radius_m: f64,
                       atmosphere_height_m: f64, bright: f32) -> Entity;
pub fn sky_uniform_update(materials: &mut Assets<SkyMaterial>, handle: &Handle<SkyMaterial>,
                          dir_to_star: Vec3, sun_irradiance: Vec3, sun_angular_radius: f32);
```

Tables are uploaded as `Rgba16Float` with a linear clamp sampler (3D for inscatter). The shell is a
radius-1.02 UV sphere (the shader only uses its transform), `AlphaMode::Add`, front faces culled, no depth
write, `NoFrustumCulling`. **Every frame** set the shell's `Transform`: `translation = planet centre
relative to the camera * k`, `scale = Vec3::splat(top_radius * k)` (k = scaled-space factor; centre and
radius are read from the world transform in the shader, so any uniform k works), and call
`SkyMaterial::set_sun` with the star direction in render space, its irradiance and (optionally, > 0) its
angular radius to draw a reddened sun disc (leave 0 if the star is drawn elsewhere). `AtmosphereGpu` is
cheap to clone; load each model once and share it between bodies.

## Shader API (`atmosphere_common.wesl`, import as `super::atmosphere_common::{...}` from `shaders/*.wesl`)

```wgsl
struct AtmParams { radii: vec4<f32> /*Rg,Rt,HR,HM*/, mie: vec4<f32> /*g,fade*/, beta_r, beta_m_sca, beta_m_ext: vec4<f32> }
// Mirrors AtmosphereParamsUniform; put it in your material uniform via AtmosphereGpu::uniform().
fn atm_transmittance(p, tex_t: texture_2d<f32>, smp: sampler, r: f32, mu: f32) -> vec3<f32>
fn atm_transmittance_sun(p, tex_t, smp, r: f32, mu_s: f32) -> vec3<f32>       // 0 below the horizon
fn atm_irradiance(p, tex_e: texture_2d<f32>, smp, r: f32, mu_s: f32) -> vec3<f32>
fn atm_path_transmittance(p, r: f32, mu: f32, d: f32) -> vec3<f32>            // analytic, finite path (km)
fn atm_inscatter_lookup(p, tex_s: texture_3d<f32>, smp, r, mu, mu_s, nu) -> vec4<f32>
fn atm_path(p, tex_s, tex_t, smp, x: vec3, v: vec3, s: vec3, t: f32) -> PathScatter {inscatter, transmittance}
fn atm_sky_radiance(p, tex_s, tex_t, smp, cam: vec3, v: vec3, s: vec3) -> SkyRadiance {radiance, transmittance, ground_dist}
fn atm_surface_irradiance(p, tex_t, tex_e, smp, pos, normal, sun_dir, sun_irradiance: vec3, sun_vis: f32) -> vec3
fn atm_apply_aerial(p, tex_s, tex_t, smp, surface: vec3, cam, pos, sun_dir, sun_irradiance: vec3) -> vec3
fn atm_ground_atmosphere(p, tex_s, tex_t, tex_e, smp, albedo: vec3, normal, sun_dir,
                         sun_irradiance: vec3, sun_vis: f32, cam, pos) -> vec3
```

`atm_path` handles cameras in space (it starts at the atmosphere entry point) and `t < 0` for "to infinity".
`atm_sky_radiance` reports `ground_dist >= 0` for rays that hit the table ground sphere; their
`radiance` is the grazing-horizon value (the surface shader owns those pixels; the shell is depth-tested
behind the planet, and the horizon value hides sub-pixel disagreements between the two horizons).

## Integrating into the engine

1. `app.add_plugins(AtmospherePlugin)`; `mod atmosphere;` in `main.rs`. At body creation call
   `load_atmosphere(&vfs, model, &mut images)` (catalog `Atmosphere { Model ... }`) and
   `spawn_sky_shell(...)`; store the entity/handle on the body and update its transform + sun every frame.
2. Planet surface / terrain shader: add to the material the uniform `AtmParams` (+ sun dir/irradiance),
   the inscatter 3D texture + sampler, transmittance and irradiance textures (see `GroundMaterial` in
   `atmo_demo.rs` for the exact `AsBindGroup` attributes: `#[texture(1, dimension = "3d")] #[sampler(2)]`,
   `#[texture(3)]`, `#[texture(4)]`). In the fragment shader compute `cam_km`/`pos_km` (planet-centred table
   km: `(x_render - centre) * (Rg / planet_radius_render)`) and return
   `atm_ground_atmosphere(...)` (or `atm_surface_irradiance` + your own BRDF + `atm_apply_aerial`). This
   gives sun reddening, sky light and aerial perspective, and also the haze over the disc seen from space.
   Surfaces seen without this call will not get atmospheric haze.
3. Scale: use the same `k` for the planet and the shell; draw order needs no care (shell is additive,
   no depth write).

## Verification

Screenshots from `atmo_demo` (`--view space|ground|sunset`, `--model`, `--alt`, `--scale`): blue sky
brightening to a pale horizon with haze on the ground at noon; orange/red horizon glow and reddened sun at
sunset; thin blue limb, soft terminator and hazy disc from space; Mars butterscotch sky; `--scale 0.001`
renders identically to `--scale 1`.

## Known limitations

* No ozone in the tables: twilight zenith is olive/grey rather than deep blue (same as Bruneton's demo).
* Catalog `Opacity`, `SkyLight`, `Height` and the model's non-grey Mie colour are only approximated
  (`Bright` is a plain multiplier). The atmosphere's height is the body's visual height (catalog `Height`, else
  SE's AtmoHeightHarb formula), the model stretched to it as SE does (`AtmosphereGpu::with_height`).
* `Rgba16Float` tables lose ~1e-3 relative precision, so aerial perspective over very short paths
  (the `S(x) - T*S(x0)` difference) may show faint banding; none was visible in the demo screenshots.
* Single spherical shell; no clouds, no shadows from other bodies (pass `sun_vis`), no ground-bounce.
