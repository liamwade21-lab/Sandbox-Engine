//! Parser for SpaceEngine `.se` scripts (the in-game scripting language used by
//! `scripts/menu/Scene1.se`, the tutorials and user scripts).
//!
//! The syntax is the same as the `.sc` catalog syntax (see [`crate::sc`]): one
//! statement per line, `Command arg arg ...`, optionally followed by a `{ ... }`
//! block of `Key value` rows (the block may start on the next line). There are no
//! expressions; `if` takes a one-comparison block. See `docs/formats/se_script.md`.
//!
//! A script parses into a flat [`Script`] of [`Command`]s. Structured control flow
//! (`if/elif/else/endif`, `loop/endloop`) is resolved into jump targets so an
//! interpreter only needs a program counter.

use crate::places::{fixed_to_f64, parse_fixed, PC_M};
use crate::sc::{self, Block, Item, Value};
use crate::time::DateTime;

/// A parsed script.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    pub commands: Vec<Command>,
    /// Problems found while parsing (unbalanced `if`, malformed arguments, ...).
    pub warnings: Vec<String>,
}

/// One statement with its source line (1-based).
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub line: u32,
    pub op: Op,
}

/// Comparison used by `if` / `elif`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    /// No operator: the variable itself is tested for non-zero.
    Truthy,
}

/// `Var <op> value` condition of `if` / `elif`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cond {
    pub var: String,
    pub op: CmpOp,
    pub rhs: Value,
}

impl Cond {
    /// Evaluates the condition given the variable's current numeric value.
    pub fn eval(&self, lhs: f64) -> bool {
        let rhs = self.rhs.as_f64().unwrap_or(0.0);
        match self.op {
            CmpOp::Lt => lhs < rhs,
            CmpOp::Le => lhs <= rhs,
            CmpOp::Gt => lhs > rhs,
            CmpOp::Ge => lhs >= rhs,
            CmpOp::Eq => (lhs - rhs).abs() < 1e-9,
            CmpOp::Ne => (lhs - rhs).abs() >= 1e-9,
            CmpOp::Truthy => lhs != 0.0,
        }
    }
}

/// `AND` / `OR` of a `BeginMultiTrigger` group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiMode {
    And,
    Or,
}

/// Camera placement of a `GotoLocation` block (same fields as a saved Place).
#[derive(Debug, Clone, PartialEq)]
pub struct Location {
    pub body: String,
    pub parent: String,
    /// Julian Date, from `Time` (exact, preferred) or else from `Date`.
    pub jd: Option<f64>,
    /// Camera position relative to the Sun's centre, metres, universe frame.
    pub pos_m: [f64; 3],
    /// Camera orientation quaternion, order x y z w (the file order is w x y z).
    pub rot: [f64; 4],
    /// Speed in parsecs per second.
    pub vel: f64,
    pub stereobase: f64,
    /// 0 free, 1 follow the body, 2 also rotate with it.
    pub bind_mode: i32,
    pub photo_mode: i32,
    pub exposure: f64,
}

/// Arguments of the `Goto { ... }` block: fly to the selected object (or `Object`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GotoArgs {
    pub object: Option<String>,
    /// Height above the surface, km (`Height` / `HeightKm`).
    pub height_km: Option<f64>,
    /// Distance from the centre, km (`DistKm`).
    pub dist_km: Option<f64>,
    /// Distance from the centre in body radii (`DistRad`).
    pub dist_rad: Option<f64>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub yaw: Option<f64>,
    pub pitch: Option<f64>,
    pub roll: Option<f64>,
    /// Flight duration, seconds.
    pub time: Option<f64>,
    /// Finish looking at the object's centre.
    pub center: bool,
}

/// What a `WaitTrigger "Kind|Name" { ... }` waits for.
#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    /// As written, e.g. `Object|Approach`.
    pub kind: String,
    pub object: Option<String>,
    /// `Type` filter (`Planet`, `Galaxy`, `Selection`, ...).
    pub ty: Option<String>,
    pub mode: Option<String>,
    pub dialog: Option<String>,
    /// `Range` in the trigger's natural unit (pc/s for speed, rate for time rate, ...).
    pub range: Option<(f64, f64)>,
    pub range_km: Option<(f64, f64)>,
    /// Distance to the object in object radii.
    pub range_rad: Option<(f64, f64)>,
    pub yaw_range: Option<(f64, f64)>,
    pub pitch_range: Option<(f64, f64)>,
    pub roll_range: Option<(f64, f64)>,
    /// Screen rectangle `(x0 x1 y0 y1)` in 0..1 window coordinates.
    pub rect: Option<[f64; 4]>,
    pub max_wait_time: Option<f64>,
    /// `Time "YYYY.MM.DD hh:mm:ss"` of a `Time` trigger, as Julian Date.
    pub time_jd: Option<f64>,
    pub raw: Block,
}

