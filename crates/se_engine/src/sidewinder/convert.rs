//! Making the Faulcon DeLacy Sidewinder mod's files from the user's Elite Dangerous install, once:
//! `cargo test -p se_engine --release copy_sidewinder -- --ignored --nocapture` (`ELITE_PATH` names the product folder that
//! holds `Win64`; the default is the user's Steam copy). Elite's install is read only here and in the ignored checks below,
//! never while the game runs (the spec's C4).
//!
//! What it writes (`assets/mods/sidewinder`, the spec's "The mod's files"):
//! - `ship.edm`: the hull is Elite's own simplified Sidewinder (`HULL_FILE`, believed to be the HUD's hologram model: the
//!   in-game hull is not among the open files, Build log step 0.3), its vertex bytes and indices unchanged (grade A); its place,
//!   scale and the landing gear are recreated (grade E) from the user's screenshots and Frontier's published sizes.
//! - `cockpit.edm`: a stand-in cockpit (seat, dashboard, canopy frame), recreated (grade E), until step 0.5's recreation.
//! - `binds/KeyboardMouseOnly.binds`, `binds/ControlPad.binds`: copies of Elite's files (grade A).
//! - `SOURCES.txt` and `mod.cfg` (a `Need` for every file).
//!
//! Model space (the mod's, both files): metres, X right, Y up, -Z forward (the nose), the gear's feet on Y = 0.

use std::path::PathBuf;

use se_core::elite::{self, Vertex};
use se_core::elite_asset::{material_flags, write_edm, Edm, EdmAnimation, EdmDraw, EdmKey, EdmMaterial, EdmPart, EdmTrack};

/// Elite's simplified Sidewinder: its outline from above matches the user's top screenshot (width/length 1.537 against 1.536).
const HULL_FILE: &str = "4387928be519b4cfc8aa67be4113e19e2fbb09f5";
/// Frontier's published size (elitedangerous.com): 14.9 m long, 21.4 m wide. The model's plan is 1.537 wide for its length
/// against 1.44 for these, so the scale makes its plan's area theirs (each side within 3.5 %).
const PUBLISHED_LENGTH_M: f64 = 14.9;
const PUBLISHED_WIDTH_M: f64 = 21.4;
/// The hull's underside over the ground on its gear: measured off the user's back screenshot (`5.webp`): the gear shows
/// 37 px under a hull 614 px wide, at the published 21.4 m width.
const GEAR_M: f32 = 1.29;
/// The gear legs' places as shares of the hull (from the user's underside and back shots, `9.webp`, `5.webp`, by eye: E):
/// the front leg on the middle line 15 % of the length back from the nose, the rear pair 78 % back, 0.21 of the width out.
const FRONT_LEG_BACK: f32 = 0.15;
const REAR_LEG_BACK: f32 = 0.78;
const REAR_LEG_OUT: f32 = 0.21;
/// The gear's travel: retracted, each leg rises this far into the hull (E).
const GEAR_TRAVEL_M: f32 = 1.2;
const GEAR_SECONDS: f32 = 3.0;

