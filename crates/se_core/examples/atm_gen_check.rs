//! Generate an atmosphere model's tables from `atmospheres.cfg` as SE does and compare them with the
//! shipped `.atm` file. Usage: atm_gen_check [MODEL] [analytic]
use se_core::atm::AtmTables;
use se_core::vfs::Vfs;

fn stats(name: &str, a: &[f32], b: &[f32]) {
    let (mut sum_d, mut sum_b, mut worst) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        let d = (*x as f64 - *y as f64).abs();
        sum_d += d;
        sum_b += (*y as f64).abs();
        if y.abs() > 1e-6 {
            worst = worst.max(d / y.abs() as f64);
        }
    }
    println!("{name}: mean abs diff / mean abs value = {:.4}, worst relative {:.3}", sum_d / sum_b.max(1e-30), worst);
}

fn main() {
    let model = std::env::args().nth(1).unwrap_or_else(|| "Earth".into());
    let analytic = std::env::args().any(|a| a == "analytic");
    let vfs = Vfs::mount_install(std::path::Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).unwrap();
    let cfg = vfs.read_string("models/atmospheres/atmospheres.cfg").unwrap();
    let models = se_core::atm_gen::parse_models(&cfg);
    let Some((_, p)) = models.iter().find(|(n, _)| n.eq_ignore_ascii_case(&model)) else {
        println!("no model {model}; have {:?}", models.iter().map(|m| &m.0).collect::<Vec<_>>());
        return;
    };
    let shipped = AtmTables::parse(&vfs.read(&format!("models/atmospheres/{model}.atm")).unwrap()).unwrap();
    println!("cfg {:?}\natm {:?}", p, shipped.params);
    let t = std::time::Instant::now();
    let g = se_core::atm_gen::generate(p, analytic);
    println!("generated in {:.1?}", t.elapsed());
    stats("transmittance", &g.transmittance, &shipped.transmittance);
    stats("irradiance", &g.irradiance, &shipped.irradiance);
    let rgb = |v: &[f32]| v.chunks(4).flat_map(|c| [c[0], c[1], c[2]]).collect::<Vec<f32>>();
    let alpha = |v: &[f32]| v.chunks(4).map(|c| c[3]).collect::<Vec<f32>>();
    stats("inscatter rgb", &rgb(&g.inscatter), &rgb(&shipped.inscatter));
    stats("inscatter mie", &alpha(&g.inscatter), &alpha(&shipped.inscatter));
    nan_report(&g);
}

#[allow(dead_code)]
fn nan_report(g: &AtmTables) {
    let w = se_core::atm::RES_MU_S * se_core::atm::RES_NU;
    for l in 0..se_core::atm::RES_R {
        let mut n = 0;
        let mut first = None;
        for y in 0..se_core::atm::RES_MU {
            for x in 0..w {
                let k = ((l * se_core::atm::RES_MU + y) * w + x) * 4;
                if g.inscatter[k].is_nan() {
                    n += 1;
                    first.get_or_insert((x, y));
                }
            }
        }
        if n > 0 {
            println!("layer {l}: {n} NaN, first {first:?}");
        }
    }
    let nan_e = g.irradiance.chunks(4).enumerate().filter(|(_, c)| c[0].is_nan()).map(|(i, _)| (i % 64, i / 64)).take(5).collect::<Vec<_>>();
    println!("irradiance NaN at {nan_e:?}");
}
