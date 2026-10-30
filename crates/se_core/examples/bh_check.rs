//! List black holes and any star whose position could not be resolved.
//! Usage: cargo run --release -p se_core --example bh_check
use se_core::catalog::Catalog;
use se_core::star::StarKind;
use se_core::vfs::Vfs;
use std::path::PathBuf;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/SteamLibrary/steamapps/common/SpaceEngine"));
    let vfs = Vfs::mount_install(&root).expect("mount");
    let cat = Catalog::load(&vfs);
    let mut unresolved = 0;
    for (i, s) in cat.stars.iter().enumerate() {
        // Skip stars removed by RemoveStar (no longer reachable by name).
        if cat.find(&s.names[0]) != Some(se_core::catalog::ObjectRef::Star(i as u32)) && !s.has_position {
            continue;
        }
        let bh = s.class.kind == StarKind::BlackHole;
        if !s.has_position {
            unresolved += 1;
        }
        if (bh || !s.has_position) && !s.barycenter {
            println!(
                "{:<40} bh={} has_pos={} parent={:?} pos_pc={:.3?} orbit={}",
                s.names[0],
                bh,
                s.has_position,
                s.parent,
                s.pos_pc,
                s.orbit.is_some()
            );
        }
    }
    println!("unresolved stars: {unresolved}");
    for n in ["ALF Eri", "NU Sco (AB)", "NU Sco", "HD 14647", "GAM Lep/HD 38393/HR 1983", "Kepler-1760"] {
        match cat.find(n) {
            Some(se_core::catalog::ObjectRef::Star(i)) => {
                let s = &cat.stars[i as usize];
                println!("{n:?} -> star {:?} has_pos={} parent={:?} src={}", s.names, s.has_position, s.parent, s.source);
            }
            r => println!("{n:?} -> {r:?}"),
        }
    }
}