/// Condition of `WaitVar`.
#[derive(Debug, Clone, PartialEq)]
pub enum VarWait {
    Equals(Value),
    Range(f64, f64),
}

/// One executable operation.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    CheckVersion(u32),
    /// Run another script (path relative to `scripts/`).
    Run(String),
    Quit,
    Restart,

    Select(String),
    Unselect,
    Center { time: Option<f64> },
    Follow,
    SyncRot,
    Free,
    Track,
    Untrack,
    GotoLocation(Box<Location>),
    Goto(GotoArgs),
    /// `FOV 45`, `FOV 45 { Time 2 }`, or `FOV { Time 0 }` (reset).
    Fov { fov: Option<f64>, time: Option<f64> },
    Orbit { axis: [f64; 3], speed: f64 },
    StopOrbit,
    StopMotion,

    TimeScale(f64),
    /// `Date "2019.05.31 18:23:57.57"` / `Time +hex`: set the simulation date (JD).
    SetDate(f64),
    StopTime,
    StartTime,

    Wait(f64),
    WaitTrigger(Box<Trigger>),
    BeginMultiTrigger(MultiMode),
    EndMultiTrigger,

    ShowMessage(String),
    WaitMessage(String),
    HideMessage,
    Print { text: String, time: Option<f64> },
    HidePrint,
    Log(String),

    /// `Set Name value`, `SetU`, or `SetForce` (`force`).
    Set { name: String, value: Value, force: bool },
    WaitVar { name: String, wait: VarWait },
    SaveVars,
    RestoreVars,
    Interpolate { name: String, to: f64, time: f64 },

    Waypoint { name: String, block: Block },
    ClearWaypoints,
    DeleteWaypoint(String),

    /// `on_false`: command index to continue at when the condition fails (the next
    /// `Elif`, the statement after `Else`, or the `EndIf`).
    If { cond: Cond, on_false: usize },
    Elif { cond: Cond, on_false: usize },
    Else,
    EndIf,
    /// Inserted before `Elif`/`Else` so a finished branch skips to `EndIf`.
    Jump(usize),
    /// `end` is the index of the matching `EndLoop`.
    Loop { count: Option<u32>, end: usize },
    EndLoop { start: usize },
    /// Leaves the innermost `Loop`; `end` is the index of its `EndLoop`.
    Break { end: usize },

    /// Any command the parser has no typed form for (GUI and sound commands,
    /// `Screenshot`, `Horizon`, ...). The key is as written.
    Other(Item),
}

impl Op {
    /// Lower-case command name for logs and "unsupported" reports.
    pub fn name(&self) -> String {
        match self {
            Op::Other(i) => i.key.to_ascii_lowercase(),
            Op::Set { force: true, .. } => "setforce".into(),
            other => {
                let dbg = format!("{other:?}");
                dbg.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("").to_ascii_lowercase()
            }
        }
    }
}

/// Parses `.se` script text.
pub fn parse(src: &str) -> Script {
    let pre = preprocess_conditions(src);
    let root = sc::parse(&pre);
    let mut b = Builder::default();
    for item in &root.items {
        b.push_item(item);
    }
    b.finish()
}

