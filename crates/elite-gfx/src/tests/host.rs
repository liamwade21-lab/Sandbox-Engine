//! A test `Host` for the proofs: the export's English string table (`strings/en.json`, a subset of
//! Elite's) and font map. Its text rules are a STAND-IN only to show translated text in place (the
//! real ones are `elite_ui::text`, a later task): `$KEY;` / `$KEY:#name=value;` look the key up
//! (case-insensitive) and expand again; `$#name:number:separated=1:decimalPlaces=2;` inserts a
//! parameter by section 8's number rule; `speed` and `distance` are printed plainly; a leading `&`
//! marks literal text. Keys missing from the subset are collected.

use crate::Host;
use elite_ui::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

pub struct TestHost {
    strings: HashMap<String, String>,
    fonts: HashMap<String, String>,
    pub missing: Mutex<BTreeSet<String>>,
    /// Every translation of a text holding a token: (text, translation).
    pub seen: Mutex<Vec<(String, String)>>,
}

impl TestHost {
    pub fn new(strings_json: &str, fontmap_json: &str) -> Self {
        let table: serde_json::Value = serde_json::from_str(strings_json).expect("strings/en.json");
        let strings = table
            .as_object()
            .expect("an object")
            .iter()
            .filter_map(|(k, e)| Some((k.to_lowercase(), e.get("text")?.as_str()?.to_string())))
            .collect();
        let map: serde_json::Value = serde_json::from_str(fontmap_json).expect("strings/fontmap.json");
        let fonts = map["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|e| (e["name"].as_str().unwrap().to_string(), e["font"].as_str().unwrap().to_string()))
            .collect();
        Self { strings, fonts, missing: Mutex::new(BTreeSet::new()), seen: Mutex::new(Vec::new()) }
    }

    fn get(&self, key: &str) -> Option<&str> {
        let found = self.strings.get(&key.to_lowercase()).map(String::as_str);
        if found.is_none() {
            self.missing.lock().unwrap().insert(key.to_string());
        }
        found
    }

    fn expand(&self, s: &str, params: &HashMap<String, String>, depth: u32) -> String {
        if depth > 8 || !s.contains('$') {
            return s.to_string();
        }
        let mut out = String::new();
        let mut rest = s;
        while let Some(i) = rest.find('$') {
            out.push_str(&rest[..i]);
            let tail = &rest[i + 1..];
            // A token's head is a name ended by ':' or ';'; any other '$' is text ($TitleFont in a
            // FACE attribute).
            let head = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '#')).unwrap_or(tail.len());
            if head == 0 || !matches!(tail[head..].chars().next(), Some(':' | ';')) {
                out.push('$');
                rest = tail;
                continue;
            }
            let Some(end) = token_end(tail) else {
                out.push_str(&rest[i..]);
                return out;
            };
            let token = &tail[..end];
            out.push_str(&self.token(token, params, depth).unwrap_or_else(|| format!("${token};")));
            rest = &tail[end + 1..];
        }
        out.push_str(rest);
        out
    }

    fn token(&self, token: &str, params: &HashMap<String, String>, depth: u32) -> Option<String> {
        let mut parts = token.split(':');
        let head = parts.next()?;
        if let Some(name) = head.strip_prefix('#') {
            let value = params.get(name)?;
            let kind = parts.next();
            let opts: HashMap<&str, &str> = parts.filter_map(|p| p.split_once('=')).collect();
            return Some(format_value(value, kind, &opts));
        }
        let template = self.get(head)?.to_string();
        let mut inner = HashMap::new();
        for p in parts {
            if let Some((k, v)) = p.strip_prefix('#').and_then(|p| p.split_once('=')) {
                inner.insert(k.to_string(), self.expand(v, params, depth + 1));
            }
        }
        Some(self.expand(&template, &inner, depth + 1))
    }
}

/// The end of a token: the first `;` not inside a nested `$...;`.
fn token_end(s: &str) -> Option<usize> {
    let mut nest = 0;
    for (i, c) in s.char_indices() {
        match c {
            '$' => nest += 1,
            ';' if nest == 0 => return Some(i),
            ';' => nest -= 1,
            _ => {}
        }
    }
    None
}

