//! Making the Skate 3 mod's files from the Skate 3 Rust Engine's converted data (the user's own copy of Skate 3, set up by that
//! engine): `cargo test -p se_engine --release copy_skate3 -- --ignored --nocapture`.
//!
//! Reads `SKATE3_ASSETS` (default: the engine's install in Downloads) and writes `assets/mods/skate3`: the default skater and
//! his board (`skater.glb`'s mesh, skin and base textures), the clips the mod plays (decoded from `OnBoard.abin` by the
//! engine's own decoder, each frame added onto the rig's T-pose as its `AddBindPose` does), Skate 3's tuning values from its
//! collections, and `mod.cfg` with a `Need` line for every file. Run it again after changing `clips::CLIPS` or the tuning list.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::math::{Mat4, Vec4};
use se_core::skate3::{self, Bone, Clip, Clips, Draw, Joint, Model, Tuning, Vertex};
use serde_json::Value;
use skate_core::animation::{
    output::{self, Sqt},
    pose_add,
};
use skate_data::{animation_banks::AnimationBanks, animation_frames::AnimationFrames, collections::Collections};

use super::clips::CLIPS;

const DEFAULT_ROOT: &str = r"C:\Users\liamw\Downloads\skate3rust-windows-x64\data\installations\f9d8d56e25a04741aadcd4e2e7e93ef1\assets";

fn source_root() -> PathBuf {
    std::env::var_os("SKATE3_ASSETS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT))
}

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/skate3")
}

/// The Skate 3 engine's `render_basis`: its GLB's bone axes are turned -90 degrees about X from the animation's frames.
fn render_basis() -> Mat4 {
    Mat4::from_cols(Vec4::X, -Vec4::Z, Vec4::Y, Vec4::W)
}

fn sqt(s: [f32; 10]) -> Sqt {
    Sqt { scale: [s[0], s[1], s[2], 1.0], rotation: [s[3], s[4], s[5], s[6]], translation: [s[7], s[8], s[9], 1.0] }
}

fn words(w: [u32; 10]) -> [f32; 10] {
    w.map(f32::from_bits)
}

fn flat(s: Sqt) -> [f32; 10] {
    [s.scale[0], s.scale[1], s.scale[2], s.rotation[0], s.rotation[1], s.rotation[2], s.rotation[3], s.translation[0], s.translation[1], s.translation[2]]
}

/// A pose's model matrices (column-major), composed as the engine's `PoseEvaluator::hierarchy` does (`TRAJECTORY` detached).
pub(crate) fn model_matrices(pose: &[[f32; 10]], parents: &[i32]) -> Vec<Mat4> {
    let mut m: Vec<_> = pose.iter().map(|s| output::sqt_to_matrix(sqt(*s))).collect();
    output::compose_hierarchy_in_place(m.len() as i32, parents, 0, &mut m).expect("hierarchy");
    m.iter().map(|r| Mat4::from_cols_array_2d(r)).collect()
}

// --- The GLB -------------------------------------------------------------------------------------------------------

struct Glb {
    json: Value,
    bin: Vec<u8>,
}

impl Glb {
    fn read(path: &Path) -> Glb {
        let f = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(&f[0..4], b"glTF");
        let jlen = u32::from_le_bytes(f[12..16].try_into().unwrap()) as usize;
        let json: Value = serde_json::from_slice(&f[20..20 + jlen]).unwrap();
        let b = 20 + jlen;
        let blen = u32::from_le_bytes(f[b..b + 4].try_into().unwrap()) as usize;
        Glb { json, bin: f[b + 8..b + 8 + blen].to_vec() }
    }

