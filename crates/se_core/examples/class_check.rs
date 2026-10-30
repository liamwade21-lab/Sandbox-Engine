//! Print SE's class temperature and class word for catalog bodies (dev probe).
//! Usage: class_check [Name ...] (default: a few Solar System bodies).
use se_core::catalog::{Catalog, ObjectRef};
use se_core::surface_gen::{catalog_input, TEMP_NAMES, VOLQ_NAMES};
use se_core::units::METERS_PER_AU;
use se_core::vfs::Vfs;

fn main() {
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    if names.is_empty() {
        names = ["Mercury", "Venus", "Earth", "Moon", "Mars", "Ceres", "Io", "Europa", "Ganymede", "Titan", "Triton", "Pluto"]
            .map(String::from)
            .to_vec();
    }
    let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).expect("mount");
    let cat = Catalog::load(&vfs);
    let eph = se_core::ephemeris::Ephemeris::new(&cat);
    let jd = 2_451_545.0;
    for n in &names {
        let Some(ObjectRef::Body(i)) = cat.find(n) else { continue };
        let b = &cat.bodies[i as usize];
        // Light from the root star of the system.
        let Some(root) = cat.system_root(ObjectRef::Body(i)) else { continue };
        let st = &cat.stars[root as usize];
        let d_au = eph.position(ObjectRef::Star(root), jd).delta_m(eph.position(ObjectRef::Body(i), jd)).length() / METERS_PER_AU;
        // SE's temperature model reads the light at the mean distance of the orbit (of the
        // planet, for a moon).
        let flux = match se_core::climate::reference_distances_au(&cat, i as usize) {
            Some((_, m, _)) => st.lum_bol() / (m * m),
            None => st.lum_bol() / (d_au * d_au),
        };
        let gh = b.block.sub("Atmosphere").and_then(|a| a.f64("Greenhouse")).unwrap_or(0.0);
        let t_eq = 278.0 * (flux * (1.0 - b.albedo)).powf(0.25);
        let (inp, _) = catalog_input(&cat, i as usize, t_eq, gh, flux, None);
        println!(
            "{:<10} flux {:>8.4} P {:>8.3} T_s {:>6.1} -> class T {:>6.1} K ({:>6.1} C) {:<9} {}",
            n,
            flux,
            inp.pressure_atm,
            inp.t_surf,
            inp.t_class,
            inp.t_class - 273.15,
            TEMP_NAMES[inp.classes.temp as usize],
            VOLQ_NAMES[inp.classes.volq as usize]
        );
    }
}
