use super::*;
use crate::dds::{DdsFormat, decode_level, parse_dds};

/// One material-record node: child count, payload size, key, payload.
fn node(children: u8, key: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![children, payload.len() as u8];
    v.extend_from_slice(&key.to_le_bytes());
    v.extend_from_slice(payload);
    v
}

#[test]
fn material_record_tree() {
    let mut b = node(3, 0, &[]);
    b.extend(node(0, KEY_NAME, b"RT_Tree01A_FOL-effect\0"));
    b.extend(node(2, KEY_EFFECT, b"DA_alpha_test_leaves\0"));
    b.extend(node(4, KEY_DIFFUSE, &[]));
    for v in [1.3f32, 1.3, 0.56, 1.0] {
        b.extend(node(0, 0x1234, &v.to_le_bytes()));
    }
    b.extend(node(0, 0x9ac4_33c7, &1e9f32.to_le_bytes()));
    b.extend(node(2, KEY_TEXTURES, &[]));
    b.extend(node(0, 0xcdcd_cdcd, b"Trees\\Branch1A\0"));
    b.extend(node(0, 0xcdcd_cdcd, b"Sphere\0"));
    let m = parse_material_record(&b).unwrap();
    assert_eq!(m.name, "RT_Tree01A_FOL-effect");
    assert_eq!(m.effect, "DA_alpha_test_leaves");
    assert_eq!(m.textures, vec!["Trees\\Branch1A".to_string(), "Sphere".to_string()]);
    assert_eq!(m.diffuse(), [1.3, 1.3, 0.56, 1.0]);
    assert_eq!(m.param("#9ac433c7"), Some(&[1e9f32][..]));
    // A byte past the tree is an error.
    b.push(0);
    assert!(parse_material_record(&b).is_err());
}

#[test]
fn halves() {
    assert_eq!(f16(0x3c00), 1.0);
    assert_eq!(f16(0xc000), -2.0);
    assert_eq!(f16(0x3800), 0.5);
    assert_eq!(f16(0), 0.0);
    // 0x33ae is 0.23999 (the notes' 0.2257 was a misreading).
    assert!((f16(0x33ae) - 0.24).abs() < 1e-4);
}

/// A minimal model: a quad of 4 vertices, one sub-mesh.
fn quad_model() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&205u32.to_le_bytes());
    b.extend_from_slice(&1u32.to_le_bytes());
    b.push(0x10);
    let stream = |b: &mut Vec<u8>, n: u32, bytes: &[u8]| {
        b.extend_from_slice(&n.to_le_bytes());
        b.extend_from_slice(bytes);
    };
    let mut pos = Vec::new();
    for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 2.0, 1.0]] {
        for v in p {
            pos.extend_from_slice(&v.to_le_bytes());
        }
    }
    stream(&mut b, 4, &pos);
    stream(&mut b, 4, &[0, 127, 0].repeat(4));
    stream(&mut b, 0, &[]);
    stream(&mut b, 0, &[]);
    stream(&mut b, 4, &[0x00, 0x3c, 0x00, 0x00].repeat(4));
    for _ in 0..3 {
        stream(&mut b, 0, &[]);
    }
    let mut idx = Vec::new();
    for i in [0u32, 2, 1, 1, 2, 3] {
        idx.extend_from_slice(&i.to_le_bytes());
    }
    stream(&mut b, 6, &idx);
    b.extend_from_slice(&1u32.to_le_bytes());
    for v in [0u32, 0, 6, 4] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    let mut mat = node(1, 0, &[]);
    mat.extend(node(0, KEY_NAME, b"RoadArrow\0"));
    b.push(1);
    b.extend_from_slice(&(mat.len() as u32).to_le_bytes());
    b.extend(mat);
    b
}

