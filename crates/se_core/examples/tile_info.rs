//! Prints the files of a planet texture layer and the pixel format of one tile, to check how
//! the bump tiles are encoded.
//! Usage: cargo run --release -p se_core --example tile_info -- textures/planets/Moon/Bump-KL [N]
use se_core::vfs::Vfs;
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = &a[0];
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let vfs = Vfs::mount_install(Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let files = vfs.list(dir);
    println!("{} entries in {dir}", files.len());
    for f in files.iter().take(n) {
        println!("  {f}");
    }
    // The first file that is an image: its header bytes and decoded format.
    for f in &files {
        let Ok(bytes) = vfs.read(f) else { continue };
        if bytes.len() < 16 {
            continue;
        }
        println!("{f}: {} bytes, head {:02x?}", bytes.len(), &bytes[..16]);
        if let Ok(img) = image::load_from_memory(&bytes) {
            println!("  decoded {:?} {}x{}", img.color(), img.width(), img.height());
            let l = img.to_luma32f();
            let v: Vec<f32> = l.into_raw();
            let (mn, mx) = v.iter().fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
            let mean = v.iter().sum::<f32>() / v.len() as f32;
            println!("  luma min {mn:.4} max {mx:.4} mean {mean:.4}");
        }
        break;
    }
}
