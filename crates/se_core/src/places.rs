//! Loader for SpaceEngine saved locations (`save/places-def.cfg`).
//!
//! Decoded format (see docs/formats/config_locale.md, section 6, "Places format (decoded)"):
//! - `Time` is a signed hex fixed-point number, Julian Date * 2^80.
//! - `Pos` is three signed hex fixed-point numbers: the camera position relative to the
//!   Sun's centre (heliocentric), in parsecs * 2^80, in the equatorial frame
//!   X = RA 6h, Y = north celestial pole, Z = RA 0h.
//! - `Rot` is a unit quaternion written in file order (w x y z). The camera looks along -Z.
//! - `Vel` is a speed in parsecs per second.

use log::warn;

/// Metres per parsec (IAU).
pub const PC_M: f64 = 3.085_677_581_491_367_3e16;
/// Metres per astronomical unit (IAU).
pub const AU_M: f64 = 1.495_978_707e11;
const FRAC_BITS: u32 = 80;

/// One saved location.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub body: String,
    pub parent: String,
    /// True for places the user saved (in the user's places.cfg), false for the shipped ones.
    pub user: bool,
    /// Raw `Time` fixed-point integer (JD * 2^80). Kept so that re-encoding is exact.
    pub time_raw: i128,
    /// Raw `Pos` fixed-point integers (parsecs * 2^80). Kept so that re-encoding is exact.
    pub pos_raw: [i128; 3],
    /// Julian Date (from `Time`, which is JD * 2^80).
    pub jd: f64,
    /// Date string as written in the file (`"YYYY.MM.DD HH:MM:SS.cc"`).
    pub date: String,
    /// Camera position relative to the Sun's centre, metres, equatorial frame
    /// (X = RA 6h, Y = north celestial pole, Z = RA 0h).
    pub pos_m: [f64; 3],
    /// Camera orientation quaternion, order x y z w (file order is w x y z).
    pub rot: [f64; 4],
    /// Speed in parsecs per second.
    pub vel: f64,
    pub stereobase: f64,
    /// 0 Free, 1 Follow, 2 SyncRot.
    pub bind_mode: i32,
    pub photo_mode: i32,
    pub exposure: f64,
}

impl Place {
    /// Direction the camera looks along (camera -Z rotated by `rot`), in the same frame as `pos_m`.
    pub fn camera_forward(&self) -> [f64; 3] {
        let [x, y, z, w] = self.rot;
        // third column of the rotation matrix, negated (-Z axis)
        [
            -2.0 * (x * z + y * w),
            -2.0 * (y * z - x * w),
            -(1.0 - 2.0 * (x * x + y * y)),
        ]
    }

    /// Builds a user place from camera state. `pos_m` is heliocentric metres (equatorial frame),
    /// `rot` is the camera quaternion in order x y z w, `jd` the simulation time.
    pub fn new_user(name: &str, body: &str, jd: f64, pos_m: [f64; 3], rot: [f64; 4]) -> Place {
        let to_fixed = |v: f64| (v * 2f64.powi(FRAC_BITS as i32)) as i128;
        let pos_raw = pos_m.map(|m| to_fixed(m / PC_M));
        Place {
            name: name.replace(['\r', '\n', '\t'], " "),
            body: body.to_string(),
            parent: String::new(),
            user: true,
            time_raw: to_fixed(jd),
            pos_raw,
            jd,
            date: format_date(jd),
            pos_m: pos_raw.map(|r| fixed_to_f64(r) * PC_M),
            rot,
            vel: 0.0,
            stereobase: 1.0,
            bind_mode: 0,
            photo_mode: 0,
            exposure: 0.0,
        }
    }
}

/// Encodes a raw fixed-point integer as the signed hex token the parser reads (`+2581...`, `-D2EF`).
pub fn format_fixed(raw: i128) -> String {
    let sign = if raw < 0 { '-' } else { '+' };
    format!("{sign}{:X}", raw.unsigned_abs())
}

