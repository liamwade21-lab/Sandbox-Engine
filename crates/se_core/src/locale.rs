//! Loader for SpaceEngine's localization files: `data/locale/<lang>-gui.cfg`
//! (UI strings and header settings) and `data/locale/<lang>-db.cfg` (object
//! names, parents and descriptions).
//!
//! These files are not parsed with [`crate::sc`]: database `Descr` strings span
//! several lines, which the `sc` lexer does not allow. This module has its own
//! small tokenizer that accepts multi-line quoted strings and `//` comments.

use std::collections::HashMap;

use crate::vfs::Vfs;

/// One `RObject { ... }` record from a `*-db.cfg` file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DbObject {
    /// Localized display name (`LocName`).
    pub loc_name: String,
    /// Catalog / internal name (`Name`). `Parent` links refer to this.
    pub name: String,
    /// `Parent`: the `Name` of the parent object.
    pub parent: Option<String>,
    /// `Descr`: long description, paragraphs separated by blank lines.
    pub descr: Option<String>,
    /// `Pioneer`: discoverer.
    pub pioneer: Option<String>,
    /// `Date`: discovery date as written in the file (`YYYY.MM.DD HH:MM:SS.cc`).
    pub date: Option<String>,
}

/// Translations, header metadata and object names for one language.
#[derive(Debug, Clone, Default)]
pub struct Locale {
    /// Header `InternationalName`, e.g. `"English"`.
    pub international_name: Option<String>,
    /// Header `LocalName`, the language's own name for itself.
    pub local_name: Option<String>,
    /// Header `FlagIndex`.
    pub flag_index: Option<u32>,
    /// Header `LocNameSteam`, matched against the Steam UI language.
    pub loc_name_steam: Option<String>,
    /// Every other top-level `Key value` setting (gender words, class word order,
    /// separators, ...). Values are unquoted.
    pub settings: HashMap<String, String>,
    /// Translation pairs, keyed by the source string including any `#context`.
    gui: HashMap<String, String>,
    objects: Vec<DbObject>,
    by_name: HashMap<String, usize>,
}

impl Locale {
    /// Loads `locale/<lang>-gui.cfg` and `locale/<lang>-db.cfg`, plus the optional
    /// `locale/WGSBN_asteroids-<lang>-db.cfg` (asteroid names, which override
    /// duplicates). Missing files are logged and skipped.
    pub fn load(vfs: &Vfs, lang: &str) -> Locale {
        let mut loc = Locale::default();
        let gui_path = format!("locale/{lang}-gui.cfg");
        match vfs.read_string(&gui_path) {
            Ok(text) => loc.parse_gui(&text),
            Err(e) => log::warn!("locale: cannot read {gui_path}: {e}"),
        }
        for path in [format!("locale/{lang}-db.cfg"), format!("locale/WGSBN_asteroids-{lang}-db.cfg")] {
            match vfs.read_string(&path) {
                Ok(text) => loc.parse_db(&text),
                Err(_) => log::debug!("locale: no {path}"),
            }
        }
        loc
    }

    /// Parses a `*-gui.cfg` file into this locale: header fields, settings and
    /// translation pairs. Can be called repeatedly; later values overwrite earlier ones.
    pub fn parse_gui(&mut self, text: &str) {
        let toks = tokenize(text);
        let mut i = 0;
        while i < toks.len() {
            let next = toks.get(i + 1);
            match (&toks[i].tok, next) {
                // `"source#context" "translation"`
                (Tok::Str(k), Some(Token { tok: Tok::Str(v), .. })) => {
                    self.gui.insert(k.clone(), strip_marker(v).to_string());
                    i += 2;
                }
                // `Key value` on one line (value may be a number, word or string)
                (Tok::Word(k), Some(Token { tok, line })) if *line == toks[i].line => match value_text(tok) {
                    Some(v) => {
                        self.settings.insert(k.clone(), v.to_string());
                        i += 2;
                    }
                    None => i += 1,
                },
                _ => i += 1,
            }
        }
        self.international_name = self.settings.get("InternationalName").cloned();
        self.local_name = self.settings.get("LocalName").cloned();
        self.loc_name_steam = self.settings.get("LocNameSteam").cloned();
        self.flag_index = self.settings.get("FlagIndex").and_then(|v| v.parse().ok());
    }

