//! Sandbox Engine patch c (vendor/ruffle/UPSTREAM.md): the stamp of Ruffle's built-in
//! ActionScript. `core/prebuilt/` holds the compiled built-ins and, in `SOURCES`, the stamp of the
//! sources they were made from; `core/build.rs` refuses prebuilt files whose stamp differs, and
//! `build_playerglobal` writes the stamp when it remakes them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The folders under `core/` holding the built-in ActionScript.
pub const SOURCE_DIRS: [&str; 2] = ["src/avm1/globals", "src/avm2/globals"];

/// The file in `core/prebuilt/` holding the stamp.
pub const STAMP_FILE: &str = "SOURCES";

/// The compiled files `core/prebuilt/` carries (what `core/build.rs` used to make).
pub const PREBUILT_FILES: [&str; 4] = [
    "playerglobal_avm1.swf",
    "playerglobal_avm2.swf",
    "playerglobal_import.abc",
    "native_table.rs",
];

/// FNV-1a (64 bit) over every `.as` file under `core`'s source folders, in path order: each file's
/// path relative to `core` with `/` separators, a zero byte, its bytes without carriage returns
/// (so a checkout's line endings do not matter), a zero byte. Written as 16 hex digits.
pub fn sources_stamp(core: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    for dir in SOURCE_DIRS {
        collect(&core.join(dir), &mut files)?;
    }
    let mut named: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|path| {
            let relative = path.strip_prefix(core).unwrap_or(&path);
            let name = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            (name, path)
        })
        .collect();
    named.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for (name, path) in named {
        feed(name.as_bytes());
        feed(&[0]);
        let bytes: Vec<u8> = fs::read(&path)?.into_iter().filter(|&b| b != b'\r').collect();
        feed(&bytes);
        feed(&[0]);
    }
    Ok(format!("{hash:016x}"))
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "as") {
            out.push(path);
        }
    }
    Ok(())
}
