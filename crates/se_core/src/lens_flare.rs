//! Lens flare presets (`textures/flares/Flares.pak`, docs/formats/lens_flares.md).
//!
//! Three kinds of script, chosen by `LensFlarePreset`, `LensGhostPreset` and `PseudoFlarePreset`:
//! - `<Name> flares.cfg` and `<Name> ghosts.cfg`: `ScaleAll`, `BrightAll`, `Texture` (an atlas), then one
//!   `Flare { (r g b) bright_c bright_e scale_c scale_e aspect position angle_c angle_e chroma (x y w h) }`
//!   per sprite. `_c`/`_e` are the values with the sun at the screen centre and at the edge.
//! - `<Name> pseudo.cfg`: the parameters of the screen-space "pseudo lens flare" (ghosts, halo, dirt).
//!
//! `seLensFlareEffect::ReadConfigScript` (FUN_14026a9c0) stores per vertex: brightness * BrightAll,
//! scale * ScaleAll, angles in radians, `1 - position`, the aspect, colour plus chroma, and the
//! atlas rectangle in pixels.

/// One sprite of a flare or ghost preset, as written in the script.
#[derive(Clone, Debug, PartialEq)]
pub struct FlareElement {
    pub color: [f32; 3],
    /// Brightness with the sun at the screen centre / at the edge.
    pub bright: [f32; 2],
    /// Half size in units of the screen half-height (before `ScaleAll`), centre / edge.
    pub scale: [f32; 2],
    /// Height / width of the sprite.
    pub aspect: f32,
    /// Place on the sun - screen centre axis: 0 at the sun, 1 at the centre, 2 mirrored.
    pub position: f32,
    /// Rotation in degrees, centre / edge.
    pub angle: [f32; 2],
    pub chroma: f32,
    /// Atlas rectangle in pixels: x, y, width, height.
    pub rect: [f32; 4],
}

/// A `flares.cfg` or `ghosts.cfg` preset.
#[derive(Clone, Debug, PartialEq)]
pub struct FlareSet {
    pub scale_all: f32,
    pub bright_all: f32,
    /// Atlas texture name as written (`"AstroNiki.*"`).
    pub texture: String,
    pub elements: Vec<FlareElement>,
}

/// A `pseudo.cfg` preset (John Chapman's screen-space lens flare: ghosts and a halo of the bright parts
/// of the frame, lens dirt and a starburst).
#[derive(Clone, Debug, PartialEq)]
pub struct PseudoFlare {
    pub rays_texture: String,
    pub dirt_texture: String,
    pub ghosts_count: u32,
    pub ghosts_dispersal: f32,
    pub ghosts_bright: f32,
    pub ghosts_blur: f32,
    pub halo_radius: f32,
    pub halo_bright: f32,
    pub halo_blur: f32,
    pub dirt_bright: f32,
    pub dirt_dispersal: f32,
    pub chromaticism: f32,
}

impl Default for PseudoFlare {
    /// `Default pseudo.cfg`.
    fn default() -> Self {
        Self {
            rays_texture: "LensRays2.*".into(),
            dirt_texture: "LensDust.*".into(),
            ghosts_count: 5,
            ghosts_dispersal: 0.39,
            ghosts_bright: 0.007,
            ghosts_blur: 1.0,
            halo_radius: 0.75,
            halo_bright: 0.005,
            halo_blur: 2.0,
            dirt_bright: 0.05,
            dirt_dispersal: 3.5,
            chromaticism: 2.2,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Num(f32),
    Open,
    Close,
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("");
        let mut chars = line.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == ',' || c == '(' || c == ')' {
                chars.next();
            } else if c == '{' || c == '}' {
                chars.next();
                out.push(if c == '{' { Tok::Open } else { Tok::Close });
            } else if c == '"' {
                chars.next();
                let s: String = chars.by_ref().take_while(|&c| c != '"').collect();
                out.push(Tok::Str(s));
            } else {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || "{}(),\"".contains(c) {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                out.push(match s.parse::<f32>() {
                    Ok(v) => Tok::Num(v),
                    Err(_) => Tok::Word(s),
                });
            }
        }
    }
    out
}

/// Parses a `flares.cfg` / `ghosts.cfg` script. Sprites with too few numbers are skipped.
pub fn parse_flare_set(text: &str) -> FlareSet {
    let toks = tokens(text);
    let mut set = FlareSet { scale_all: 1.0, bright_all: 1.0, texture: String::new(), elements: Vec::new() };
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) if w == "Flare" => {
                let mut nums = Vec::new();
                i += 1;
                if toks.get(i) == Some(&Tok::Open) {
                    i += 1;
                }
                while i < toks.len() && toks[i] != Tok::Close {
                    if let Tok::Num(v) = toks[i] {
                        nums.push(v);
                    }
                    i += 1;
                }
                if nums.len() >= 16 {
                    let n = &nums;
                    set.elements.push(FlareElement {
                        color: [n[0], n[1], n[2]],
                        bright: [n[3], n[4]],
                        scale: [n[5], n[6]],
                        aspect: n[7],
                        position: n[8],
                        angle: [n[9], n[10]],
                        chroma: n[11],
                        rect: [n[12], n[13], n[14], n[15]],
                    });
                }
            }
            Tok::Word(w) => {
                match (w.as_str(), toks.get(i + 1)) {
                    ("ScaleAll", Some(Tok::Num(v))) => set.scale_all = *v,
                    ("BrightAll", Some(Tok::Num(v))) => set.bright_all = *v,
                    ("Texture", Some(Tok::Str(s))) => set.texture = s.clone(),
                    _ => {}
                }
            }
            _ => {}
        }
        i += 1;
    }
    set
}

