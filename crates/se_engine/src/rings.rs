//! Planetary rings: a flat annulus in the body's equatorial plane textured with
//! SpaceEngine's 2-row radial ring textures (row 1 = lit side colour, row 0 =
//! transmitted / dusty colour, alpha = optical density).

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat,
};
use bevy::shader::ShaderRef;
use se_core::catalog::ObjectRef;
use se_core::rings::RingsParams;
use se_core::vfs::Vfs;

use crate::eclipse::{EclipseCasters, EclipseLight};
use crate::switches::RenderSwitches;
use crate::system::{BatchedSprite, BodyLighting, LightingFade, MAX_LIGHTS, SystemBody, SystemMember};
use crate::universe::{MeterScale, UOrientation, UPosition, UniverseSet};

#[derive(Clone, Copy, Debug, ShaderType, Default)]
pub struct RingUniform {
    pub sun_dir: Vec4,
    pub sun_color: Vec4,
    /// x inner radius / planet radius, y outer / planet radius, z the density of the rings' shadow on
    /// the planet and its sky ([`planet_shadow`], SE's RingsParams.z; terrain.rs reads it from here),
    /// w `Brightness`.
    pub geom: Vec4,
    /// x `FrontBright`, y `BackBright`, z has texture, w unused.
    pub params: Vec4,
    /// SE's DensityParams: `Density` (lit and back rows), `Opacity`, `SelfShadow`.
    pub density: Vec4,
    /// x `SpotBright`, y 1 / `SpotWidth`, z `Hapke`, w `SpotBrightCB`.
    pub hapke: Vec4,
    /// x 1 / `SpotWidthCB`, y the radial detail-noise frequency, z SE's `Oblateness.g` (1 / the
    /// planet's polar semi-axis, for its shadow), w the eclipse intensity (SE's AmbientColor.a).
    pub hapke2: Vec4,
    /// SE's host light (HostLightIndex): rgb the planet's light at the rings' outer edge, w 1 when on.
    pub host: Vec4,
    /// The further stars ([`BodyLighting::extra_uniforms`] for rings).
    pub light_dir: [Vec4; MAX_LIGHTS - 1],
    pub light_color: [Vec4; MAX_LIGHTS - 1],
    /// Each light's eclipse casters ([`crate::eclipse::EclipseCasters`]).
    pub eclipse: [EclipseLight; MAX_LIGHTS],
    /// The volumetric rings' share (`rings3d.rs`, SE's Fade3DParams and RingsParams.z): x 0.95
    /// `VolumDrawDist`, y the half thickness (planet radii), z 1 when `VolumetricRings` is on: the
    /// flat ring is drawn only that far from the eye and fades out within two half thicknesses of
    /// its plane.
    pub fade3d: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct RingMaterial {
    #[uniform(0)]
    pub u: RingUniform,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
}

impl Material for RingMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/rings.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/rings.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }
    fn depth_bias(&self) -> f32 {
        // After the sky shells, which would otherwise hide it in daylight (see `SkyMaterial`).
        1.0e10
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Ring entity attached to a body entity.
#[derive(Component)]
pub struct Rings {
    pub body: Entity,
    pub material: Handle<RingMaterial>,
    pub params: RingsParams,
    pub texture: Handle<Image>,
    /// Each texel's front and back alpha (`rings3d::texture_alphas`).
    pub alpha: Arc<Vec<[f32; 2]>>,
    /// The draw SE takes for a missing `Thickness` (from the ring's seed).
    pub thickness_draw: f64,
}

pub struct RingsPlugin;

impl Plugin for RingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((MaterialPlugin::<RingMaterial>::default(), crate::rings3d::Rings3DPlugin)).add_systems(
            PostUpdate,
            update_rings.after(crate::eclipse::update_eclipse_casters).before(UniverseSet::FloatingOrigin),
        );
    }
}

