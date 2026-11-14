//! The Faulcon DeLacy Sidewinder mod's own files: the reader for Elite Dangerous's `.binds` control files (the mod ships
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

#[cfg(test)]
mod tests {
    use super::*;

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
