//! Parser for SpaceEngine's script syntax, shared by `.sc` catalogs and `.cfg` files.
//!
//! ```text
//! Planet "Earth/Terra"        // a tag, a name, then a block
//! {
//!     Radius   6378.14
//!     RA       10 44 21.26    // several values on one line
//!     Color   ( 0.85 0.85 1 ) // tuple
//!     RotationModel "IAU" { Epoch 2451545 }
//!     PeriodicTerms { 1.0 2.0 3.0 } // bare numbers become anonymous items
//! }
//! ```
//!
//! Statements end at a newline (outside parentheses) unless the next line opens a
//! block, which then belongs to the statement. The parser is lenient: unknown
//! characters are skipped and unbalanced braces are tolerated, mirroring the
//! forgiving behaviour of the original loader.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
    Ident(String),
    Tuple(Vec<Value>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Ident(s) => match s.to_ascii_lowercase().as_str() {
                "true" => Some(1.0),
                "false" => Some(0.0),
                _ => None,
            },
            Value::Str(s) => s.trim().parse().ok(),
            Value::Tuple(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Ident(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Ident(s) | Value::Str(s) => match s.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" => Some(true),
                "false" | "no" | "off" => Some(false),
                _ => None,
            },
            Value::Num(n) => Some(*n != 0.0),
            Value::Tuple(_) => None,
        }
    }

    /// Numeric contents of a tuple (or a single number as a 1-element vector).
    pub fn as_nums(&self) -> Option<Vec<f64>> {
        match self {
            Value::Tuple(v) => v.iter().map(Value::as_f64).collect(),
            Value::Num(n) => Some(vec![*n]),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Num(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "\"{s}\""),
            Value::Ident(s) => write!(f, "{s}"),
            Value::Tuple(v) => {
                write!(f, "(")?;
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{x}")?;
                }
                write!(f, ")")
            }
        }
    }
}

/// One statement: `key arg arg ... [{ block }]`. Anonymous value rows have an empty key.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub key: String,
    pub args: Vec<Value>,
    pub block: Option<Block>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    pub items: Vec<Item>,
}

impl Item {
    /// The first string argument, typically the object name in `Star "Name" { }`.
    pub fn name(&self) -> Option<&str> {
        self.args.iter().find_map(|a| match a {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        })
    }

    /// All `/`-separated names of an object (SE uses `"Sun/Sol"` for aliases).
    pub fn names(&self) -> Vec<String> {
        self.name()
            .map(|n| n.split('/').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default()
    }

    pub fn f64(&self) -> Option<f64> {
        self.args.first().and_then(Value::as_f64)
    }

    /// All numeric arguments, flattening tuples (`RA 10 44 21.26` -> `[10, 44, 21.26]`).
    pub fn nums(&self) -> Vec<f64> {
        let mut out = Vec::new();
        for a in &self.args {
            match a {
                Value::Tuple(t) => out.extend(t.iter().filter_map(Value::as_f64)),
                other => {
                    if let Some(n) = other.as_f64() {
                        out.push(n)
                    }
                }
            }
        }
        out
    }

    pub fn str(&self) -> Option<&str> {
        self.args.first().and_then(Value::as_str)
    }

    pub fn bool(&self) -> Option<bool> {
        match self.args.first() {
            Some(v) => v.as_bool(),
            // A bare flag like `NoRings` with no value means true.
            None => Some(true),
        }
    }

    pub fn block(&self) -> Option<&Block> {
        self.block.as_ref()
    }
}

impl Block {
    /// First item with this key (case-insensitive). SE treats later duplicates as
    /// overrides in some places, so see also [`Block::get_last`].
    pub fn get(&self, key: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.key.eq_ignore_ascii_case(key))
    }

    pub fn get_last(&self, key: &str) -> Option<&Item> {
        self.items.iter().rev().find(|i| i.key.eq_ignore_ascii_case(key))
    }

