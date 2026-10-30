//! Print the size and pixel format of DDS files in the VFS, and whether `se_core::dds` decodes them.
//! Usage: dds_probe PATH...
use se_core::vfs::Vfs;

fn main() {
    let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    for p in std::env::args().skip(1) {
        let Ok(b) = vfs.read(&p) else {
            println!("{p}: unreadable");
            continue;
        };
        let u = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let fourcc = String::from_utf8_lossy(&b[84..88]).to_string();
        let mips = u(28);
        let dx10 = if &b[84..88] == b"DX10" { format!(" dxgi {}", u(128)) } else { String::new() };
        let mean = se_core::dds::decode_dds(&b).ok().map(|d| {
            let n = (d.rgba.len() / 4) as f64;
            let s = d.rgba.chunks_exact(4).fold([0.0f64; 3], |a, c| [a[0] + c[0] as f64, a[1] + c[1] as f64, a[2] + c[2] as f64]);
            s.map(|v| (v / n / 255.0 * 1000.0).round() / 1000.0)
        });
        println!(
            "{p}: {}x{} fourcc {fourcc:?}{dx10} rgbbits {} mips {mips} bytes {} mean rgb {mean:?}",
            u(16),
            u(12),
            u(88),
            b.len()
        );
    }
}
