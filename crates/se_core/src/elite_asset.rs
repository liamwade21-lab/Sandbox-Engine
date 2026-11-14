//! The Faulcon DeLacy Sidewinder mod's own files: its model format (`.edm`, below `Binds`) and the reader for Elite
//! Dangerous's `.binds` control files (the mod ships
//! copies of `KeyboardMouseOnly.binds` and `ControlPad.binds` and reads its key table from them: grade A, the spec's T4).
//!
//! A `.binds` file is XML: one element per action or setting under `<Root PresetName=..>`. A button action holds
//! `<Primary Device=".." Key="..">` and `<Secondary ..>` (each may hold `<Modifier Device=".." Key=".."/>`) and sometimes
//! `<ToggleOn Value="1"/>`; an axis holds `<Binding Device=".." Key=".."/>`, `<Inverted Value=".."/>` and
//! `<Deadzone Value=".."/>`; a setting is an element with a `Value` attribute (`<MouseXMode Value="Bindings_MouseRoll"/>`).
//! An unused slot names the device `{NoDevice}`.

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BindsError {
    #[error("XML: {0} at byte {1}")]
    Xml(&'static str, usize),
    #[error("not a .binds file (no Root element)")]
    NoRoot,
}

/// One XML element: its name, attributes and children (text between tags is not used by `.binds` files).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// A small XML reader for the subset `.binds` files use: elements, double- or single-quoted attributes, self-closing
/// tags, the `<?xml?>` declaration and comments.
pub fn parse_xml(text: &str) -> Result<Element, BindsError> {
    let b = text.trim_start_matches('\u{feff}').as_bytes();
    let mut i = 0;
    let mut stack: Vec<Element> = vec![Element { name: String::new(), ..Default::default() }];
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"<?") {
            i += b[i..].windows(2).position(|w| w == b"?>").ok_or(BindsError::Xml("unclosed declaration", i))? + 2;
            continue;
        }
        if b[i..].starts_with(b"<!--") {
            i += b[i..].windows(3).position(|w| w == b"-->").ok_or(BindsError::Xml("unclosed comment", i))? + 3;
            continue;
        }
        if b[i..].starts_with(b"</") {
            let end = i + b[i..].iter().position(|&c| c == b'>').ok_or(BindsError::Xml("unclosed end tag", i))?;
            let name = std::str::from_utf8(&b[i + 2..end]).unwrap_or("").trim();
            let done = stack.pop().ok_or(BindsError::Xml("end tag without start", i))?;
            if done.name != name || stack.is_empty() {
                return Err(BindsError::Xml("mismatched end tag", i));
            }
            stack.last_mut().unwrap().children.push(done);
            i = end + 1;
            continue;
        }
        // A start tag.
        let mut j = i + 1;
        while j < b.len() && !matches!(b[j], b' ' | b'\t' | b'\r' | b'\n' | b'/' | b'>') {
            j += 1;
        }
        let mut el = Element { name: String::from_utf8_lossy(&b[i + 1..j]).into_owned(), ..Default::default() };
        if el.name.is_empty() {
            return Err(BindsError::Xml("empty tag name", i));
        }
        loop {
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= b.len() {
                return Err(BindsError::Xml("unclosed start tag", i));
            }
            if b[j] == b'>' {
                stack.push(el);
                j += 1;
                break;
            }
            if b[j..].starts_with(b"/>") {
                stack.last_mut().unwrap().children.push(el);
                j += 2;
                break;
            }
            let k0 = j;
            while j < b.len() && !matches!(b[j], b'=' | b' ' | b'>' | b'/') {
                j += 1;
            }
            let key = String::from_utf8_lossy(&b[k0..j]).trim().to_string();
            if j >= b.len() || b[j] != b'=' {
                return Err(BindsError::Xml("attribute without value", j));
            }
            j += 1;
            let q = *b.get(j).ok_or(BindsError::Xml("attribute without value", j))?;
            if q != b'"' && q != b'\'' {
                return Err(BindsError::Xml("unquoted attribute", j));
            }
            let v0 = j + 1;
            let v1 = v0 + b[v0..].iter().position(|&c| c == q).ok_or(BindsError::Xml("unclosed attribute", j))?;
            el.attrs.push((key, unescape(&String::from_utf8_lossy(&b[v0..v1]))));
            j = v1 + 1;
        }
        i = j;
    }
    if stack.len() != 1 {
        return Err(BindsError::Xml("unclosed element", b.len()));
    }
    Ok(stack.pop().unwrap())
}