    pub fn all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Item> + 'a {
        self.items.iter().filter(move |i| i.key.eq_ignore_ascii_case(key))
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get_last(key).and_then(Item::f64)
    }

    /// First present key among alternatives (e.g. `Teff` / `Temperature`).
    pub fn f64_any(&self, keys: &[&str]) -> Option<f64> {
        keys.iter().find_map(|k| self.f64(k))
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.get_last(key).and_then(Item::str)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.get_last(key).and_then(Item::bool)
    }

    pub fn nums(&self, key: &str) -> Option<Vec<f64>> {
        self.get_last(key).map(Item::nums)
    }

    pub fn vec3(&self, key: &str) -> Option<[f64; 3]> {
        let n = self.nums(key)?;
        (n.len() >= 3).then(|| [n[0], n[1], n[2]])
    }

    pub fn vec4(&self, key: &str) -> Option<[f64; 4]> {
        let n = self.nums(key)?;
        (n.len() >= 4).then(|| [n[0], n[1], n[2], n[3]])
    }

    /// Sexagesimal value (`10 44 21.26` or `-61 12 35.4`) or plain decimal.
    /// A leading minus sign applies to the whole value, including `-0 30 00`.
    pub fn sexagesimal(&self, key: &str) -> Option<f64> {
        let item = self.get_last(key)?;
        let nums = item.nums();
        let first = *nums.first()?;
        let negative = first.is_sign_negative();
        let mut v = first.abs();
        if let Some(m) = nums.get(1) {
            v += m / 60.0;
        }
        if let Some(s) = nums.get(2) {
            v += s / 3600.0;
        }
        Some(if negative { -v } else { v })
    }

    pub fn sub(&self, key: &str) -> Option<&Block> {
        self.get_last(key).and_then(|i| i.block.as_ref())
    }

    /// The first item named `key` here or in any nested block, for settings files whose keys sit in
    /// groups such as `PointRendering { }` (`config/main-def.cfg`).
    pub fn find_deep(&self, key: &str) -> Option<&Item> {
        self.items
            .iter()
            .find(|i| i.key == key)
            .or_else(|| self.items.iter().filter_map(|i| i.block.as_ref()).find_map(|b| b.find_deep(key)))
    }

    /// Anonymous numeric rows of a block (e.g. periodic term tables), flattened.
    pub fn anonymous_numbers(&self) -> Vec<f64> {
        self.items
            .iter()
            .filter(|i| i.key.is_empty())
            .flat_map(|i| i.nums())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Num(f64),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Newline,
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
}

impl<'a> Lexer<'a> {
    fn tokenize(src: &'a str) -> Vec<(Tok, u32)> {
        let mut lx = Lexer {
            src: src.as_bytes(),
            pos: 0,
            line: 1,
        };
        let mut out = Vec::new();
        while let Some(t) = lx.next() {
            out.push(t);
        }
        out
    }

    fn peek(&self, off: usize) -> u8 {
        *self.src.get(self.pos + off).unwrap_or(&0)
    }

    fn next(&mut self) -> Option<(Tok, u32)> {
        loop {
            let c = self.peek(0);
            if self.pos >= self.src.len() {
                return None;
            }
            match c {
                b'\n' => {
                    self.pos += 1;
                    let line = self.line;
                    self.line += 1;
                    return Some((Tok::Newline, line));
                }
                b' ' | b'\t' | b'\r' | b',' | b';' => self.pos += 1,
                b'/' if self.peek(1) == b'/' => {
                    while self.pos < self.src.len() && self.peek(0) != b'\n' {
                        self.pos += 1;
                    }
                }
                b'/' if self.peek(1) == b'*' => {
                    self.pos += 2;
                    while self.pos < self.src.len() && !(self.peek(0) == b'*' && self.peek(1) == b'/') {
                        if self.peek(0) == b'\n' {
                            self.line += 1;
                        }
                        self.pos += 1;
                    }
                    self.pos = (self.pos + 2).min(self.src.len());
                }
                b'"' => return Some((self.string(), self.line)),
                b'(' => return self.single(Tok::LParen),
                b')' => return self.single(Tok::RParen),
                b'{' => return self.single(Tok::LBrace),
                b'}' => return self.single(Tok::RBrace),
                b'0'..=b'9' | b'.' => return Some((self.number_or_word(), self.line)),
                b'+' | b'-' if matches!(self.peek(1), b'0'..=b'9' | b'.') => {
                    return Some((self.number_or_word(), self.line));
                }
                c if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 || c == b'#' || c == b'$' => {
                    return Some((self.word(), self.line));
                }
                _ => self.pos += 1,
            }
        }
    }

    fn single(&mut self, t: Tok) -> Option<(Tok, u32)> {
        self.pos += 1;
        Some((t, self.line))
    }

