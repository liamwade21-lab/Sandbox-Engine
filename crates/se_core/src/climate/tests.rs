//! Climate model tests. The `#[ignore]` ones read the gas table from `SpaceEngine.exe`
//! (`cargo test -p se_core climate -- --ignored`).

use super::*;

fn tables() -> ClimateTables {
    ClimateTables::from_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).expect("SpaceEngine.exe readable")
}

#[test]
fn greenhouse_follows_molar_mass_and_pressure() {
    // Earth-like: k = 1, ((1.75)^0.25 - 1) * 0.43 * 255 K.
    let g = greenhouse(1.0, 29.0, 255.0);
    assert!((g - 16.47).abs() < 0.1, "{g}");
    assert_eq!(greenhouse(1.0, 120.0, 300.0), 0.0);
    assert!(greenhouse(90.0, 44.0, 230.0) > greenhouse(9.0, 44.0, 230.0));
    assert_eq!(greenhouse(1e4, 2.0, 3000.0), 1000.0);
}

#[test]
fn temperatures_and_lifetimes() {
    let (t_eq, t_exo) = equilibrium_temps(1.0, 0.0, Some(0.3));
    assert!((t_eq - 254.3).abs() < 0.2, "{t_eq}");
    assert_eq!(t_exo, t_eq);
    let (_, t_exo) = equilibrium_temps(1.0, 0.072, Some(0.3));
    assert!((t_exo - 1500.0).abs() < 0.5);
    assert!((star_lifetime_gyr(1.0, 7) - 10.0).abs() < 1e-4);
    assert!(star_lifetime_gyr(0.122, 7) > 100.0);
    assert!((star_lifetime_gyr(2.0, 5) - 10.0 * 2f32.powf(-3.75) * 1.1).abs() < 1e-4);
    assert_eq!(star_lifetime_gyr(1.0, 11), 12.87);
    assert_eq!(system_age_gyr(0, [5.0]), 0.0);
    assert!((system_age_gyr(u32::MAX, [300.0]) - 12.87).abs() < 1e-3);
}