    /// Parses a `*-db.cfg` file, adding its `RObject` records. A record whose `Name`
    /// is already known replaces the earlier one.
    pub fn parse_db(&mut self, text: &str) {
        let toks = tokenize(text);
        let mut i = 0;
        while i < toks.len() {
            // `PObject` is the record kind used by the tribute files.
            let is_record = matches!(&toks[i].tok, Tok::Word(w) if w == "RObject" || w == "PObject")
                && matches!(toks.get(i + 1), Some(Token { tok: Tok::Open, .. }));
            if !is_record {
                i += 1;
                continue;
            }
            i += 2;
            let mut depth = 1;
            let mut obj = DbObject::default();
            while i < toks.len() && depth > 0 {
                match &toks[i].tok {
                    Tok::Open => {
                        depth += 1;
                        i += 1;
                    }
                    Tok::Close => {
                        depth -= 1;
                        i += 1;
                    }
                    Tok::Word(key) if depth == 1 => {
                        let value = toks
                            .get(i + 1)
                            .filter(|next| next.line == toks[i].line)
                            .and_then(|next| value_text(&next.tok));
                        match value {
                            Some(v) => {
                                let v = v.to_string();
                                match key.as_str() {
                                    "LocName" => obj.loc_name = v,
                                    "Name" => obj.name = v,
                                    "Parent" => obj.parent = Some(v),
                                    "Descr" => obj.descr = Some(v.trim().to_string()),
                                    "Pioneer" => obj.pioneer = Some(v),
                                    "Date" => obj.date = Some(v),
                                    _ => {}
                                }
                                i += 2;
                            }
                            None => i += 1,
                        }
                    }
                    _ => i += 1,
                }
            }
            if !obj.name.is_empty() {
                self.insert_object(obj);
            }
        }
    }

    fn insert_object(&mut self, obj: DbObject) {
        let existing = self.by_name.get(&obj.name).copied();
        match existing {
            Some(idx) => self.objects[idx] = obj,
            None => {
                self.by_name.insert(obj.name.clone(), self.objects.len());
                self.objects.push(obj);
            }
        }
    }

    /// The source strings this file translates (with their `#context`).
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.gui.keys().map(String::as_str)
    }

    /// Translation of a UI string, or `key` itself when there is none.
    pub fn tr<'a>(&'a self, key: &'a str) -> &'a str {
        self.gui.get(key).map(String::as_str).unwrap_or(key)
    }

    /// Translation of `key` with the given context (`key#ctx`), falling back to
    /// [`Locale::tr`] on `key`.
    pub fn tr_ctx<'a>(&'a self, key: &'a str, ctx: &str) -> &'a str {
        let full = format!("{key}#{ctx}");
        match self.gui.get(&full) {
            Some(v) => v.as_str(),
            None => self.tr(key),
        }
    }

    /// Localized display name for an object's catalog `Name`, e.g. `"Sol"` -> `"Sun"`.
    pub fn object_name(&self, original: &str) -> Option<&str> {
        let obj = self.object(original)?;
        (!obj.loc_name.is_empty()).then_some(obj.loc_name.as_str())
    }

    /// Full database record for a catalog `Name`.
    pub fn object(&self, name: &str) -> Option<&DbObject> {
        self.by_name.get(name).map(|&i| &self.objects[i])
    }

    /// All database records in file order.
    pub fn objects(&self) -> &[DbObject] {
        &self.objects
    }

    /// Number of translation pairs loaded.
    pub fn gui_len(&self) -> usize {
        self.gui.len()
    }

    /// Number of database records loaded.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }
}

/// Language codes available in the VFS, from `locale/*-gui.cfg` (e.g. `"eng"`, `"ger"`).
pub fn available_languages(vfs: &Vfs) -> Vec<String> {
    let mut langs: Vec<String> = vfs
        .list_ext("locale", "cfg")
        .iter()
        .filter_map(|p| {
            let file = p.rsplit('/').next().unwrap_or(p.as_str());
            file.strip_suffix("-gui.cfg").map(str::to_string)
        })
        .collect();
    langs.sort();
    langs.dedup();
    langs
}

