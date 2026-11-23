//! Manually builds `playerglobal_avm1.swf` and `playerglobal_avm2.swf` without building the `core` crate.
//! This binary is invoked as:
//! `cargo run --package=build_playerglobal <repo_root> <out_dir>`
//! where `<repo_root>` is the location of the Ruffle repository,
//! and `out_dir` is the directory where the two SWFs should
//! be written
//!
//! Sandbox Engine patch c (vendor/ruffle/UPSTREAM.md): it writes both built-ins and the stamp of
//! their sources, which is how `core/prebuilt/` is made (`remake_playerglobal.sh`).

mod cli;

use clap::Parser;

use cli::Commands;
use std::path::{Path, PathBuf};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = root.join("../../");

    let args = cli::Cli::parse();
    match args.command {
        Commands::Compile { out_dir } => {
            let out_dir = Path::new(&out_dir);
            build_playerglobal::build_avm1_playerglobal(&repo_root, out_dir).unwrap();
            build_playerglobal::build_avm2_playerglobal(&repo_root, out_dir, false).unwrap();
            let stamp = playerglobal_stamp::sources_stamp(&repo_root.join("core")).unwrap();
            std::fs::write(out_dir.join(playerglobal_stamp::STAMP_FILE), stamp + "\n").unwrap();
        }
    }
}
