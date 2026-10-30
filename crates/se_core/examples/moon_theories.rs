//! Print moon-theory positions (planet-centred, J2000 ecliptic km) for a model.
//! Usage: cargo run -p se_core --example moon_theories -- <install> <model> <jd>...
use se_core::moon_theories::MoonTheories;

fn main() {
    let mut args = std::env::args().skip(1);
    let install = args.next().unwrap_or_else(|| "D:/SteamLibrary/steamapps/common/SpaceEngine".into());
    let model = args.next().unwrap_or_else(|| "Miranda-GUST86".into());
    let th = MoonTheories::from_install(std::path::Path::new(&install)).expect("load");
    for jd in args.map(|a| a.parse::<f64>().expect("jd")) {
        match th.internal_au(&model, jd) {
            Some([x, y, z]) => {
                let k = 149_597_870.7;
                println!("{jd} {:.3} {:.3} {:.3}", x * k, -z * k, y * k);
            }
            None => println!("{jd} unsupported"),
        }
    }
}
