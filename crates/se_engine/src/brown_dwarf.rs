//! Brown dwarfs as SE draws them (bulk class 8, its generator 6: tg_browndwarf_height/glow): the
//! gas giant cloud bands (HeightMapCloudsGasGiant) glowing at `1 - 0.2 height` of the surface
//! temperature, mixed with the solar granulation (GlowMapSun) by the star's `erosion`, then SE's
//! thermal colour. `system.rs` gives every star the photosphere material; the brown dwarfs among them
//! get [`BrownDwarfMaterial`] in its place, which shares its uniform, noise texture and vertex stage
//! (`star_surface.wesl`) and adds the clouds (`brown_dwarf.wesl`).

use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use se_core::catalog::ObjectRef;
use se_core::surface_gen as sg;

use crate::data::SeData;
use crate::system::{StarSphere, StarSurfaceMaterial, StarSurfaceUniform, SystemBody};

/// The clouds of [`BrownDwarfMaterial`].
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct BrownDwarfUniform {
    /// SE's cloud parameters, laid out as `surface_tex::BiomeUniform::giant`: (cloudsFreq,
    /// cloudsOctaves, stripeZones, stripeTwist), (stripeFluct, 1, 0, 0), the equatorial and the polar
    /// cyclones (magn, freq, density or latitude, octaves), Randomize.
    pub clouds: [Vec4; 5],
    /// x SE's `erosion` (the granulation's share), y the bands' mean height.
    pub params: Vec4,
}

impl BrownDwarfUniform {
    pub fn new(g: &sg::SurfaceGen) -> Self {
        let v = |i: usize| g.v(i);
        let fluct = v(sg::STRIPE_FLUCT);
        Self {
            clouds: [
                Vec4::new(v(sg::CLOUDS_MAIN_FREQ).max(0.1), v(sg::CLOUDS_MAIN_OCTAVES).clamp(1.0, 15.0), v(sg::STRIPE_ZONES), v(sg::STRIPE_TWIST)),
                Vec4::new(fluct, 1.0, 0.0, 0.0),
                Vec4::new(v(sg::CYCLONE_MAGN), v(sg::CYCLONE_FREQ), v(sg::CYCLONE_DENSITY), v(sg::CYCLONE_OCTAVES)),
                Vec4::new(v(sg::CYCLONE_MAGN2), v(sg::CYCLONE_FREQ2), v(sg::CYCLONE_LATITUDE2), v(sg::CYCLONE_OCTAVES2)),
                Vec4::new(v(0), v(1), v(2), 0.0),
            ],
            // The bands' height: zones 0.25 on average plus stripeFluct x (0.7 Fbm + 0.5).
            params: Vec4::new(v(sg::EROSION).clamp(0.0, 1.0), 0.25 + 0.5 * fluct, 0.0, 0.0),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct BrownDwarfMaterial {
    #[uniform(0)]
    pub star: StarSurfaceUniform,
    #[texture(1, dimension = "3d")]
    pub noise: Handle<Image>,
    #[uniform(2)]
    pub bd: BrownDwarfUniform,
}

impl Material for BrownDwarfMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/star_surface.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/brown_dwarf.wesl".into()
    }
}

pub struct BrownDwarfPlugin;

impl Plugin for BrownDwarfPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<BrownDwarfMaterial>::default()).add_systems(Update, dress_brown_dwarfs);
    }
}

/// Stars already looked at.
#[derive(Component)]
struct BrownDwarfChecked;

/// Whether SE draws a catalog star as a brown dwarf: its star type (class T/Y, or 0.012..0.0767
/// solar masses).
pub fn is_brown_dwarf(s: &se_core::catalog::StarRecord) -> bool {
    let mass = (s.mass_kg > 0.0).then(|| s.mass_kg / se_core::units::SOLAR_MASS_KG);
    se_core::classname::star_type(&s.class, mass) == "Brown dwarf"
}

fn dress_brown_dwarfs(
    mut commands: Commands,
    data: Res<SeData>,
    star_mats: Res<Assets<StarSurfaceMaterial>>,
    mut bd_mats: ResMut<Assets<BrownDwarfMaterial>>,
    q: Query<(Entity, &SystemBody, &MeshMaterial3d<StarSurfaceMaterial>), (With<StarSphere>, Without<BrownDwarfChecked>)>,
) {
    for (e, body, mat) in &q {
        commands.entity(e).insert(BrownDwarfChecked);
        let ObjectRef::Star(i) = body.obj else { continue };
        let s = &data.catalog.stars[i as usize];
        if !is_brown_dwarf(s) {
            continue;
        }
        let Some(m) = star_mats.get(&mat.0) else { continue };
        let g = sg::brown_dwarf(&sg::star_input(s), &sg::given_from(s.block.sub("Surface"), None));
        let material = bd_mats.add(BrownDwarfMaterial { star: m.u, noise: m.noise.clone(), bd: BrownDwarfUniform::new(&g) });
        commands.entity(e).remove::<MeshMaterial3d<StarSurfaceMaterial>>().insert(MeshMaterial3d(material));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brown_dwarf_uniform_layout() {
        let mut g = sg::SurfaceGen { p: [sg::UNSET; sg::N_PARAMS], color: None, classes: Default::default() };
        g.p[sg::CLOUDS_MAIN_FREQ] = 2.0;
        g.p[sg::CLOUDS_MAIN_OCTAVES] = 12.0;
        g.p[sg::STRIPE_FLUCT] = 0.8;
        g.p[sg::EROSION] = 0.0;
        let u = BrownDwarfUniform::new(&g);
        assert_eq!((u.clouds[0].x, u.clouds[0].y), (2.0, 12.0));
        assert_eq!(u.clouds[1], Vec4::new(0.8, 1.0, 0.0, 0.0));
        assert_eq!(u.params, Vec4::new(0.0, 0.65, 0.0, 0.0));
    }
}
