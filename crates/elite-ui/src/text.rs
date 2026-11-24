//! Elite's text rules (ARCHITECTURE.md A10; `re/elite/notes/ui_messages_hud.md` section 8): the string table, the
//! translator of `$KEY;` / `$KEY:#field=value;` tokens, and the number, speed, distance and time formats behind it. The
//! game's `Host` answers a movie's `Translate`, `GetString` and `LocaliseNumber` with these. Bytes in, text out: the table
//! is read from what the caller hands over, never from a file.

mod format;
mod translate;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use crate::Value;

pub use format::TimeFlags;

/// The string table: key (lower case) to text; and the player's bindings the `binding` type writes (`Text::bind`).
#[derive(Clone, Debug, Default)]
pub struct Text {
    table: HashMap<String, String>,
    bindings: HashMap<String, Bound>,
}

/// One action's keys as a `.binds` file gives them (task UI7): its primary and secondary key and, for an axis action, the
/// axis's binding. Each is a chord of Elite's own key names (`Key_Backspace`, `Mouse_1`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bound {
    pub primary: Option<Chord>,
    pub secondary: Option<Chord>,
    pub axis: Option<Chord>,
}

/// A key and the modifier keys held with it, by Elite's key names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chord {
    pub key: String,
    pub modifiers: Vec<String>,
}

