//! SE's object type strings: the planet classification ("Temperate oceanic terra with life") and
//! the star type ("Yellow dwarf", "Red giant"), in the interface language.
//!
//! Planets follow FUN_1404d7c40: up to five words in the locale's `PlanetClassOrder*` order, the
//! temperature word (FUN_140389160), the atmosphere word when `TypeAtmoClass` is on (never for
//! airless worlds or giants), the volatiles word ("airless" .. "superglacial", FUN_140388b70; none
//! for asteroids and giants), the bulk word with its size (FUN_1403888d0: "microterra" ..
//! "megajupiter", per `TerrestrialClassStyle` / `GasGiantClassStyle`) and "with life", joined by
//! spaces and the first letter capitalised. Adjectives take the bulk word's gender (`Gender*`).
//! Main-def.cfg's defaults: no atmosphere or volatile-temperature words, style 0.
//!
//! Stars follow FUN_1401db610: the colour from the spectral letter (O, B blue; A, F white; G
//! yellow; K orange; M, L red) and the row from the luminosity class; masses of 0.012 to 0.0767
//! solar masses are brown dwarfs and below 0.012 planemos; white dwarfs, neutron stars, black
//! holes, Wolf-Rayet and carbon (C) or zirconium (S) stars have their own names.

use crate::locale::Locale;
use crate::star::{LumClass, SpectralClass, StarKind};
use crate::surface_gen::{Classes, bulk};

const TEMP: [&str; 7] = ["torrid", "hot", "warm", "temperate", "cool", "cold", "frigid"];
const ATMO: [&str; 6] = ["airless", "infrabaric", "hypobaric", "mesobaric", "hyperbaric", "ultrabaric"];
const VOL: [&str; 9] = ["airless", "arid", "lacustrine", "marine", "oceanic", "superoceanic", "glacial", "superglacial", "non-arid"];
const SIZE: [&str; 6] = ["micro", "mini", "sub", "", "super", "mega"];

/// main-def.cfg `Interface` switches of the class string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassStyle {
    /// `TypeAtmoClass`.
    pub atmo: bool,
    /// `TerrestrialClassStyle` 1: "superearth" for "superterra".
    pub earth: bool,
    /// `GasGiantClassStyle` 1: "ice giant" / "gas giant" / "helium giant" for "neptune" / "jupiter" / "chthonia".
    pub giant_words: bool,
}

impl Default for ClassStyle {
    fn default() -> Self {
        Self { atmo: false, earth: false, giant_words: false }
    }
}

/// The bulk word with its size prefix (English, before translation) and its gender key.
fn bulk_word(c: &Classes, style: ClassStyle) -> (String, &'static str) {
    let size = if c.size == 6 { 3 } else { c.size.clamp(0, 5) } as usize;
    // Giants have no "micro": their words start at "mini" ("ice minigiant" in style 1).
    let giant = |base: &str, words: &str, g0: &'static str, g1: &'static str| {
        let pre = SIZE[size.max(1)];
        if style.giant_words { (format!("{words} {pre}giant"), g1) } else { (format!("{pre}{base}"), g0) }
    };
    match c.bulk {
        bulk::ASTEROID => ("asteroid".into(), "GenderAsteroid"),
        bulk::FERRIA => (format!("{}ferria", SIZE[size]), "GenderFerria"),
        bulk::TERRA if style.earth => (format!("{}earth", SIZE[size]), "GenderEarth"),
        bulk::TERRA => (format!("{}terra", SIZE[size]), "GenderTerra"),
        bulk::CARBONIA => (format!("{}carbonia", SIZE[size]), "GenderCarbonia"),
        bulk::AQUARIA => (format!("{}aquaria", SIZE[size]), "GenderAquaria"),
        bulk::NEPTUNE => giant("neptune", "ice", "GenderNeptune", "GenderIceGiant"),
        bulk::JUPITER => giant("jupiter", "gas", "GenderJupiter", "GenderGasGiant"),
        bulk::CHTHONIA => giant("chthonia", "helium", "GenderChthonia", "GenderHeliumGiant"),
        _ => (String::new(), "GenderPlanet"),
    }
}