fn install() -> PathBuf {
    std::env::var_os("ELITE_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"D:\SteamLibrary\steamapps\common\Elite Dangerous\Products\elite-dangerous-odyssey-64"))
}

fn out_dir() -> PathBuf {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets").join("mods").join("sidewinder");
    std::fs::create_dir_all(out.join("binds")).unwrap();
    out
}

/// The hull's most detailed part as Elite stores it: (indices, vertices).
fn hull_part() -> (Vec<[u16; 3]>, Vec<Vertex>) {
    let raw = std::fs::read(elite::data_file(&install().join("Win64"), HULL_FILE)).expect("the hull's file in the install");
    let m = elite::parse_mesh(&elite::unpack(&raw).unwrap()).unwrap();
    let p = m.chunks.iter().flat_map(|c| c.parts.iter()).max_by_key(|p| p.triangles.len()).unwrap();
    (p.triangles.clone(), p.vertices.clone())
}

/// A vertex in Elite's layout, for the recreated pieces: position in metres at `unit` metres a step, a unit normal, a uv.
fn vertex(p: [f32; 3], n: [f32; 3], uv: [f32; 2], unit: f64) -> Vertex {
    let mut v = [0u8; 40];
    let f = |x: f32| ((x as f64 / unit).round() as i64 + (1 << 20)).clamp(0, (1 << 21) - 1) as u64;
    v[0..8].copy_from_slice(&(f(p[0]) | f(p[1]) << 21 | f(p[2]) << 42).to_le_bytes());
    let b = |x: f32| (x * 127.0 + 127.0).round().clamp(0.0, 254.0) as u8;
    v[8..11].copy_from_slice(&[b(n[0]), b(n[1]), b(n[2])]);
    // A tangent at right angles to the normal.
    let t = if n[1].abs() < 0.9 { [n[2], 0.0, -n[0]] } else { [1.0, 0.0, 0.0] };
    let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-6);
    v[12..15].copy_from_slice(&[b(t[0] / tl), b(t[1] / tl), b(t[2] / tl)]);
    let u = |x: f32| ((x * 2047.0 + 32767.0).round().clamp(0.0, 65535.0) as u16).to_le_bytes();
    v[16..18].copy_from_slice(&u(uv[0]));
    v[18..20].copy_from_slice(&u(uv[1]));
    v[20..24].copy_from_slice(&[0, 0, 0, 255]);
    v[24..32].copy_from_slice(&[0xff, 0x7f, 0xff, 0x7f, 0xff, 0x7f, 0xff, 0x7f]);
    v[36] = 255;
    Vertex(v)
}

/// A flat-shaded box from `lo` to `hi` (metres) as Elite-layout vertices and indices.
fn cuboid(lo: [f32; 3], hi: [f32; 3], unit: f64) -> (Vec<Vertex>, Vec<u16>) {
    let mut v = Vec::new();
    let mut i = Vec::new();
    // (axis, sign): each face's four corners, wound counter-clockwise seen from outside.
    for (axis, sign) in [(0, 1.0f32), (0, -1.0), (1, 1.0), (1, -1.0), (2, 1.0), (2, -1.0)] {
        let (a, b) = ((axis + 1) % 3, (axis + 2) % 3);
        let mut n = [0.0f32; 3];
        n[axis] = sign;
        let corner = |ua: bool, ub: bool| {
            let mut p = [0.0f32; 3];
            p[axis] = if sign > 0.0 { hi[axis] } else { lo[axis] };
            p[a] = if ua { hi[a] } else { lo[a] };
            p[b] = if ub { hi[b] } else { lo[b] };
            p
        };
        let first = v.len() as u16;
        let quad = [(false, false), (true, false), (true, true), (false, true)];
        for (k, (ua, ub)) in quad.iter().enumerate() {
            v.push(vertex(corner(*ua, *ub), n, [(k == 1 || k == 2) as u8 as f32, (k >= 2) as u8 as f32], unit));
        }
        let t = if sign > 0.0 { [0, 1, 2, 0, 2, 3] } else { [0, 2, 1, 0, 3, 2] };
        i.extend(t.iter().map(|k| first + k));
    }
    (v, i)
}

/// Several boxes as one draw.
fn boxes(list: &[([f32; 3], [f32; 3])], unit: f64) -> (Vec<Vertex>, Vec<u16>) {
    let mut v = Vec::new();
    let mut i = Vec::new();
    for (lo, hi) in list {
        let (bv, bi) = cuboid(*lo, *hi, unit);
        let first = v.len() as u16;
        v.extend(bv);
        i.extend(bi.iter().map(|k| first + k));
    }
    (v, i)
}

fn identity_part(name: &str, parent: i16, at: [f32; 3]) -> EdmPart {
    EdmPart { name: name.into(), parent, translation: at, rotation: [0.0, 0.0, 0.0, 1.0] }
}

fn material(name: &str, grey: f32, flags: u32) -> EdmMaterial {
    EdmMaterial { name: name.into(), textures: Vec::new(), tint: [grey, grey, grey, 0.0], flags }
}