/// `LocalName` from a `*-gui.cfg` header: the language's name in that language (e.g. `"Русский"`).
pub fn header_local_name(gui_text: &str) -> Option<String> {
    gui_text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("LocalName")?.trim();
        let value = value.strip_prefix('"')?.split('"').next()?;
        Some(value.to_string())
    })
}

/// SE font file (in `locale/`) that the UI of `lang` needs: `NotoSansCJKsc` for Chinese,
/// Japanese and Korean, else SE's `PrimaryFont` `NotoSans` (main-def.cfg), English included.
pub fn ui_font_file(lang: &str) -> Option<&'static str> {
    match lang {
        "chs" | "cht" | "jpn" | "kor" => Some("NotoSansCJKsc-Regular.otf"),
        _ => Some("NotoSans-Regular.ttf"),
    }
}

/// Language used when a caller has no preference.
pub const DEFAULT_LANG: &str = "eng";

/// Wiki text for one object: localized name and plain-text description.
#[derive(Debug, Clone, PartialEq)]
pub struct WikiEntry {
    pub loc_name: String,
    /// Plain text (see [`bbcode_to_text`]); `None` when the record has no `Descr`.
    pub description: Option<String>,
}

/// Wiki entries keyed by lowercase catalog `Name`.
pub type WikiIndex = HashMap<String, WikiEntry>;

/// Builds the wiki index for `lang` from `locale/<lang>-db.cfg` and every other
/// `locale/*-<lang>-db.cfg` (e.g. `WGSBN_asteroids-eng-db.cfg`, `tribute-eng-db.cfg`).
/// Files are applied in that order, so later records replace earlier ones.
pub fn wiki_index(vfs: &Vfs, lang: &str) -> WikiIndex {
    let texts: Vec<String> = db_files(vfs, lang)
        .iter()
        .filter_map(|p| match vfs.read_string(p) {
            Ok(t) => Some(t),
            Err(e) => {
                log::debug!("wiki: cannot read {p}: {e}");
                None
            }
        })
        .collect();
    wiki_index_from_texts(texts.iter().map(String::as_str))
}

/// The database files of a language: the base file first, then the suffixed ones sorted by name.
fn db_files(vfs: &Vfs, lang: &str) -> Vec<String> {
    let base = format!("{lang}-db.cfg");
    let suffix = format!("-{base}");
    let mut extra: Vec<String> = vfs
        .list_ext("locale", "cfg")
        .into_iter()
        .filter(|p| p.rsplit('/').next().is_some_and(|f| f.ends_with(&suffix)))
        .collect();
    extra.sort();
    let mut out = vec![format!("locale/{base}")];
    out.extend(extra);
    out
}

