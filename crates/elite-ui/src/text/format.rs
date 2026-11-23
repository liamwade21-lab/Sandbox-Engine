//! Elite's number, speed, distance and time formats (`re/elite/notes/ui_messages_hud.md` section 8, the types registered
//! by `FUN_1444e70a0`). Every symbol they write (separators, signs, unit texts) is the string table's, never ours.

use super::Text;

/// Light's speed (m/s), the speed and distance ladders' divisor (A: section 8).
pub const LIGHT_M_S: f64 = 299_792_458.0;
/// Where the ladders leave mega-units: a tenth of a light second, c x 0.1 s (A: section 8).
const MEGA_LIMIT: f64 = 29_979_245.8;
/// The game's light year in metres (A: `ui_messages_panels.md` 6.4, the distance formatter `FUN_144509790`).
const LIGHT_YEAR_M: f64 = 9_221_136_340_147_200.0;
/// The distance ladder's light years start at a tenth of one (A: the same table, 9.2211363e14 m).
const LIGHT_YEAR_LIMIT: f64 = LIGHT_YEAR_M / 10.0;
/// `compactDistance=1` writes kilo-light-seconds above this (A: the same table, "v > 2.99792e11").
const KILO_LIGHT_SECOND_FROM: f64 = 2.99792e11;

/// The `time` type's parameters (section 8: `showWeeks`, `showDays`, `showHours`, `showMinutes`, `showShortMinutes`,
/// `showSeconds`, `showMilis`, `showUnits`, all 0 by default, and `termLimit`, -1 by default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeFlags {
    pub weeks: bool,
    pub days: bool,
    pub hours: bool,
    pub minutes: bool,
    /// `showShortMinutes`: read but not used, its effect was not decoded (E).
    pub short_minutes: bool,
    pub seconds: bool,
    pub milis: bool,
    pub units: bool,
    /// At most this many parts are written; negative: no limit.
    pub term_limit: i32,
}

impl Default for TimeFlags {
    fn default() -> Self {
        TimeFlags {
            weeks: false,
            days: false,
            hours: false,
            minutes: false,
            short_minutes: false,
            seconds: false,
            milis: false,
            units: false,
            term_limit: -1,
        }
    }
}

/// The decimals a scaled value gets (A: section 8 and the distance formatter's end, constants 10.0 and 100.0): 0 from 100,
/// 1 from 10, else 2.
fn scaled_decimals(s: f64) -> u32 {
    if s >= 100.0 {
        0
    } else if s >= 10.0 {
        1
    } else {
        2
    }
}

/// Digits of a whole number with `sep` between each three from the right.
fn grouped(int: i64, sep: Option<&str>) -> String {
    let digits = int.to_string();
    let Some(sep) = sep else { return digits };
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 * sep.len());
    for (k, ch) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push_str(sep);
        }
        out.push(ch);
    }
    out
}

impl Text {
    /// The `number` type (A: `FUN_144509ce0` -> `FUN_14450d830`): v = value x multiplier; a negative v gets `minus_prefix`
    /// and is written by its size, `isDiff` gives a positive one `plus_prefix`; t = |v| + 0.5 x 0.1^decimals (half up on the
    /// size, in doubles); the whole part floor(t) as a 64-bit integer, `block_separator` every three digits when
    /// `separated`; with decimals, `decimal_separator` and that many digits, each floor(fraction x 10) of what is left; then
    /// `minus_suffix` / `plus_suffix` (empty in English). 0.1^decimals is taken by repeated products, as the exe's
    /// constants (0.1, 0.5, 10.0) suggest (B).
    pub fn number(&self, value: f64, separated: bool, decimal_places: u32, multiplier: f64, is_diff: bool) -> String {
        let v = value * multiplier;
        let negative = v < 0.0;
        let mut out = String::new();
        if negative {
            out.push_str(self.symbol("minus_prefix"));
        } else if is_diff {
            out.push_str(self.symbol("plus_prefix"));
        }
        let half_step = (0..decimal_places).fold(0.5, |h, _| h * 0.1);
        let t = v.abs() + half_step;
        let whole = t.floor();
        out.push_str(&grouped(whole as i64, separated.then(|| self.symbol("block_separator"))));
        if decimal_places > 0 {
            out.push_str(self.symbol("decimal_separator"));
            let mut rest = t - whole;
            for _ in 0..decimal_places {
                rest *= 10.0;
                let digit = rest.floor().clamp(0.0, 9.0);
                out.push(char::from(b'0' + digit as u8));
                rest -= digit;
            }
        }
        if negative {
            out.push_str(self.symbol("minus_suffix"));
        } else if is_diff {
            out.push_str(self.symbol("plus_suffix"));
        }
        out
    }

    /// A scaled value with the ladders' decimals, grouped, and its unit text appended with no space between (B: section 8
    /// does not show a space; the notes' samples read "2.50km/s").
    fn scaled(&self, s: f64, decimals: u32, unit_key: &str) -> String {
        let mut out = self.number(s, true, decimals, 1.0, false);
        out.push_str(self.symbol(unit_key));
        out
    }

    /// The `speed` type (A: `FUN_14450a8a0`): under 1,000 m/s in `meters_per_second` with no decimals; to 1e6 m/s in
    /// `kilometers_per_second`; to a tenth of light's speed in `megameters_per_second`; beyond in `speed_of_light`; the
    /// scaled value's decimals by `scaled_decimals`, grouped.
    pub fn speed(&self, m_per_s: f64) -> String {
        let v = m_per_s;
        if v < 1000.0 {
            self.scaled(v, 0, "meters_per_second")
        } else if v < 1.0e6 {
            let s = v / 1000.0;
            self.scaled(s, scaled_decimals(s), "kilometers_per_second")
        } else if v < MEGA_LIMIT {
            let s = v / 1.0e6;
            self.scaled(s, scaled_decimals(s), "megameters_per_second")
        } else {
            let s = v / LIGHT_M_S;
            self.scaled(s, scaled_decimals(s), "speed_of_light")
        }
    }