/// `if { A < 7 }`: the `.sc` lexer drops comparison characters, so quote them.
fn preprocess_conditions(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 16);
    for raw in src.split_inclusive('\n') {
        let t = raw.trim_start().to_ascii_lowercase();
        let is_cond = (t.starts_with("if") || t.starts_with("elif"))
            && t.trim_start_matches("elif").trim_start_matches("if").trim_start().starts_with('{');
        if !is_cond {
            out.push_str(raw);
            continue;
        }
        let b = raw.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            let two = if i + 1 < b.len() { &raw[i..i + 2] } else { "" };
            if matches!(two, "<=" | ">=" | "==" | "!=") {
                out.push_str(&format!(" \"{two}\" "));
                i += 2;
            } else if matches!(c, b'<' | b'>' | b'=') {
                out.push_str(&format!(" \"{}\" ", c as char));
                i += 1;
            } else {
                // Copy whole UTF-8 characters.
                let ch = raw[i..].chars().next().unwrap_or(' ');
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

#[derive(Default)]
struct Builder {
    ops: Vec<Command>,
    warnings: Vec<String>,
    ifs: Vec<IfFrame>,
    loops: Vec<LoopFrame>,
}

struct IfFrame {
    /// Index of the `If`/`Elif` whose `on_false` is still open.
    pending: Option<usize>,
    /// `Jump` ops waiting for the `EndIf` index.
    jumps: Vec<usize>,
    line: u32,
}

struct LoopFrame {
    start: usize,
    breaks: Vec<usize>,
}

impl Builder {
    fn warn(&mut self, line: u32, msg: impl Into<String>) {
        self.warnings.push(format!("line {line}: {}", msg.into()));
    }

    fn push(&mut self, line: u32, op: Op) -> usize {
        self.ops.push(Command { line, op });
        self.ops.len() - 1
    }

    fn set_on_false(&mut self, at: usize, target: usize) {
        match &mut self.ops[at].op {
            Op::If { on_false, .. } | Op::Elif { on_false, .. } => *on_false = target,
            _ => {}
        }
    }

    fn push_item(&mut self, item: &Item) {
        let line = item.line;
        let key = item.key.to_ascii_lowercase();
        match key.as_str() {
            "if" => {
                let cond = self.cond(item);
                let idx = self.push(line, Op::If { cond, on_false: 0 });
                self.ifs.push(IfFrame { pending: Some(idx), jumps: Vec::new(), line });
            }
            "elif" => {
                let cond = self.cond(item);
                if self.ifs.is_empty() {
                    self.warn(line, "elif without if");
                    return;
                }
                let j = self.push(line, Op::Jump(0));
                let idx = self.push(line, Op::Elif { cond, on_false: 0 });
                let f = self.ifs.last_mut().unwrap();
                let pending = f.pending.replace(idx);
                f.jumps.push(j);
                if let Some(p) = pending {
                    self.set_on_false(p, idx);
                }
            }
            "else" => {
                if self.ifs.is_empty() {
                    self.warn(line, "else without if");
                    return;
                }
                let j = self.push(line, Op::Jump(0));
                let idx = self.push(line, Op::Else);
                let f = self.ifs.last_mut().unwrap();
                let pending = f.pending.take();
                f.jumps.push(j);
                if let Some(p) = pending {
                    // Skip the `Else` marker itself.
                    self.set_on_false(p, idx + 1);
                }
            }
            "endif" => {
                let idx = self.push(line, Op::EndIf);
                match self.ifs.pop() {
                    Some(f) => {
                        if let Some(p) = f.pending {
                            self.set_on_false(p, idx);
                        }
                        for j in f.jumps {
                            self.ops[j].op = Op::Jump(idx);
                        }
                    }
                    None => self.warn(line, "endif without if"),
                }
            }
            "loop" => {
                let count = item
                    .f64()
                    .or_else(|| item.block().and_then(|b| b.f64_any(&["Count", "Times", "N"])))
                    .map(|n| n.max(0.0) as u32);
                let idx = self.push(line, Op::Loop { count, end: 0 });
                self.loops.push(LoopFrame { start: idx, breaks: Vec::new() });
            }
            "endloop" => match self.loops.pop() {
                Some(f) => {
                    let idx = self.push(line, Op::EndLoop { start: f.start });
                    if let Op::Loop { end, .. } = &mut self.ops[f.start].op {
                        *end = idx;
                    }
                    for b in f.breaks {
                        self.ops[b].op = Op::Break { end: idx };
                    }
                }
                None => self.warn(line, "endloop without loop"),
            },
            "break" => {
                if let Some(f) = self.loops.last_mut() {
                    let idx = self.ops.len();
                    f.breaks.push(idx);
                    self.push(line, Op::Break { end: 0 });
                } else {
                    // Outside a loop `Break` is a debugger breakpoint in SE; ignore.
                    self.push(line, Op::Other(item.clone()));
                }
            }
            _ => {
                let op = self.convert(item, &key);
                self.push(line, op);
            }
        }
    }

    fn finish(mut self) -> Script {
        for f in std::mem::take(&mut self.ifs) {
            self.warnings.push(format!("line {}: if without endif", f.line));
        }
        for f in std::mem::take(&mut self.loops) {
            let line = self.ops[f.start].line;
            self.warnings.push(format!("line {line}: loop without endloop"));
        }
        Script { commands: self.ops, warnings: self.warnings }
    }

    fn cond(&mut self, item: &Item) -> Cond {
        // `if { Var <op> value }`: the variable is the block's first row.
        let row = item.block().and_then(|b| b.items.first());
        let Some(row) = row else {
            self.warn(item.line, "if without condition");
            return Cond { var: String::new(), op: CmpOp::Truthy, rhs: Value::Num(0.0) };
        };
        let (op, rhs) = match row.args.as_slice() {
            [Value::Str(o), rhs, ..] => (
                match o.as_str() {
                    "<" => CmpOp::Lt,
                    "<=" => CmpOp::Le,
                    ">" => CmpOp::Gt,
                    ">=" => CmpOp::Ge,
                    "=" | "==" => CmpOp::Eq,
                    "!=" => CmpOp::Ne,
                    _ => CmpOp::Truthy,
                },
                rhs.clone(),
            ),
            _ => (CmpOp::Truthy, Value::Num(0.0)),
        };
        Cond { var: row.key.clone(), op, rhs }
    }

    fn convert(&mut self, item: &Item, key: &str) -> Op {
        let line = item.line;
        let blk = item.block();
        let blk_f = |k: &str| blk.and_then(|b| b.f64(k));
        match key {
            "checkversion" => Op::CheckVersion(item.f64().unwrap_or(0.0) as u32),
            "run" => Op::Run(item.str().unwrap_or("").to_string()),
            "quit" | "exit" | "gtfo" => Op::Quit,
            "restart" => Op::Restart,
            "select" => match item.str() {
                Some(n) => Op::Select(n.to_string()),
                None => {
                    self.warn(line, "Select without a name");
                    Op::Other(item.clone())
                }
            },
            "unselect" | "deselect" => Op::Unselect,
            "center" => Op::Center { time: blk_f("Time") },
            "follow" => Op::Follow,
            "syncrot" => Op::SyncRot,
            "free" => Op::Free,
            "track" => Op::Track,
            "untrack" => Op::Untrack,
            "gotolocation" => match blk.and_then(parse_location) {
                Some(l) => Op::GotoLocation(Box::new(l)),
                None => {
                    self.warn(line, "GotoLocation needs Pos, Rot and a date");
                    Op::Other(item.clone())
                }
            },
            "goto" => Op::Goto(blk.map(parse_goto).unwrap_or_default()),
            "fov" => Op::Fov { fov: item.f64().or_else(|| blk_f("FOV")), time: blk_f("Time") },
            "orbit" => {
                let axis = blk.and_then(|b| b.vec3("Axis")).unwrap_or([0.0, 1.0, 0.0]);
                Op::Orbit { axis, speed: blk_f("AngularSpeed").unwrap_or(10.0) }
            }
            "stoporbit" => Op::StopOrbit,
            "stopmotion" | "stopfly" => Op::StopMotion,
            "timescale" => Op::TimeScale(item.f64().unwrap_or(1.0)),
            "date" | "time" => match item.args.first().and_then(value_to_jd) {
                Some(jd) => Op::SetDate(jd),
                None => {
                    self.warn(line, "bad date");
                    Op::Other(item.clone())
                }
            },
            "stoptime" => Op::StopTime,
            "starttime" => Op::StartTime,
            "wait" => Op::Wait(item.f64().or_else(|| blk_f("Time")).unwrap_or(0.0)),
            "waittrigger" => match item.str() {
                Some(k) => Op::WaitTrigger(Box::new(parse_trigger(k, blk))),
                None => {
                    self.warn(line, "WaitTrigger without a kind");
                    Op::Other(item.clone())
                }
            },
            "beginmultitrigger" => Op::BeginMultiTrigger(
                if item.str().is_some_and(|s| s.eq_ignore_ascii_case("or")) { MultiMode::Or } else { MultiMode::And },
            ),
            "endmultitrigger" => Op::EndMultiTrigger,
            "showmessage" => Op::ShowMessage(item.str().unwrap_or("").to_string()),
            "waitmessage" => Op::WaitMessage(item.str().unwrap_or("").to_string()),
            "hidemessage" => Op::HideMessage,
            "print" => Op::Print { text: item.str().unwrap_or("").to_string(), time: blk_f("Time") },
            "hideprint" => Op::HidePrint,
            "log" => Op::Log(item.str().unwrap_or("").to_string()),
            "set" | "setu" | "setforce" => match (item.args.first().and_then(Value::as_str), item.args.get(1)) {
                (Some(name), Some(v)) => Op::Set { name: name.to_string(), value: v.clone(), force: key == "setforce" },
                (Some(name), None) => {
                    Op::Set { name: name.to_string(), value: Value::Num(1.0), force: key == "setforce" }
                }
                _ => {
                    self.warn(line, "Set without a variable name");
                    Op::Other(item.clone())
                }
            },
            "waitvar" => match item.args.first().and_then(Value::as_str) {
                Some(name) => {
                    let wait = match (item.args.get(1), blk.and_then(|b| b.nums("Range"))) {
                        (_, Some(r)) if r.len() >= 2 => VarWait::Range(r[0].min(r[1]), r[0].max(r[1])),
                        (Some(v), _) => VarWait::Equals(v.clone()),
                        _ => VarWait::Equals(Value::Num(1.0)),
                    };
                    Op::WaitVar { name: name.to_string(), wait }
                }
                None => {
                    self.warn(line, "WaitVar without a variable name");
                    Op::Other(item.clone())
                }
            },
            "savevars" => Op::SaveVars,
            "restorevars" => Op::RestoreVars,
            "interpolate" => match item.args.first().and_then(Value::as_str) {
                Some(name) => Op::Interpolate {
                    name: name.to_string(),
                    to: blk_f("To").unwrap_or(0.0),
                    time: blk_f("Time").unwrap_or(0.0),
                },
                None => Op::Other(item.clone()),
            },
            "waypoint" => Op::Waypoint {
                name: item.str().unwrap_or("").to_string(),
                block: blk.cloned().unwrap_or_default(),
            },
            "clearwaypoints" => Op::ClearWaypoints,
            "deletewaypoint" => Op::DeleteWaypoint(item.str().unwrap_or("").to_string()),
            _ => Op::Other(item.clone()),
        }
    }
}

fn range2(b: &Block, key: &str) -> Option<(f64, f64)> {
    let n = b.nums(key)?;
    (n.len() >= 2).then(|| (n[0], n[1]))
}

fn parse_trigger(kind: &str, blk: Option<&Block>) -> Trigger {
    let empty = Block::default();
    let b = blk.unwrap_or(&empty);
    let rect = b.nums("Rect").filter(|n| n.len() >= 4).map(|n| [n[0], n[1], n[2], n[3]]);
    Trigger {
        kind: kind.to_string(),
        object: b.str("Object").map(str::to_string),
        ty: b.str("Type").map(str::to_string),
        mode: b.str("Mode").map(str::to_string),
        dialog: b.str("Dialog").map(str::to_string),
        range: range2(b, "Range"),
        range_km: range2(b, "RangeKm"),
        range_rad: range2(b, "RangeRad"),
        yaw_range: range2(b, "YawRange"),
        pitch_range: range2(b, "PitchRange"),
        roll_range: range2(b, "RollRange"),
        rect,
        max_wait_time: b.f64("MaxWaitTime"),
        time_jd: b.get("Time").and_then(|i| i.args.first()).and_then(value_to_jd),
        raw: b.clone(),
    }
}

fn parse_goto(b: &Block) -> GotoArgs {
    GotoArgs {
        object: b.str("Object").map(str::to_string),
        height_km: b.f64_any(&["Height", "HeightKm"]),
        dist_km: b.f64("DistKm"),
        dist_rad: b.f64("DistRad"),
        lat: b.f64("Lat"),
        lon: b.f64("Lon"),
        yaw: b.f64("Yaw"),
        pitch: b.f64("Pitch"),
        roll: b.f64("Roll"),
        time: b.f64("Time"),
        center: b.bool("Center").unwrap_or(false),
    }
}

/// Token text of a value, for hex fixed-point numbers the lexer returned as words.
fn token_text(v: &Value) -> Option<String> {
    match v {
        Value::Ident(s) | Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(format!("{n}")),
        Value::Tuple(_) => None,
    }
}

fn parse_location(b: &Block) -> Option<Location> {
    let tuple = |key: &str| -> Option<Vec<String>> {
        match b.get_last(key)?.args.first()? {
            Value::Tuple(t) => t.iter().map(token_text).collect(),
            _ => None,
        }
    };
    let pos = tuple("Pos")?;
    if pos.len() != 3 {
        return None;
    }
    let mut pos_m = [0.0; 3];
    for (o, t) in pos_m.iter_mut().zip(&pos) {
        *o = fixed_to_f64(parse_fixed(t)?) * PC_M;
    }
    let r = b.nums("Rot")?;
    if r.len() != 4 {
        return None;
    }
    let jd = b
        .get_last("Time")
        .and_then(|i| i.args.first())
        .and_then(|v| token_text(v))
        .and_then(|t| parse_fixed(&t))
        .map(fixed_to_f64)
        .or_else(|| b.str("Date").and_then(parse_date));
    Some(Location {
        body: b.str("Body").unwrap_or("").to_string(),
        parent: b.str("Parent").unwrap_or("").to_string(),
        jd,
        pos_m,
        // file order w x y z -> x y z w
        rot: [r[1], r[2], r[3], r[0]],
        vel: b.f64("Vel").unwrap_or(0.0),
        stereobase: b.f64("Stereobase").unwrap_or(0.0),
        bind_mode: b.f64("BindMode").unwrap_or(0.0) as i32,
        photo_mode: b.f64("PhotoMode").unwrap_or(0.0) as i32,
        exposure: b.f64("Exposure").unwrap_or(0.0),
    })
}

/// Julian Date of a `Date`/`Time` argument: a date string or a hex fixed-point JD * 2^80.
fn value_to_jd(v: &Value) -> Option<f64> {
    match v {
        Value::Str(s) => parse_date(s).or_else(|| parse_fixed(s).map(fixed_to_f64)),
        Value::Ident(s) => parse_fixed(s).map(fixed_to_f64),
        Value::Num(n) => Some(*n),
        Value::Tuple(_) => None,
    }
}

/// Parses `YYYY.MM.DD hh:mm:ss.cc` (also `-` separators and a date-only form).
/// Returns a Julian Date.
pub fn parse_date(s: &str) -> Option<f64> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (date, time) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
    let d: Vec<&str> = date.split(['.', '-', '/']).filter(|p| !p.is_empty()).collect();
    if d.len() < 3 {
        return None;
    }
    let year: i64 = d[0].parse().ok()?;
    let month: u32 = d[1].parse().ok()?;
    let day: u32 = d[2].parse().ok()?;
    let t: Vec<f64> = time.split(':').filter_map(|p| p.trim().parse().ok()).collect();
    let dt = DateTime::new(
        if neg { -year } else { year },
        month,
        day,
        t.first().copied().unwrap_or(0.0) as u32,
        t.get(1).copied().unwrap_or(0.0) as u32,
        t.get(2).copied().unwrap_or(0.0),
    );
    let jd = dt.to_jd();
    jd.is_finite().then_some(jd)
}

/// One rendered line of a `ShowMessage` text.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkupLine {
    pub big: bool,
    pub text: String,
}