/// The ship: the hull (Elite's bytes) placed and scaled, and the recreated landing gear.
pub(super) fn ship_model() -> (Edm, Summary) {
    let (tris, verts) = hull_part();
    let raw: Vec<[i32; 3]> = verts.iter().map(|v| v.position_raw()).collect();
    let lo = [0, 1, 2].map(|k| raw.iter().map(|p| p[k]).min().unwrap());
    let hi = [0, 1, 2].map(|k| raw.iter().map(|p| p[k]).max().unwrap());
    let ext = [0, 1, 2].map(|k| (hi[k] - lo[k]) as f64);
    let unit = (PUBLISHED_LENGTH_M * PUBLISHED_WIDTH_M / (ext[0] * ext[2])).sqrt();
    // Which end is the nose: the narrower one (the user's top shot is a trapezoid narrow at the front).
    let width_near = |front: bool| {
        let cut = if front { hi[2] as f64 - 0.2 * ext[2] } else { lo[2] as f64 + 0.2 * ext[2] };
        let xs: Vec<i32> = raw.iter().filter(|p| if front { p[2] as f64 >= cut } else { p[2] as f64 <= cut }).map(|p| p[0]).collect();
        (xs.iter().max().unwrap() - xs.iter().min().unwrap()) as f64
    };
    let nose_plus_z = width_near(true) < width_near(false);
    let (len, wid, ht) = ((ext[2] * unit) as f32, (ext[0] * unit) as f32, (ext[1] * unit) as f32);
    // The hull's box middle to the model's origin, its underside at the gear's height, its nose along -Z: turned half a turn
    // about Y when Elite's nose is at +Z.
    let mid = [0, 1, 2].map(|k| (lo[k] as f64 + hi[k] as f64) / 2.0 * unit);
    let (rot, sx) = if nose_plus_z { ([0.0, 1.0, 0.0, 0.0], -1.0) } else { ([0.0, 0.0, 0.0, 1.0], 1.0) };
    let hull_at = [(-mid[0] * sx) as f32, GEAR_M - (lo[1] as f64 * unit) as f32, (-mid[2] * sx) as f32];

    let mut parts = vec![identity_part("ship", -1, [0.0; 3]), EdmPart { name: "hull".into(), parent: 0, translation: hull_at, rotation: rot }];
    let materials = vec![material("hull", 0.82, 0), material("gear", 0.32, 0)];
    let mut draws = vec![EdmDraw {
        part: 1,
        material: 0,
        source: format!("{HULL_FILE}#0/0 (Elite's simplified Sidewinder, bytes unchanged)"),
        vertices: verts.clone(),
        indices: tris.iter().flat_map(|t| t.iter().copied()).collect(),
    }];
    // The gear: three legs, each a strut into the hull and a foot, hung from a part of its own at the hull's underside so the
    // gear animation can raise it (recreated: E).
    let legs = [("gear_front", 0.0f32, FRONT_LEG_BACK), ("gear_left", -REAR_LEG_OUT, REAR_LEG_BACK), ("gear_right", REAR_LEG_OUT, REAR_LEG_BACK)];
    let mut tracks = Vec::new();
    for (name, out, back) in legs {
        let at = [out * wid, GEAR_M, -len / 2.0 + back * len];
        parts.push(identity_part(name, 0, at));
        let part = (parts.len() - 1) as u16;
        let (v, i) = boxes(&[([-0.17, -GEAR_M + 0.1, -0.17], [0.17, 0.3, 0.17]), ([-0.5, -GEAR_M, -0.35], [0.5, -GEAR_M + 0.12, 0.35])], unit);
        draws.push(EdmDraw { part, material: 1, source: "recreated: a landing gear leg (E), placed from the user's screenshots".into(), vertices: v, indices: i });
        let key = |t: f32, up: f32| EdmKey { time: t, translation: [at[0], at[1] + up, at[2]], rotation: [0.0, 0.0, 0.0, 1.0] };
        tracks.push(EdmTrack { part, keys: vec![key(0.0, 0.0), key(GEAR_SECONDS, GEAR_TRAVEL_M)] });
    }
    let animations = vec![EdmAnimation { name: "gear_up".into(), seconds: GEAR_SECONDS, tracks }];
    let edm = Edm { metres_per_unit: unit as f32, parts, materials, draws, animations };
    (edm, Summary { unit, len, wid, ht, nose_plus_z, triangles: tris.len() })
}

pub(super) struct Summary {
    pub unit: f64,
    pub len: f32,
    pub wid: f32,
    pub ht: f32,
    pub nose_plus_z: bool,
    pub triangles: usize,
}