    /// The `distance` type (A: `FUN_144509790`, the table in `ui_messages_panels.md` 6.4): under 1,000 m in
    /// `meters_symbol`, no decimals; to 1e6 m in `kilometers_symbol`; to a tenth of a light second in `megameters_symbol`;
    /// to a tenth of a light year in `light_second_symbol` (with `compact`, the `compactDistance=1` parameter, above
    /// 2.99792e11 m in `kilo_light_second_symbol`); beyond in `light_year_symbol`; decimals by `scaled_decimals`. Grouped
    /// as the speed is (B: "the same ladder").
    pub fn distance(&self, m: f64, compact: bool) -> String {
        let v = m;
        if v < 1000.0 {
            self.scaled(v, 0, "meters_symbol")
        } else if v < 1.0e6 {
            let s = v / 1000.0;
            self.scaled(s, scaled_decimals(s), "kilometers_symbol")
        } else if v < MEGA_LIMIT {
            let s = v / 1.0e6;
            self.scaled(s, scaled_decimals(s), "megameters_symbol")
        } else if v < LIGHT_YEAR_LIMIT {
            if compact && v > KILO_LIGHT_SECOND_FROM {
                let s = v / (LIGHT_M_S * 1000.0);
                self.scaled(s, scaled_decimals(s), "kilo_light_second_symbol")
            } else {
                let s = v / LIGHT_M_S;
                self.scaled(s, scaled_decimals(s), "light_second_symbol")
            }
        } else {
            let s = v / LIGHT_YEAR_M;
            self.scaled(s, scaled_decimals(s), "light_year_symbol")
        }
    }

    /// The `time` type (A: `FUN_14450b440` -> `FUN_14450e360`): the seconds split into weeks (604,800), days (86,400),
    /// hours (3,600), minutes (60) and seconds, each part shown in that order, joined by `time_separator`; once a part is
    /// written the later ones are zero-padded to two digits; `units` appends `week_symbol`, `day_symbol`, `hours_symbol`,
    /// `minutes_symbol`, `seconds_symbol`; `term_limit` stops after that many parts. Readings of ours where the notes are
    /// silent (B): the first shown part takes all the larger units that are not shown (125 s with minutes and seconds is
    /// "2:05"); the first part is not padded ("1:02:05", the notes' B); milliseconds are the fraction's whole thousandths,
    /// padded as the other parts and with no unit; negative seconds are written as 0.
    pub fn time(&self, seconds: f64, flags: &TimeFlags) -> String {
        let total = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        let mut rest = total.floor() as u64;
        let millis = ((total - total.floor()) * 1000.0).floor() as u64;
        let parts = [
            (flags.weeks, 604_800u64, "week_symbol"),
            (flags.days, 86_400, "day_symbol"),
            (flags.hours, 3_600, "hours_symbol"),
            (flags.minutes, 60, "minutes_symbol"),
            (flags.seconds, 1, "seconds_symbol"),
        ];
        let mut out = String::new();
        let mut written = 0i32;
        let mut put = |out: &mut String, n: u64, unit: Option<&str>| {
            if flags.term_limit >= 0 && written >= flags.term_limit {
                return;
            }
            if written > 0 {
                out.push_str(self.symbol("time_separator"));
                if n < 10 {
                    out.push('0');
                }
            }
            out.push_str(&n.to_string());
            if let Some(unit) = unit {
                out.push_str(unit);
            }
            written += 1;
        };
        for (shown, unit_s, symbol) in parts {
            if !shown {
                continue;
            }
            let n = rest / unit_s;
            rest -= n * unit_s;
            put(&mut out, n, flags.units.then(|| self.symbol(symbol)));
        }
        if flags.milis {
            put(&mut out, millis, None);
        }
        out
    }

    /// The manager's `LocaliseNumber(number, format, extra)` (`CommunicationManager.as`: a Number, a String and an optional
    /// String in, a String out). Elite's native one was not read and no movie in scope calls it, so this is the decoded
    /// manager's own rule (stand-in, E for Elite's): 0 is "0"; else the whole part floor(number) written as ActionScript
    /// writes it, a "," before each three digits from the right; then, if the number has a fraction, "." and the first
    /// two characters after the point of the fraction (number % 1) as ActionScript writes it. `format` and `extra` are
    /// unused, as there.
    pub fn localise_number(&self, number: f64, _format: &str, _extra: Option<&str>) -> String {
        if number == 0.0 {
            return "0".to_string();
        }
        let whole = number.floor().to_string();
        let len = whole.len() as i64;
        let groups = (len + 2) / 3;
        let mut out = String::new();
        for k in 0..groups {
            let mut start = len - (k + 1) * 3;
            let mut take = 3;
            if start < 0 {
                take += start;
                start = 0;
            }
            let piece = &whole[start as usize..(start + take) as usize];
            out = if out.is_empty() { piece.to_string() } else { format!("{piece},{out}") };
        }
        let fraction = number % 1.0;
        if fraction != 0.0 {
            let text = fraction.to_string();
            let after = text.find('.').map_or(0, |p| p + 1);
            out.push('.');
            out.push_str(&text[after..(after + 2).min(text.len())]);
        }
        out
    }
}
