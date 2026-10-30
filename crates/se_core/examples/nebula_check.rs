//! Generate the procedural nebula models (Planetary / SNR / Spherical) and compare their clipRadius /
//! sumColor with the values stored in the shipped `.pst` files. Usage: nebula_check
use se_core::galaxy_model::{ModelImages, ModelMethod, RgbaImage, build_galaxy_model, model_textures, parse_pst_kind};
use se_core::vfs::Vfs;

fn load(vfs: &Vfs, name: &Option<String>) -> Option<RgbaImage> {
    let bytes = vfs.read(&format!("textures/nebulae/{}", name.as_ref()?)).ok()?;
    let dynimg = image::load_from_memory(&bytes).ok()?;
    let has_alpha = dynimg.color().has_alpha();
    let mut img = dynimg.to_rgba8();
    if !has_alpha {
        for p in img.pixels_mut() {
            p.0[3] = p.0[2];
        }
    }
    let (w, h) = img.dimensions();
    RgbaImage::new(w, h, img.into_raw())
}

fn assign(vfs: &Vfs) {
    use se_core::galaxy_model::{ModelCatalog, ModelSelection, nebula_position_hash};
    let mut defs = Vec::new();
    for p in vfs.list_ext("models/nebulae", "pst") {
        if let Ok(t) = vfs.read_string(&p) {
            if let Ok(m) = parse_pst_kind(&t, "NebulaModel") {
                defs.extend(m);
            }
        }
    }
    let mut pools: std::collections::HashMap<String, Vec<usize>> = Default::default();
    for (i, d) in defs.iter().enumerate() {
        if let Some(t) = &d.use_for_type {
            pools.entry(t.clone()).or_default().push(i);
        }
    }
    let models = ModelCatalog::new(defs);
    let cat = se_core::catalog::Catalog::load(vfs);
    for rec in &cat.nebulae {
        let mut pick = None;
        for n in &rec.names {
            if let ModelSelection::Model(i) = models.select(n, "E0", [0.0; 3]) {
                pick = Some((i, "named"));
                break;
            }
        }
        if pick.is_none() {
            if let Some(pool) = pools.get(&rec.kind).filter(|p| !p.is_empty()) {
                let h = nebula_position_hash([rec.pos_pc.x, rec.pos_pc.y, rec.pos_pc.z]);
                pick = Some((pool[(h % pool.len() as u32) as usize], "pool"));
            }
        }
        if let Some((i, how)) = pick {
            let m = &models.models()[i];
            if std::env::args().any(|a| a == "everything") || matches!(m.method, ModelMethod::Planetary | ModelMethod::Snr | ModelMethod::Spherical) || rec.kind == "SNR" {
                println!("{:?} [{}] r={:.3}pc d={:.0}pc -> {} {:?} ({how})", rec.names, rec.kind, rec.radius_pc, rec.pos_pc.length(), m.name, m.method);
            }
        }
    }
}

fn main() {
    let all = std::env::args().any(|a| a == "all");
    if std::env::args().any(|a| a == "assign") {
        let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
        assign(&vfs);
        return;
    }
    let (mut ok, mut bad, mut skipped) = (0, 0, 0);
    let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let textures: Vec<String> =
        vfs.list("textures/nebulae").into_iter().filter_map(|p| p.strip_prefix("textures/nebulae/").map(str::to_string)).collect();
    for p in vfs.list_ext("models/nebulae", "pst") {
        let Ok(t) = vfs.read_string(&p) else { continue };
        let Ok(models) = parse_pst_kind(&t, "NebulaModel") else { continue };
        for m in models {
            if !all && !matches!(m.method, ModelMethod::Planetary | ModelMethod::Snr | ModelMethod::Spherical) {
                continue;
            }
            let tex = model_textures(&m, &textures);
            let front = load(&vfs, &tex.front_image);
            let side = load(&vfs, &tex.side_image);
            let cube: Vec<Option<RgbaImage>> = tex.cube.iter().map(|n| load(&vfs, n)).collect();
            let images = ModelImages {
                front: front.as_ref(),
                side: side.as_ref(),
                cube: std::array::from_fn(|i| cube[i].as_ref()),
            };
            if all {
                match (build_galaxy_model(&m, &images), m.clip_radius) {
                    (Ok(d), Some(c)) if (d.clip_radius - c).abs() < 2e-4 => ok += 1,
                    (Ok(d), Some(c)) => {
                        bad += 1;
                        println!("MISMATCH {} {:?} clip {:.5} stored {c:.5}", m.name, m.method, d.clip_radius)
                    }
                    _ => skipped += 1,
                }
                continue;
            }
            match build_galaxy_model(&m, &images) {
                Ok(d) => {
                    let (mut ne, mut na) = (0, 0);
                    for s in &d.sprites {
                        if s.layer() == se_core::galaxy_model::SpriteLayer::Emission { ne += 1 } else { na += 1 }
                    }
                    let mc = d.mean_color.map(|c| (c * 255.0).round());
                    println!(
                        "{} {:?}: sprites em {ne} abs {na} clip {:.5} (stored {:?}) sumColor {:?} (stored {:?}) total_b {:.2}",
                        m.name, m.method, d.clip_radius, m.clip_radius, mc, m.sum_color, d.total_brightness
                    );
                }
                Err(e) => println!("{}: {e}", m.name),
            }
        }
    }
    if all {
        println!("clipRadius match {ok}, mismatch {bad}, skipped/failed {skipped}");
    }
}
