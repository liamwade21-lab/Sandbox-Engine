//! What a movie asks the game and waits for (ARCHITECTURE.md A10): the communication manager's
//! calls that return a value at once. In Elite they are native methods of the manager object
//! (`ui_messages_panels.md` 0, the 33 registered by the exe); here the bridge's ActionScript
//! forwards them to the game's `Host`. Everything else the manager does only tells the game
//! something, and comes out of `Movie::drain` as an `Out`.
//!
//! Of these, the movies' code calls `Translate`, `FindInTranslationString`, `GetMappedFontName`,
//! `OptionsFlag`, `DoesIconDataResourceExist`, `DoesTextSymbolExist` and `GetDatabaseItemData`;
//! the rest are on the manager for completeness. The text of every text field also passes through
//! `translate` when it is set (Scaleform's Translator, Ruffle patch a), unless the field is marked
//! no-translate.

use elite_ui::Value;

/// The game's answers to a movie's questions. One host may serve several movies.
pub trait Host: Send + Sync {
    /// Elite's text formatter: every `$KEY;` / `$KEY:#field=value;` token of `text` expanded
    /// (`ui_messages_hud.md` 8). Text without tokens comes back as it is.
    fn translate(&self, text: &str) -> String;

    /// The string table's text for `key`.
    fn get_string(&self, key: &str) -> String;

    /// `text` marked so that it is shown as it is, never translated.
    fn make_untranslated_string(&self, text: &str) -> String;

    /// Where `find` is in the translation of `text`, searching from `from`; -1 if nowhere.
    fn find_in_translation_string(&self, text: &str, find: &str, from: f64) -> f64;

    /// `number` written by Elite's number rules in `format` (`extra` as the call gives it).
    fn localise_number(&self, number: f64, format: &str, extra: Option<&str>) -> String;

    /// Whether the string table has `key`.
    fn does_text_symbol_exist(&self, key: &str) -> bool;

    /// The font a font-map name stands for (`$TitleFont` is Euro Caps: `strings/fontmap.json`).
    fn get_mapped_font_name(&self, name: &str) -> String;

    /// The item database's record of `id` (`longName`, `moduleSize`, `moduleClass`, `weaponMode`...,
    /// `ui_messages_panels.md` 0), or `Null` if there is none. Ship-specific, so no default.
    fn get_database_item_data(&self, id: u32) -> Value;

    /// The movie's own name as the game knows it (`GetMovieName`).
    fn get_movie_name(&self, movie: &str) -> String {
        movie.to_string()
    }

    /// An option flag (`DisableGuiEffects`, `FocusOnTextEntryField`): Elite's defaults are off.
    fn options_flag(&self, _name: &str) -> bool {
        false
    }

    /// Whether the game has an icon image of that name. The mod ships no icon images (a movie's
    /// `LoadImage` gets nothing), so none exists.
    fn does_icon_data_resource_exist(&self, _name: &str) -> bool {
        false
    }

    /// Named game data (`GetData`); no movie in the mod's scope calls it.
    fn get_data(&self, _name: &str, _id: u32) -> Value {
        Value::Null
    }
}