    fn view(&self, i: usize) -> (&[u8], usize) {
        let v = &self.json["bufferViews"][i];
        let off = v["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = v["byteLength"].as_u64().unwrap() as usize;
        (&self.bin[off..off + len], v["byteStride"].as_u64().unwrap_or(0) as usize)
    }

    /// An accessor's elements as floats (normalized integers to 0..1, plain integers as they are).
    fn floats(&self, i: usize) -> Vec<Vec<f32>> {
        let a = &self.json["accessors"][i];
        let n = match a["type"].as_str().unwrap() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            t => panic!("accessor type {t}"),
        };
        let ct = a["componentType"].as_u64().unwrap();
        let size = match ct {
            5126 | 5125 => 4,
            5123 | 5122 => 2,
            5121 | 5120 => 1,
            _ => panic!("component type {ct}"),
        };
        let normalized = a["normalized"].as_bool().unwrap_or(false);
        let count = a["count"].as_u64().unwrap() as usize;
        let (data, stride) = self.view(a["bufferView"].as_u64().unwrap() as usize);
        let data = &data[a["byteOffset"].as_u64().unwrap_or(0) as usize..];
        let stride = if stride == 0 { n * size } else { stride };
        (0..count)
            .map(|e| {
                (0..n)
                    .map(|c| {
                        let p = &data[e * stride + c * size..];
                        match ct {
                            5126 => f32::from_le_bytes(p[..4].try_into().unwrap()),
                            5125 => u32::from_le_bytes(p[..4].try_into().unwrap()) as f32,
                            5123 => {
                                let v = u16::from_le_bytes(p[..2].try_into().unwrap()) as f32;
                                if normalized { v / 65535.0 } else { v }
                            }
                            5121 => {
                                let v = p[0] as f32;
                                if normalized { v / 255.0 } else { v }
                            }
                            _ => panic!("component type {ct}"),
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn indices(&self, i: usize) -> Vec<u32> {
        self.floats(i).into_iter().map(|v| v[0] as u32).collect()
    }
}

/// The skater from `skater.glb`: his skin bound to the animation skeleton's bones by name, and each material's base texture
/// (written beside the model).
fn convert_model(root: &Path, frames: &AnimationFrames, out: &Path) -> (Model, Vec<String>) {
    let glb = Glb::read(&root.join("private/skater.glb"));
    let j = &glb.json;
    let bones: Vec<Bone> = frames.bone_names.iter().zip(&frames.parents).map(|(n, p)| Bone { name: n.clone(), parent: *p as i16 }).collect();
    let skin = &j["skins"][0];
    let ibm = glb.floats(skin["inverseBindMatrices"].as_u64().unwrap() as usize);
    let joints: Vec<Joint> = skin["joints"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&ibm)
        .map(|(node, ib)| {
            let name = j["nodes"][node.as_u64().unwrap() as usize]["name"].as_str().unwrap();
            let bone = frames.bone_names.iter().position(|b| b.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("no animation bone {name}"));
            let ib = Mat4::from_cols_slice(ib);
            Joint { bone: bone as u16, bind: (render_basis() * ib).to_cols_array() }
        })
        .collect();
    let mesh_node = j["nodes"].as_array().unwrap().iter().find(|n| n.get("mesh").is_some()).unwrap();
    assert!(mesh_node.get("matrix").is_none() && mesh_node.get("translation").is_none() && mesh_node.get("rotation").is_none(), "the mesh's node has a transform");
    let mut model = Model { bones, joints, ..Default::default() };
    let mut files = Vec::new();
    std::fs::create_dir_all(out.join("tex")).unwrap();
    for prim in j["meshes"][0]["primitives"].as_array().unwrap() {
        let at = &prim["attributes"];
        let get = |k: &str| glb.floats(at[k].as_u64().unwrap() as usize);
        let (pos, nrm, uv, jt, wt) = (get("POSITION"), get("NORMAL"), get("TEXCOORD_0"), get("JOINTS_0"), get("WEIGHTS_0"));
        let base = model.vertices.len() as u32;
        for i in 0..pos.len() {
            // Weights to 255ths that add up to 255, the largest taking the rounding.
            let w = &wt[i];
            let sum: f32 = w.iter().sum::<f32>().max(1e-6);
            let mut q: [u8; 4] = std::array::from_fn(|k| (w[k] / sum * 255.0).round() as u8);
            let total: i32 = q.iter().map(|v| *v as i32).sum();
            let big = (0..4).max_by(|a, b| w[*a].total_cmp(&w[*b])).unwrap();
            q[big] = (q[big] as i32 + 255 - total).clamp(0, 255) as u8;
            model.vertices.push(Vertex {
                position: [pos[i][0], pos[i][1], pos[i][2]],
                normal: [nrm[i][0], nrm[i][1], nrm[i][2]],
                uv: [uv[i][0], uv[i][1]],
                joints: std::array::from_fn(|k| jt[i][k] as u8),
                weights: q,
            });
        }
        let first = model.indices.len() as u32;
        let idx = glb.indices(prim["indices"].as_u64().unwrap() as usize);
        model.indices.extend(idx.iter().map(|i| i + base));
        let mat = &j["materials"][prim["material"].as_u64().unwrap() as usize];
        let name = mat["name"].as_str().unwrap().trim_start_matches("Retail_").to_lowercase();
        let pbr = &mat["pbrMetallicRoughness"];
        let tint: [f32; 4] = pbr["baseColorFactor"].as_array().map(|a| std::array::from_fn(|k| a[k].as_f64().unwrap() as f32)).unwrap_or([1.0; 4]);
        let texture = match pbr["baseColorTexture"]["index"].as_u64() {
            Some(t) => {
                let image = j["textures"][t as usize]["source"].as_u64().unwrap() as usize;
                let img = &j["images"][image];
                assert_eq!(img["mimeType"].as_str(), Some("image/png"));
                let (bytes, _) = glb.view(img["bufferView"].as_u64().unwrap() as usize);
                let rel = format!("tex/{name}.png");
                std::fs::write(out.join(&rel), bytes).unwrap();
                files.push(rel.clone());
                rel
            }
            None => String::new(),
        };
        let mask = mat["alphaMode"].as_str() == Some("MASK");
        model.draws.push(Draw {
            texture,
            tint,
            alpha_cut: if mask { mat["alphaCutoff"].as_f64().unwrap_or(0.5) as f32 } else { 0.0 },
            two_sided: mat["doubleSided"].as_bool().unwrap_or(false),
            first_index: first,
            index_count: idx.len() as u32,
        });
    }
    (model, files)
}

/// The clips the mod plays, each frame added onto the T-pose, the trajectory left out.
fn convert_clips(frames: &AnimationFrames) -> Clips {
    let tpose: Vec<[f32; 10]> = frames.named_pose("RIG_TPOSE").expect("RIG_TPOSE").samples.iter().map(|w| words(*w)).collect();
    let bones = frames.bone_names.len();
    let identity = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let clips = CLIPS
        .iter()
        .map(|&(name, looping)| {
            let c = frames.clip(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            let fps = f32::from_bits(c.fps_bits);
            let mut samples = Vec::with_capacity(c.frames.len() * bones);
            for f in &c.frames {
                assert_eq!(f.len(), bones);
                for (b, w) in f.iter().enumerate() {
                    samples.push(if b == 0 { identity } else { flat(pose_add::add(sqt(words(*w)), sqt(tpose[b]), true)) });
                }
            }
            Clip { name: name.to_string(), fps, looping, frames: c.frames.len() as u32, samples }
        })
        .collect();
    Clips { bones: bones as u16, clips }
}

/// Skate 3's tuning values the mod uses, by collection, key and field (`skate/physics.rs` reads them by the same names).
fn convert_tuning(root: &Path) -> Tuning {
    let data = Collections::load(root).expect("skater collections");
    let mut t = Tuning::default();
    for (class, key, field) in super::physics::VALUES {
        let v = data.float(class, key, field).unwrap_or_else(|e| panic!("{e}"));
        t.values.insert(format!("{class}/{key}/{field}"), v);
    }
    let gravity = data.words::<4>("physics", "default", "WorldGravity").expect("gravity").map(f32::from_bits);
    t.values.insert("physics/default/WorldGravityY".into(), gravity[1]);
    // Curves: eight points (as 16 or 20 words: `curve8`) or sixteen (32 words).
    for (class, key, field) in super::physics::CURVES {
        let len = data.field(class, key, field).unwrap_or_else(|e| panic!("{e}")).data.chars().filter(|c| !c.is_whitespace()).count() / 8;
        let w: Vec<f32> = match len {
            16 => data.words::<16>(class, key, field).unwrap().map(f32::from_bits).to_vec(),
            20 => data.words::<20>(class, key, field).unwrap().map(f32::from_bits)[4..].to_vec(),
            32 => data.words::<32>(class, key, field).unwrap().map(f32::from_bits).to_vec(),
            n => panic!("{class}/{key}/{field}: {n} words"),
        };
        let n = w.len() / 2;
        t.curves.insert(format!("{class}/{key}/{field}"), (w[..n].to_vec(), w[n..].to_vec()));
    }
    t
}

#[test]
#[ignore]
fn copy_skate3() {
    let root = source_root();
    let out = out_dir();
    let banks = AnimationBanks::load(&root).expect("Skate 3's animation banks");
    let frames = AnimationFrames::from_banks(&banks).unwrap();
    let (model, mut files) = convert_model(&root, &frames, &out);
    let clips = convert_clips(&frames);
    skate3::check_pair(&model, &clips).unwrap();

    // The skin is right if the rig's T-pose puts every vertex back where the GLB has it.
    let tpose: Vec<[f32; 10]> = frames.named_pose("RIG_TPOSE").unwrap().samples.iter().map(|w| words(*w)).collect();
    let mut tp = tpose.clone();
    tp[0] = [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let m = model_matrices(&tp, &frames.parents);
    let skins: Vec<Mat4> = model.joints.iter().map(|j| m[j.bone as usize] * Mat4::from_cols_array(&j.bind)).collect();
    let mut worst = 0.0f32;
    for v in &model.vertices {
        let p = bevy::math::Vec3::from_array(v.position);
        let s: Mat4 = (0..4).map(|k| skins[v.joints[k] as usize] * (v.weights[k] as f32 / 255.0)).fold(Mat4::ZERO, |a, b| a + b);
        worst = worst.max((s.transform_point3(p) - p).length());
    }
    println!("the T-pose puts the vertices within {:.4} m of the bind pose", worst);
    assert!(worst < 0.02, "the skin doesn't fit the skeleton: {worst} m");

    let tuning = convert_tuning(&root);
    std::fs::write(out.join("skater.s3m"), skate3::write_model(&model)).unwrap();
    std::fs::write(out.join("clips.s3c"), skate3::write_clips(&clips)).unwrap();
    std::fs::write(out.join("tuning.cfg"), tuning.write()).unwrap();
    files.extend(["skater.s3m".to_string(), "clips.s3c".to_string(), "tuning.cfg".to_string()]);
    // The sounds (`sfx/`) are written by `copy_skate3_sounds` and listed here when present.
    if let Ok(dir) = std::fs::read_dir(out.join("sfx")) {
        let mut s: Vec<String> = dir.filter_map(|e| e.ok()).map(|e| format!("sfx/{}", e.file_name().to_string_lossy())).filter(|n| n.ends_with(".ogg")).collect();
        s.sort();
        files.extend(s);
    }
    files.sort();
    let mut cfg = String::from(
        "// The Mods menu reads this at start (mods.rs). Written by copy_skate3 (skate/convert.rs): every file the mod needs.\n\
         Name \"Skate 3 physics\"\n\
         Description \"Play as Skate 3's skater: push, carve, ollie and flip tricks on any ground, with Skate 3's own clips and tuning.\"\n\
         Code \"skate3\"\n",
    );
    for f in &files {
        cfg.push_str(&format!("Need \"{f}\"\n"));
    }
    std::fs::write(out.join("mod.cfg"), cfg).unwrap();
    let size: u64 = files.iter().map(|f| std::fs::metadata(out.join(f)).map(|m| m.len()).unwrap_or(0)).sum();
    let per: HashMap<&str, usize> = HashMap::new();
    drop(per);
    println!(
        "skate3: {} bones, {} joints, {} draws, {} vertices, {} triangles; {} clips; {} tuning values and {} curves; {} files, {:.1} MB",
        model.bones.len(),
        model.joints.len(),
        model.draws.len(),
        model.vertices.len(),
        model.indices.len() / 3,
        clips.clips.len(),
        tuning.values.len(),
        tuning.curves.len(),
        files.len(),
        size as f64 / 1e6
    );
}
