//! Trace files and the gate.
//!
//! A trace is a CSV file: one header line of column names, then rows of numbers only (one row a sample, the first
//! column the time in seconds). Measured traces of the real game live in `tests/traces/`, scrubbed: no commander name,
//! no letters outside the header (the spec's privacy rule and C18). The mod writes the same columns (`sw trace FILE`).
//!
//! **The gate** (the user's D10, the spec's C10): for grades C and D a rule passes only inside what the measurement itself
//! can resolve: steady numbers within 1 m/s, 1 degree a second, 1 % heat; timed events within 2 video frames (at 60 a
//! second) or 2 %, whichever is larger. Nothing is ever tuned by feel.

/// Steady speeds: within 1 m/s.
pub const GATE_SPEED_MPS: f64 = 1.0;
/// Steady turn rates: within 1 degree a second.
pub const GATE_TURN_DPS: f64 = 1.0;
/// Heat: within 1 percentage point.
pub const GATE_HEAT_PCT: f64 = 1.0;
/// One frame of the measuring video.
pub const VIDEO_FRAME_S: f64 = 1.0 / 60.0;

/// The tolerance on a timed event lasting `t` seconds: 2 frames or 2 %, whichever is larger.
pub fn gate_time_s(t: f64) -> f64 {
    (2.0 * VIDEO_FRAME_S).max(0.02 * t.abs())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<f64>>,
}

#[derive(Debug, PartialEq)]
pub enum TraceError {
    Empty,
    BadHeader(String),
    /// A row (1-based line number) that isn't all numbers, or has the wrong count.
    BadRow(usize),
}

impl Trace {
    pub fn new(columns: &[&str]) -> Self {
        Trace { columns: columns.iter().map(|c| c.to_string()).collect(), rows: Vec::new() }
    }

    /// Reads a trace. The header's names may hold letters, digits and `_` only; every other line must be numbers.
    pub fn parse(text: &str) -> Result<Trace, TraceError> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let head = lines.next().ok_or(TraceError::Empty)?;
        let columns: Vec<String> = head.split(',').map(|c| c.trim().to_string()).collect();
        if columns.iter().any(|c| c.is_empty() || !c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')) {
            return Err(TraceError::BadHeader(head.to_string()));
        }
        let mut rows = Vec::new();
        for (i, l) in lines.enumerate() {
            // Only digits, signs, points and an exponent's e: no names can hide in a row.
            if !l.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E' | ',' | ' ')) {
                return Err(TraceError::BadRow(i + 2));
            }
            let r: Result<Vec<f64>, _> = l.split(',').map(|v| v.trim().parse::<f64>()).collect();
            match r {
                Ok(r) if r.len() == columns.len() && r.iter().all(|v| v.is_finite()) => rows.push(r),
                _ => return Err(TraceError::BadRow(i + 2)),
            }
        }
        Ok(Trace { columns, rows })
    }

    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }

    /// The values of one column.
    pub fn values(&self, name: &str) -> Option<Vec<f64>> {
        let i = self.column(name)?;
        Some(self.rows.iter().map(|r| r[i]).collect())
    }

    pub fn to_csv(&self) -> String {
        let mut s = self.columns.join(",");
        s.push('\n');
        for r in &self.rows {
            let line: Vec<String> = r.iter().map(|v| format!("{v}")).collect();
            s.push_str(&line.join(","));
            s.push('\n');
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_numbers_are_d10s() {
        assert_eq!((GATE_SPEED_MPS, GATE_TURN_DPS, GATE_HEAT_PCT), (1.0, 1.0, 1.0));
        assert!((gate_time_s(1.0) - 2.0 / 60.0).abs() < 1e-12);
        assert!((gate_time_s(15.0) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn traces_round_trip_and_hold_no_names() {
        let mut t = Trace::new(&["t", "speed", "heat"]);
        t.rows.push(vec![0.0, 0.0, 21.5]);
        t.rows.push(vec![0.016_666, 3.25, -1e-3]);
        let back = Trace::parse(&t.to_csv()).unwrap();
        assert_eq!(back, t);
        assert_eq!(back.values("speed").unwrap(), vec![0.0, 3.25]);
        assert_eq!(Trace::parse("t,speed\n0,1\nCmdr X,2\n"), Err(TraceError::BadRow(3)));
        assert_eq!(Trace::parse("t,speed\n0,1,2\n"), Err(TraceError::BadRow(2)));
        assert!(matches!(Trace::parse("t,Commander name\n"), Err(TraceError::BadHeader(_))));
    }

    /// C18: every measured trace in `tests/traces` reads as numbers only and never names a commander.
    #[test]
    fn measured_traces_are_scrubbed() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("traces");
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("csv") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            assert!(!text.to_lowercase().contains("commander") && !text.to_lowercase().contains("cmdr"), "{p:?} names a commander");
            Trace::parse(&text).unwrap_or_else(|e| panic!("{p:?}: {e:?}"));
        }
    }
}
