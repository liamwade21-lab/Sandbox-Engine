//! Builds the port's assets (shaders, textures, models, Megaton Rainfall's copy, the mods) into the exe, so a player's copy is the
//! exe alone and needs nothing but SpaceEngine (`embedded_assets.rs` unpacks them on first start).
//! Megaton Rainfall's copy (`assets/mr`) and the mods (`assets/mods`, the Omni-Man mod) are built in
//! too (the user's decision of 2026-11-09). The Garry's Mod files the sandbox uses are built in by the `se_gmod_pack` crate (one
//! pack read from memory, never unpacked).

use std::path::{Path, PathBuf};

const DIRS: [&str; 5] = ["shaders", "textures", "models", "mr", "mods"];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets");
    let mut files = Vec::new();
    for d in DIRS {
        println!("cargo:rerun-if-changed={}", root.join(d).display());
        walk(&root.join(d), &mut files);
    }
    // FNV-1a over every name and its bytes: the unpacked copy is rewritten when this changes.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |b: &[u8]| {
        for &x in b {
            hash ^= x as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    let mut code = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        feed(rel.as_bytes());
        feed(&std::fs::read(f).unwrap());
        code.push_str(&format!("    ({rel:?}, include_bytes!({:?})),\n", f.to_string_lossy()));
    }
    code.push_str("];\n");
    code.push_str(&format!("pub const STAMP: &str = \"{hash:016x}-{}\";\n", files.len()));
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("embedded_assets.rs");
    std::fs::write(out, code).unwrap();
}