fn format_value(value: &str, kind: Option<&str>, opts: &HashMap<&str, &str>) -> String {
    let Ok(v) = value.parse::<f64>() else { return value.to_string() };
    match kind {
        Some("number") => {
            let d: i32 = opts.get("decimalPlaces").and_then(|s| s.parse().ok()).unwrap_or(0);
            let mult: f64 = opts.get("multiplier").and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let separated = opts.get("separated").is_some_and(|s| *s != "0");
            let x = v * mult;
            let t = x.abs() + 0.5 * 0.1f64.powi(d);
            let int_part = t.floor() as u64;
            let mut digits = int_part.to_string();
            if separated {
                let b = digits.clone().into_bytes();
                digits.clear();
                for (i, c) in b.iter().enumerate() {
                    if i > 0 && (b.len() - i).is_multiple_of(3) {
                        digits.push(',');
                    }
                    digits.push(*c as char);
                }
            }
            if d > 0 {
                let mut frac = t - t.floor();
                digits.push('.');
                for _ in 0..d {
                    frac *= 10.0;
                    digits.push(char::from(b'0' + frac.floor() as u8));
                    frac -= frac.floor();
                }
            }
            if x < 0.0 { format!("-{digits}") } else { digits }
        }
        Some("time") => {
            // Section 8's time rule for the parts the HUD names; the first part is not padded.
            let s = v.max(0.0).floor() as u64;
            let parts = [("showHours", s / 3600), ("showMinutes", s / 60 % 60), ("showSeconds", s % 60)];
            let shown: Vec<u64> = parts.iter().filter(|(k, _)| opts.get(k) == Some(&"1")).map(|(_, n)| *n).collect();
            shown
                .iter()
                .enumerate()
                .map(|(i, n)| if i == 0 { n.to_string() } else { format!("{n:02}") })
                .collect::<Vec<_>>()
                .join(":")
        }
        Some("speed") => format!("{v:.0}m/s"),
        Some("distance") => format!("{v:.0}m"),
        _ => value.to_string(),
    }
}

impl Host for TestHost {
    fn translate(&self, text: &str) -> String {
        // A leading '&' is the engine's "literal text, do not expand" marker (ui_messages_panels.md,
        // the comms log's entries); it is not shown.
        if let Some(literal) = text.strip_prefix('&') {
            return literal.to_string();
        }
        let out = self.expand(text, &HashMap::new(), 0);
        if text.contains('$') {
            self.seen.lock().unwrap().push((text.to_string(), out.clone()));
        }
        out
    }

    fn get_string(&self, key: &str) -> String {
        self.get(key).unwrap_or(key).to_string()
    }

    fn make_untranslated_string(&self, text: &str) -> String {
        text.to_string()
    }

    fn find_in_translation_string(&self, text: &str, find: &str, from: f64) -> f64 {
        let t = self.translate(text);
        let from = from.max(0.0) as usize;
        t.get(from..).and_then(|rest| rest.find(find)).map_or(-1.0, |i| (i + from) as f64)
    }

    fn localise_number(&self, number: f64, _format: &str, _extra: Option<&str>) -> String {
        format_value(&number.to_string(), Some("number"), &HashMap::from([("separated", "1")]))
    }

    fn does_text_symbol_exist(&self, key: &str) -> bool {
        self.strings.contains_key(&key.to_lowercase())
    }

    fn get_mapped_font_name(&self, name: &str) -> String {
        self.fonts.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    /// A stand-in record (a Sidewinder module's), enough for the Modules page's rows.
    fn get_database_item_data(&self, id: u32) -> Value {
        Value::object([
            ("name", Value::from(format!("Module {id}"))),
            ("longName", Value::from(format!("Module {id}"))),
            ("info", Value::from("")),
            ("marketCategoryName", Value::from("")),
            ("marketCategoryAbbrev", Value::from("")),
            ("marketCategoryIcon", Value::from("")),
            ("moduleSize", Value::from(1)),
            ("moduleClass", Value::from(5)),
            ("weaponMode", Value::from(-1)),
            ("isGuardianModule", Value::from(false)),
            ("isWingModule", Value::from(false)),
        ])
    }
}
