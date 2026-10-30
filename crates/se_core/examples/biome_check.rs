//! Load the surface presets from the install and print the preset chosen for a few bodies.
//! Usage: cargo run --release -p se_core --example biome_check [INSTALL_DIR]
use se_core::biomes::{BodyInfo, SurfacePresets, temp_class};
use se_core::vfs::Vfs;
use std::path::PathBuf;

fn body(name: &str, parent: &str, class: &str, temp: f64, atmo: bool, ocean: bool) -> BodyInfo {
    BodyInfo {
        name: name.into(),
        parent: parent.into(),
        class: class.into(),
        temperature_k: temp,
        has_atmosphere: atmo,
        has_ocean: ocean,
        surf_style: 0.5,
        ..Default::default()
    }
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/SteamLibrary/steamapps/common/SpaceEngine"));
    let vfs = Vfs::mount_install(&root).expect("mount");
    let sp = SurfacePresets::load(&vfs);
    println!(
        "{} presets, {} soils, {} sample textures, {} warnings",
        sp.presets.len(),
        sp.soils.len(),
        sp.sample_textures().len(),
        sp.warnings.len()
    );
    for w in sp.warnings.iter().take(20) {
        println!("  warning: {w}");
    }

    let mut earth = body("Earth", "Earth-Moon", "Terra", 288.0, true, true);
    earth.life_organic = true;
    earth.multicellular = true;
    let bodies = [
        body("Moon", "Earth-Moon", "Selena", 250.0, false, false),
        body("Mars", "Sol", "Terra", 210.0, true, false),
        earth,
        body("Synthetic airless Selena 250K", "X", "Selena", 250.0, false, false),
        body("Synthetic arid Terra 300K atmo", "X", "Terra", 300.0, true, false),
        body("Synthetic Aquaria 150K", "X", "Aquaria", 150.0, false, false),
        body("Synthetic Ferria 900K", "X", "Ferria", 900.0, false, false),
    ];
    for b in &bodies {
        match sp.find_for_ex(b, None) {
            Some((p, relaxed)) => {
                let soils = p.soils_of("Mineral").map_or(0, <[String]>::len);
                println!(
                    "{:<32} {:<10} -> {:<36} w={:<7} {} biomes, {} mineral soils, relaxed={:?}",
                    b.name,
                    temp_class(b.temperature_k),
                    p.file,
                    p.weight,
                    p.biomes.len(),
                    soils,
                    relaxed
                );
            }
            None => println!("{:<32} -> no preset", b.name),
        }
    }
}
