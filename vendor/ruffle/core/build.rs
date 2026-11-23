//! Sandbox Engine patch c (vendor/ruffle/UPSTREAM.md): the built-in ActionScript is carried already
//! compiled in `prebuilt/` and copied to OUT_DIR, so a build needs no Java. `prebuilt/` is remade by
//! `vendor/ruffle/remake_playerglobal.sh` (Java 21); the build stops if the ActionScript sources
//! are not the ones the prebuilt files were made from.

use std::path::{Path, PathBuf};

fn main() {
    assert!(
        !cfg!(feature = "known_stubs"),
        "known_stubs needs the stub list, which the prebuilt built-ins do not carry"
    );
    let core = Path::new(".");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let prebuilt = core.join("prebuilt");

    let stamp = std::fs::read_to_string(prebuilt.join(playerglobal_stamp::STAMP_FILE))
        .expect("core/prebuilt/SOURCES is missing: run vendor/ruffle/remake_playerglobal.sh");
    let sources = playerglobal_stamp::sources_stamp(core).expect("read the built-in ActionScript");
    assert_eq!(
        stamp.trim(),
        sources,
        "core/prebuilt was made from other ActionScript than core/src/avm*/globals: \
         run vendor/ruffle/remake_playerglobal.sh (needs Java 21)"
    );
    for file in playerglobal_stamp::PREBUILT_FILES {
        std::fs::copy(prebuilt.join(file), out_dir.join(file))
            .unwrap_or_else(|e| panic!("copy core/prebuilt/{file}: {e}"));
    }

    println!(
        "cargo:rustc-env=RUFFLE_PLAYERGLOBAL_ABC_PATH={}/playerglobal_import.abc",
        out_dir.to_str().unwrap()
    );
    println!("cargo:rerun-if-changed=prebuilt/");
    for dir in playerglobal_stamp::SOURCE_DIRS {
        println!("cargo:rerun-if-changed={dir}/");
    }
}