/// SE's planet classification string for `c`, in `loc`'s language.
pub fn planet_class(loc: &Locale, c: &Classes, life: bool, style: ClassStyle) -> String {
    let (bulk_en, gender_key) = bulk_word(c, style);
    if bulk_en.is_empty() {
        return String::new();
    }
    let gender = loc.settings.get(gender_key).map_or("neuter", String::as_str);
    let adj = |w: &str| loc.tr(&format!("{w}#{gender}")).to_string();
    let mut slots: [String; 5] = Default::default();
    if let Some(t) = TEMP.get(c.temp as usize) {
        slots[0] = adj(t);
    }
    if style.atmo && c.atmo != 0 && c.atmo != 7 {
        if let Some(a) = ATMO.get(c.atmo as usize) {
            slots[1] = adj(a);
        }
    }
    if c.volq != 9 {
        if let Some(v) = VOL.get(c.volq as usize) {
            slots[2] = adj(v);
        }
    }
    slots[3] = loc.tr(&bulk_en).to_string();
    if life {
        slots[4] = loc.tr("with life").to_string();
    }
    // The locale's word order (PlanetClassOrderTemp/Atmo/Vol/Bulk/Life).
    let keys = ["PlanetClassOrderTemp", "PlanetClassOrderAtmo", "PlanetClassOrderVol", "PlanetClassOrderBulk", "PlanetClassOrderLife"];
    let mut order: Vec<(usize, usize)> = keys.iter().enumerate().map(|(i, k)| (loc.settings.get(*k).and_then(|v| v.parse().ok()).unwrap_or(i), i)).collect();
    order.sort();
    let adj_sep = loc.settings.get("ClassNameAdjectiveSeparator").map_or("", String::as_str);
    let noun_sep = loc.settings.get("ClassNameNounSeparator").map_or("", String::as_str);
    let mut out = String::new();
    let mut prev_bulk = false;
    for (_, i) in order {
        if slots[i].is_empty() {
            continue;
        }
        if !out.is_empty() {
            out += if i == 3 || prev_bulk { noun_sep } else { adj_sep };
            out.push(' ');
        }
        out += &slots[i];
        prev_bulk = i == 3;
    }
    capitalize(&out)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// SE's star type (English, to be translated), from the spectral class and the mass (solar masses).
pub fn star_type(class: &SpectralClass, mass_sol: Option<f64>) -> &'static str {
    match class.kind {
        StarKind::WhiteDwarf => return "White dwarf",
        StarKind::NeutronStar => return "Neutron star",
        StarKind::BlackHole => return "Black hole",
        StarKind::WolfRayet => return "Wolf-Rayet star",
        StarKind::Carbon => return if class.letter == 'S' { "Zirconium star" } else { "Carbon star" },
        _ => {}
    }
    if let Some(m) = mass_sol {
        if (0.012..0.0767).contains(&m) {
            return "Brown dwarf";
        }
        if m < 0.012 && m > 0.0 {
            return "Planemo";
        }
    }
    let color = match class.letter {
        'O' | 'B' => 0,
        'A' | 'F' => 1,
        'G' => 2,
        'K' => 3,
        'M' | 'L' => 4,
        'T' | 'Y' => return "Brown dwarf",
        _ => return "Unknown",
    };
    let row = match class.lum_class {
        LumClass::Hypergiant => 0,
        LumClass::Ia => 1,
        LumClass::Ib => 2,
        LumClass::II => 3,
        LumClass::III => 4,
        LumClass::IV => 5,
        LumClass::V => 6,
        LumClass::VI => 7,
        LumClass::VII => return "White dwarf",
    };
    const NAMES: [[&str; 5]; 8] = [
        ["Blue hypergiant", "White hypergiant", "Yellow hypergiant", "Orange hypergiant", "Red hypergiant"],
        ["Blue luminous supergiant", "White luminous supergiant", "Yellow luminous supergiant", "Orange luminous supergiant", "Red luminous supergiant"],
        ["Blue supergiant", "White supergiant", "Yellow supergiant", "Orange supergiant", "Red supergiant"],
        ["Blue bright giant", "White bright giant", "Yellow bright giant", "Orange bright giant", "Red bright giant"],
        ["Blue giant", "White giant", "Yellow giant", "Orange giant", "Red giant"],
        ["Blue subgiant", "White subgiant", "Yellow subgiant", "Orange subgiant", "Red subgiant"],
        ["Blue main sequence star", "White main sequence star", "Yellow dwarf", "Orange dwarf", "Red dwarf"],
        ["Blue subdwarf", "White subdwarf", "Yellow subdwarf", "Orange subdwarf", "Red subdwarf"],
    ];
    NAMES[row][color]
}

/// SE's multiplicity word for a system of `n` stars ("single", "binary", ... "octuple", "x multiple").
pub fn multiplicity(n: usize) -> &'static str {
    ["single", "single", "binary", "triple", "quadruple", "quintuple", "sextuple", "septuple", "octuple"].get(n).copied().unwrap_or("x multiple")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eng() -> Locale {
        let mut l = Locale::default();
        l.parse_gui("GenderTerra \"neuter\"\nPlanetClassOrderTemp 0\nPlanetClassOrderAtmo 1\nPlanetClassOrderVol 2\nPlanetClassOrderBulk 3\nPlanetClassOrderLife 4\nTranslations\n{\n\"temperate#neuter\" \"temperate\"\n\"oceanic#neuter\" \"oceanic\"\n\"cold#neuter\" \"cold\"\n}\n");
        l
    }

    #[test]
    fn planet_strings() {
        let earth = Classes { bulk: bulk::TERRA, temp: 3, atmo: 3, volq: 4, size: 3, surface: 1 };
        assert_eq!(planet_class(&eng(), &earth, true, ClassStyle::default()), "Temperate oceanic terra with life");
        let jupiter = Classes { bulk: bulk::JUPITER, temp: 5, atmo: 7, volq: 9, size: 3, surface: 8 };
        assert_eq!(planet_class(&eng(), &jupiter, false, ClassStyle::default()), "Cold jupiter");
    }

    #[test]
    fn star_types() {
        let t = |s: &str| star_type(&SpectralClass::parse(s), None);
        assert_eq!(t("G2V"), "Yellow dwarf");
        assert_eq!(t("F5IV"), "White subgiant");
        assert_eq!(t("M2Ia"), "Red luminous supergiant");
        assert_eq!(t("K1III"), "Orange giant");
        assert_eq!(t("B8Ia"), "Blue luminous supergiant");
        assert_eq!(t("DA2"), "White dwarf");
        assert_eq!(star_type(&SpectralClass::parse("M9V"), Some(0.05)), "Brown dwarf");
    }
}
