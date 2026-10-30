//! Print lines (with context) matching a needle in every .sc/.cfg file under a VFS dir.
//! Usage: grep_vfs DIR NEEDLE [CONTEXT_AFTER]
use se_core::vfs::Vfs;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (dir, needle) = (&a[0], &a[1]);
    let after: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(12);
    let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    for ext in ["sc", "cfg", "se", "csv"] {
        for f in vfs.list_ext(dir, ext) {
            let Ok(t) = vfs.read_string(&f) else { continue };
            let lines: Vec<&str> = t.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                if l.contains(needle.as_str()) {
                    println!("{f}:{}", i + 1);
                    for l in &lines[i.saturating_sub(1)..(i + after).min(lines.len())] {
                        println!("  {l}");
                    }
                }
            }
        }
    }
}
