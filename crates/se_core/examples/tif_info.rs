//! Print the first-IFD tags of a TIFF in the SE install (read in memory) and, for a single-channel image,
//! a grid of its values.
//! Usage: cargo run -p se_core --example tif_info -- VIRTUAL/PATH.tif
use se_core::vfs::Vfs;
use std::path::PathBuf;

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let vfs = Vfs::mount_install(&PathBuf::from("D:/SteamLibrary/steamapps/common/SpaceEngine")).expect("mount");
    let b = vfs.read(&path).expect("read");
    let le = &b[0..2] == b"II";
    let u16_ = |o: usize| if le { u16::from_le_bytes([b[o], b[o + 1]]) } else { u16::from_be_bytes([b[o], b[o + 1]]) };
    let u32_ = |o: usize| if le { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) } else { u32::from_be_bytes(b[o..o + 4].try_into().unwrap()) };
    let ifd = u32_(4) as usize;
    let n = u16_(ifd) as usize;
    println!("{} bytes, le {le}, {n} tags", b.len());
    for i in 0..n {
        let e = ifd + 2 + i * 12;
        let (tag, typ, count) = (u16_(e), u16_(e + 2), u32_(e + 4));
        let val = if typ == 3 && count == 1 { u16_(e + 8) as u32 } else { u32_(e + 8) };
        println!("tag {tag} type {typ} count {count} value {val}");
    }
    if let Some((w, h, px)) = se_core::tiff::read_gray_f32(&b) {
        for y in [0, 8, 16, 24, 32, 40, 48, 56, 63u32.min(h - 1)] {
            let row: Vec<String> = [0, 32, 64, 128, 256, 384, 511u32.min(w - 1)].iter().map(|&x| format!("{:.4}", px[(y * w + x) as usize])).collect();
            println!("row {y}: {}", row.join(" "));
        }
    }
}