/// Formats a Julian Date as the `Date` string `"YYYY.MM.DD HH:MM:SS.cc"` (UT, centiseconds).
pub fn format_date(jd: f64) -> String {
    // Civil calendar from Julian Day (Fliegel and Van Flandern), Gregorian dates only.
    let x = jd + 0.5;
    let mut z = x.floor() as i64;
    let mut cs = ((x - z as f64) * 8_640_000.0).round() as i64;
    if cs >= 8_640_000 {
        z += 1;
        cs -= 8_640_000;
    }
    let a = if z < 2_299_161 {
        z
    } else {
        let al = ((z as f64 - 1_867_216.25) / 36_524.25).floor() as i64;
        z + 1 + al - al / 4
    };
    let b = a + 1524;
    let c = ((b as f64 - 122.1) / 365.25).floor() as i64;
    let d = (365.25 * c as f64).floor() as i64;
    let e = ((b - d) as f64 / 30.6001).floor() as i64;
    let day = b - d - (30.6001 * e as f64).floor() as i64;
    let month = if e < 14 { e - 1 } else { e - 13 };
    let year = if month > 2 { c - 4716 } else { c - 4715 };
    let (h, m, s) = (cs / 360_000, cs / 6000 % 60, cs / 100 % 60);
    format!("{year:04}.{month:02}.{day:02} {h:02}:{m:02}:{s:02}.{:02}", cs % 100)
}

/// Writes one place as a `Place "<name>" { ... }` block, in the syntax `parse_places` reads.
/// Time and Pos use the raw fixed-point integers; Rot is written as (w x y z).
pub fn write_place(p: &Place) -> String {
    let [x, y, z, w] = p.rot;
    let [px, py, pz] = p.pos_raw.map(format_fixed);
    format!(
        "Place \"{name}\"\r\n{{\r\n\tVer        991\r\n\tBody    \"{body}\"\r\n\tParent    \"{parent}\"\r\n\tDate    \"{date}\"\r\n\tTime    {time}\r\n\tPos        ({px} {py} {pz})\r\n\tRot        ({w:?} {x:?} {y:?} {z:?})\r\n\tVel        {vel:?}\r\n\tStereobase    {sb:?}\r\n\tBindMode    {bind}\r\n\tPhotoMode    {photo}\r\n\tExposure    {exp:?}\r\n}}\r\n",
        name = p.name,
        body = p.body,
        parent = p.parent,
        date = p.date,
        time = format_fixed(p.time_raw),
        vel = p.vel,
        sb = p.stereobase,
        bind = p.bind_mode,
        photo = p.photo_mode,
        exp = p.exposure,
    )
}

/// Writes a whole places file: one block after another.
pub fn write_places(places: &[Place]) -> String {
    let mut out = String::from("// SpaceEngine-rs saved places\r\n");
    for p in places {
        out.push_str(&write_place(p));
    }
    out
}

/// Parses a signed hex fixed-point token such as `+2581097EE258858C20ECA37980` or `-D2EF`.
/// Returns the raw integer (value * 2^80 in the case of Time/Pos).
pub fn parse_fixed(tok: &str) -> Option<i128> {
    let (neg, digits) = match tok.as_bytes().first()? {
        b'-' => (true, &tok[1..]),
        b'+' => (false, &tok[1..]),
        _ => (false, tok),
    };
    if digits.is_empty() || digits.len() > 31 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = i128::from_str_radix(digits, 16).ok()?;
    Some(if neg { -v } else { v })
}

/// Converts a raw fixed-point integer (scaled by 2^80) to f64 without losing the integer part.
pub fn fixed_to_f64(raw: i128) -> f64 {
    let int_part = raw >> FRAC_BITS;
    let frac_part = raw & ((1i128 << FRAC_BITS) - 1);
    int_part as f64 + frac_part as f64 / 2f64.powi(FRAC_BITS as i32)
}

