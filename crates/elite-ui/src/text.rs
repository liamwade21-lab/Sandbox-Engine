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

/// The string table: key (lower case) to text.
#[derive(Clone, Debug, Default)]
pub struct Text {
    table: HashMap<String, String>,
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
        Ok(Text { table })
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
