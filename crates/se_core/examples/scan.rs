//! Mount a SpaceEngine install and load every catalog, printing statistics.
//! Usage: cargo run --release -p se_core --example scan -- "D:/.../SpaceEngine"
use se_core::catalog::{Catalog, ObjectRef};
use se_core::vfs::Vfs;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/SteamLibrary/steamapps/common/SpaceEngine"));
    let t = Instant::now();
    let vfs = Vfs::mount_install(&root).expect("mount");
    let enc: usize = vfs.archives().iter().map(|a| a.encrypted_entries).sum();
    println!(
        "mounted {} paks, {} files ({} encrypted) in {:.2?}",
        vfs.archives().len(),
        vfs.file_count(),
        enc,
        t.elapsed()
    );
    let t = Instant::now();
    let cat = Catalog::load(&vfs);
    println!(
        "catalog: {} stars, {} bodies, {} galaxies, {} clusters, {} nebulae in {:.2?}",
        cat.stars.len(),
        cat.bodies.len(),
        cat.galaxies.len(),
        cat.clusters.len(),
        cat.nebulae.len(),
        t.elapsed()
    );
    for w in cat.warnings.iter().take(8) {
        println!("  warn: {w}");
    }
    for name in ["Sun", "Sirius", "Betelgeuse", "Proxima Centauri", "Earth", "Moon", "Titan", "Andromeda", "Milky Way", "Pleiades", "Orion Nebula"] {
        match cat.find(name) {
            Some(ObjectRef::Star(i)) => {
                let s = &cat.stars[i as usize];
                println!(
                    "  {name}: star class={} Mv={:.2} T={:.0} R={:.0}km d={:.3}pc parent={:?}",
                    s.class.raw, s.abs_mag, s.teff, s.radius_km, s.pos_pc.length(), s.parent
                );
            }
            Some(ObjectRef::Body(i)) => {
                let b = &cat.bodies[i as usize];
                println!(
                    "  {name}: body parent={} R={}km class={:?} period={:.3}d",
                    b.parent,
                    b.radius_km,
                    b.class,
                    b.orbit.as_ref().map(|o| o.period / 86400.0).unwrap_or(0.0)
                );
            }
            Some(ObjectRef::Galaxy(i)) => {
                let g = &cat.galaxies[i as usize];
                println!("  {name}: galaxy {} d={:.0}pc R={:.0}pc M={:.2}", g.kind, g.pos_pc.length(), g.radius_pc, g.abs_mag);
            }
            Some(r) => println!("  {name}: {r:?}"),
            None => println!("  {name}: NOT FOUND"),
        }
    }
    let t = Instant::now();
    let bytes = vfs.read("textures/planets/Earth/Surface-PBC/neg_x/5_0_0.jpg").map(|b| b.len());
    println!("read tile: {bytes:?} in {:.2?}", t.elapsed());
    println!("shader read: {:?}", vfs.read("shaders/3D_anaglyph.glsl").map(|b| b.len()).err().map(|e| e.to_string()));
}