/// Parses every `Place "<name>" { ... }` block in the file text.
/// Malformed blocks are skipped with a `log::warn`.
pub fn parse_places(text: &str) -> Vec<Place> {
    let mut out = Vec::new();
    let mut cur: Option<(String, Builder)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line == "{" {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Place") {
            if rest.trim_start().starts_with('"') {
                let name = match (line.find('"'), line.rfind('"')) {
                    (Some(a), Some(b)) if b > a => line[a + 1..b].to_string(),
                    _ => String::new(),
                };
                if let Some((n, _)) = cur.take() {
                    warn!("places: block '{n}' has no closing brace, skipped");
                }
                cur = Some((name, Builder::default()));
                continue;
            }
        }
        if line == "}" {
            if let Some((name, b)) = cur.take() {
                match b.finish(name.clone()) {
                    Ok(p) => out.push(p),
                    Err(e) => warn!("places: skipping '{name}': {e}"),
                }
            }
            continue;
        }
        if let Some((_, b)) = cur.as_mut() {
            if let Some((key, val)) = line.split_once(char::is_whitespace) {
                b.set(key, val.trim());
            }
        }
    }
    out
}

#[derive(Default)]
struct Builder {
    body: Option<String>,
    parent: String,
    date: String,
    time: Option<i128>,
    pos: Option<[i128; 3]>,
    rot: Option<[f64; 4]>,
    vel: Option<f64>,
    stereobase: f64,
    bind_mode: i32,
    photo_mode: i32,
    exposure: f64,
}

fn unquote(v: &str) -> String {
    v.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(v)
        .to_string()
}

/// Splits `(a b c)` into its whitespace-separated tokens.
fn paren_tokens(v: &str) -> Option<Vec<&str>> {
    let inner = v.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner.split_whitespace().collect())
}

impl Builder {
    fn set(&mut self, key: &str, val: &str) {
        match key {
            "Body" => self.body = Some(unquote(val)),
            "Parent" => self.parent = unquote(val),
            "Date" => self.date = unquote(val),
            "Time" => self.time = parse_fixed(val),
            "Pos" => {
                self.pos = paren_tokens(val).and_then(|t| {
                    if t.len() != 3 {
                        return None;
                    }
                    Some([parse_fixed(t[0])?, parse_fixed(t[1])?, parse_fixed(t[2])?])
                })
            }
            "Rot" => {
                self.rot = paren_tokens(val).and_then(|t| {
                    if t.len() != 4 {
                        return None;
                    }
                    let f: Vec<f64> = t.iter().map(|s| s.parse::<f64>().ok()).collect::<Option<_>>()?;
                    // file order w x y z -> stored x y z w
                    Some([f[1], f[2], f[3], f[0]])
                })
            }
            "Vel" => self.vel = val.parse().ok(),
            "Stereobase" => self.stereobase = val.parse().unwrap_or(0.0),
            "BindMode" => self.bind_mode = val.parse().unwrap_or(0),
            "PhotoMode" => self.photo_mode = val.parse().unwrap_or(0),
            "Exposure" => self.exposure = val.parse().unwrap_or(0.0),
            _ => {}
        }
    }

