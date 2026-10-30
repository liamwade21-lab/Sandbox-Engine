//! Run SE's completion (se_core::climate) over the planets and moons of the named host stars
//! that have no `Atmosphere` block, with flux from the semi-major axis.
//! Usage: cargo run --release -p se_core --example climate_check -- [install] [star ...]
use se_core::catalog::{Catalog, ObjectRef};
use se_core::climate::{self as cl, ClimateTables, StarLight};
use se_core::surface_gen::{BULK_NAMES, TEMP_NAMES, VOLQ_NAMES};
use se_core::units::EARTH_MASS_KG;
use se_core::vfs::Vfs;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().unwrap_or_else(|| "D:/SteamLibrary/steamapps/common/SpaceEngine".into()));
    let mut hosts: Vec<String> = args.collect();
    if hosts.is_empty() {
        hosts = vec!["Sun".into(), "Proxima".into()];
    }
    let vfs = Vfs::mount_install(&root).expect("mount");
    let mut cat = Catalog::load(&vfs);
    se_core::system_gen::complete_catalog_orbits(&mut cat);
    // Optional overrides "Name=mass_earth" in SE_MASS (comma separated), to compare with SE.
    for kv in std::env::var("SE_MASS").unwrap_or_default().split(',').filter(|s| !s.is_empty()) {
        let (n, m) = kv.split_once('=').expect("Name=mass");
        if let Some(ObjectRef::Body(i)) = cat.find(n) {
            cat.bodies[i as usize].mass_kg = m.parse::<f64>().unwrap() * EARTH_MASS_KG;
        }
    }
    let t = ClimateTables::from_install(&root).expect("tables");
    for host in &hosts {
        let Some(ObjectRef::Star(s)) = cat.find(host) else {
            println!("{host}: no star");
            continue;
        };
        let st = &cat.stars[s as usize];
        let lum = st.lum_bol();
        let (lc, ty, sub) = cl::star_class_bits(&st.class);
        let age = cl::catalog_system_age(&[st], &st.block);
        println!("{host}: L_bol {lum:.5} Teff {:.0} class {:?} xuv {:.3} age {age:.2} Gyr", st.teff, (lc, ty, sub), t.xuv_fraction(lc, ty, sub));
        for (i, b) in cat.bodies.iter().enumerate() {
            // Planets of the star and their moons.
            let parent_mass = if st.names.iter().any(|n| *n == b.parent) {
                st.mass_kg
            } else {
                match cat.find(&b.parent) {
                    Some(ObjectRef::Body(p)) if st.names.iter().any(|n| *n == cat.bodies[p as usize].parent) => cat.bodies[p as usize].mass_kg,
                    _ => continue,
                }
            };
            let Some(o) = b.orbit.as_ref() else { continue };
            let _ = o;
            let Some((q, m, _)) = cl::reference_distances_au(&cat, i) else { continue };
            let light = StarLight {
                flux: lum / (q * q),
                xuv: t.xuv_fraction(lc, ty, sub) as f64 * lum / (q * q),
                flux_mean: lum / (m * m),
                age_gyr: age,
                parent_mass_earth: parent_mass / EARTH_MASS_KG,
                star_c_to_o: st.block.f64("CtoO"),
            };
            let Some(bi) = cl::catalog_body_in(&cat, i, &light) else { continue };
            let Some(c) = cl::complete(&t, &bi) else {
                println!("  {:<14} not completed", b.names[0]);
                continue;
            };
            let classes = se_core::surface_gen::Classes::new(bi.obj_type, c.bulk, c.t_class, c.pressure_atm, bi.mass_earth, c.ocean_info());
            let look = cl::atmosphere_look(bi.seeds.0, &c, classes.volq == 1, false);
            let gases: Vec<String> = c.gas_percent(&t).iter().take(6).map(|(n, p)| format!("{n} {p:.2}%")).collect();
            println!(
                "  {:<14} {:>9.4} M_E {:>9.4} atm (panel {:.4e}) {:>5.0} K avg {:>5.0} K {:<9} {:<11} {:<8} ocean {:>6.2} km bump {:>5.1} km A {:.2} {} [{}]",
                b.names[0],
                bi.mass_earth,
                c.pressure_atm,
                c.panel_pressure_atm,
                c.t_surf,
                c.t_class,
                TEMP_NAMES[classes.temp as usize],
                VOLQ_NAMES[classes.volq as usize],
                BULK_NAMES[c.bulk as usize],
                c.ocean_depth_km,
                c.bump_km,
                c.bond_albedo,
                look.map_or("-".to_string(), |l| t.atm_models[l.model].clone()),
                gases.join(", "),
            );
        }
    }
}
