//! List virtual paths of the SE install containing a substring (case-insensitive), with sizes.
//! Usage: cargo run -p se_core --example vfs_find -- NEEDLE [SE_DIR]
use se_core::vfs::Vfs;
use std::path::PathBuf;

fn main() {
    let needle = std::env::args().nth(1).unwrap_or_default().to_ascii_lowercase();
    let root = std::env::args()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("D:/SteamLibrary/steamapps/common/SpaceEngine"));
    let vfs = Vfs::mount_install(&root).expect("mount");
    for p in vfs.list("") {
        if p.to_ascii_lowercase().contains(&needle) {
            println!("{p} {}", vfs.size(&p).unwrap_or(0));
        }
    }
}