fn wiki_index_from_texts<'a>(texts: impl IntoIterator<Item = &'a str>) -> WikiIndex {
    let mut loc = Locale::default();
    for t in texts {
        loc.parse_db(t);
    }
    loc.objects()
        .iter()
        .map(|o| {
            let entry = WikiEntry {
                loc_name: if o.loc_name.is_empty() { o.name.clone() } else { o.loc_name.clone() },
                description: o.descr.as_deref().map(bbcode_to_text).filter(|s| !s.is_empty()),
            };
            (o.name.to_lowercase(), entry)
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Script {
    Sup,
    Sub,
}

/// Converts a database description to plain text.
///
/// Handled: `[sup]..[/sup]` and `[sub]..[/sub]` (Unicode super/subscript characters
/// where they exist, otherwise the character is kept), `[br]` (newline), and the
/// unit or symbol tags `[deg]` `°`, `[hour]` `h`, `[min]` `m`, `[sec]` `s`,
/// `[mag]` ` mag`, `[EARTH]` `⊕`. Any other tag, including `[url=..]`, `[b]`,
/// `[i]`, `[big]`, `[small]`, `[color=..]` and numeric citations such as `[19]`,
/// is removed and its text kept. A `[` that does not start a tag is kept.
pub fn bbcode_to_text(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut script: Option<Script> = None;
    let mut rest = src;
    while let Some(open) = rest.find('[') {
        push_text(&mut out, &rest[..open], script);
        let after = &rest[open..];
        match parse_tag(after) {
            Some((closing, name, len)) => {
                rest = &after[len..];
                match (closing, name.to_ascii_lowercase().as_str()) {
                    (false, "sup") => script = Some(Script::Sup),
                    (false, "sub") => script = Some(Script::Sub),
                    (true, "sup" | "sub") => script = None,
                    (false, "br") => out.push('\n'),
                    (false, "deg") => out.push('°'),
                    (false, "earth") => out.push('⊕'),
                    (false, "hour") => out.push('h'),
                    (false, "min") => out.push('m'),
                    (false, "sec") => out.push('s'),
                    (false, "mag") => out.push_str(" mag"),
                    _ => {}
                }
            }
            None => {
                out.push('[');
                rest = &after[1..];
            }
        }
    }
    push_text(&mut out, rest, script);
    out.trim().to_string()
}

fn push_text(out: &mut String, s: &str, script: Option<Script>) {
    for c in s.chars() {
        out.push(match script {
            Some(Script::Sup) => superscript(c),
            Some(Script::Sub) => subscript(c),
            None => c,
        });
    }
}

/// Parses a tag at the start of `s` (which begins with `[`). Returns
/// `(is_closing, name, byte_length)` for `[name]`, `[name=value]` or `[/name]`.
/// Names are ASCII alphanumeric, so text such as `[The primary ...]` is not a tag.
fn parse_tag(s: &str) -> Option<(bool, &str, usize)> {
    let end = s.find(']')?;
    let inner = &s[1..end];
    if inner.len() > 64 {
        return None;
    }
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner),
    };
    let name = inner.split('=').next().unwrap_or("");
    let valid = !name.is_empty() && name.len() <= 16 && name.chars().all(|c| c.is_ascii_alphanumeric());
    valid.then_some((closing, name, end + 1))
}

fn superscript(c: char) -> char {
    const DIGITS: [char; 10] = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    match c {
        '0'..='9' => DIGITS[c as usize - '0' as usize],
        '+' => '⁺',
        '-' | '−' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'n' => 'ⁿ',
        'i' => 'ⁱ',
        _ => c,
    }
}

fn subscript(c: char) -> char {
    match c {
        '0'..='9' => char::from_u32('₀' as u32 + (c as u32 - '0' as u32)).unwrap_or(c),
        '+' => '₊',
        '-' | '−' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        's' => 'ₛ',
        't' => 'ₜ',
        'x' => 'ₓ',
        _ => c,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Str(String),
    Word(String),
    Open,
    Close,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    line: u32,
}

fn value_text(tok: &Tok) -> Option<&str> {
    match tok {
        Tok::Str(s) | Tok::Word(s) => Some(s),
        _ => None,
    }
}

/// Drops a trailing `#context` marker from a translation value
/// (`"spacecraft#plural"` -> `"spacecraft"`).
fn strip_marker(v: &str) -> &str {
    match v.rsplit_once('#') {
        Some((head, tail)) if !head.is_empty() && !tail.is_empty() && !tail.contains(char::is_whitespace) => head,
        _ => v,
    }
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut line = 1u32;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => line += 1,
            '\r' => {}
            c if c.is_whitespace() => {}
            '/' if chars.peek() == Some(&'/') => {
                while let Some(&n) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '{' => out.push(Token { tok: Tok::Open, line }),
            '}' => out.push(Token { tok: Tok::Close, line }),
            '"' => {
                let start = line;
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' if chars.peek() == Some(&'"') => {
                            chars.next();
                            s.push('"');
                        }
                        '\n' => {
                            line += 1;
                            s.push('\n');
                        }
                        '\r' => {}
                        _ => s.push(c),
                    }
                }
                out.push(Token { tok: Tok::Str(s), line: start });
            }
            _ => {
                let mut w = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || matches!(n, '{' | '}' | '"') {
                        break;
                    }
                    w.push(n);
                    chars.next();
                }
                out.push(Token { tok: Tok::Word(w), line });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const GUI: &str = "// Translation made by someone\r\n\
\r\n\
InternationalName\t\"English\"\r\n\
LocalName\t\t\t\"English\"\r\n\
FlagIndex           109\r\n\
LocNameSteam\t\t\"english\"\r\n\
GenderPlanet\t\t\"neuter\"\r\n\
PlanetClassOrderTemp 0\t// temperature class word\r\n\
ClassNameAdjectiveSeparator\t\"\"\r\n\
Translations\r\n\
{\r\n\
\"Spacecraft#plural\"\t\t\t\"Spacecraft\"\r\n\
\"spacecraft#plural\"\t\t\t\"spacecraft#plural\"\r\n\
\"temperate#masculine\"\t\t\"temperate\"\r\n\
\"Format#video\" \"Format\"\r\n\
\"Format#plural\" \"Formats\"\r\n\
\"Black body colouration\" \"Black body coloration\"\r\n\
}\r\n";

    const DB: &str = r#"RObject
{
	LocName "Sun"
	Name	"Sol"
	Parent	"Solar System"
	Descr	"Sol, or the Sun, is a star. // not a comment

It is hot."
	Pioneer	"Asaph Hall"
	Date	"1877.08.18 00:00:00.00"
}

RObject { LocName "Mercury"  Name "Mercury"  Parent "Sol" }
"#;

    fn sample() -> Locale {
        let mut loc = Locale::default();
        loc.parse_gui(GUI);
        loc.parse_db(DB);
        loc
    }

    #[test]
    fn header_local_name_and_ui_font() {
        let text = "\u{feff}// Translation\r\nInternationalName   \"Russian\"\r\nLocalName           \"Русский\"\r\nFlagIndex 79\r\n";
        assert_eq!(header_local_name(&crate::vfs::decode_text(text.as_bytes())).as_deref(), Some("Русский"));
        assert_eq!(header_local_name("InternationalName \"English\"\n"), None);
        assert_eq!(ui_font_file("eng"), Some("NotoSans-Regular.ttf"));
        assert_eq!(ui_font_file("rus"), Some("NotoSans-Regular.ttf"));
        assert_eq!(ui_font_file("chs"), Some("NotoSansCJKsc-Regular.otf"));
        assert_eq!(ui_font_file("jpn"), Some("NotoSansCJKsc-Regular.otf"));
    }

    #[test]
    fn gui_header_and_settings() {
        let loc = sample();
        assert_eq!(loc.international_name.as_deref(), Some("English"));
        assert_eq!(loc.local_name.as_deref(), Some("English"));
        assert_eq!(loc.loc_name_steam.as_deref(), Some("english"));
        assert_eq!(loc.flag_index, Some(109));
        assert_eq!(loc.settings.get("GenderPlanet").map(String::as_str), Some("neuter"));
        assert_eq!(loc.settings.get("PlanetClassOrderTemp").map(String::as_str), Some("0"));
        assert_eq!(loc.settings.get("ClassNameAdjectiveSeparator").map(String::as_str), Some(""));
        assert_eq!(loc.gui_len(), 6);
    }

    #[test]
    fn tr_and_context() {
        let loc = sample();
        assert_eq!(loc.tr("temperate#masculine"), "temperate");
        assert_eq!(loc.tr("Black body colouration"), "Black body coloration");
        // Value carrying its own marker is stripped.
        assert_eq!(loc.tr("Spacecraft#plural"), "Spacecraft");
        assert_eq!(loc.tr("spacecraft#plural"), "spacecraft");
        // Missing key returns itself.
        assert_eq!(loc.tr("Nothing here"), "Nothing here");
        assert_eq!(loc.tr_ctx("Format", "plural"), "Formats");
        assert_eq!(loc.tr_ctx("Format", "video"), "Format");
        assert_eq!(loc.tr_ctx("Format", "unknown"), "Format");
        assert_eq!(loc.tr_ctx("Missing", "x"), "Missing");
    }

    #[test]
    fn db_records() {
        let loc = sample();
        assert_eq!(loc.object_count(), 2);
        assert_eq!(loc.object_name("Sol"), Some("Sun"));
        assert_eq!(loc.object_name("Mercury"), Some("Mercury"));
        assert_eq!(loc.object_name("Nope"), None);
        let sun = loc.object("Sol").unwrap();
        assert_eq!(sun.parent.as_deref(), Some("Solar System"));
        assert_eq!(sun.pioneer.as_deref(), Some("Asaph Hall"));
        assert_eq!(sun.date.as_deref(), Some("1877.08.18 00:00:00.00"));
        let descr = sun.descr.as_deref().unwrap();
        assert!(descr.contains("// not a comment"));
        assert!(descr.ends_with("It is hot."));
        let mercury = loc.object("Mercury").unwrap();
        assert_eq!(mercury.parent.as_deref(), Some("Sol"));
        assert_eq!(mercury.descr, None);
    }

    #[test]
    fn db_later_record_replaces_same_name() {
        let mut loc = sample();
        loc.parse_db(r#"RObject { LocName "Soleil" Name "Sol" }"#);
        assert_eq!(loc.object_count(), 2);
        assert_eq!(loc.object_name("Sol"), Some("Soleil"));
    }

    #[test]
    fn wiki_index_from_inline_robject() {
        const SNIPPET: &str = r#"RObject
{
	LocName "Sun"
	Name	"Sol"
	Parent	"Solar System"
	Descr	"Sol has a mass of 2·10[sup]30[/sup] kilograms.[br]Its surface is hot."
}
RObject { LocName "Moon" Name "Luna" }
"#;
        let idx = wiki_index_from_texts([SNIPPET]);
        assert_eq!(idx.len(), 2);
        let sun = &idx["sol"];
        assert_eq!(sun.loc_name, "Sun");
        assert_eq!(sun.description.as_deref(), Some("Sol has a mass of 2·10³⁰ kilograms.\nIts surface is hot."));
        // A record without Descr has no description; an empty LocName falls back to Name.
        assert_eq!(idx["luna"].description, None);
        assert_eq!(idx["luna"].loc_name, "Moon");
        // A later file replaces the record with the same name.
        let idx = wiki_index_from_texts([SNIPPET, r#"PObject { LocName "Soleil" Name "Sol" }"#]);
        assert_eq!(idx["sol"].loc_name, "Soleil");
        assert_eq!(idx["sol"].description, None);
    }

    #[test]
    fn bbcode_tags_to_plain_text() {
        assert_eq!(bbcode_to_text("30[sup]30[/sup]"), "30³⁰");
        assert_eq!(bbcode_to_text("H[sub]2[/sub]O"), "H₂O");
        assert_eq!(bbcode_to_text("a[br]b"), "a\nb");
        assert_eq!(bbcode_to_text("23.4[deg] and 5[hour]55[min]04[sec]"), "23.4° and 5h55m04s");
        // Unknown tags are removed and their text kept.
        assert_eq!(
            bbcode_to_text("[url=http://x.y]link[/url] [b]bold[/b] [color=aaaaaa]grey[/color][19]"),
            "link bold grey"
        );
        // A bracket that is not a tag is kept.
        assert_eq!(bbcode_to_text("[The primary is named X.]"), "[The primary is named X.]");
    }

    #[test]
    #[ignore]
    fn real_install_wiki_index() {
        let vfs = Vfs::mount_install(Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).expect("mount install");
        let idx = wiki_index(&vfs, DEFAULT_LANG);
        let sun = idx.get("sol").expect("Sol in the English wiki index");
        println!("{} -> {:?}", sun.loc_name, sun.description.as_deref().map(|d| &d[..d.len().min(120)]));
        assert!(idx.len() > 1000);
    }

    #[test]
    #[ignore]
    fn real_install_files_parse() {
        let vfs = Vfs::mount_install(Path::new("D:/SteamLibrary/steamapps/common/SpaceEngine")).expect("mount install");
        println!("languages: {:?}", available_languages(&vfs));
        for lang in ["eng", "ger"] {
            let loc = Locale::load(&vfs, lang);
            println!(
                "{lang}: gui={} objects={} name={:?} flag={:?}",
                loc.gui_len(),
                loc.object_count(),
                loc.international_name,
                loc.flag_index
            );
            assert!(loc.gui_len() > 0);
            assert!(loc.object_count() > 0);
        }
    }
}
