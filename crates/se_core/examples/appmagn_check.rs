//! Catalog stars whose magnitude SE and the port could take from different keys: an intrinsic
//! luminosity (`Lum`, `Luminosity`, `AbsMagn`) next to `AppMagn` and a distance, the
//! `ForceAppMagn` barycentres and the `Hidden` stars.
//! Usage: cargo run --release -p se_core --example appmagn_check
use se_core::catalog::Catalog;
use se_core::vfs::Vfs;
use std::path::PathBuf;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/SteamLibrary/steamapps/common/SpaceEngine"));
    let vfs = Vfs::mount_install(&root).expect("mount");
    let cat = Catalog::load(&vfs);
    let mut both = 0;
    for s in &cat.stars {
        let b = &s.block;
        let lum = b.f64_any(&["Lum", "Luminosity"]).map(se_core::star::abs_mag_from_lum).or(b.f64("AbsMagn"));
        let app = b.f64_any(&["AppMagn", "AppMag", "AppMagnV"]);
        let dist = b.f64("Dist").is_some() || b.f64("Parallax").is_some();
        if let (Some(l), Some(_), true) = (lum, app, dist) {
            both += 1;
            if (l - s.abs_mag).abs() > 0.5 {
                println!("{:<40} intrinsic {:+.2}  from AppMagn {:+.2}", s.names[0], l, s.abs_mag);
            }
        }
        if b.bool("ForceAppMagn") == Some(true) || b.bool("Hidden") == Some(true) {
            println!(
                "{:<40} bary={} ForceAppMagn={:?} Hidden={:?} abs {:+.2}",
                s.names[0],
                s.barycenter,
                b.bool("ForceAppMagn"),
                b.bool("Hidden"),
                s.abs_mag
            );
        }
    }
    println!("{both} stars give an intrinsic luminosity, AppMagn and a distance");
    for (s, off) in cat.stars.iter().zip(cat.forced_system_offsets()) {
        if off != 0.0 {
            println!("{:<40} abs {:+.2} drawn {:+.2} (ForceAppMagn system)", s.names[0], s.abs_mag, s.abs_mag + off as f64);
        }
    }
}