/// Where the pilot's eyes are in the model (the stand-in's seat is built round it; the seat camera starts here). The canopy
/// in the user's top shot is on the middle line, 35-58 % of the length back from the nose: the eyes at 42 %, near the top.
pub(super) fn eye_point(s: &Summary) -> [f32; 3] {
    [0.0, GEAR_M + 0.72 * s.ht, -s.len / 2.0 + 0.42 * s.len]
}

/// The stand-in cockpit (E): a seat under the eyes, a dashboard ahead, a canopy frame of struts.
pub(super) fn cockpit_model(s: &Summary) -> Edm {
    let unit = s.unit;
    let [ex, ey, ez] = eye_point(s);
    let seat = boxes(
        &[
            ([ex - 0.32, ey - 1.15, ez - 0.25], [ex + 0.32, ey - 0.95, ez + 0.35]), // the cushion
            ([ex - 0.32, ey - 0.95, ez + 0.28], [ex + 0.32, ey + 0.15, ez + 0.42]), // the back
            ([ex - 0.45, ey - 0.85, ez - 0.2], [ex - 0.33, ey - 0.75, ez + 0.3]),   // the arm rests
            ([ex + 0.33, ey - 0.85, ez - 0.2], [ex + 0.45, ey - 0.75, ez + 0.3]),
        ],
        unit,
    );
    let dash = boxes(&[([ex - 1.4, ey - 1.05, ez - 1.25], [ex + 1.4, ey - 0.55, ez - 0.75]), ([ex - 1.9, ey - 1.1, ez - 0.75], [ex - 1.1, ey - 0.65, ez + 0.4]), ([ex + 1.1, ey - 1.1, ez - 0.75], [ex + 1.9, ey - 0.65, ez + 0.4])], unit);
    let t = 0.06;
    let frame = boxes(
        &[
            // An arch over the head, two struts from the dashboard's corners up to it, a sill along each side.
            ([ex - 1.6, ey + 0.55, ez - 0.1], [ex + 1.6, ey + 0.55 + t, ez + 0.1]),
            ([ex - 1.45 - t, ey - 0.55, ez - 1.2], [ex - 1.45 + t, ey + 0.58, ez - 1.08]),
            ([ex + 1.45 - t, ey - 0.55, ez - 1.2], [ex + 1.45 + t, ey + 0.58, ez - 1.08]),
            ([ex - 0.04, ey + 0.55, ez - 1.2], [ex + 0.04, ey + 0.6, ez + 0.1]),
            ([ex - 1.95, ey - 0.65, ez - 1.2], [ex - 1.85, ey - 0.55, ez + 0.6]),
            ([ex + 1.85, ey - 0.65, ez - 1.2], [ex + 1.95, ey - 0.55, ez + 0.6]),
        ],
        unit,
    );
    let draw = |material: u16, (vertices, indices): (Vec<Vertex>, Vec<u16>), what: &str| EdmDraw { part: 0, material, source: format!("recreated: {what} (E), a stand-in"), vertices, indices };
    Edm {
        metres_per_unit: unit as f32,
        // The eyes: where the seat's camera sits (`SidewinderAssets::eye`).
        parts: vec![identity_part("cockpit", -1, [0.0; 3]), identity_part("eye", 0, [ex, ey, ez])],
        materials: vec![material("seat", 0.18, material_flags::TWO_SIDED), material("dash", 0.12, material_flags::TWO_SIDED), material("frame", 0.08, material_flags::TWO_SIDED)],
        draws: vec![draw(0, seat, "the seat"), draw(1, dash, "the dashboard and side consoles"), draw(2, frame, "the canopy frame")],
        animations: Vec::new(),
    }
}