/// Parses a `pseudo.cfg` script; missing keys keep `Default pseudo.cfg`'s values.
pub fn parse_pseudo(text: &str) -> PseudoFlare {
    let toks = tokens(text);
    let mut p = PseudoFlare::default();
    for w in toks.windows(2) {
        let Tok::Word(k) = &w[0] else { continue };
        match &w[1] {
            Tok::Str(s) => match k.as_str() {
                "RaysTexture" => p.rays_texture = s.clone(),
                "DirtTexture" => p.dirt_texture = s.clone(),
                _ => {}
            },
            Tok::Num(v) => match k.as_str() {
                "GhostsCount" => p.ghosts_count = v.max(0.0) as u32,
                "GhostsDispersal" => p.ghosts_dispersal = *v,
                "GhostsBright" => p.ghosts_bright = *v,
                "GhostsBlur" => p.ghosts_blur = *v,
                "HaloRadius" => p.halo_radius = *v,
                "HaloBright" => p.halo_bright = *v,
                "HaloBlur" => p.halo_blur = *v,
                "DirtBright" => p.dirt_bright = *v,
                "DirtDispersal" => p.dirt_dispersal = *v,
                "Chromaticism" => p.chromaticism = *v,
                _ => {}
            },
            _ => {}
        }
    }
    p
}

/// Splits a preset file name into (preset name, kind): `"Astroniki2 flares.cfg"` -> `("Astroniki2", "flares")`.
pub fn preset_of(file: &str) -> Option<(&str, &str)> {
    let name = file.rsplit('/').next()?.strip_suffix(".cfg")?;
    let (preset, kind) = name.rsplit_once(' ')?;
    matches!(kind, "flares" | "ghosts" | "pseudo").then_some((preset, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLARES: &str = r#"// global scale and bright parameters
ScaleAll  0.03
BrightAll 0.6
Texture  "AstroNiki.*"
// central flare
Flare { (1.0, 1.0, 1.0)    1.0     1.0     10.0    10.0    0.9     0.0      0.0      0.0   0.0   ( 768   0 256 256 ) }
Flare { (0.2, 0.25, 0.3)   0.015   0.2     34.0    34.0    0.9     0.0     -3.5     22.0   0.0   (
   0   0 512 512 ) }
//Flare { (1.0, 1.0, 1.0) 1 1 1 1 1 1 1 1 1 ( 0 0 1 1 ) }
"#;

    #[test]
    fn parses_flare_sets() {
        let s = parse_flare_set(FLARES);
        assert_eq!((s.scale_all, s.bright_all, s.texture.as_str()), (0.03, 0.6, "AstroNiki.*"));
        assert_eq!(s.elements.len(), 2);
        let e = &s.elements[1];
        assert_eq!(e.color, [0.2, 0.25, 0.3]);
        assert_eq!((e.bright, e.scale, e.aspect, e.position), ([0.015, 0.2], [34.0, 34.0], 0.9, 0.0));
        assert_eq!((e.angle, e.chroma, e.rect), ([-3.5, 22.0], 0.0, [0.0, 0.0, 512.0, 512.0]));
    }

    #[test]
    fn parses_pseudo_flares() {
        let p = parse_pseudo("RaysTexture  \"LensRays.*\"\nGhostsCount 7\nChromaticism 1.5 // comment\n");
        assert_eq!((p.rays_texture.as_str(), p.ghosts_count, p.chromaticism), ("LensRays.*", 7, 1.5));
        assert_eq!(p.dirt_texture, "LensDust.*");
        assert_eq!(preset_of("textures/flares/Clean lens pseudo.cfg"), Some(("Clean lens", "pseudo")));
        assert_eq!(preset_of("Anamorphic.png"), None);
    }
}
