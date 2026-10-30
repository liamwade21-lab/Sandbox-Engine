//! The Milky Way model's emission voxel map as SE's star generator reads it (the red channel at
//! box-normalised coordinates, FUN_140440840): statistics and the value around the Sun's ring.
//! Usage: mw_map_probe [MODEL_NAME]
use se_core::galaxy_model::{ModelCatalog, ModelImages, RgbaImage, build_galaxy_model, model_textures, parse_pst};
use se_core::vfs::Vfs;

fn load(vfs: &Vfs, name: &Option<String>) -> Option<RgbaImage> {
    let bytes = vfs.read(&format!("textures/galaxies/{}", name.as_ref()?)).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    RgbaImage::new(w, h, img.into_raw())
}

fn main() {
    let want = std::env::args().nth(1).unwrap_or_else(|| "Milky Way".into());
    let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let mut defs = Vec::new();
    for p in vfs.list_ext("models/galaxies", "pst") {
        if let Ok(Ok(m)) = vfs.read_string(&p).map(|t| parse_pst(&t)) {
            defs.extend(m);
        }
    }
    let textures: Vec<String> = vfs.list("textures/galaxies").into_iter().filter_map(|p| p.strip_prefix("textures/galaxies/").map(str::to_string)).collect();
    let models = ModelCatalog::new(defs);
    let Some(m) = models.models().iter().find(|m| m.use_for_object.iter().any(|n| *n == want) || m.name == want) else {
        println!("no model for {want}");
        return;
    };
    let tex = model_textures(m, &textures);
    let (front, side) = (load(&vfs, &tex.front_image), load(&vfs, &tex.side_image));
    let cube: Vec<Option<RgbaImage>> = tex.cube.iter().map(|n| load(&vfs, n)).collect();
    let images = ModelImages { front: front.as_ref(), side: side.as_ref(), cube: std::array::from_fn(|i| cube[i].as_ref()) };
    let d = build_galaxy_model(m, &images).unwrap();
    let Some(map) = d.emission_map else {
        println!("{} has no emission map", m.name);
        return;
    };
    println!("{} {:?}: map {}x{}x{}, model scale {:?}", m.name, m.method, map.nx, map.ny, map.nz, m.model_scale());
    let r: Vec<f32> = map.data.chunks(3).map(|c| c[0]).collect();
    let nz_vox: Vec<f32> = r.iter().copied().filter(|v| *v > 0.0).collect();
    let max = r.iter().copied().fold(0.0, f32::max);
    let mean = nz_vox.iter().sum::<f32>() / nz_vox.len().max(1) as f32;
    println!("R channel: max {max:.3}, mean of non-zero voxels {mean:.3} ({} of {})", nz_vox.len(), r.len());
    for rr in [0.0f32, 0.1, 0.2, 0.3, 0.4, 0.45, 0.5, 0.6, 0.8] {
        let n = 72;
        let vals: Vec<f32> = (0..n)
            .map(|k| {
                let a = k as f32 / n as f32 * std::f32::consts::TAU;
                map.sample_se(rr * a.cos(), rr * a.sin(), 0.0)
            })
            .collect();
        let avg = vals.iter().sum::<f32>() / n as f32;
        let hi = vals.iter().copied().fold(0.0, f32::max);
        let lo = vals.iter().copied().fold(f32::MAX, f32::min);
        println!("ring r={rr:.2}: R mean {avg:.3} min {lo:.3} max {hi:.3}");
    }
    for z in [0.0f32, 0.05, 0.1, 0.2, 0.4, 0.7] {
        println!("r=0.42 z={z:.2}: R {:.3}", map.sample_se(0.42, 0.0, z));
    }
}
