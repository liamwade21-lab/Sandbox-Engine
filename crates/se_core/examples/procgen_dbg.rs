//! Print a Sun-like procedural system (stars and planets, no asteroids or moons):
//! `cargo run -p se_core --release --example procgen_dbg -- [NAME] [SEED]`.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = args.get(1).cloned().unwrap_or("RS 0-8-1-4".into());
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4 * 7919);
    let s = se_core::procgen::StarParams { name, mass_sol: 1.0, lum_bol: 1.0, teff: 5778.0, radius_sol: 1.0, age_gyr: 4.6, feh: 0.0, kind: se_core::star::StarKind::Normal, class: None, c_to_o: None, se_seed: None };
    let g = se_core::procgen::generate_system(&s, seed);
    let mut cur = String::new();
    for l in g.sc.lines() {
        let t = l.trim();
        if t.starts_with("Star ") || t.starts_with("Barycenter ") {
            cur = t.to_string();
        } else if cur.is_empty() {
            continue;
        } else if ["Class", "MassSol", "LumBol", "Teff", "SemiMajorAxis", "Eccentricity", "Period "].iter().any(|k| t.starts_with(k)) {
            cur.push_str(" | ");
            cur.push_str(&t.split_whitespace().take(2).collect::<Vec<_>>().join(" "));
        } else if t == "}" && !cur.is_empty() && !l.starts_with('\t') {
            println!("{cur}");
            cur.clear();
        }
    }
    for b in g.bodies.iter().filter(|b| b.tag == "Planet" || b.tag == "DwarfPlanet") {
        println!("{:12} {:22} parent {:16} {:9} a {:9.4} M {:.3e} R {:.0}", b.tag, b.name, b.parent, b.class, b.a_au, b.mass_me, b.radius_km);
    }
}
