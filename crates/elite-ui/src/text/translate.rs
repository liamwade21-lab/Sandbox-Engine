//! Elite's translator (`re/elite/notes/ui_messages_hud.md` section 8, A; the nested form from `ui_messages_panels.md`
//! 3.6, B): a `$KEY;` token is the string table's text for the key (looked up lower-cased); `$KEY:#field=value:#f2=v2;`
//! hands that text fields, which it uses as `$#field;` (the value as given) or `$#field:type:param=value;` (the value
//! written by the type: `number`, `speed`, `distance`, `time`). A value may itself hold tokens (`#name=$OTHER;;`), and
//! the table's text may hold further keys (`percent` = `$#value:number:separated=1;$PERCENT_SYMBOL;`): both are
//! translated in turn. Parameter, field and type names match whatever their case (the exe parses parameters with
//! `_wcsnicmp`; the same for fields and types is our reading, B).
//!
//! What the notes do not give (E), done here as stated: a key the table lacks, a field the token was not given, a type
//! not built (`currency`, the ordinals, `binding`, `style`, `keyword`: no flight HUD text uses them) and a typed value
//! that is not a number are all left exactly as written. Nesting deeper than `MAX_DEPTH` is left as written too, and the
//! parser stops looking into values nested deeper than that (their `$` is then plain text), so no input can run it out of
//! stack.
//!
//! A text that starts with `&` is literal (B: `ui_messages_panels.md`, the comms log's entries and the game's message
//! record: the engine's "literal text, do not expand" marker, `&` = 0x26 at DAT_144e348f8; the gauge movie puts it before
//! numbers too, `"&" + int(...)` in `DockingSpeed`): the `&` is dropped and the rest is left as it is.

use super::{Text, TimeFlags};

/// How deep keys and values may nest (ours: the table's deepest real nesting is two).
const MAX_DEPTH: usize = 8;

/// One token: where it starts and ends in its text, and what it is.
struct Token<'a> {
    start: usize,
    end: usize,
    kind: Kind<'a>,
}

enum Kind<'a> {
    /// `$KEY;` or `$KEY:#name=value...;`
    Key { key: &'a str, fields: Vec<(&'a str, &'a str)> },
    /// `$#name;` or `$#name:type:param=value...;`
    Field { name: &'a str, ty: Option<&'a str>, params: Vec<(&'a str, &'a str)> },
}

/// A name of letters, digits and underscores at `*i`, moving `*i` past it.
fn ident<'a>(s: &'a str, i: &mut usize) -> Option<&'a str> {
    let start = *i;
    let b = s.as_bytes();
    while *i < b.len() && (b[*i].is_ascii_alphanumeric() || b[*i] == b'_') {
        *i += 1;
    }
    (*i > start).then(|| &s[start..*i])
}

/// The token starting with the `$` at `at`, or None if what follows is not one (the `$` is then plain text).
fn parse_token(s: &str, at: usize, depth: usize) -> Option<Token<'_>> {
    if depth >= MAX_DEPTH {
        return None;
    }
    let b = s.as_bytes();
    let mut i = at + 1;
    if b.get(i) == Some(&b'#') {
        i += 1;
        let name = ident(s, &mut i)?;
        let mut ty = None;
        let mut params = Vec::new();
        if *b.get(i)? == b':' {
            i += 1;
            ty = Some(ident(s, &mut i)?);
            while *b.get(i)? == b':' {
                i += 1;
                let param = ident(s, &mut i)?;
                if *b.get(i)? != b'=' {
                    return None;
                }
                i += 1;
                let start = i;
                while i < b.len() && b[i] != b':' && b[i] != b';' {
                    i += 1;
                }
                params.push((param, &s[start..i]));
            }
        }
        if *b.get(i)? != b';' {
            return None;
        }
        return Some(Token { start: at, end: i + 1, kind: Kind::Field { name, ty, params } });
    }
    let key = ident(s, &mut i)?;
    let mut fields = Vec::new();
    match *b.get(i)? {
        b';' => {}
        b':' if b.get(i + 1) == Some(&b'#') => loop {
            // At ":#": a field's name, '=', then its value up to the next ":#" or ';' outside any nested token.
            i += 2;
            let name = ident(s, &mut i)?;
            if *b.get(i)? != b'=' {
                return None;
            }
            i += 1;
            let start = i;
            loop {
                match *b.get(i)? {
                    b'$' => i = parse_token(s, i, depth + 1).map_or(i + 1, |t| t.end),
                    b';' => break,
                    b':' if b.get(i + 1) == Some(&b'#') => break,
                    _ => i += 1,
                }
            }
            fields.push((name, &s[start..i]));
            if b[i] == b';' {
                break;
            }
        },
        _ => return None,
    }
    Some(Token { start: at, end: i + 1, kind: Kind::Key { key, fields } })
}