    fn string(&mut self) -> Tok {
        self.pos += 1;
        let mut bytes = Vec::new();
        while self.pos < self.src.len() {
            let c = self.peek(0);
            if c == b'"' {
                self.pos += 1;
                break;
            }
            if c == b'\\' && self.peek(1) == b'"' {
                bytes.push(b'"');
                self.pos += 2;
                continue;
            }
            if c == b'\n' {
                // Unterminated string: stop at end of line.
                break;
            }
            bytes.push(c);
            self.pos += 1;
        }
        Tok::Str(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn word(&mut self) -> Tok {
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.peek(0);
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-' | b'#' | b'$' | b'\'') || c >= 0x80 {
                self.pos += 1;
            } else {
                break;
            }
        }
        Tok::Word(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
    }

    fn number_or_word(&mut self) -> Tok {
        let start = self.pos;
        if matches!(self.peek(0), b'+' | b'-') {
            self.pos += 1;
        }
        while self.pos < self.src.len() {
            let c = self.peek(0);
            let exp_sign = matches!(c, b'+' | b'-') && matches!(self.src[self.pos - 1], b'e' | b'E');
            if c.is_ascii_digit() || c == b'.' || c == b'e' || c == b'E' || exp_sign {
                self.pos += 1;
            } else {
                break;
            }
        }
        // Words that start with a digit (e.g. `2MASS`) continue as words.
        if self.pos < self.src.len() && (self.peek(0).is_ascii_alphabetic() || self.peek(0) == b'_') {
            while self.pos < self.src.len()
                && (self.peek(0).is_ascii_alphanumeric() || matches!(self.peek(0), b'_' | b'-' | b'.'))
            {
                self.pos += 1;
            }
            return Tok::Word(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned());
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("0");
        match text.parse::<f64>() {
            Ok(n) => Tok::Num(n),
            Err(_) => {
                // Things like "1.2.3": keep the longest numeric prefix.
                let mut n = 0.0;
                for end in (1..=text.len()).rev() {
                    if let Ok(v) = text[..end].parse::<f64>() {
                        n = v;
                        break;
                    }
                }
                Tok::Num(n)
            }
        }
    }
}

struct Parser {
    toks: Vec<(Tok, u32)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }

    fn line(&self) -> u32 {
        self.toks.get(self.pos).map(|t| t.1).unwrap_or(0)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|t| t.0.clone());
        self.pos += 1;
        t
    }

    fn skip_newlines(&mut self) {
        while self.peek() == Some(&Tok::Newline) {
            self.pos += 1;
        }
    }

    /// Parse items until `}` (consumed) or end of input.
    fn block(&mut self) -> Block {
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                None => break,
                Some(Tok::RBrace) => {
                    self.pos += 1;
                    break;
                }
                Some(Tok::LBrace) => {
                    // Stray anonymous block: parse and attach as an anonymous item.
                    let line = self.line();
                    self.pos += 1;
                    let b = self.block();
                    items.push(Item {
                        key: String::new(),
                        args: Vec::new(),
                        block: Some(b),
                        line,
                    });
                }
                Some(Tok::RParen) => self.pos += 1,
                Some(_) => items.push(self.item()),
            }
        }
        Block { items }
    }

    fn item(&mut self) -> Item {
        let line = self.line();
        let key = match self.peek() {
            Some(Tok::Word(w)) => {
                let w = w.clone();
                self.pos += 1;
                w
            }
            _ => String::new(),
        };
        let mut item = Item {
            key,
            args: Vec::new(),
            block: None,
            line,
        };
        loop {
            match self.peek() {
                None | Some(Tok::RBrace) => return item,
                Some(Tok::Newline) => {
                    // A block on the following line still belongs to this item.
                    let save = self.pos;
                    self.skip_newlines();
                    if self.peek() == Some(&Tok::LBrace) && !item.key.is_empty() {
                        continue;
                    }
                    self.pos = save;
                    return item;
                }
                Some(Tok::LBrace) => {
                    self.pos += 1;
                    item.block = Some(self.block());
                    return item;
                }
                Some(Tok::LParen) => {
                    self.pos += 1;
                    item.args.push(self.tuple());
                }
                Some(Tok::RParen) => self.pos += 1,
                // Single-line blocks (`{ Type "E0" Dist 1.5e6 }`): a capitalised word
                // after at least one value starts the next statement.
                Some(Tok::Word(w))
                    if !item.key.is_empty()
                        && !item.args.is_empty()
                        && w.starts_with(|c: char| c.is_ascii_uppercase())
                        && !matches!(w.as_str(), "True" | "False" | "TRUE" | "FALSE") =>
                {
                    return item;
                }
                Some(_) => match self.bump() {
                    Some(Tok::Num(n)) => item.args.push(Value::Num(n)),
                    Some(Tok::Str(s)) => item.args.push(Value::Str(s)),
                    Some(Tok::Word(w)) => item.args.push(Value::Ident(w)),
                    _ => {}
                },
            }
        }
    }

    fn tuple(&mut self) -> Value {
        let mut v = Vec::new();
        loop {
            match self.bump() {
                None | Some(Tok::RParen) => break,
                Some(Tok::Num(n)) => v.push(Value::Num(n)),
                Some(Tok::Str(s)) => v.push(Value::Str(s)),
                Some(Tok::Word(w)) => v.push(Value::Ident(w)),
                Some(Tok::LParen) => v.push(self.tuple()),
                Some(Tok::Newline) => {}
                Some(Tok::LBrace) | Some(Tok::RBrace) => {
                    // Malformed: put the brace back and end the tuple.
                    self.pos -= 1;
                    break;
                }
            }
        }
        Value::Tuple(v)
    }
}