/// Annulus in the XZ plane (normal +Y), radii in planet radii; uv.x = radial 0..1.
fn annulus(inner: f32, outer: f32, segments: usize, radial: usize) -> Mesh {
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    for s in 0..=segments {
        let a = std::f32::consts::TAU * s as f32 / segments as f32;
        let (sn, cs) = a.sin_cos();
        for r in 0..=radial {
            let t = r as f32 / radial as f32;
            let rad = inner + (outer - inner) * t;
            pos.push([cs * rad, 0.0, sn * rad]);
            uv.push([t, s as f32 / segments as f32]);
        }
    }
    let mut idx = Vec::new();
    let row = (radial + 1) as u32;
    for s in 0..segments as u32 {
        for r in 0..radial as u32 {
            let a = s * row + r;
            let b = a + 1;
            let c = a + row;
            let d = c + 1;
            idx.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(idx))
}

/// Decode a ring texture, halving its width until it fits common GPU limits.
fn load_ring_texture(vfs: &Vfs, path: &str) -> Option<Image> {
    let bytes = vfs.read(path).ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    while img.width() > 8192 {
        img = image::imageops::resize(&img, img.width() / 2, img.height(), image::imageops::FilterType::Triangle);
    }
    let (w, h) = img.dimensions();
    let mut image = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        img.into_raw(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    Some(image)
}

fn hash1(i: i64, seed: u64) -> f64 {
    let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 32;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 32;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth 1D value noise in [0, 1].
fn noise1(x: f64, seed: u64) -> f64 {
    let i = x.floor();
    let f = x - i;
    let u = f * f * (3.0 - 2.0 * f);
    hash1(i as i64, seed) * (1.0 - u) + hash1(i as i64 + 1, seed) * u
}

/// Rings without a texture are procedural in SpaceEngine: build the same 2-row
/// radial texture (row 1 lit colour and opacity, row 0 transmitted light) from the
/// catalog's `frequency`, `density*`, `colorContrast` and colour keys.
fn procedural_ring_image(p: &RingsParams, seed: u64) -> Image {
    const W: usize = 2048;
    // Catalog frequencies (~50..200) count fine ringlets; the base band scale is coarser.
    let freq = if p.frequency > 0.0 { p.frequency * 0.15 } else { 24.0 };
    let front = p.front_color.unwrap_or([0.82, 0.76, 0.66]);
    let back = p.back_thick_color.or(p.back_dust_color).unwrap_or([0.55, 0.48, 0.40]);
    let contrast = if p.color_contrast > 0.0 { p.color_contrast } else { 0.35 };
    let mut px = vec![0u8; W * 2 * 4];
    for u in 0..W {
        let x = (u as f64 + 0.5) / W as f64;
        // Bands on several scales, plus a few sharp gaps.
        let (mut n, mut norm) = (0.0, 0.0);
        let mut amp = 0.5;
        let mut f = freq;
        for k in 0..6 {
            n += amp * noise1(x * f, seed ^ (0x51 + k));
            norm += amp;
            amp *= 0.55;
            f *= 2.3;
        }
        n /= norm;
        let gaps = (0..3).fold(1.0, |g, k| {
            let c = hash1(k, seed ^ 0x6A9) * 0.8 + 0.1;
            let w = 0.004 + hash1(k + 7, seed ^ 0x6A9) * 0.012;
            g * (((x - c).abs() / w).min(1.0)).powf(0.5)
        });
        let edge = (x / 0.03).min(1.0) * ((1.0 - x) / 0.03).min(1.0);
        // SpaceEngine's shaping: saturate(noise * densityScale + densityOffset) ^ densityPower.
        // Its defaults (1, 0, 1) would leave the noise as is; stretch it a little then.
        let (scale, offset) = if p.density_scale == 1.0 && p.density_offset == 0.0 { (1.8, -0.35) } else { (p.density_scale, p.density_offset) };
        let d = (n * scale + offset).clamp(0.0, 1.0);
        let density = (d.powf(p.density_power.max(0.05)) * gaps * edge * p.density.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let tint = 1.0 + contrast * (noise1(x * freq * 0.7, seed ^ 0xC01) - 0.5) * 2.0;
        let srgb = |v: f64| ((v.clamp(0.0, 1.0)).powf(1.0 / 2.2) * 255.0) as u8;
        let a = (density * 255.0) as u8;
        // Row 0: light through the ring (thin parts glow); row 1: lit side.
        let i0 = u * 4;
        let i1 = (W + u) * 4;
        for c in 0..3 {
            px[i1 + c] = srgb(front[c] * tint);
            px[i0 + c] = srgb(back[c] * tint * (1.0 - density * 0.5));
        }
        px[i1 + 3] = a;
        px[i0 + 3] = a;
    }
    let mut image = Image::new(
        Extent3d {
            width: W as u32,
            height: 2,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

const RING_DETAIL_NOISE_SCALE: f32 = 0.0005;

/// The density of the rings' shadow on their planet and its sky (SE's RingsParams.z, the ring's
/// +0x1b58, FUN_14044fe70 / FUN_1404ae4a0): the catalog's `PlanetShadow` (or its alias `Shadow`,
/// FUN_14046cef0). Without one SE derives it (FUN_140508600) from the `Density` as `Density` x 1, or
/// x 0.5 for the thin rings it draws 20 % of the time (`U < 0.2` on its loader's random stream; the
/// port draws it from the ring's seed).
fn planet_shadow(block: &se_core::sc::Block, density: f64, seed: u64) -> f64 {
    block.f64("PlanetShadow").or_else(|| block.f64("Shadow")).unwrap_or_else(|| density * if hash1(0x5_4AD0, seed) < 0.2 { 0.5 } else { 1.0 })
}

/// Spawn rings for a body if its catalog block defines them.
#[allow(clippy::too_many_arguments)]
pub fn spawn_rings(
    commands: &mut Commands,
    vfs: &Vfs,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<RingMaterial>,
    body_entity: Entity,
    body_block: &se_core::sc::Block,
    radius_m: f64,
    white: Handle<Image>,
) -> Option<Entity> {
    if body_block.bool("NoRings").unwrap_or(false) {
        return None;
    }
    let block = body_block.sub("Rings")?;
    let p = RingsParams::from_block(block);
    if p.outer_radius_km <= p.inner_radius_km || radius_m <= 0.0 {
        return None;
    }
    let seed = (p.inner_radius_km.to_bits() ^ p.outer_radius_km.to_bits().rotate_left(17)) ^ (radius_m.to_bits() >> 7);
    let img = p.texture_path().and_then(|path| load_ring_texture(vfs, &path)).unwrap_or_else(|| procedural_ring_image(&p, seed));
    let alpha = crate::rings3d::texture_alphas(&img);
    let tex = Some(images.add(img));
    let has_tex = tex.is_some();
    let r_km = radius_m / 1000.0;
    let inner = (p.inner_radius_km / r_km) as f32;
    let outer = (p.outer_radius_km / r_km) as f32;
    // SE's opposition spots (`SpotWidth*` are half-widths in tan(phase / 2)); none without a width.
    let inv = |w: f64| if w > 0.0 { (1.0 / w) as f32 } else { 0.0 };
    let spot = |b: f64, w: f64| if w > 0.0 { b as f32 } else { 0.0 };
    let material = materials.add(RingMaterial {
        u: RingUniform {
            sun_dir: Vec4::new(0.0, 1.0, 0.0, 1.0),
            sun_color: Vec4::ONE,
            geom: Vec4::new(inner, outer, planet_shadow(block, p.density, seed).clamp(0.0, 1.0) as f32, p.brightness.max(0.0) as f32),
            params: Vec4::new(p.front_bright.max(0.0) as f32, p.back_bright.max(0.0) as f32, if has_tex { 1.0 } else { 0.0 }, 0.0),
            density: Vec4::new(p.density as f32, p.density as f32, p.opacity as f32, p.self_shadow as f32).max(Vec4::ZERO),
            hapke: Vec4::new(spot(p.spot_bright, p.spot_width), inv(p.spot_width), p.hapke.clamp(0.0, 1.0) as f32, spot(p.spot_bright_cb, p.spot_width_cb)),
            // main-def.cfg `RingDetailNoiseScale` 0.0005: noise features that fraction of the ring's
            // width across (assumed reading).
            hapke2: Vec4::new(inv(p.spot_width_cb), 1.0 / RING_DETAIL_NOISE_SCALE, 1.0, 1.0),
            ..Default::default()
        },
        texture: tex.clone().unwrap_or_else(|| white.clone()),
    });
    let e = commands
        .spawn((
            SystemMember,
            Rings {
                body: body_entity,
                material: material.clone(),
                params: p.clone(),
                texture: tex.unwrap_or(white),
                alpha,
                thickness_draw: hash1(0x7_41C, seed),
            },
            Mesh3d(meshes.add(annulus(inner, outer, 720, 8))),
            MeshMaterial3d(material),
            UPosition::default(),
            UOrientation::default(),
            MeterScale(bevy::math::DVec3::splat(radius_m)),
            Transform::IDENTITY,
            Visibility::Hidden,
            NoFrustumCulling,
        ))
        .id();
    Some(e)
}

/// SE's albedo for a body without one (FUN_1404da600, as system.rs's planet shine).
const DEFAULT_ALBEDO: f64 = 0.36;

/// SE's host light (FUN_1404e0b50 sets HostLightIndex to the planet's own planet-shine light when
/// planet shine is on): the planet's light on its rings at their outer edge, where the shader's
/// `(outer / d)^2` is 1. As the port's planet shine (system.rs `planet_shine`): the light reflected
/// at full phase, `F (R / outer)^2 albedo` (relative to the star's `F` on the rings), times the
/// luminance of the planet's colour, coloured by it (brightest channel 1) and the star's light.
fn host_light(albedo: f64, outer_radii: f32, planet_rgb: Vec3, star: Vec3, bright: f64) -> Vec3 {
    let max = planet_rgb.max_element();
    if max <= 0.0 || outer_radii <= 1.0 {
        return Vec3::ZERO;
    }
    let k = albedo * bright / (outer_radii as f64 * outer_radii as f64);
    star * planet_rgb / max * (k as f32 * planet_rgb.dot(Vec3::new(0.299, 0.587, 0.114)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_rings(
    bodies: Query<(&SystemBody, &BodyLighting, Option<&EclipseCasters>)>,
    mut rings: Query<(&Rings, &mut UPosition, &mut UOrientation, &mut Visibility)>,
    mut materials: ResMut<Assets<RingMaterial>>,
    switches: Res<RenderSwitches>,
    fade: Res<LightingFade>,
    data: Res<crate::data::SeData>,
    sprites: Query<&BatchedSprite>,
) {
    for (ring, mut pos, mut rot, mut vis) in &mut rings {
        let Ok((body, light, casters)) = bodies.get(ring.body) else { continue };
        pos.0 = body.pos;
        rot.0 = body.orient;
        // Rings extend to a few planet radii; show them a bit earlier than the disk.
        let show = body.px_radius > 0.15;
        *vis = if show { Visibility::Visible } else { Visibility::Hidden };
        if show {
            if let Some(mut m) = materials.get_mut(&ring.material) {
                m.u.sun_dir = light.sun_dir.extend(light.rings_intensity);
                m.u.sun_color = light.sun_color.extend(1.0);
                (m.u.light_dir, m.u.light_color) = light.extra_uniforms(true);
                m.u.eclipse = casters.map_or_else(Default::default, |c| c.lights);
                m.u.hapke2.z = 1.0 / body.shape.y.max(1e-3) as f32;
                m.u.hapke2.w = switches.eclipse_intensity();
                m.u.host = Vec4::ZERO;
                if fade.planet_shine && light.rings_intensity > 0.0 {
                    let albedo = match body.obj {
                        ObjectRef::Body(i) => Some(data.catalog.bodies[i as usize].albedo).filter(|&a| a > 0.0).unwrap_or(DEFAULT_ALBEDO),
                        _ => DEFAULT_ALBEDO,
                    };
                    let rgb = sprites.iter().find(|s| s.owner == ring.body).map_or(Vec3::ONE, |s| Vec3::from_array(s.rgb));
                    let host = host_light(albedo, m.u.geom.y, rgb, light.sun_color * light.rings_intensity, fade.planet_shine_bright);
                    m.u.host = host.extend(if host.max_element() > 0.0 { 1.0 } else { 0.0 });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_light_is_the_planet_shine_at_the_outer_edge() {
        // Saturn: albedo 0.34, rings out to 2.27 radii, a white planet under a white star.
        let h = host_light(0.34, 2.27, Vec3::ONE, Vec3::ONE, 1.0);
        assert!((h.x - 0.34 / (2.27 * 2.27)).abs() < 1e-6, "{h}");
        // Coloured by the planet, brightest channel at its luminance share.
        let c = host_light(0.5, 2.0, Vec3::new(1.0, 0.5, 0.0), Vec3::ONE, 1.0);
        assert!(c.z == 0.0 && (c.y / c.x - 0.5).abs() < 1e-6);
        assert_eq!(host_light(0.5, 0.9, Vec3::ONE, Vec3::ONE, 1.0), Vec3::ZERO);
    }

    #[test]
    fn planet_shadow_from_the_catalog_or_density() {
        let rings = |s: &str| se_core::sc::parse(s).sub("Rings").cloned().unwrap();
        // Neptune's PlanetShadow, not its Opacity; SE's `Shadow` alias.
        assert_eq!(planet_shadow(&rings("Rings\n{\n\tOpacity 0.15\n\tPlanetShadow 0.05\n}\n"), 0.15, 1), 0.05);
        assert_eq!(planet_shadow(&rings("Rings\n{\n\tShadow 0.3\n}\n"), 1.0, 1), 0.3);
        // Without one: the Density, halved for the thin fifth.
        let derived: Vec<f64> = (0..400).map(|s| planet_shadow(&rings("Rings\n{\n\tDensity 0.4\n}\n"), 0.4, s)).collect();
        assert!(derived.iter().all(|&v| v == 0.4 || v == 0.2));
        let thin = derived.iter().filter(|&&v| v == 0.2).count();
        assert!((40..120).contains(&thin), "{thin}");
    }
}
