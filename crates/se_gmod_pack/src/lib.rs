//! The Garry's Mod files the sandbox uses (the user's 2026-11-12: built into the game, nothing read from Garry's Mod's
//! install while it runs), as one pack (`se_core::gmod::write_pack`), read from memory by `GmodInstall::open_pack`.
//! Empty when `gmod.pak` was missing at build time.

include!(concat!(env!("OUT_DIR"), "/pack.rs"));