    fn finish(self, name: String) -> Result<Place, String> {
        let body = self.body.ok_or("missing Body")?;
        let time = self.time.ok_or("missing or bad Time")?;
        let pos = self.pos.ok_or("missing or bad Pos")?;
        let rot = self.rot.ok_or("missing or bad Rot")?;
        let vel = self.vel.ok_or("missing or bad Vel")?;
        let pc = |raw: i128| fixed_to_f64(raw) * PC_M;
        Ok(Place {
            name,
            body,
            parent: self.parent,
            user: false,
            time_raw: time,
            pos_raw: pos,
            jd: fixed_to_f64(time),
            date: self.date,
            pos_m: [pc(pos[0]), pc(pos[1]), pc(pos[2])],
            rot,
            vel,
            stereobase: self.stereobase,
            bind_mode: self.bind_mode,
            photo_mode: self.photo_mode,
            exposure: self.exposure,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EARTH: &str = r#"
Place "Earth: Civilization"
{
    Ver        991
    Body    "Earth"
    Parent    "Earth-Moon"
    Date    "2017.04.21 23:53:43.39"
    Time    +2581097EE258858C20ECA37980
    Pos        (-273288B4E73AF200 -11006ECA695B3C00 -453B55921E3A3800)
    Rot        (-0.2413280043436836 0.08777268599160623 0.9663531009695026 -0.01477951825807379)
    Vel        8.3662154e-11
    Stereobase    1
    BindMode    1
    PhotoMode    1
    Exposure    -1.75
}
"#;

    const ANDROMEDA: &str = r#"
Place "The Andromeda Galaxy: Our Neighbor"
{
    Ver        991
    Body    "Andromeda"
    Parent    ""
    Date    "27278.12.15 21:01:14.16"
    Time    +B24A8E60384A1A13A1D4C0C7A0
    Pos        (+1DCBE64B50895F50524A952EE +7E841795276499517B441CEEC +95467AE9EFDC57001142F5150)
    Rot        (0.4241215242262729 -0.1851757678306584 0.02288993698667866 -0.8861754445261136)
    Vel        1157.0599
    Stereobase    1
    BindMode    1
    PhotoMode    0
    Exposure    0
}
"#;

    const M74: &str = r#"
Place "M74: A Perfect Spiral"
{
    Ver        991
    Body    "M 74"
    Parent    ""
    Date    "27278.12.15 21:01:14.18"
    Time    +B24A8E60384A1A13A1D4C0C7A0
    Pos        (+255D78312369DE269B57A03E65 +19CB2C558C0B11FAEEC4005A4A +5340FA6CAC3E6DE0ADF4D530B6)
    Rot        (0.2423984597169364 0.6941756168263296 -0.6760808740865851 -0.04772684176604147)
    Vel        12741.186
    Stereobase    1
    BindMode    1
    PhotoMode    0
    Exposure    0
}
"#;

    #[test]
    fn fixed_point_parsing() {
        assert_eq!(parse_fixed("+10"), Some(16));
        assert_eq!(parse_fixed("-10"), Some(-16));
        assert_eq!(parse_fixed("-D2EF91F41565B2F4"), Some(-0xD2EF_91F4_1565_B2F4));
        assert_eq!(parse_fixed("1F"), Some(31));
        assert_eq!(parse_fixed("+"), None);
        assert_eq!(parse_fixed("+G1"), None);
    }

    #[test]
    fn time_is_julian_date_times_2_pow_80() {
        let p = &parse_places(EARTH)[0];
        // 2017-04-21 00:00 UT is JD 2457864.5; plus 23:53:43.39
        let expected = 2_457_864.5 + (23.0 * 3600.0 + 53.0 * 60.0 + 43.39) / 86400.0;
        assert!((p.jd - expected).abs() < 1e-6, "jd {} vs {}", p.jd, expected);
    }

    #[test]
    fn earth_position_is_about_one_au_from_sun() {
        let p = &parse_places(EARTH)[0];
        let r = p.pos_m.iter().map(|v| v * v).sum::<f64>().sqrt() / AU_M;
        assert!((0.98..1.02).contains(&r), "r = {r} AU");
        // direction: X = RA 6h, Y = NCP, Z = RA 0h; Earth in April is at about x=-0.48 y=-0.21 z=-0.85
        let d: Vec<f64> = p.pos_m.iter().map(|v| v / (r * AU_M)).collect();
        assert!((d[0] + 0.4818).abs() < 0.01 && (d[2] + 0.851).abs() < 0.01, "dir {d:?}");
    }

    #[test]
    fn quaternion_is_stored_xyzw_from_wxyz_file_order() {
        let p = &parse_places(EARTH)[0];
        let want = [0.08777268599160623, 0.9663531009695026, -0.01477951825807379, -0.2413280043436836];
        for (a, b) in p.rot.iter().zip(want) {
            assert!((a - b).abs() < 1e-15);
        }
    }

    #[test]
    fn andromeda_is_about_0_8_mpc_away_from_sun() {
        let p = &parse_places(ANDROMEDA)[0];
        let mpc = p.pos_m.iter().map(|v| v * v).sum::<f64>().sqrt() / PC_M / 1e6;
        assert!((0.7..0.9).contains(&mpc), "{mpc} Mpc");
        assert_eq!(p.parent, "");
    }

    #[test]
    fn m74_camera_looks_along_position_vector() {
        // Far object: the view direction must point (within 1 deg) along the Sun->object vector.
        let p = &parse_places(M74)[0];
        let f = p.camera_forward();
        let n = p.pos_m.iter().map(|v| v * v).sum::<f64>().sqrt();
        let dot: f64 = f.iter().zip(p.pos_m.iter()).map(|(a, b)| a * b).sum::<f64>() / n;
        let ang = dot.clamp(-1.0, 1.0).acos().to_degrees();
        assert!(ang < 1.0, "angle {ang}");
    }

    #[test]
    fn malformed_block_is_skipped() {
        let text = format!("{EARTH}\nPlace \"Broken\"\n{{\n    Body \"X\"\n}}\n");
        let v = parse_places(&text);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Earth: Civilization");
    }

    #[test]
    fn real_save_file_parses_if_present() {
        let path = "D:/SteamLibrary/steamapps/common/SpaceEngine/save/places-def.cfg";
        if let Ok(text) = std::fs::read_to_string(path) {
            let v = parse_places(&text);
            assert_eq!(v.len(), 75);
            assert!(v.iter().all(|p| p.jd > 0.0 && p.vel >= 0.0));
        }
    }

    fn line_with(text: &str, key: &str) -> String {
        text.lines().map(str::trim).find(|l| l.starts_with(key)).unwrap_or_default().to_string()
    }

    #[test]
    fn fixed_point_tokens_format_like_the_file() {
        assert_eq!(format_fixed(16), "+10");
        assert_eq!(format_fixed(-0xD2EF), "-D2EF");
        assert_eq!(format_fixed(0), "+0");
    }

    #[test]
    fn date_string_from_exact_julian_dates() {
        assert_eq!(format_date(2_451_544.5), "2000.01.01 00:00:00.00");
        assert_eq!(format_date(2_457_865.0), "2017.04.21 12:00:00.00");
        // The doc's Earth Time agrees with its Date string only to about 0.02 s, so it is not exact here.
        let p = &parse_places(EARTH)[0];
        assert!(format_date(p.jd).starts_with("2017.04.21 23:53:43."));
    }

    #[test]
    fn writer_reencodes_doc_line_to_identical_hex() {
        let p = &parse_places(EARTH)[0];
        let text = write_place(p);
        assert_eq!(line_with(&text, "Time"), line_with(EARTH, "Time"));
        assert_eq!(line_with(&text, "Pos"), line_with(EARTH, "Pos"));
        assert_eq!(line_with(&text, "Body"), line_with(EARTH, "Body"));
        assert_eq!(parse_places(&text)[0], *p);
    }

    #[test]
    fn user_place_round_trips_through_text() {
        let mut p = Place::new_user(
            "Test: Sol view",
            "Earth",
            2_457_865.5,
            [1.5e11, -2.0e10, 3.0e10],
            [0.1, 0.2, 0.3, 0.927],
        );
        // The user flag is not stored in the file; the loader sets it.
        p.user = false;
        let back = parse_places(&write_place(&p));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0], p);
        let r = back[0].pos_m.iter().zip(p.pos_m).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(r < 1e-3, "pos drift {r} m");
    }

    #[test]
    fn whole_file_round_trips() {
        let a = parse_places(EARTH).remove(0);
        let b = parse_places(ANDROMEDA).remove(0);
        let text = write_places(&[a.clone(), b.clone()]);
        assert_eq!(parse_places(&text), vec![a, b]);
    }
}