/// Parse a whole document into its top-level block.
pub fn parse(src: &str) -> Block {
    let mut p = Parser {
        toks: Lexer::tokenize(src),
        pos: 0,
    };
    let mut items = Vec::new();
    loop {
        let b = p.block();
        items.extend(b.items);
        // A stray `}` at top level ends `block()` early; keep going.
        if p.peek().is_none() {
            break;
        }
    }
    Block { items }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
LogLevel    0
OctreeSize  ( 300 300 300 )
Star	"Gliese 3618/GJ 3618"
{
	RA          10 44 21.26
	Dec        -61 12 35.4
	Dist        4.83512
	Class      "M5 V"   // comment
	/* block
	   comment */
	TidalLocked false
	Color ( 0.850, 0.850, 1.000 )
	RotationModel "IAU"
	{
		Epoch 2451545
		PeriodicTermsDiurnal
		{
			1.0 2.0 3.0
			4e-1 -5 +6
		}
	}
	NoRings
}
Galaxy "2MASX J1" { Type "E0" Dist 1.5e6 }
"#;

    #[test]
    fn parses_structure() {
        let doc = parse(SAMPLE);
        assert_eq!(doc.f64("LogLevel"), Some(0.0));
        assert_eq!(doc.vec3("OctreeSize"), Some([300.0, 300.0, 300.0]));
        let star = doc.get("Star").unwrap();
        assert_eq!(star.names(), vec!["Gliese 3618", "GJ 3618"]);
        let b = star.block().unwrap();
        let ra = b.sexagesimal("RA").unwrap();
        assert!((ra - (10.0 + 44.0 / 60.0 + 21.26 / 3600.0)).abs() < 1e-12);
        let dec = b.sexagesimal("Dec").unwrap();
        assert!((dec + (61.0 + 12.0 / 60.0 + 35.4 / 3600.0)).abs() < 1e-12);
        assert_eq!(b.str("Class"), Some("M5 V"));
        assert_eq!(b.bool("TidalLocked"), Some(false));
        assert_eq!(b.bool("NoRings"), Some(true));
        assert_eq!(b.vec3("Color"), Some([0.85, 0.85, 1.0]));
        let rot = b.get("RotationModel").unwrap();
        assert_eq!(rot.str(), Some("IAU"));
        let terms = rot.block().unwrap().sub("PeriodicTermsDiurnal").unwrap();
        assert_eq!(terms.anonymous_numbers(), vec![1.0, 2.0, 3.0, 0.4, -5.0, 6.0]);
        let gal = doc.get("Galaxy").unwrap();
        assert_eq!(gal.name(), Some("2MASX J1"));
        assert_eq!(gal.block().unwrap().f64("Dist"), Some(1.5e6));
    }

    #[test]
    fn negative_zero_degrees() {
        let doc = parse("Dec -0 30 00\n");
        assert!((doc.sexagesimal("Dec").unwrap() + 0.5).abs() < 1e-12);
    }

    #[test]
    fn words_starting_with_digits() {
        let doc = parse("Name 2MASS\nX 1e5\n");
        assert_eq!(doc.str("Name"), Some("2MASS"));
        assert_eq!(doc.f64("X"), Some(1e5));
    }

    #[test]
    fn stray_braces_are_tolerated() {
        let doc = parse("A 1\n}\nB 2\n");
        assert_eq!(doc.f64("A"), Some(1.0));
        assert_eq!(doc.f64("B"), Some(2.0));
    }
}
