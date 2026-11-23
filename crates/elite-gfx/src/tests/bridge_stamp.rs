//! `as3/bridge.abc` (and the test block `as3/proof.abc`) must be compiled from the ActionScript beside
//! them: `as3/bridge.stamp` holds the stamp of the sources they were made from (written by
//! `as3/build.sh` through `write_bridge_stamp`).

use std::path::{Path, PathBuf};

fn as3() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("as3")
}

/// FNV-1a (64 bit) over the `.as` files of `as3/elitegfx` and `as3/proof` in name order: each
/// `folder/name`, a zero byte, its bytes without carriage returns, a zero byte. 16 hex digits.
fn sources_stamp() -> String {
    let mut names: Vec<String> = Vec::new();
    for folder in ["elitegfx", "proof"] {
        for entry in std::fs::read_dir(as3().join(folder)).expect(folder) {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if name.ends_with(".as") {
                names.push(format!("{folder}/{name}"));
            }
        }
    }
    names.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for name in names {
        feed(name.as_bytes());
        feed(&[0]);
        let bytes: Vec<u8> = std::fs::read(as3().join(&name)).unwrap().into_iter().filter(|&b| b != b'\r').collect();
        feed(&bytes);
        feed(&[0]);
    }
    format!("{hash:016x}")
}

#[test]
fn bridge_block_is_compiled_from_its_sources() {
    let abc = std::fs::read(as3().join("bridge.abc")).expect("as3/bridge.abc: run as3/build.sh");
    assert!(abc.len() > 1000, "as3/bridge.abc is too small to be the bridge");
    assert!(as3().join("proof.abc").is_file(), "as3/proof.abc: run as3/build.sh");
    let stamp = std::fs::read_to_string(as3().join("bridge.stamp")).expect("as3/bridge.stamp: run as3/build.sh");
    assert_eq!(stamp.trim(), sources_stamp(), "as3/bridge.abc is older than its sources: run as3/build.sh");
}

/// Writes the stamp of the sources: only as3/build.sh, after compiling, runs it (it sets
/// ELITE_GFX_WRITE_STAMP), so running the ignored proofs never hides a stale block.
#[test]
#[ignore]
fn write_bridge_stamp() {
    if std::env::var_os("ELITE_GFX_WRITE_STAMP").is_none() {
        return;
    }
    std::fs::write(as3().join("bridge.stamp"), sources_stamp() + "\n").unwrap();
}
