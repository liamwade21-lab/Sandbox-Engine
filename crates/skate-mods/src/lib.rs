//! Trimmed stand-in for upstream `skate-mods`: only the data types the skater simulation names
//! (`presentation`, `extensions`), copied unchanged with the two small helpers they use. No Lua.
// (Parts of the copied files the simulation never reaches.)
#![allow(dead_code)]
pub mod extensions;
pub mod presentation;
mod lua_list;
mod schema;
