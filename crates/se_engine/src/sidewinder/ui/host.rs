//! The game's answers to a movie's questions (`elite_gfx::Host`): Elite's text rules (`elite_ui::Text`) on the shipped
//! string table (`ui/strings/en.json`) and the font map (`ui/fonts/fontmap.json`); the item database from the panels'
//! modules (`ItemDatabase`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use elite_ui::panel_facts::{Module, PanelFacts};
use elite_ui::{Text, Value};

/// The item database record's fields (`ui_messages_panels.md` "Native functions": `GetDatabaseItemData` builder
/// `FUN_14452a0a0`, A), each with its neutral value.
const ITEM_FIELDS: [(&str, ItemValue); 14] = [
    ("name", ItemValue::Text),
    ("info", ItemValue::Text),
    ("longName", ItemValue::Text),
    ("marketCategoryName", ItemValue::Text),
    ("marketCategoryIcon", ItemValue::Text),
    ("marketCategoryAbbrev", ItemValue::Text),
    ("basePrice", ItemValue::Number),
    ("moduleSize", ItemValue::Number),
    ("moduleClass", ItemValue::Number),
    ("modulePowerConsumption", ItemValue::Number),
    ("weaponMode", ItemValue::Number),
    ("isGuardianModule", ItemValue::Flag),
    ("isWingModule", ItemValue::Flag),
    ("moduleMass", ItemValue::Number),
];

#[derive(Clone, Copy)]
enum ItemValue {
    Text,
    Number,
    Flag,
}

/// The ship's modules as the item database answers them (`GetDatabaseItemData`): the panels' facts' modules, written by
/// `panels.rs` when they change, read by the host when a movie asks (shared: the host is the movies', on the main thread
/// too, but held behind `elite_gfx::Host`'s `Send + Sync`).
#[derive(Clone, Default)]
pub struct ItemDatabase(Arc<Mutex<Vec<Module>>>);

impl ItemDatabase {
    /// The modules now (kept only when they differ from what is held).
    pub fn set(&self, modules: &[Module]) {
        let mut held = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if held.as_slice() != modules {
            *held = modules.to_vec();
        }
    }

    /// The record of an item id (a `Module::id`): `elite_ui::feed::database_item`, the pure crate's answer (its proof's).
    fn item(&self, id: u32) -> Option<Value> {
        let modules = self.0.lock().unwrap_or_else(|p| p.into_inner()).clone();
        elite_ui::feed::database_item(&PanelFacts { modules, ..PanelFacts::default() }, id)
    }
}

/// One host serves every movie.
pub struct UiHost {
    text: Text,
    /// A font-map name (`$TitleFont`) to the face it stands for (`Euro Caps`).
    fonts: HashMap<String, String>,
    items: ItemDatabase,
}

impl UiHost {
    pub fn new(text: Text, fonts: HashMap<String, String>, items: ItemDatabase) -> UiHost {
        UiHost { text, fonts, items }
    }
}

impl elite_gfx::Host for UiHost {
    fn translate(&self, text: &str) -> String {
        self.text.translate(text)
    }

    fn get_string(&self, key: &str) -> String {
        self.text.get_string(key)
    }

    fn make_untranslated_string(&self, text: &str) -> String {
        text.to_string()
    }

    /// Positions count UTF-16 units, as Flash's strings do (`String.indexOf`).
    fn find_in_translation_string(&self, text: &str, find: &str, from: f64) -> f64 {
        find_utf16(&self.translate(text), find, from)
    }

    fn localise_number(&self, number: f64, format: &str, extra: Option<&str>) -> String {
        self.text.localise_number(number, format, extra)
    }

    fn does_text_symbol_exist(&self, key: &str) -> bool {
        self.text.contains(key)
    }

    fn get_mapped_font_name(&self, name: &str) -> String {
        self.fonts.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    /// A module of the ship by its id (the panels' `Module::id`, `ItemDatabase`); any other id gets the record's every
    /// field with its neutral value (an empty text, 0, false; grade E): a movie's handler reads every field it names on any
    /// path, and an undefined one would stop it.
    fn get_database_item_data(&self, id: u32) -> Value {
        if let Some(record) = self.items.item(id) {
            return record;
        }
        Value::Object(
            ITEM_FIELDS
                .iter()
                .map(|(name, kind)| {
                    let v = match kind {
                        ItemValue::Text => Value::Str(String::new()),
                        ItemValue::Number => Value::Num(0.0),
                        ItemValue::Flag => Value::Bool(false),
                    };
                    (name.to_string(), v)
                })
                .collect(),
        )
    }
}

/// Where `find` is in `text` at or after `from`, both in UTF-16 units (Flash's), -1 if nowhere.
fn find_utf16(text: &str, find: &str, from: f64) -> f64 {
    let (t, f): (Vec<u16>, Vec<u16>) = (text.encode_utf16().collect(), find.encode_utf16().collect());
    let from = if from.is_finite() { from.max(0.0) as usize } else { 0 };
    if from > t.len() || f.len() > t.len() {
        return -1.0;
    }
    (from..=t.len() - f.len()).find(|&i| t[i..].starts_with(&f)).map_or(-1.0, |i| i as f64)
}

#[cfg(test)]
mod tests {
    use super::find_utf16;

    /// Flash counts UTF-16 units: an e acute is one unit (two bytes in UTF-8), a rocket two.
    #[test]
    fn positions_are_utf16_units() {
        assert_eq!(find_utf16("Caf\u{e9} Ls", "Ls", 0.0), 5.0);
        assert_eq!(find_utf16("\u{1F680}ab", "b", 0.0), 3.0);
        assert_eq!(find_utf16("abcabc", "abc", 1.0), 3.0);
        assert_eq!(find_utf16("abc", "x", 0.0), -1.0);
        assert_eq!(find_utf16("abc", "c", 9.0), -1.0);
    }
}