impl Text {
    /// The string table from `strings/en.json`'s shape: an object of keys, each either the text itself or an object whose
    /// `text` is it (the export's subset keeps a `why` beside it). Keys are kept lower-cased: the translator looks them up
    /// lower-cased (section 8: `$NAME:#value=12.5;` looks up `name`).
    pub fn from_json(bytes: &[u8]) -> Result<Text, String> {
        let raw: Value = serde_json::from_slice(bytes).map_err(|e| format!("the string table is not JSON: {e}"))?;
        let Value::Object(entries) = raw else {
            return Err("the string table is not an object of keys".to_string());
        };
        let mut table = HashMap::with_capacity(entries.len());
        for (key, entry) in entries {
            let text = match &entry {
                Value::Str(s) => s.clone(),
                Value::Object(_) => entry
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("the string table's key {key} has no text"))?
                    .to_string(),
                _ => return Err(format!("the string table's key {key} is neither a text nor an object")),
            };
            table.insert(key.to_lowercase(), text);
        }
        Ok(Text { table, bindings: HashMap::new() })
    }

    /// The player's keys for an action (the game hands over its binds once), for the `binding` type.
    pub fn bind(&mut self, action: &str, bound: Bound) {
        self.bindings.insert(action.to_lowercase(), bound);
    }

    /// The `binding` type (`$#value:binding:index=N:showUnboundText=B;`, the value an action's name; `binding_input_primary`
    /// is index 1, `_secondary` 2, `_either` 3). Elite's own writer of this type was not decoded (the notes name it and its
    /// parameters only: `ui_messages_hud.md` section 8), nor does the movies' code draw keyboard keys (`ButtonIcons` maps
    /// pad buttons only), so this is the stated stand-in, E: the bound key's name in Elite's own key-name strings
    /// (`key_backspace` "BACKSPACE", `mouse_1` "MOUSE 1"; a name the table lacks is written as the binds name it), a chord's
    /// modifiers first, joined by `binding_input_chord_conjunction` ("+"); index 1 the primary key (an axis action's axis
    /// when it has no primary), 2 the secondary, 3 both, joined by `binding_input_multiple_conjunction` ("or"), any other
    /// index as 1; nothing bound: `binding_input_unbound` ("??"), or nothing with `showUnboundText=0`. `showBraces` is not
    /// built (what it draws is not known).
    fn binding(&self, action: &str, index: u32, show_unbound: bool) -> String {
        let bound = self.bindings.get(&action.trim().to_lowercase());
        let chord = |c: &Chord| {
            let name = |k: &str| self.lookup(k).map_or_else(|| k.to_string(), str::to_string);
            let mut parts: Vec<String> = c.modifiers.iter().map(|m| name(m)).collect();
            parts.push(name(&c.key));
            parts.join(self.symbol("binding_input_chord_conjunction"))
        };
        let keys: Vec<&Chord> = match bound {
            None => Vec::new(),
            Some(b) => match index {
                2 => b.secondary.iter().collect(),
                3 => {
                    let both: Vec<&Chord> = b.primary.iter().chain(b.secondary.iter()).collect();
                    if both.is_empty() { b.axis.iter().collect() } else { both }
                }
                _ => b.primary.iter().chain(b.axis.iter()).take(1).collect(),
            },
        };
        if keys.is_empty() {
            return if show_unbound { self.symbol("binding_input_unbound").to_string() } else { String::new() };
        }
        let or = format!(" {} ", self.symbol("binding_input_multiple_conjunction"));
        keys.into_iter().map(chord).collect::<Vec<_>>().join(&or)
    }

    /// The table's text for `key`, matched lower-cased.
    fn lookup(&self, key: &str) -> Option<&str> {
        self.table.get(&key.to_lowercase()).map(String::as_str)
    }

    /// A symbol the formatters write (`block_separator`, `meters_symbol`...): its text, or nothing if the table lacks it.
    fn symbol(&self, key: &str) -> &str {
        self.lookup(key).unwrap_or("")
    }

    /// Whether the table has `key` (the manager's `DoesTextSymbolExist`).
    pub fn contains(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    /// The manager's `GetString(key)` (`CommunicationManager.as`: one string in, one out): the table's text for the key,
    /// as stored (not translated). A key the table lacks comes back as given, as the decoded manager's own `GetString`
    /// does (it returns its argument; Elite's native one was not read: stand-in, E). No movie in scope calls it.
    pub fn get_string(&self, key: &str) -> String {
        self.lookup(key).map_or_else(|| key.to_string(), str::to_string)
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;

    fn text() -> Text {
        let table = br#"{"binding_input_primary": "$#value:binding:index=1;", "binding_input_secondary": "$#value:binding:index=2;",
            "binding_input_either": "$#value:binding:index=3;", "binding_input_hideunbound": "$#value:binding:showUnboundText=0;",
            "binding_input_chord_conjunction": "+", "binding_input_multiple_conjunction": "or", "binding_input_unbound": "??",
            "key_backspace": "BACKSPACE", "key_apostrophe": "'", "key_leftshift": "LEFT SHIFT", "mouse_1": "MOUSE 1"}"#;
        let mut t = Text::from_json(table).unwrap();
        let chord = |key: &str, modifiers: &[&str]| Some(Chord { key: key.into(), modifiers: modifiers.iter().map(|m| m.to_string()).collect() });
        t.bind("ExplorationFSSQuit", Bound { primary: chord("Key_Backspace", &[]), secondary: chord("Key_Apostrophe", &[]), axis: None });
        t.bind("ExplorationFSSZoomIn", Bound { primary: chord("Mouse_1", &[]), ..Bound::default() });
        t.bind("Chorded", Bound { primary: chord("Key_Apostrophe", &["Key_LeftShift"]), ..Bound::default() });
        t.bind("AxisOnly", Bound { axis: chord("Key_NoSuchName", &[]), ..Bound::default() });
        t
    }

    #[test]
    fn a_binding_token_writes_the_bound_keys_names() {
        let t = text();
        assert_eq!(t.translate("$BINDING_input_primary:#value=ExplorationFSSQuit;"), "BACKSPACE");
        assert_eq!(t.translate("$BINDING_input_secondary:#value=ExplorationFSSQuit;"), "'");
        assert_eq!(t.translate("$BINDING_input_either:#value=ExplorationFSSQuit;"), "BACKSPACE or '");
        assert_eq!(t.translate("$BINDING_input_either:#value=explorationfsszoomin;"), "MOUSE 1", "names match whatever their case");
        assert_eq!(t.translate("$BINDING_input_primary:#value=Chorded;"), "LEFT SHIFT+'");
        assert_eq!(t.translate("$BINDING_input_primary:#value=AxisOnly;"), "Key_NoSuchName", "an axis; a name the table lacks as written");
        assert_eq!(t.translate("$BINDING_input_primary:#value=Unbound;"), "??");
        assert_eq!(t.translate("$BINDING_input_hideunbound:#value=Unbound;"), "");
        assert_eq!(t.translate("Leave: $BINDING_input_primary:#value=ExplorationFSSQuit; now"), "Leave: BACKSPACE now");
    }
}