/// A key or button: the device (`Keyboard`, `Mouse`, a pad's id) and its key name (`Key_A`, `Mouse_1`, `Pad_A`), with
/// the modifier keys that must be held with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBind {
    pub device: String,
    pub key: String,
    pub modifiers: Vec<(String, String)>,
}

fn key_of(e: &Element) -> Option<KeyBind> {
    let device = e.attr("Device")?;
    let key = e.attr("Key")?;
    if device == "{NoDevice}" || key.is_empty() {
        return None;
    }
    let modifiers = e
        .children
        .iter()
        .filter(|m| m.name == "Modifier")
        .filter_map(|m| Some((m.attr("Device")?.to_string(), m.attr("Key")?.to_string())))
        .collect();
    Some(KeyBind { device: device.to_string(), key: key.to_string(), modifiers })
}

/// An action or setting of a `.binds` file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Bind {
    pub primary: Option<KeyBind>,
    pub secondary: Option<KeyBind>,
    /// An axis's binding (`<Binding>`).
    pub axis: Option<KeyBind>,
    pub inverted: Option<bool>,
    pub deadzone: Option<f64>,
    /// `<ToggleOn Value="1"/>`: a press toggles instead of holding.
    pub toggle_on: Option<bool>,
    /// A setting's `Value` attribute.
    pub value: Option<String>,
}