/// Splits a message into lines: `[BR]` breaks a line and `[BIG]...[/BIG]` marks
/// large text. Other bracketed words (`[G]`, `[Shift]`, `[Ok]`) are key labels and
/// stay in the text unchanged.
pub fn markup_lines(text: &str) -> Vec<MarkupLine> {
    let mut lines = Vec::new();
    let mut big = false;
    let mut cur = String::new();
    let mut cur_big = false;
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        cur.push_str(&rest[..open]);
        let after = &rest[open..];
        let Some(close) = after.find(']') else {
            cur.push_str(after);
            rest = "";
            break;
        };
        let tag = &after[1..close];
        match tag.to_ascii_uppercase().as_str() {
            "BR" => {
                lines.push(MarkupLine { big: cur_big, text: std::mem::take(&mut cur) });
                cur_big = big;
            }
            "BIG" => {
                big = true;
                cur_big = true;
            }
            "/BIG" => big = false,
            _ => cur.push_str(&after[..=close]),
        }
        rest = &after[close + 1..];
    }
    cur.push_str(rest);
    if !cur.is_empty() || lines.is_empty() {
        lines.push(MarkupLine { big: cur_big, text: cur });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r#"
GotoLocation
{
	Body	"Menu Moon 1.2"
	Parent	"Menu Planet 1"
	Date	"2019.05.31 18:23:57.57"
	Pos		(-4F5791FF91DD5932E1680000 -260FF17FB4C8FB29B6FA0000 -20BC307DCE5E99787800000)
	Rot		(0.6784887885700137 -0.3884777664761343 0.5280721251295764 -0.3314782336740115)
	Vel		2.0513983e-011
	Stereobase	1
	BindMode	1
	PhotoMode	0
	Exposure	0
}

TimeScale 50

/*
GotoLocation
{
	Body "ignored"
}
/**/
"#;

    #[test]
    fn goto_location_and_timescale() {
        let s = parse(SCENE);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(s.commands.len(), 2);
        let Op::GotoLocation(l) = &s.commands[0].op else { panic!("{:?}", s.commands[0]) };
        assert_eq!(l.body, "Menu Moon 1.2");
        assert_eq!(l.parent, "Menu Planet 1");
        assert_eq!(l.bind_mode, 1);
        // w x y z in the file -> x y z w
        assert!((l.rot[3] - 0.6784887885700137).abs() < 1e-12);
        assert!((l.rot[0] + 0.3884777664761343).abs() < 1e-12);
        // Pos is parsecs * 2^80: 0x4F57.. / 2^80 is about 2.07e4 pc, negative.
        let pc = l.pos_m[0] / PC_M;
        assert!(pc < -1.0e4 && pc > -3.0e4, "{pc}");
        // No Time field: the JD comes from the Date string (2019-05-31 18:23:57.57).
        let jd = l.jd.unwrap();
        assert!((jd - 2458635.2666).abs() < 1e-3, "{jd}");
        assert_eq!(s.commands[1].op, Op::TimeScale(50.0));
    }

    #[test]
    fn time_field_wins_over_date() {
        let src = r#"GotoLocation
{
	Body	"Earth"
	Parent	"Earth-Moon"
	Date	"2018.12.18 17:01:36.35"
	Time	+258367359E6C1E7E84FB2E0000
	Pos		(+49D3973BBA5E0000 +20005BC9018E0000 +4DFA944D84C0000)
	Rot		(0.4997503719243814 0.1681250497132221 -0.8491387566376538 0.03077182796372254)
	Vel		8.2680578e-11
}"#;
        let s = parse(src);
        let Op::GotoLocation(l) = &s.commands[0].op else { panic!() };
        let jd = l.jd.unwrap();
        // 2018-12-18 17:01:36 UTC is JD 2458471.2. Time is JD * 2^80.
        assert!((jd - 2458471.2).abs() < 1.0, "{jd}");
        // Earth is 1 AU from the Sun: |pos| about 4.85e-6 pc.
        let r = (l.pos_m[0].powi(2) + l.pos_m[1].powi(2) + l.pos_m[2].powi(2)).sqrt();
        assert!((r / crate::places::AU_M - 1.0).abs() < 0.1, "{}", r / crate::places::AU_M);
    }

    #[test]
    fn plain_commands_and_blocks() {
        let src = r#"
CheckVersion 991
SaveVars
Run "tutorials/DefaultSettings.se"
Select    "Earth-Moon|Earth"
Orbit
{
	Axis (0 1 0)
	AngularSpeed 10
}
FOV 45
FOV { Time 0 }
Wait 3.0 // pause
WaitTrigger "LoadingComplete" { MaxWaitTime 30 }
Goto
{
	Height  64000 // 20000
	Lat  	0
	Lon 	-88
	Pitch   -90
	Time	3
	Center true
}
Set ShowOrbits false
Set	EnablePseudoFlares      false
SetForce LessonPassed 3
WaitVar StarMagnLimit { Range (7.5, 9.4) }
WaitVar PhotoMode 2
Interpolate TimeScale { To 1.0 Time 2.0 }
HighlightWidget "Main toolbar|Wiki"
"#;
        let s = parse(src);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        let ops: Vec<&Op> = s.commands.iter().map(|c| &c.op).collect();
        assert_eq!(ops[0], &Op::CheckVersion(991));
        assert_eq!(ops[1], &Op::SaveVars);
        assert_eq!(ops[2], &Op::Run("tutorials/DefaultSettings.se".into()));
        assert_eq!(ops[3], &Op::Select("Earth-Moon|Earth".into()));
        assert_eq!(ops[4], &Op::Orbit { axis: [0.0, 1.0, 0.0], speed: 10.0 });
        assert_eq!(ops[5], &Op::Fov { fov: Some(45.0), time: None });
        assert_eq!(ops[6], &Op::Fov { fov: None, time: Some(0.0) });
        assert_eq!(ops[7], &Op::Wait(3.0));
        let Op::WaitTrigger(t) = ops[8] else { panic!() };
        assert_eq!((t.kind.as_str(), t.max_wait_time), ("LoadingComplete", Some(30.0)));
        let Op::Goto(g) = ops[9] else { panic!() };
        assert_eq!(g.height_km, Some(64000.0));
        assert_eq!((g.lat, g.lon, g.pitch, g.time), (Some(0.0), Some(-88.0), Some(-90.0), Some(3.0)));
        assert!(g.center);
        assert_eq!(ops[10], &Op::Set { name: "ShowOrbits".into(), value: Value::Ident("false".into()), force: false });
        assert!(matches!(ops[11], Op::Set { name, .. } if name == "EnablePseudoFlares"));
        assert!(matches!(ops[12], Op::Set { force: true, .. }));
        assert_eq!(ops[13], &Op::WaitVar { name: "StarMagnLimit".into(), wait: VarWait::Range(7.5, 9.4) });
        assert_eq!(ops[14], &Op::WaitVar { name: "PhotoMode".into(), wait: VarWait::Equals(Value::Num(2.0)) });
        assert_eq!(ops[15], &Op::Interpolate { name: "TimeScale".into(), to: 1.0, time: 2.0 });
        assert!(matches!(ops[16], Op::Other(i) if i.key == "HighlightWidget"));
    }

    #[test]
    fn triggers() {
        let src = r#"
BeginMultiTrigger "AND"
WaitTrigger "Control|Rotate"
{
	Mode		"OR"
	PitchRange	(-30, 30)
	YawRange	(-30, 30)
}
WaitTrigger "Object|See"
{
	Object	"Sun"
	Rect	(0.2, 0.8, 0.2, 0.8)
}
EndMultitrigger
WaitTrigger "Object|Approach" { Object "Gliese 667 C f" RangeRad (0, 10) }
WaitTrigger "Time" { Mode "Later" Time "2016.11.15 21:00:00" }
"#;
        let s = parse(src);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(s.commands[0].op, Op::BeginMultiTrigger(MultiMode::And));
        let Op::WaitTrigger(rot) = &s.commands[1].op else { panic!() };
        assert_eq!(rot.pitch_range, Some((-30.0, 30.0)));
        assert_eq!(rot.mode.as_deref(), Some("OR"));
        let Op::WaitTrigger(see) = &s.commands[2].op else { panic!() };
        assert_eq!(see.object.as_deref(), Some("Sun"));
        assert_eq!(see.rect, Some([0.2, 0.8, 0.2, 0.8]));
        assert_eq!(s.commands[3].op, Op::EndMultiTrigger);
        let Op::WaitTrigger(app) = &s.commands[4].op else { panic!() };
        assert_eq!(app.object.as_deref(), Some("Gliese 667 C f"));
        assert_eq!(app.range_rad, Some((0.0, 10.0)));
        let Op::WaitTrigger(t) = &s.commands[5].op else { panic!() };
        let jd = t.time_jd.unwrap();
        assert!((jd - 2457708.375).abs() < 1e-6, "{jd}");
    }

    #[test]
    fn if_endif() {
        let src = "if { LessonPassed < 2 }\nSetForce LessonPassed 2\nendif\nWait 1\n";
        let s = parse(src);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(s.commands.len(), 4);
        let Op::If { cond, on_false } = &s.commands[0].op else { panic!("{:?}", s.commands[0]) };
        assert_eq!((cond.var.as_str(), cond.op), ("LessonPassed", CmpOp::Lt));
        assert!(cond.eval(1.0) && !cond.eval(2.0));
        assert_eq!(*on_false, 2);
        assert_eq!(s.commands[2].op, Op::EndIf);
    }

    #[test]
    fn if_elif_else_jumps() {
        let src = "if { A == 1 }\nWait 1\nelif { A > 5 }\nWait 2\nelse\nWait 3\nendif\nWait 4\n";
        let s = parse(src);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        // 0 If, 1 Wait1, 2 Jump, 3 Elif, 4 Wait2, 5 Jump, 6 Else, 7 Wait3, 8 EndIf, 9 Wait4
        assert_eq!(s.commands.len(), 10);
        assert!(matches!(s.commands[0].op, Op::If { on_false: 3, .. }));
        assert_eq!(s.commands[2].op, Op::Jump(8));
        assert!(matches!(s.commands[3].op, Op::Elif { on_false: 7, .. }));
        assert_eq!(s.commands[5].op, Op::Jump(8));
        assert_eq!(s.commands[6].op, Op::Else);
        assert_eq!(s.commands[8].op, Op::EndIf);
    }

    #[test]
    fn unbalanced_if_warns() {
        let s = parse("if { A }\nWait 1\n");
        assert_eq!(s.warnings.len(), 1);
        let s = parse("endif\n");
        assert_eq!(s.warnings.len(), 1);
    }

    #[test]
    fn loops() {
        let s = parse("loop 3\nWait 1\nbreak\nendloop\n");
        assert_eq!(s.commands[0].op, Op::Loop { count: Some(3), end: 3 });
        assert_eq!(s.commands[2].op, Op::Break { end: 3 });
        assert_eq!(s.commands[3].op, Op::EndLoop { start: 0 });
    }

    #[test]
    fn dates() {
        let jd = parse_date("2000.01.01 12:00:00.00").unwrap();
        assert!((jd - 2451545.0).abs() < 1e-6);
        assert!(parse_date("nonsense").is_none());
        assert!((parse_date("2016-11-15").unwrap() - 2457707.5).abs() < 1e-6);
    }

    #[test]
    fn message_markup() {
        let l = markup_lines("Welcome![BR][BR]Press [G] to go.");
        assert_eq!(
            l,
            vec![
                MarkupLine { big: false, text: "Welcome!".into() },
                MarkupLine { big: false, text: "".into() },
                MarkupLine { big: false, text: "Press [G] to go.".into() },
            ]
        );
        let l = markup_lines("[BIG]Farewell, Space Explorer![BR][/BIG]");
        assert_eq!(l, vec![MarkupLine { big: true, text: "Farewell, Space Explorer!".into() }]);
    }

    #[test]
    fn unknown_commands_are_kept() {
        let s = parse("HideAllDialogs\nOpenMenu \"Main|Planetarium|Tutorial\"\nUserMoveControl \"Disabled\"\n");
        assert_eq!(s.commands.len(), 3);
        assert_eq!(s.commands[0].op.name(), "hidealldialogs");
        assert_eq!(s.commands[1].op.name(), "openmenu");
    }
}
