//! Samples the Moon's bump tiles at the Apollo landing sites the way the port's terrain does, and
//! with the face's v or u flipped, against the lander radii in SE's ApolloLandingSites.sc (which sit
//! on SE's ground), to check how the height tiles are registered.
//! Usage: cargo run --release -p se_core --example moon_heights
use se_core::planet_tiles::{CubeFace, TileKey, TileLayer};
use se_core::vfs::Vfs;
use std::path::Path;

fn main() {
    let vfs = Vfs::mount_install(Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let layer = TileLayer::discover(&vfs, "Moon/Bump-KL", &[]).expect("layer");
    // (name, lon, lat, SE radius km)
    let sites = [
        ("A11", 23.47314, 0.67416, 1735.47409),
        ("A12", -23.4219, -3.0128, 1735.96628),
        ("A14", -17.47194, -3.64589, 1736.32848),
        ("A15", 3.63330, 26.13239, 1734.80195),
        ("A16", 15.5011, -8.9734, 1737.31378),
        ("A17", 30.7723, 20.1911, 1734.36757),
    ];
    let level = layer.max_level.min(7);
    println!("max level {}, sampling level {level}", layer.max_level);
    let flips: [(&str, fn(f64, f64) -> (f64, f64)); 4] =
        [("as port", |u, v| (u, v)), ("v flipped", |u, v| (u, 1.0 - v)), ("u flipped", |u, v| (1.0 - u, v)), ("both", |u, v| (1.0 - u, 1.0 - v))];
    for (name, lon, lat, r_se) in sites {
        let d = se_core::se_script::polar_dir(lon, lat);
        let (face, u, v) = CubeFace::from_dir(glam::DVec3::from_array(d));
        let want = (r_se - 1737.4) * 1000.0;
        let mut line = format!("{name} {:?} u {u:.3} v {v:.3}  SE {want:7.0} m:", face);
        for (label, f) in flips {
            let (fu, fv) = f(u, v);
            let h = height(&vfs, &layer, face, level, fu, fv).map_or(f64::NAN, |s| (s * 19.91 - 9.129) * 1000.0);
            line += &format!("  {label} {h:7.0}");
        }
        println!("{line}");
    }
}

/// The tile's 16-bit sample at face coordinates (u, v), texel i at (i - 1.5) / 252 of the tile.
fn height(vfs: &Vfs, layer: &TileLayer, face: CubeFace, level: u8, u: f64, v: f64) -> Option<f64> {
    let n = (1u32 << level) as f64;
    let key = TileKey { face, level, row: ((v * n).floor() as u32).min(n as u32 - 1), col: ((u * n).floor() as u32).min(n as u32 - 1) };
    let key = layer.source(key);
    let (u0, v0, size) = key.uv_rect();
    let img = image::load_from_memory(&vfs.read(&layer.tile_path(key)).ok()?).ok()?.to_luma32f();
    let (w, h) = (img.width() as f64, img.height() as f64);
    let x = (1.5 + (u - u0) / size * (w - 4.0)).clamp(0.0, w - 1.0);
    let y = (1.5 + (v - v0) / size * (h - 4.0)).clamp(0.0, h - 1.0);
    Some(img.get_pixel(x.round() as u32, y.round() as u32)[0] as f64)
}