impl Bind {
    /// Every key that triggers it (primary, secondary).
    pub fn keys(&self) -> impl Iterator<Item = &KeyBind> {
        self.primary.iter().chain(self.secondary.iter())
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Binds {
    pub preset: String,
    pub actions: BTreeMap<String, Bind>,
}

impl Binds {
    pub fn parse(text: &str) -> Result<Binds, BindsError> {
        let doc = parse_xml(text)?;
        let root = doc.child("Root").ok_or(BindsError::NoRoot)?;
        let mut actions = BTreeMap::new();
        for e in &root.children {
            let num = |n: &str| e.child(n).and_then(|c| c.attr("Value")).and_then(|v| v.trim().parse::<f64>().ok());
            let b = Bind {
                primary: e.child("Primary").and_then(key_of),
                secondary: e.child("Secondary").and_then(key_of),
                axis: e.child("Binding").and_then(key_of),
                inverted: num("Inverted").map(|v| v != 0.0),
                deadzone: num("Deadzone"),
                toggle_on: e.child("ToggleOn").map(|c| c.attr("Value").map(|v| v.trim() == "1").unwrap_or(true)),
                value: e.attr("Value").map(|v| v.to_string()),
            };
            actions.insert(e.name.clone(), b);
        }
        Ok(Binds { preset: root.attr("PresetName").unwrap_or("").to_string(), actions })
    }

    pub fn get(&self, action: &str) -> Option<&Bind> {
        self.actions.get(action)
    }

    /// The primary key's name of a button action on `device` (`Keyboard`, `Mouse`).
    pub fn key(&self, action: &str, device: &str) -> Option<&str> {
        self.get(action)?.keys().find(|k| k.device == device).map(|k| k.key.as_str())
    }

    /// A setting's value.
    pub fn value(&self, setting: &str) -> Option<&str> {
        self.get(setting)?.value.as_deref()
    }
}

// --- The mod's model format (`ship.edm`, `cockpit.edm`) ----------------------------------------------------------------
//
// Written only by the converter (`se_engine/src/sidewinder/convert.rs`, an ignored test), read on the mod's first enable.
// It keeps Elite's own vertex bytes and triangle indices unchanged (40-byte vertices, `crate::elite::Vertex`; u16 indices
// counted from the draw's first vertex), so a test can compare them with the install's (grade A), and adds what the open
// files don't hold, recreated (grade E): the parts (a tree with rest transforms: the hull, each gear leg, the hardpoints,
// the scoop), which part and material each draw has, the materials' textures, the metres per raw position step, and keyed
// part animations (the gear coming down). Model space: metres after `metres_per_unit`, Elite's axes (Y up).
//
// Layout: magic `SWEDM001`, u32 version; f32 metres_per_unit; then counted lists (u16 count unless said): parts (name,
// i16 parent, translation f32x3, rotation f32x4 x y z w), materials (name, u8 texture count, each (slot, file), tint f32x4,
// u32 flags), draws (u16 part, u16 material, source text, u32 vertex count then the vertices' bytes, u32 index count then
// the indices), animations (name, f32 seconds, u16 tracks: each u16 part, u16 keys, each (f32 time, translation f32x3,
// rotation f32x4)).

const EDM_MAGIC: &[u8; 8] = b"SWEDM001";
const EDM_VERSION: u32 = 1;

/// A material's flags.
pub mod material_flags {
    /// Drawn see-through after the solids (the canopy's glass).
    pub const GLASS: u32 = 1;
    /// Lit by itself (lights, displays).
    pub const EMISSIVE: u32 = 2;
    /// Seen from both sides.
    pub const TWO_SIDED: u32 = 4;
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EdmError {
    #[error("not a Sidewinder model file (bad magic)")]
    Magic,
    #[error("unsupported version {0}")]
    Version(u32),
    #[error("the file ends early at byte {0}")]
    Short(usize),
    #[error("{0}")]
    Bad(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdmPart {
    pub name: String,
    /// -1 for the root.
    pub parent: i16,
    pub translation: [f32; 3],
    /// x, y, z, w.
    pub rotation: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdmMaterial {
    pub name: String,
    /// (slot, file beside the model): slots `diffuse`, `normal`, `mask`, `glow` as the converter names them.
    pub textures: Vec<(String, String)>,
    pub tint: [f32; 4],
    pub flags: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdmDraw {
    pub part: u16,
    pub material: u16,
    /// Where it came from: an Elite file, chunk and part (`NAME#chunk/part`), or `recreated: ...`.
    pub source: String,
    pub vertices: Vec<crate::elite::Vertex>,
    pub indices: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdmKey {
    pub time: f32,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdmTrack {
    pub part: u16,
    pub keys: Vec<EdmKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdmAnimation {
    pub name: String,
    pub seconds: f32,
    pub tracks: Vec<EdmTrack>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edm {
    pub metres_per_unit: f32,
    pub parts: Vec<EdmPart>,
    pub materials: Vec<EdmMaterial>,
    pub draws: Vec<EdmDraw>,
    pub animations: Vec<EdmAnimation>,
}

impl Edm {
    /// A vertex's position in metres in its part's frame.
    pub fn position_m(&self, v: &crate::elite::Vertex) -> [f32; 3] {
        let p = v.position_raw();
        [p[0] as f32 * self.metres_per_unit, p[1] as f32 * self.metres_per_unit, p[2] as f32 * self.metres_per_unit]
    }
}

struct Wr(Vec<u8>);

impl Wr {
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32s(&mut self, v: &[f32]) {
        v.iter().for_each(|x| self.0.extend_from_slice(&x.to_le_bytes()));
    }
    fn str(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.0.extend_from_slice(s.as_bytes());
    }
}

pub fn write_edm(m: &Edm) -> Vec<u8> {
    let mut w = Wr(EDM_MAGIC.to_vec());
    w.u32(EDM_VERSION);
    w.f32s(&[m.metres_per_unit]);
    w.u16(m.parts.len() as u16);
    for p in &m.parts {
        w.str(&p.name);
        w.0.extend_from_slice(&p.parent.to_le_bytes());
        w.f32s(&p.translation);
        w.f32s(&p.rotation);
    }
    w.u16(m.materials.len() as u16);
    for mat in &m.materials {
        w.str(&mat.name);
        w.0.push(mat.textures.len() as u8);
        for (slot, file) in &mat.textures {
            w.str(slot);
            w.str(file);
        }
        w.f32s(&mat.tint);
        w.u32(mat.flags);
    }
    w.u16(m.draws.len() as u16);
    for d in &m.draws {
        w.u16(d.part);
        w.u16(d.material);
        w.str(&d.source);
        w.u32(d.vertices.len() as u32);
        d.vertices.iter().for_each(|v| w.0.extend_from_slice(&v.0));
        w.u32(d.indices.len() as u32);
        d.indices.iter().for_each(|i| w.u16(*i));
    }
    w.u16(m.animations.len() as u16);
    for a in &m.animations {
        w.str(&a.name);
        w.f32s(&[a.seconds]);
        w.u16(a.tracks.len() as u16);
        for t in &a.tracks {
            w.u16(t.part);
            w.u16(t.keys.len() as u16);
            for k in &t.keys {
                w.f32s(&[k.time]);
                w.f32s(&k.translation);
                w.f32s(&k.rotation);
            }
        }
    }
    w.0
}

struct Rdr<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Rdr<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], EdmError> {
        let s = self.b.get(self.at..self.at + n).ok_or(EdmError::Short(self.at))?;
        self.at += n;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, EdmError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, EdmError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, EdmError> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().unwrap());
        if !v.is_finite() {
            return Err(EdmError::Bad(format!("a value that isn't a number at byte {}", self.at - 4)));
        }
        Ok(v)
    }
    fn f3(&mut self) -> Result<[f32; 3], EdmError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    fn f4(&mut self) -> Result<[f32; 4], EdmError> {
        Ok([self.f32()?, self.f32()?, self.f32()?, self.f32()?])
    }
    fn str(&mut self) -> Result<String, EdmError> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| EdmError::Bad("a name that isn't text".into()))
    }
}

pub fn parse_edm(b: &[u8]) -> Result<Edm, EdmError> {
    let mut r = Rdr { b, at: 0 };
    if r.take(8).map_err(|_| EdmError::Magic)? != EDM_MAGIC {
        return Err(EdmError::Magic);
    }
    match r.u32()? {
        EDM_VERSION => {}
        v => return Err(EdmError::Version(v)),
    }
    let metres_per_unit = r.f32()?;
    let mut parts = Vec::new();
    for _ in 0..r.u16()? {
        let name = r.str()?;
        let parent = i16::from_le_bytes(r.take(2)?.try_into().unwrap());
        parts.push(EdmPart { name, parent, translation: r.f3()?, rotation: r.f4()? });
    }
    for (i, p) in parts.iter().enumerate() {
        if p.parent >= i as i16 || p.parent < -1 {
            return Err(EdmError::Bad(format!("part {i}'s parent {} does not come before it", p.parent)));
        }
    }
    let mut materials = Vec::new();
    for _ in 0..r.u16()? {
        let name = r.str()?;
        let n = r.take(1)?[0];
        let mut textures = Vec::new();
        for _ in 0..n {
            textures.push((r.str()?, r.str()?));
        }
        materials.push(EdmMaterial { name, textures, tint: r.f4()?, flags: r.u32()? });
    }
    let mut draws = Vec::new();
    for _ in 0..r.u16()? {
        let part = r.u16()?;
        let material = r.u16()?;
        let source = r.str()?;
        let nv = r.u32()? as usize;
        let vb = r.take(nv.checked_mul(crate::elite::VERTEX_BYTES).ok_or(EdmError::Short(r.at))?)?;
        let vertices: Vec<crate::elite::Vertex> =
            vb.chunks_exact(crate::elite::VERTEX_BYTES).map(|c| crate::elite::Vertex(c.try_into().unwrap())).collect();
        let ni = r.u32()? as usize;
        let ib = r.take(ni.checked_mul(2).ok_or(EdmError::Short(r.at))?)?;
        let indices: Vec<u16> = ib.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        if part as usize >= parts.len() || material as usize >= materials.len() || indices.len() % 3 != 0 || indices.iter().any(|&i| i as usize >= nv) {
            return Err(EdmError::Bad(format!("draw {} is broken", draws.len())));
        }
        draws.push(EdmDraw { part, material, source, vertices, indices });
    }
    let mut animations = Vec::new();
    for _ in 0..r.u16()? {
        let name = r.str()?;
        let seconds = r.f32()?;
        if !(seconds.is_finite() && seconds > 0.0) {
            return Err(EdmError::Bad(format!("animation {name} lasts {seconds} s")));
        }
        let mut tracks = Vec::new();
        for _ in 0..r.u16()? {
            let part = r.u16()?;
            if part as usize >= parts.len() {
                return Err(EdmError::Bad(format!("animation {name} moves a part that isn't there")));
            }
            let mut keys = Vec::new();
            for _ in 0..r.u16()? {
                keys.push(EdmKey { time: r.f32()?, translation: r.f3()?, rotation: r.f4()? });
            }
            if keys.is_empty() {
                return Err(EdmError::Bad(format!("animation {name} has a track with no keys")));
            }
            tracks.push(EdmTrack { part, keys });
        }
        animations.push(EdmAnimation { name, seconds, tracks });
    }
    if r.at != b.len() {
        return Err(EdmError::Bad(format!("{} bytes after the end", b.len() - r.at)));
    }
    Ok(Edm { metres_per_unit, parts, materials, draws, animations })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_edm() -> Edm {
        let mut v = [0u8; 40];
        v[8..11].copy_from_slice(&[127, 254, 127]);
        v[36] = 255;
        Edm {
            metres_per_unit: 1.0 / 32768.0,
            parts: vec![
                EdmPart { name: "hull".into(), parent: -1, translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0] },
                EdmPart { name: "gear_front".into(), parent: 0, translation: [0.0, -1.0, 4.0], rotation: [0.0, 0.0, 0.0, 1.0] },
            ],
            materials: vec![EdmMaterial {
                name: "hull".into(),
                textures: vec![("diffuse".into(), "tex/hull.dds".into())],
                tint: [1.0; 4],
                flags: material_flags::TWO_SIDED,
            }],
            draws: vec![EdmDraw { part: 1, material: 0, source: "abc#0/1".into(), vertices: vec![crate::elite::Vertex(v); 3], indices: vec![0, 1, 2] }],
            animations: vec![EdmAnimation {
                name: "gear".into(),
                seconds: 2.5,
                tracks: vec![EdmTrack {
                    part: 1,
                    keys: vec![
                        EdmKey { time: 0.0, translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0] },
                        EdmKey { time: 2.5, translation: [0.0, -1.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0] },
                    ],
                }],
            }],
        }
    }

    #[test]
    fn edm_round_trips() {
        let m = sample_edm();
        let b = write_edm(&m);
        assert_eq!(parse_edm(&b).unwrap(), m);
        assert!(matches!(parse_edm(&b[..b.len() - 1]), Err(EdmError::Short(_))));
        let mut bad = b.clone();
        bad[0] = b'X';
        assert_eq!(parse_edm(&bad), Err(EdmError::Magic));
        let mut broken = m.clone();
        broken.draws[0].indices = vec![0, 1, 3];
        assert!(matches!(parse_edm(&write_edm(&broken)), Err(EdmError::Bad(_))));
    }

    const SAMPLE: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Root PresetName=\"KeyboardMouseOnly\" SortOrder=\"0\">\n\
        <!-- a comment -->\n\t<YawAxisRaw>\n\t\t<Binding Device=\"{NoDevice}\" Key=\"\" />\n\t\t<Inverted Value=\"0\" />\n\
        \t\t<Deadzone Value=\"0.05000000\" />\n\t</YawAxisRaw>\n\t<YawLeftButton>\n\t\t<Primary Device=\"Keyboard\" Key=\"Key_A\" />\n\
        \t\t<Secondary Device=\"{NoDevice}\" Key=\"\" />\n\t</YawLeftButton>\n\t<MouseXMode Value=\"Bindings_MouseRoll\" />\n\
        \t<ToggleFlightAssist>\n\t\t<Primary Device=\"Keyboard\" Key=\"Key_Z\">\n\t\t\t<Modifier Device=\"Keyboard\" Key=\"Key_LeftShift\" />\n\
        \t\t</Primary>\n\t\t<Secondary Device=\"Mouse\" Key=\"Mouse_4\" />\n\t\t<ToggleOn Value=\"1\" />\n\t</ToggleFlightAssist>\n</Root>\n";

    #[test]
    fn reads_a_binds_file() {
        let b = Binds::parse(SAMPLE).unwrap();
        assert_eq!(b.preset, "KeyboardMouseOnly");
        assert_eq!(b.key("YawLeftButton", "Keyboard"), Some("Key_A"));
        assert_eq!(b.get("YawLeftButton").unwrap().secondary, None);
        assert_eq!(b.value("MouseXMode"), Some("Bindings_MouseRoll"));
        let y = b.get("YawAxisRaw").unwrap();
        assert_eq!((y.axis.as_ref(), y.inverted, y.deadzone), (None, Some(false), Some(0.05)));
        let fa = b.get("ToggleFlightAssist").unwrap();
        assert_eq!(fa.primary.as_ref().unwrap().modifiers, vec![("Keyboard".to_string(), "Key_LeftShift".to_string())]);
        assert_eq!(b.key("ToggleFlightAssist", "Mouse"), Some("Mouse_4"));
        assert_eq!(fa.toggle_on, Some(true));
    }

    #[test]
    fn refuses_broken_xml() {
        assert!(parse_xml("<Root><A></B></Root>").is_err());
        assert!(parse_xml("<Root><A x=1/></Root>").is_err());
        assert!(parse_xml("<Root>").is_err());
        assert_eq!(Binds::parse("<Other/>"), Err(BindsError::NoRoot));
    }

    /// The install's default keyboard and mouse file gives the spec's Controls table (read only; skips without Elite).
    #[test]
    fn install_keyboard_defaults_are_the_specs() {
        let base = std::env::var_os("ELITE_PATH").map(std::path::PathBuf::from).unwrap_or_else(|| {
            std::path::PathBuf::from(r"D:\SteamLibrary\steamapps\common\Elite Dangerous\Products\elite-dangerous-odyssey-64")
        });
        let Ok(text) = std::fs::read_to_string(base.join("ControlSchemes").join("KeyboardMouseOnly.binds")) else {
            eprintln!("no Elite Dangerous install: skipped");
            return;
        };
        let b = Binds::parse(&text).unwrap();
        assert_eq!(b.preset, "KeyboardMouseOnly");
        for (action, key) in [
            ("YawLeftButton", "Key_A"),
            ("YawRightButton", "Key_D"),
            ("ForwardKey", "Key_W"),
            ("BackwardKey", "Key_S"),
            ("SetSpeedZero", "Key_X"),
            ("ToggleFlightAssist", "Key_Z"),
            ("UseBoostJuice", "Key_Tab"),
            ("LeftThrustButton", "Key_Q"),
            ("RightThrustButton", "Key_E"),
            ("UpThrustButton", "Key_R"),
            ("DownThrustButton", "Key_F"),
        ] {
            assert_eq!(b.key(action, "Keyboard"), Some(key), "{action}");
        }
        assert_eq!(b.value("MouseXMode"), Some("Bindings_MouseRoll"));
        assert_eq!(b.value("MouseYMode"), Some("Bindings_MousePitch"));
        eprintln!("{} actions and settings", b.actions.len());
    }
}