#[test]
fn bump_default_and_composition_helpers() {
    assert!((se_bump_default_km(1.0, 6371.0, false) - 20.0).abs() < 1e-4);
    assert!((se_bump_default_km(0.0123, 1737.0, false) - 20.0).abs() < 1e-4);
    assert!((se_bump_default_km(0.107, 3390.0, false) - 20.0f32.min(20.0 / (0.107f32 / (3390.0f32 / 6371.0).powi(2)).sqrt())).abs() < 1e-3);
    assert!((se_bump_default_km(1e-9, 100.0, true) - 20.0).abs() < 1e-4);
    let mut c = [0.0, 0.0, 0.3, 0.4, 0.1, 0.2];
    assert_eq!(bulk_from_composition(&c), Some(bulk::AQUARIA));
    set_ice(&mut c, 0.0);
    assert_eq!(c[2], 0.0);
    assert!((c.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    assert_eq!(bulk_from_composition(&c), Some(bulk::TERRA));
    assert!(ice_layer_km(&[0.0, 0.0, 0.5, 0.5, 0.0, 0.0], 6371.0, 1.0) > 500.0);
}

#[test]
fn catalog_class_dominates_the_composition() {
    let mk = |class| BodyIn {
        seeds: crate::surface_gen::catalog_seeds("Test b", "Test"),
        obj_type: obj::PLANET,
        bulk: class,
        mass_earth: 1.0,
        radius_km: 6371.0,
        bond_albedo: None,
        age_gyr: 4.6,
        star_c_to_o: None,
        flux: 1.0,
        xuv: 0.0,
        flux_mean: 1.0,
        orbit: Some((1.0, 0.0, 365.25, 332_946.0)),
        orbit_incl_deg: 0.0,
        orbit_node_deg: 0.0,
        orbit_angles_drawn: [false; 2],
        star_parent: true,
        obliquity_deg: None,
        eq_node_deg: None,
        rotation_days: None,
        tidally_locked: false,
        no_ocean: false,
        no_clouds: false,
        composition: None,
        procedural: false,
    };
    for (class, top) in [(bulk::TERRA, part::SILICATE), (bulk::FERRIA, part::IRON), (bulk::CARBONIA, part::CARBON)] {
        let (c, got) = bulk_composition(&mk(class), 255.0).unwrap();
        assert_eq!(got, class);
        assert!((3..6).all(|i| c[i] <= c[top]), "{class}: {c:?}");
        assert!(c[part::ICE] < 0.1);
    }
    let (c, _) = bulk_composition(&mk(bulk::AQUARIA), 255.0).unwrap();
    assert!(c[part::ICE] >= 0.1);
    assert!(bulk_composition(&mk(bulk::JUPITER), 255.0).is_none());
}

#[test]
#[ignore = "reads SpaceEngine.exe from the install"]
fn gas_table_reads_from_the_exe() {
    let t = tables();
    assert_eq!(t.species.len(), N_GAS);
    let names: Vec<&str> = t.species.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(&names[..12], ["H2", "He", "Ne", "Ar", "Kr", "Xe", "O2", "N2", "NH3", "H2O", "CO", "CO2"]);
    assert_eq!(t.species[CH4].name, "CH4");
    // Boiling points at 1 atm.
    assert!((vapour_pressure(&t, H2O, 373.15) - 1.0).abs() < 0.03);
    assert!((vapour_pressure(&t, CO2, 194.7) - 1.0).abs() < 0.1);
    assert!((vapour_pressure(&t, CH4, 111.7) - 1.0).abs() < 0.05);
    // The table holds ln(mmHg) Antoine coefficients that SE evaluates as log10(Pa), so the
    // generic gases come out far more volatile than they are: N2 boils at 43 atm, not 1.
    assert!((vapour_pressure(&t, 7, 77.4) - 42.98).abs() < 0.1, "N2 {}", vapour_pressure(&t, 7, 77.4));
    assert!(vapour_pressure(&t, H2O, 250.0) < 0.001);
    // M5.5 dwarfs fall past the table (SE's subtype >= 5 rule); a G2 dwarf keeps 7.2%.
    assert_eq!(t.xuv_fraction(7, 6, 55), 0.0);
    assert!((t.xuv_fraction(7, 4, 20) - 0.072).abs() < 1e-4);
}

/// A Proxima planet as SE's completion sees it: SE's resolved mass, the catalog radius, the
/// geometric albedo standing in for the Bond albedo, periapsis light.
fn proxima(names: &str, mass_earth: f32, radius_earth: f32, a_au: f64, e: f64, period_days: f64) -> BodyIn {
    let q = a_au * (1.0 - e);
    BodyIn {
        seeds: crate::surface_gen::catalog_seeds(names, "Proxima"),
        obj_type: obj::PLANET,
        bulk: bulk::TERRA,
        mass_earth,
        radius_km: radius_earth * 6371.0,
        bond_albedo: Some(0.367),
        age_gyr: 4.85,
        star_c_to_o: None,
        flux: (0.0017 / (q * q)) as f32,
        xuv: 0.0,
        flux_mean: (0.0017 / (a_au * (1.0 + 0.5 * e * e)).powi(2)) as f32,
        orbit: Some((a_au, e, period_days, 0.122 * 332_946.0)),
        orbit_incl_deg: 0.0,
        orbit_node_deg: 0.0,
        orbit_angles_drawn: [false; 2],
        star_parent: true,
        obliquity_deg: None,
        eq_node_deg: None,
        rotation_days: None,
        tidally_locked: false,
        no_ocean: false,
        no_clouds: false,
        composition: None,
        procedural: false,
    }
}

fn proxima_b() -> BodyIn {
    proxima("Proxima b/Proxima Cen b/ALF Cen C b", 1.4435, 1.31, 0.04857, 0.0109, 11.18418)
}

#[test]
#[ignore = "reads SpaceEngine.exe from the install"]
fn proxima_planets_complete_like_se() {
    use crate::surface_gen::{temp_class, TEMP_NAMES};
    let t = tables();
    // SE 0.991's panels: b "Cold arid terra", CO2 SO2 N2 H2 at 0.163 atm; d "Cool lacustrine
    // terra", CO2 N2 H2O SO2 at 1.511 atm, water seas.
    let d = proxima("Proxima d/Proxima Cen d/ALF Cen C d", 0.35017, 0.81, 0.02881, 0.0, 5.12338);
    for (body, p_se, gases, word, ocean) in
        [(proxima_b(), 0.163, ["CO2", "SO2", "N2", "H2"], "Cold", false), (d, 1.511, ["CO2", "N2", "H2O", "SO2"], "Cool", true)]
    {
        let c = complete(&t, &body).unwrap();
        let g: Vec<&str> = c.gas_percent(&t).iter().map(|x| x.0).take(4).collect();
        eprintln!("P {} T_s {} avg {} gases {g:?}", c.pressure_atm, c.t_surf, c.t_class);
        assert!(c.locked, "a close orbit around an M dwarf despins within its age");
        assert!((c.pressure_atm / p_se - 1.0).abs() < 0.02, "{} atm", c.pressure_atm);
        assert_eq!(g, gases);
        assert_eq!(TEMP_NAMES[temp_class(c.t_class) as usize], word);
        assert_eq!(c.has_ocean, ocean);
        assert_eq!(c.bulk, bulk::TERRA);
    }
}

#[test]
#[ignore = "reads SpaceEngine.exe from the install"]
fn completion_spread_over_seeds() {
    // Earth analogues at 1 AU: SE's model gives a spread of atmospheres; most keep one.
    let t = tables();
    let mut with_atm = 0;
    let mut oceans = 0;
    for i in 0..200 {
        let mut b = proxima_b();
        b.seeds = crate::surface_gen::catalog_seeds(&format!("Test {i}"), "Sun");
        b.mass_earth = 1.0;
        b.radius_km = 6371.0;
        b.flux = 1.0;
        b.orbit = Some((1.0, 0.0167, 365.25, 332_946.0));
        let c = complete(&t, &b).unwrap();
        assert!(c.pressure_atm.is_finite() && c.t_surf.is_finite(), "{i}: {c:?}");
        with_atm += usize::from(c.has_atmosphere());
        oceans += usize::from(c.has_ocean);
    }
    eprintln!("atmospheres {with_atm}/200, oceans {oceans}/200");
    assert!(with_atm > 100);
}

fn thermal(flux: f32, albedo: f32, pressure: f32, density: f32, greenhouse: f32) -> ThermalIn {
    ThermalIn {
        flux,
        bond_albedo: albedo,
        heat: 1.0,
        greenhouse_k: greenhouse,
        pressure_atm: pressure,
        density_kg_m3: density,
        bulk: bulk::TERRA,
        steam: false,
        locked_to_star: false,
        obliquity_rad: 0.0,
        rotation_days: 1.0,
        mass_earth: 1.0,
        radius_km: 6371.0,
    }
}

#[test]
fn class_temperature_follows_se_model() {
    use crate::surface_gen::{temp_class, TEMP_NAMES};
    // Locked to a star: the day-side temperature (f = 2) times 0.525, then heat and greenhouse.
    let mut m = thermal(1.0, 0.3, 0.0, 0.0, 0.0);
    m.locked_to_star = true;
    m.heat = 0.0;
    let day = (1370.0f32 * (3.826e26 / 3.846e26) as f32 * 0.7 / (2.0 * 5.670_374e-8)).powf(0.25);
    assert!((class_temperature(&m) - day * 0.525).abs() < 0.01);
    m.greenhouse_k = 10.0;
    assert!((class_temperature(&m) - (day * 0.525 + 10.0)).abs() < 0.01);
    // Earth (mean-distance flux of its barycentre orbit) is temperate; Mars cool.
    let mut earth = thermal(0.921, 0.306, 1.0, 1.225, 33.0);
    earth.obliquity_rad = 23.44f64.to_radians();
    let t = class_temperature(&earth);
    assert!((278.0..292.0).contains(&t), "{t}");
    let mut mars = thermal(0.427, 0.25, 0.00636, 0.015, 5.0);
    mars.obliquity_rad = 25.19f64.to_radians();
    mars.rotation_days = 1.026;
    assert_eq!(TEMP_NAMES[temp_class(class_temperature(&mars)) as usize], "Cool");
    // No light at all: 20 K plus heat.
    assert!(class_temperature(&thermal(0.0, 0.3, 0.0, 0.0, 0.0)) < 40.0);
}

#[test]
#[ignore = "reads the catalogs from the install"]
fn solar_system_class_words_match_se() {
    use crate::catalog::{Catalog, ObjectRef};
    use crate::surface_gen::{catalog_input, TEMP_NAMES};
    let vfs = crate::vfs::Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let cat = Catalog::load(&vfs);
    // SE 0.991's info panels.
    for (name, word) in [
        ("Mercury", "Warm"),
        ("Venus", "Hot"),
        ("Earth", "Temperate"),
        ("Moon", "Temperate"),
        ("Mars", "Cool"),
        ("Ceres", "Cool"),
        ("Jupiter I", "Cold"),
        ("Europa", "Cold"),
        ("Ganymede", "Cold"),
        ("Callisto", "Cold"),
        ("Titan", "Cold"),
    ] {
        let Some(ObjectRef::Body(i)) = cat.find(name) else { panic!("{name}") };
        let (_, m, _) = reference_distances_au(&cat, i as usize).unwrap();
        let b = &cat.bodies[i as usize];
        let gh = b.block.sub("Atmosphere").and_then(|a| a.f64("Greenhouse")).unwrap_or(0.0);
        let flux = 1.0 / (m * m);
        let t_eq = 278.0 * (flux * (1.0 - b.albedo.min(0.95))).powf(0.25);
        let (inp, _) = catalog_input(&cat, i as usize, t_eq, gh, flux, None);
        assert_eq!(TEMP_NAMES[inp.classes.temp as usize], word, "{name}: {} K", inp.t_class);
    }
}

#[test]
#[ignore = "reads the catalogs from the install"]
fn panel_temperatures_match_se() {
    use crate::catalog::{Catalog, ObjectRef};
    let vfs = crate::vfs::Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let mut cat = Catalog::load(&vfs);
    crate::system_gen::complete_catalog_orbits(&mut cat);
    let t = tables();
    let eph = crate::ephemeris::Ephemeris::new(&cat);
    let jd = 2_451_545.0;
    // SE 0.991's "Average temperature" on 2000.01.01 12:00 (K) and the tolerance. Proxima b needs
    // SE's default mean anomaly (orbit.rs); 51 Peg b differs by 0.3%, its light not pinned down.
    for (name, se, tol) in [
        ("Mars", 236.632, 0.3),
        ("Ceres", 177.341, 0.3),
        ("Mercury", 402.58, 0.3),
        ("Moon", 276.734, 0.3),
        ("Tethys", 75.69, 0.3),
        ("Hyperion", 103.44, 0.3),
        ("Proxima b", 143.85, 0.3),
        ("Dimidium", 1239.01, 5.0),
    ] {
        let Some(ObjectRef::Body(i)) = cat.find(name) else { panic!("{name}") };
        let light = star_light(&eph, &cat, ObjectRef::Body(i), jd);
        let c = catalog_body_in(&cat, i as usize, &light).and_then(|b| complete(&t, &b));
        let model = display::catalog_model(&cat, i as usize, &light, c.as_ref());
        let p = model.panel(&display::catalog_view(&eph, &cat, i as usize, jd));
        eprintln!("{name:<10} SE {se:8.3}  port {:8.3} (night {:7.2} day {:7.2})", p.average, p.night, p.day);
        assert!((p.average - se).abs() < tol, "{name}: {} K against SE's {se}", p.average);
        assert!(p.night <= p.average && p.average <= p.day, "{name}: {p:?}");
        // The same panels: "Torrid subjupiter" of H2 and He at 1.297e5 atm, "Frigid airless
        // microaquaria", "Cold asteroid".
        let bulk = c.as_ref().map(|c| c.bulk);
        match name {
            "Dimidium" => {
                let c = c.as_ref().unwrap();
                assert!((c.panel_pressure_atm / 1.297e5 - 1.0).abs() < 1e-3, "{}", c.panel_pressure_atm);
                assert_eq!(c.gas_percent(&t).iter().take(2).map(|g| g.0).collect::<Vec<_>>(), ["H2", "He"]);
                assert_eq!(bulk, Some(bulk::JUPITER));
            }
            "Tethys" => assert_eq!(bulk, Some(bulk::AQUARIA)),
            "Hyperion" => assert_eq!(bulk, Some(bulk::ASTEROID)),
            _ => {}
        }
    }
}

#[test]
fn kept_system_walk_gives_the_same_stars_and_views() {
    use crate::catalog::{Catalog, ObjectRef};
    // A binary under a barycentre with a black hole, a planet with a moon and a barycentre of its own.
    let mut cat = Catalog::new();
    cat.load_sc(
        r#"
StarBarycenter "Alpha Centauri" { RA 14 39 36.5 Dec -60 50 02 Dist 1.3474 }
Star "Alpha Centauri A" { ParentBody "Alpha Centauri" Class "G2V" MassSol 1.1 AppMagn 0.01
  Orbit { SemiMajorAxis 10.9 Period 79.91 Eccentricity 0.52 } }
Star "Alpha Centauri B" { ParentBody "Alpha Centauri" Class "K1V" MassSol 0.9
  Orbit { SemiMajorAxis 12.0 Period 79.91 Eccentricity 0.52 ArgOfPericenter 180 } }
Star "Alpha Centauri X" { ParentBody "Alpha Centauri" Class "X" MassSol 8 Orbit { SemiMajorAxis 300 Period 4000 } }
Barycenter "Test bc" { ParentBody "Alpha Centauri B" Orbit { SemiMajorAxis 0.05 Period 4.1 } }
Planet "Test b" { ParentBody "Test bc" Radius 6000 Mass 1 Obliquity 23 RotationPeriod 20 Orbit { SemiMajorAxis 0.0001 Period 1.3 } }
Moon "Test b I" { ParentBody "Test bc" Radius 1000 Mass 0.01 Orbit { SemiMajorAxis 0.001 Period 1.3 } }
"#,
        "test.sc",
    );
    cat.finalize();
    let eph = crate::ephemeris::Ephemeris::new(&cat);
    // Both bodies' year is Test bc's orbit around B.
    let Some(ObjectRef::Star(main)) = cat.find("Alpha Centauri B") else { panic!() };
    // The walk per call, as catalog_view and system_stars did it before the walk was kept.
    let walk_view = |i: usize, jd: f64| {
        let obj = ObjectRef::Body(i as u32);
        let Some(root) = cat.system_root(obj) else { return Vec::new() };
        let mut stars = cat.system_members(&cat.stars[root as usize].names[0]);
        stars.push(ObjectRef::Star(root));
        stars.dedup();
        let pos = eph.position(obj, jd);
        let pole = eph.orientation(obj, jd) * glam::DVec3::Y;
        stars
            .into_iter()
            .filter_map(|s| {
                let ObjectRef::Star(k) = s else { return None };
                let st = &cat.stars[k as usize];
                if st.barycenter || matches!(st.class.kind, crate::star::StarKind::BlackHole | crate::star::StarKind::Planemo) {
                    return None;
                }
                let d = eph.position(s, jd).delta_m(pos);
                let lum = st.lum_bol() as f32;
                let dist = (d.length() / crate::units::METERS_PER_PC) as f32;
                let decl = pole.dot(d.normalize_or_zero()).clamp(-1.0, 1.0).asin() as f32;
                Some(format!("{} {} {} {}", lum.to_bits(), dist.to_bits(), decl.to_bits(), k == main))
            })
            .collect::<Vec<_>>()
    };
    let bits = |v: &[display::StarView]| v.iter().map(|s| format!("{} {} {} {}", s.lum.to_bits(), s.dist_pc.to_bits(), s.decl.to_bits(), s.main)).collect::<Vec<_>>();
    let mut checked = 0;
    for name in ["Test b", "Test b I"] {
        let Some(ObjectRef::Body(i)) = cat.find(name) else { panic!("{name}") };
        let root = cat.system_root(ObjectRef::Body(i)).unwrap();
        let walk = system_walk(&cat, root);
        let (lit, shown) = (system_stars_in(&cat, &walk), display::view_stars(&cat, walk));
        assert_eq!(lit, system_stars(&cat, root));
        // Both stars shown (the black hole left out); the black hole still counts for the light.
        assert_eq!((shown.len(), lit.len()), (2, 3), "{name}");
        for k in 0..20 {
            let jd = 2_451_545.0 + k as f64 * 3.7;
            let view = display::catalog_view_from(&eph, &cat, i as usize, jd, &shown);
            assert_eq!(bits(&view), walk_view(i as usize, jd), "{name} {jd}");
            assert_eq!(bits(&view), bits(&display::catalog_view(&eph, &cat, i as usize, jd)));
            let lat = 0.3 * k as f64 - 1.5;
            let a = display::day_lines_from(&eph, &cat, i as usize, jd, lat, &view);
            assert_eq!(format!("{a:?}"), format!("{:?}", display::day_lines(&eph, &cat, i as usize, jd, lat)));
            let (x, y) = (star_light_from(&eph, &cat, ObjectRef::Body(i), jd, &lit), star_light(&eph, &cat, ObjectRef::Body(i), jd));
            assert_eq!(format!("{x:?}"), format!("{y:?}"));
            checked += view.iter().filter(|s| s.main).count();
        }
    }
    assert_eq!(checked, 40, "each view has its main star");
}

#[test]
fn visual_atmosphere_heights() {
    // SE's AtmoHeightHarb formula: p^0.1 x 7.8 scale heights.
    let earth = se_visual_atm_height_km(1.0, 8.43, 1.0, 6371.0);
    assert!((earth - 65.75).abs() < 0.1, "{earth}");
    // A thin atmosphere is held at twice SE's default BumpHeight (20 km at Earth's gravity).
    assert!((se_visual_atm_height_km(1e-6, 8.0, 1.0, 6371.0) - 40.0).abs() < 1e-3);
    // And at most 10% of the radius.
    assert!((se_visual_atm_height_km(90.0, 200.0, 0.01, 300.0) - 30.0).abs() < 1e-3);
}

#[test]
fn cloud_layers_follow_se() {
    // Earth-like: 10 km relief over a 3 km sea, so the cloud base is mid-relief, 2 km up.
    let c = CloudsIn { radius_km: 6371.0, bump_km: 10.0, sea_km: 3.0, atm_height_km: 100.0, pressure_atm: 1.0, t_surf: 288.0 };
    let mut counts = [0usize; 4];
    for s in 0..600u32 {
        let mut r = SeRng::new(s.wrapping_mul(2_654_435_761), s ^ 0x5bd1);
        let u = draw_a(&mut r);
        let l = cloud_layers(&mut r, u, c);
        counts[l.len()] += 1;
        let n = l.len() as f32;
        // The span is 2 n U(2, 3) km here (under 0.2 H); the lowest layer one step above the base.
        let step = l[0].height_km - 2.0;
        assert!(step >= 4.0 - 1e-3 && step <= 6.0 + 1e-3, "{step}");
        for (i, g) in l.iter().enumerate() {
            assert!((g.height_km - (2.0 + (i + 1) as f32 * step)).abs() < 1e-3);
            assert!(g.bump_km > 0.0 && g.bump_km <= (step * 0.5).min(7.0) + 1e-4);
            assert!((2.0..=500.0).contains(&g.velocity_kmh), "{}", g.velocity_kmh);
        }
        assert!(l.last().unwrap().height_km <= 2.0 + 6.0 * n + 1e-3);
    }
    assert_eq!(counts[0], 0);
    assert!(counts[1..].iter().all(|&k| k > 150), "{counts:?}");
    // Under an atmosphere thinner than 0.5% of the radius: one layer.
    let mut r = SeRng::new(7, 9);
    assert!((0..50).all(|u| cloud_layers(&mut r, u, CloudsIn { atm_height_km: 30.0, ..c }).len() == 1));
    // Dense (Venus-like): the span is U(0.2, 0.3) H whatever the count; no ocean, so mid-relief.
    let venus = CloudsIn { sea_km: 0.0, atm_height_km: 250.0, pressure_atm: 90.0, t_surf: 735.0, ..c };
    let l = cloud_layers(&mut r, 2, venus);
    let top = l.last().unwrap().height_km - 5.0;
    assert_eq!(l.len(), 3);
    assert!((50.0..=75.0).contains(&top), "{top}");
}

#[test]
#[ignore = "reads the catalogs from the install"]
fn catalog_cloud_layers() {
    use crate::catalog::{Catalog, ObjectRef};
    let vfs = crate::vfs::Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let mut cat = Catalog::load(&vfs);
    crate::system_gen::complete_catalog_orbits(&mut cat);
    let t = tables();
    let eph = crate::ephemeris::Ephemeris::new(&cat);
    let mut counts = [0usize; 4];
    for i in 0..cat.bodies.len() {
        let light = star_light(&eph, &cat, ObjectRef::Body(i as u32), 2_451_545.0);
        let Some(c) = catalog_body_in(&cat, i, &light).and_then(|b| complete(&t, &b)) else { continue };
        if !c.has_clouds || c.clouds.is_empty() {
            continue;
        }
        counts[c.clouds.len().min(3)] += 1;
        if counts[1..].iter().sum::<usize>() <= 12 {
            eprintln!("{:<24} {:.2} atm, H {:.0} km: {:?}", cat.bodies[i].names[0], c.pressure_atm, c.atm_height_km, c.clouds);
        }
    }
    eprintln!("layers: {counts:?}");
    assert!(counts[2] > 0 && counts[3] > 0, "{counts:?}");
}

#[test]
fn equilibrium_temperature_follows_se() {
    let mut cat = crate::catalog::Catalog::default();
    cat.load_sc(
        "Planet \"Bond\" { ParentBody \"Sol\" Radius 6371 Mass 1 AlbedoBond 0.3 AlbedoGeom 0.5 }\n\
         Planet \"Geom\" { ParentBody \"Sol\" Radius 6371 Mass 1 AlbedoGeom 0.5 }\n\
         Planet \"Plain\" { ParentBody \"Sol\" Radius 6371 Mass 1 }\n\
         Planet \"Warm\" { ParentBody \"Sol\" Radius 6371 Mass 1 Albedo 0.3 Atmosphere { Greenhouse 33 } }\n",
        "test.sc",
    );
    let t = |name: &str, flux: f64| {
        let Some(crate::catalog::ObjectRef::Body(i)) = cat.find(name) else { panic!("{name}") };
        equilibrium_temperature(flux, &cat.bodies[i as usize])
    };
    // SE's max(20, 278 (F (1 - A_bond))^1/4): the Bond albedo first, the geometric one only alone.
    let earth = 278.0 * 0.7f64.powf(0.25);
    assert!((t("Bond", 1.0) - earth).abs() < 1e-9, "{}", t("Bond", 1.0));
    assert!((t("Geom", 1.0) - 278.0 * 0.5f64.powf(0.25)).abs() < 1e-9);
    assert!((t("Plain", 1.0) - earth).abs() < 1e-9);
    assert!((t("Warm", 1.0) - earth - 33.0).abs() < 1e-9);
    assert!((t("Bond", 1.0) as f32 - equilibrium_temps(1.0, 0.0, Some(0.3)).0).abs() < 1e-3);
    assert_eq!(t("Plain", 1e-12), 20.0);
}