#[test]
#[ignore]
fn copy_sidewinder() {
    let out = out_dir();
    let version = std::fs::read_to_string(install().join("VersionInfo.txt")).unwrap_or_default().lines().next().unwrap_or("").trim().to_string();
    let (ship, s) = ship_model();
    let cockpit = cockpit_model(&s);
    std::fs::write(out.join("ship.edm"), write_edm(&ship)).unwrap();
    std::fs::write(out.join("cockpit.edm"), write_edm(&cockpit)).unwrap();
    let mut files = vec!["ship.edm".to_string(), "cockpit.edm".to_string()];
    let mut sources = format!(
        "The Faulcon DeLacy Sidewinder mod's files: where each came from. Written by copy_sidewinder (sidewinder/convert.rs).\n\
         Elite Dangerous is Frontier Developments'; every file below taken from its install is theirs ({}).\n\
         Grades are the plan's (docs/spec-mods-sidewinder.md): A = Elite's own bytes, E = recreated.\n\n\
         ship.edm: the hull is Elite file {HULL_FILE} (chunk 0, its most detailed part: {} triangles), vertex bytes and indices unchanged \
         (A). It is Elite's simplified Sidewinder, believed to be the HUD's hologram model; the in-game hull is not among the open files. \
         Its place, scale ({:.1} x {:.1} x {:.1} m) and the landing gear are recreated (E) from the user's screenshots and Frontier's \
         published size. A stand-in until step 0.5's recreation.\n\
         cockpit.edm: recreated (E), a stand-in seat, dashboard and canopy frame.\n",
        if version.is_empty() { "version unknown".to_string() } else { format!("version {version}") },
        s.triangles,
        s.len,
        s.wid,
        s.ht
    );
    for b in ["KeyboardMouseOnly.binds", "ControlPad.binds"] {
        let from = install().join("ControlSchemes").join(b);
        std::fs::copy(&from, out.join("binds").join(b)).unwrap_or_else(|e| panic!("{b}: {e}"));
        files.push(format!("binds/{b}"));
        sources.push_str(&format!("binds/{b}: a copy of Elite's ControlSchemes/{b} (A).\n"));
    }
    std::fs::write(out.join("SOURCES.txt"), sources).unwrap();
    files.push("SOURCES.txt".to_string());
    files.sort();
    let mut cfg = String::from(
        "// The Mods menu reads this at start (mods.rs). Written by copy_sidewinder (sidewinder/convert.rs): every file the mod needs.\n\
         Name \"Faulcon DeLacy Sidewinder\"\n\
         Description \"Elite Dangerous's starter ship: spawn it where you look, walk up and press E to fly it by Elite's rules.\"\n\
         Code \"sidewinder\"\n",
    );
    for f in &files {
        cfg.push_str(&format!("Need \"{f}\"\n"));
    }
    std::fs::write(out.join("mod.cfg"), cfg).unwrap();
    // Everything reads back.
    let back = se_core::elite_asset::parse_edm(&std::fs::read(out.join("ship.edm")).unwrap()).unwrap();
    assert_eq!(back, ship);
    se_core::elite_asset::parse_edm(&std::fs::read(out.join("cockpit.edm")).unwrap()).unwrap();
    for b in ["KeyboardMouseOnly.binds", "ControlPad.binds"] {
        se_core::elite_asset::Binds::parse(&std::fs::read_to_string(out.join("binds").join(b)).unwrap()).unwrap();
    }
    println!(
        "sidewinder: hull {} triangles at {:.3e} m a step, {:.2} x {:.2} x {:.2} m (nose at Elite's {}Z), eyes at {:?}; {} files",
        s.triangles,
        s.unit,
        s.len,
        s.wid,
        s.ht,
        if s.nose_plus_z { "+" } else { "-" },
        eye_point(&s),
        files.len()
    );
}

/// C6 (geometry): the hull's draw holds the install's bytes, unchanged.
#[test]
#[ignore]
fn geometry_is_elites() {
    let ship = se_core::elite_asset::parse_edm(&std::fs::read(out_dir().join("ship.edm")).unwrap()).unwrap();
    let (tris, verts) = hull_part();
    let d = &ship.draws[0];
    assert!(d.source.starts_with(HULL_FILE));
    assert_eq!(d.vertices, verts, "vertex bytes");
    assert_eq!(d.indices, tris.iter().flat_map(|t| t.iter().copied()).collect::<Vec<u16>>(), "indices");
}

/// C12: the shipped key files are Elite's, byte for byte.
#[test]
#[ignore]
fn bindings_match_elite() {
    for b in ["KeyboardMouseOnly.binds", "ControlPad.binds"] {
        let ours = std::fs::read(out_dir().join("binds").join(b)).unwrap();
        let theirs = std::fs::read(install().join("ControlSchemes").join(b)).unwrap();
        assert_eq!(ours, theirs, "{b}");
    }
}