#[test]
fn model_layout() {
    let m = parse_model(&quad_model()).unwrap();
    assert_eq!(m.positions.len(), 4);
    assert_eq!(m.normals[0], [0.0, 1.0, 0.0]);
    assert!(m.tangents.is_empty());
    assert_eq!(m.uvs[3], [1.0, 0.0]);
    assert_eq!(m.indices, vec![0, 2, 1, 1, 2, 3]);
    assert_eq!(m.triangle_count(), 2);
    assert_eq!(m.submeshes.len(), 1);
    assert_eq!(m.submeshes[0].material.name, "RoadArrow");
    assert_eq!(m.bounds(), ([0.0, 0.0, 0.0], [1.0, 2.0, 1.0]));
    // Truncation anywhere is an error, never a panic.
    let full = quad_model();
    for cut in 0..full.len() {
        assert!(parse_model(&full[..cut]).is_err(), "cut at {cut}");
    }
}

#[test]
fn global_txt_materials() {
    let src = r#"
MATERIAL
{
	name "billboard_grass"
	effect "DA_Vertex_Alpha_Test" { g_vDiffuse { x 0.45 y 0.50 z 0.29 w 1.0 } }
	textures {"Billboard_Grass"}
	flags8 flags 0xc0 // comment { with a brace
}
MESH_IMPORT { name "tree_0" mesh_gen model COLLADA { file_name "C:\x\y.dae" } }
MATERIAL  { name "RT_Bark1A-effect" effect "D_near" { } textures {"Trees\Bark1A" "Trees\Bark1A_N"} } // [bark_texture]
MATERIAL { preload_textures no name "x" effect "D" { g_fSpecular 1.0 g_fZBias -0.1 } textures {} }
/* MATERIAL { name "hidden" } */
"#;
    let m = parse_materials(src);
    assert_eq!(m.len(), 3);
    let g = &m["billboard_grass"];
    assert_eq!(g.effect, "DA_Vertex_Alpha_Test");
    assert_eq!(g.textures, vec!["Billboard_Grass".to_string()]);
    assert_eq!(g.diffuse(), [0.45, 0.50, 0.29, 1.0]);
    let b = &m["rt_bark1a-effect"];
    assert_eq!(b.textures, vec!["Trees\\Bark1A".to_string(), "Trees\\Bark1A_N".to_string()]);
    assert_eq!(m["x"].param("g_fZBias"), Some(&[-0.1f32][..]));
    assert_eq!(m["x"].param("g_fSpecular"), Some(&[1.0f32][..]));
    assert!(!m.contains_key("tree_0"));
}

#[test]
fn texture_paths() {
    assert_eq!(texture_path("Trees\\Bark1A"), "Textures/Trees/Bark1A.dds");
    assert_eq!(texture_path("windows"), "Textures/windows.dds");
    assert_eq!(texture_path("a/b.DDS"), "Textures/a/b.DDS");
}

#[test]
fn missing_install_is_reported() {
    let dir = std::env::temp_dir().join("se_mr_missing_install_test");
    let e = MrInstall::open(&dir).err().expect("no install there");
    assert!(matches!(e, MrError::NotInstalled(_, "Models.fcm")), "{e}");
}

// ---- the user's install ----------------------------------------------------------------------

