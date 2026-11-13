//! Builds `gmod.pak` (written by se_engine's ignored test `copy_garrys_mod`) into the crate, or an empty pack when it is
//! missing (the sandbox then has plain props and plain sounds).

use std::path::PathBuf;

fn main() {
    let pack = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("gmod.pak");
    println!("cargo:rerun-if-changed={}", pack.display());
    let code = if pack.is_file() {
        format!("pub static PACK: &[u8] = include_bytes!({:?});\n", pack.to_string_lossy())
    } else {
        "pub static PACK: &[u8] = &[];\n".to_string()
    };
    std::fs::write(PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pack.rs"), code).unwrap();
}