/// A parameter's number, or its default when absent or not a number.
fn param(params: &[(&str, &str)], name: &str, default: f64) -> f64 {
    params
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(default)
}

impl Text {
    /// Elite's translation of `s`: every token expanded, the rest unchanged (a text without tokens comes back as it is);
    /// a text starting with `&` comes back literal, without the `&`.
    pub fn translate(&self, s: &str) -> String {
        if let Some(literal) = s.strip_prefix('&') {
            return literal.to_string();
        }
        self.expand(s, &[], 0)
    }

    /// `s` with its tokens expanded, `fields` being the values its `$#field` tokens may use.
    fn expand(&self, s: &str, fields: &[(String, String)], depth: usize) -> String {
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while i < s.len() {
            if s.as_bytes()[i] == b'$'
                && let Some(token) = parse_token(s, i, depth)
            {
                out.push_str(&self.render(&token, s, fields, depth));
                i = token.end;
                continue;
            }
            let ch = s[i..].chars().next().expect("i is on a character");
            out.push(ch);
            i += ch.len_utf8();
        }
        out
    }

    fn render(&self, token: &Token, s: &str, fields: &[(String, String)], depth: usize) -> String {
        let written = &s[token.start..token.end];
        if depth >= MAX_DEPTH {
            return written.to_string();
        }
        match &token.kind {
            Kind::Key { key, fields: given } => {
                let Some(text) = self.lookup(key) else { return written.to_string() };
                let own: Vec<(String, String)> =
                    given.iter().map(|(name, value)| (name.to_string(), self.expand(value, fields, depth + 1))).collect();
                self.expand(text, &own, depth + 1)
            }
            Kind::Field { name, ty, params } => {
                let Some((_, value)) = fields.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) else {
                    return written.to_string();
                };
                match ty {
                    None => value.clone(),
                    Some(ty) => self.typed(ty, value, params).unwrap_or_else(|| written.to_string()),
                }
            }
        }
    }

    /// A typed field's value written by its type, or None if the type is not built or the value is not a number.
    fn typed(&self, ty: &str, value: &str, params: &[(&str, &str)]) -> Option<String> {
        let v: f64 = value.trim().parse().ok()?;
        let on = |name: &str| param(params, name, 0.0) != 0.0;
        match ty.to_ascii_lowercase().as_str() {
            "number" => {
                let decimals = param(params, "decimalPlaces", 0.0).clamp(0.0, 15.0) as u32;
                Some(self.number(v, on("separated"), decimals, param(params, "multiplier", 1.0), on("isDiff")))
            }
            "speed" => Some(self.speed(v)),
            "distance" => Some(self.distance(v, on("compactDistance"))),
            "time" => {
                let flags = TimeFlags {
                    weeks: on("showWeeks"),
                    days: on("showDays"),
                    hours: on("showHours"),
                    minutes: on("showMinutes"),
                    short_minutes: on("showShortMinutes"),
                    seconds: on("showSeconds"),
                    milis: on("showMilis"),
                    units: on("showUnits"),
                    term_limit: param(params, "termLimit", -1.0) as i32,
                };
                Some(self.time(v, &flags))
            }
            _ => None,
        }
    }
}