/// The user's install (`MR_PATH` or the default); `None` (the test passes, saying so) when absent.
fn install() -> Option<MrInstall> {
    let root = std::env::var_os("MR_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(DEFAULT_MR_PATH));
    match MrInstall::open(&root) {
        Ok(i) => Some(i),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

/// Every model the vegetation and cities notes name.
fn named_models() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut range = |base: &str, n: usize| v.extend((0..n).map(|i| format!("{base}_{i}")));
    range("tree", 16);
    range("bunch", 4);
    range("billboard_grass", 4);
    range("window", 10);
    range("air_condition_window", 4);
    range("chimney", 7);
    range("chimney_facade", 2);
    range("hvac_big", 3);
    range("hvac_small", 3);
    range("antena_small", 5);
    range("antena", 4);
    range("cabin_prop", 2);
    range("pot", 4);
    range("streetlight", 9);
    range("streetlight2", 6);
    range("semaphore", 5);
    range("hydrant", 2);
    range("bench", 5);
    range("dustbin", 7);
    range("dustbin_big", 3);
    range("telephone", 3);
    range("newspaper_vending_machine", 2);
    range("furniture", 3);
    range("vehicle", 25);
    for n in [
        "TerrainGrass",
        "Stone",
        "air_condition_0",
        "awning_0",
        "awning_low_0",
        "antena",
        "roof_fan",
        "roof_sideboard",
        "spotlight_0",
        "water_deposit",
        "bricks_heap_0",
        "fence_0",
        "railing",
        "pole_0",
        "bus_stop_0",
        "mailbox",
        "Drain",
        "Sewer",
        "RoadArrow",
        "Trash",
        "TrashGroup",
        "RoadCrack",
    ] {
        v.push(n.to_string());
    }
    v
}

/// Textures the notes name directly (the rest come from the models' and materials' lists).
const NAMED_TEXTURES: [&str; 30] = [
    "Billboard_Grass",
    "grass_01",
    "rock_01",
    "rock_02",
    "stones_01",
    "NoiseTileableR8G8",
    "Sphere",
    "windows",
    "windows_NG",
    "Asphalt",
    "AsphaltLanes",
    "RoadLines",
    "RoadLine",
    "RoadCrossWalk",
    "RoadIntersection",
    "RoadArrow",
    "road_cracks",
    "cracks",
    "sidewalk",
    "sidewalk_border",
    "RoofTop",
    "brick_0",
    "brick_0_ng",
    "bricks",
    "Windows_ceiling",
    "noise",
    "noise_signed",
    "circle",
    "vehicles_colors",
    "Stonehenge/stones",
];

/// Materials the cities draw by name (not embedded in any model).
const NAMED_MATERIALS: [&str; 9] = [
    "windows",
    "windows_near",
    "windows_night",
    "Road_med",
    "billboard_grass",
    "billboard_grass_xz",
    "streetlight_glow_night",
    "streetlight_glow_night_small",
    "brick_0",
];

/// Every model the notes name loads, and every texture its materials use decodes.
#[test]
fn named_assets_load() {
    let Some(mr) = install() else { return };
    let mut textures: Vec<String> = NAMED_TEXTURES.iter().map(|s| s.to_string()).collect();
    textures.extend((1..=7).chain([9, 10]).flat_map(|i| [format!("Trees\\Bark{i}A"), format!("Trees\\Bark{i}A_N"), format!("Trees\\Bark{i}A_S")]));
    textures.extend((1..=7).map(|i| format!("Trees\\Branch{i}A")).chain(["Trees\\Branch10".to_string()]));
    let mut failures = Vec::new();
    for name in named_models() {
        match mr.model(&name) {
            Ok(m) => {
                if m.positions.is_empty() || m.submeshes.is_empty() {
                    failures.push(format!("model {name}: empty"));
                }
                for s in &m.submeshes {
                    let mat = mr.resolve(&s.material);
                    textures.extend(mat.textures.iter().cloned());
                }
            }
            Err(e) => failures.push(e.to_string()),
        }
    }
    for name in NAMED_MATERIALS {
        match mr.material(name) {
            Some(m) => textures.extend(m.textures.iter().cloned()),
            None => failures.push(format!("material {name} missing from global.txt")),
        }
    }
    textures.retain(|t| !t.is_empty());
    textures.sort_by_key(|t| t.to_ascii_lowercase());
    textures.dedup_by_key(|t| t.to_ascii_lowercase());
    let mut decoded = 0;
    let mut absent = Vec::new();
    for t in &textures {
        if !mr.has_texture(t) {
            // Named by a model's embedded material but not shipped with the game (the window
            // modules' brick_1..6, one HVAC dish): the port draws those with a neutral texture.
            absent.push(t.clone());
            continue;
        }
        match mr.texture(t) {
            Ok((bytes, info)) => {
                // Decode the smallest level of every layer (fast) and the top level of the first.
                for layer in 0..info.layers {
                    if let Err(e) = decode_level(&bytes, &info, layer, info.mips - 1) {
                        failures.push(format!("texture {t} layer {layer}: {e}"));
                    }
                }
                if let Err(e) = decode_level(&bytes, &info, 0, 0) {
                    failures.push(format!("texture {t}: {e}"));
                }
                decoded += 1;
            }
            Err(e) => failures.push(e.to_string()),
        }
    }
    eprintln!("{} models, {} textures decoded; not in the game's files: {absent:?}", named_models().len(), decoded);
    assert!(absent.len() <= 10, "{absent:?}");
    for t in &absent {
        assert!(!NAMED_TEXTURES.iter().any(|n| n.eq_ignore_ascii_case(t)), "{t}");
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
    assert!(decoded > 100, "only {decoded} textures");
}

/// The 16 trees: vertices, triangles, sub-meshes and heights as the notes give them.
#[test]
fn tree_counts() {
    let Some(mr) = install() else { return };
    // (vertices, triangles, height m) from docs/megaton-rainfall-vegetation.md 7.1.
    let table = [
        (2437, 813, 25.3),
        (2926, 980, 24.1),
        (3139, 1047, 22.1),
        (2485, 829, 20.1),
        (3286, 1096, 16.6),
        (3117, 1039, 27.5),
        (3365, 1125, 17.5),
        (3275, 1093, 19.9),
        (4075, 1361, 24.8),
        (3641, 1215, 16.1),
        (2255, 753, 28.4),
        (2776, 928, 17.0),
        (3071, 1027, 30.3),
        (2628, 878, 28.4),
        (3046, 1018, 18.9),
        (3004, 1002, 12.3),
    ];
    for (i, &(verts, tris, height)) in table.iter().enumerate() {
        let m = mr.model(&format!("tree_{i}")).unwrap();
        assert_eq!(m.positions.len(), verts, "tree_{i} vertices");
        assert_eq!(m.triangle_count(), tris, "tree_{i} triangles");
        assert_eq!(m.submeshes.len(), 2, "tree_{i} sub-meshes");
        assert_eq!(m.normals.len(), verts);
        assert_eq!(m.uvs.len(), verts);
        let (lo, hi) = m.bounds();
        assert!((hi[1] - lo[1] - height).abs() < 0.06, "tree_{i} height {}", hi[1] - lo[1]);
        // One bark (D_near) and one leaf (DA_alpha_test_leaves) material, in either order.
        let effects: Vec<&str> = m.submeshes.iter().map(|s| mr.resolve(&s.material).effect.as_str()).collect();
        assert!(effects.contains(&"D_near") && effects.contains(&"DA_alpha_test_leaves"), "tree_{i}: {effects:?}");
        let counts: u32 = m.submeshes.iter().map(|s| s.vertex_count).sum();
        assert_eq!(counts as usize, verts);
    }
    let t0 = mr.model("tree_0").unwrap();
    let s = &t0.submeshes;
    assert_eq!((s[0].first_index, s[0].first_vertex, s[0].index_count, s[0].vertex_count), (0, 0, 1683, 1683));
    assert_eq!((s[1].first_index, s[1].first_vertex, s[1].index_count, s[1].vertex_count), (1683, 1683, 756, 754));
    assert_eq!(s[0].material.name, "RT_Bark1A-effect");
    assert_eq!(s[0].material.textures, vec!["Trees\\Bark1A".to_string(), "Trees\\Bark1A_N".to_string()]);
    // The leaf tint embedded in the model equals global.txt's.
    assert_eq!(s[1].material.diffuse(), [1.3, 1.3, 0.56, 1.0]);
    assert_eq!(mr.resolve(&s[1].material).diffuse(), [1.3, 1.3, 0.56, 1.0]);
}

#[test]
fn known_assets() {
    let Some(mr) = install() else { return };
    let g = mr.model("TerrainGrass").unwrap();
    assert_eq!((g.positions.len(), g.uvs.len(), g.indices.len()), (50_000, 50_000, 0));
    let (lo, hi) = g.bounds();
    assert!(lo[0] >= -75.01 && hi[0] <= 75.01 && lo[2] >= -75.01 && hi[2] <= 75.01);
    let w = mr.model("window_0").unwrap();
    assert_eq!((w.positions.len(), w.submeshes.len()), (20, 2));
    let names: Vec<&str> = w.submeshes.iter().map(|s| s.material.name.as_str()).collect();
    assert_eq!(names, vec!["brick_0", "window"]);
    let v = mr.model("vehicle_6").unwrap();
    assert_eq!((v.positions.len(), v.indices.len(), v.colors.len()), (1710, 1941, 1710));
    assert_eq!(v.submeshes[1].material.name, "streetlight_glow_night_small");
    assert_eq!(mr.resolve(&v.submeshes[0].material).effect, "Vehicle");
    // pot_0: tangents and UVs only for the first sub-mesh's 8 vertices.
    let p = mr.model("pot_0").unwrap();
    assert_eq!((p.positions.len(), p.tangents.len(), p.uvs.len()), (24, 8, 8));

    let (bytes, info) = mr.texture("windows").unwrap();
    assert_eq!((info.width, info.height, info.mips, info.layers, info.format), (1024, 1024, 11, 18, DdsFormat::Bc3));
    assert_eq!(bytes.len(), info.data_offset + info.data_bytes());
    let (_, info) = mr.texture("Billboard_Grass").unwrap();
    assert_eq!((info.width, info.mips, info.format), (2048, 12, DdsFormat::Bc3));
    let (_, info) = mr.texture("Trees\\Bark1A").unwrap();
    assert_eq!((info.width, info.height, info.mips, info.format), (512, 1024, 11, DdsFormat::Bc2));
    let (_, info) = mr.texture("NoiseTileableR8G8").unwrap();
    assert_eq!(info.format, DdsFormat::Rg8Snorm);
    let (_, info) = mr.texture("bricks").unwrap();
    assert_eq!((info.format, info.mips), (DdsFormat::Bgra8, 11));
    // The leaf card's alpha is a cut-out: some texels opaque, some clear.
    let (bytes, info) = mr.texture("Trees\\Branch1A").unwrap();
    let px = decode_level(&bytes, &info, 0, 0).unwrap();
    let clear = px.chunks_exact(4).filter(|p| p[3] < 128).count();
    assert!(clear > px.len() / 40 && clear < px.len() / 4 * 3, "{clear} clear texels");
}

/// Every model in the archive parses, except the skinned ones (people, hands, an animation test)
/// and one landmark shell with other version flags, which the port never draws.
#[test]
fn all_models_parse() {
    let Some(mr) = install() else { return };
    let names = mr.model_names();
    assert!(names.len() > 500, "{} models", names.len());
    let failed: Vec<String> = names.iter().filter(|n| mr.model(n).is_err()).cloned().collect();
    eprintln!("{} of {} models parse; not read: {failed:?}", names.len() - failed.len(), names.len());
    assert!(failed.len() <= 10, "{failed:?}");
    for n in &failed {
        let low = n.to_ascii_lowercase();
        assert!(!low.starts_with("tree_") && !low.starts_with("vehicle_") && !low.starts_with("window_"), "{n}");
    }
}

/// Every DDS in the textures archive parses (header and data length).
#[test]
fn all_textures_parse() {
    let Some(mr) = install() else { return };
    let all = mr.vfs().list("Textures");
    let dds: Vec<&String> = all.iter().filter(|p| p.to_ascii_lowercase().ends_with(".dds")).collect();
    let mut bad = Vec::new();
    for p in &dds {
        let bytes = mr.vfs().read(p).unwrap();
        if let Err(e) = parse_dds(&bytes) {
            bad.push(format!("{p}: {e}"));
        }
    }
    eprintln!("{} of {} textures parse", dds.len() - bad.len(), dds.len());
    // The one volume texture is the only format not read.
    assert!(bad.len() <= 2, "{bad:#?}");
}

/// Writes MR's window array (`windows`, 18 layers) as a contact sheet to `MR_DUMP` (a PNG path), to
/// see what its layers hold (cities, step 7d). `cargo test -p se_core dump_windows -- --ignored`.
#[test]
#[ignore]
fn dump_windows() {
    let Some(mr) = install() else { return };
    let Ok(out) = std::env::var("MR_DUMP") else { return };
    for name in ["windows", "windows_NG", "Windows_ceiling", "noise"] {
        let Ok((bytes, info)) = mr.texture(name) else {
            println!("{name}: missing");
            continue;
        };
        println!("{name}: {}x{} layers {} mips {} format {:?}", info.width, info.height, info.layers, info.mips, info.format);
        let n = info.layers.max(1);
        let s = 256u32;
        let mut sheet = image::RgbaImage::new(s * 6, s * n.div_ceil(6));
        for l in 0..n {
            let mip = (info.width / s).max(1).trailing_zeros().min(info.mips.saturating_sub(1));
            let Ok(rgba) = crate::dds::decode_level(&bytes, &info, l, mip) else { continue };
            let (w, h) = ((info.width >> mip).max(1), (info.height >> mip).max(1));
            let img = image::RgbaImage::from_raw(w, h, rgba).unwrap();
            let img = image::imageops::resize(&img, s, s, image::imageops::FilterType::Triangle);
            image::imageops::overlay(&mut sheet, &img, ((l % 6) * s) as i64, ((l / 6) * s) as i64);
        }
        sheet.save(format!("{out}_{name}.png")).unwrap();
    }
}

/// Prints the sizes, sub-meshes and textures of the street objects (for choosing them; run with
/// `cargo test -p se_core print_street_models -- --ignored --nocapture`).
#[test]
#[ignore]
fn print_street_models() {
    let mr = MrInstall::open(std::path::Path::new(DEFAULT_MR_PATH)).expect("install");
    let mut names: Vec<String> = mr
        .model_names()
        .into_iter()
        .filter(|n| ["vehicle_", "streetlight", "bench_", "dustbin_", "hydrant_", "semaphore_"].iter().any(|p| n.starts_with(p)) && !n.contains("damaged"))
        .collect();
    names.sort();
    for n in names {
        let Ok(m) = mr.model(&n) else {
            println!("{n}: unreadable");
            continue;
        };
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &m.positions {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let subs: Vec<String> = m
            .submeshes
            .iter()
            .map(|s| {
                let r = mr.resolve(&s.material);
                format!("{}:{}i/{}/{:?}", s.material.name, s.index_count, r.effect, r.textures)
            })
            .collect();
        println!("{n}: {}v x {:.2}..{:.2} y {:.2}..{:.2} z {:.2}..{:.2} | {}", m.positions.len(), lo[0], hi[0], lo[1], hi[1], lo[2], hi[2], subs.join(" ; "));
        let mut cols: Vec<[u8; 4]> = m.colors.clone();
        cols.sort();
        cols.dedup();
        let params: Vec<String> = m.submeshes.iter().flat_map(|s| mr.resolve(&s.material).params.iter().map(|(k, v)| format!("{k}={v:?}")).collect::<Vec<_>>()).collect();
        println!("   colours {} {:?} params {:?}", cols.len(), &cols[..cols.len().min(6)], params);
        for s in m.submeshes.iter().filter(|s| s.index_count == 0) {
            let pts: Vec<[f32; 3]> = m.positions[s.first_vertex as usize..(s.first_vertex + s.vertex_count) as usize].to_vec();
            let c: Vec<[u8; 4]> = m.colors[s.first_vertex as usize..(s.first_vertex + s.vertex_count) as usize].to_vec();
            let (mut a, mut b) = (f32::MAX, f32::MIN);
            for i in &m.indices {
                a = a.min(m.positions[*i as usize][0]);
                b = b.max(m.positions[*i as usize][0]);
            }
            println!("   sprites {} {:?} {:?} mesh x {a:.2}..{b:.2}", s.vertex_count, &pts[..pts.len().min(8)], c);
        }
    }
}
