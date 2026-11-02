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
    /// Flight profile (FUN_140563f30): seconds speeding up, cruising and braking, and the
    /// powers of the speed-up and braking curves.
    pub accel_time: Option<f64>,
    pub drift_time: Option<f64>,
    pub decel_time: Option<f64>,
    pub accel_power: Option<f64>,
    pub decel_power: Option<f64>,
}

/// The user input a `User*Control` command locks: `UserMoveControl` (FUN_140552c30),
/// `UserRotationControl` (FUN_140552900), `UserTimeControl` (FUN_140552330) and
/// `UserStereobaseControl` (FUN_140551ef0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Move,
    Rotation,
    Time,
    Stereobase,
}

/// `"Free"`, `"Disabled"` or `"Limited"` (the last with a block of limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Free,
    Disabled,
    Limited,
}

/// Limits of a `"Limited"` user control.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ControlLimits {
    /// Movement: the object (`Center`) the camera must stay near, the distance (`DistKm`,
    /// `DistLy` as km, or `DistRad` in its radii) and the top speed (`SpeedKm`, or `Speed` pc/s,
    /// as km/s).
    pub center: Option<String>,
    pub dist_km: Option<f64>,
    pub dist_rad: Option<f64>,
    pub speed_kms: Option<f64>,
    /// Time: the allowed dates (`TimeRange`, Julian Dates) and time rates (`TimeRateRange`).
    pub time_range: Option<(f64, f64)>,
    pub rate_range: Option<(f64, f64)>,
    /// Stereobase: `Range`.
    pub range: Option<(f64, f64)>,
}

/// `Shape` of a waypoint (FUN_140549e90: CIRCLE 0 .. WIRESPHERE 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaypointShape {
    #[default]
    Circle,
    Triangle,
    Square,
    Diamond,
    Hexagon,
    Billboard,
    Sphere,
    WireSphere,
}

impl WaypointShape {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_uppercase().as_str() {
            "CIRCLE" => Self::Circle,
            "TRIANGLE" => Self::Triangle,
            "SQUARE" => Self::Square,
            "DIAMOND" => Self::Diamond,
            "HEXAGON" => Self::Hexagon,
            "BILLBOARD" => Self::Billboard,
            "SPHERE" => Self::Sphere,
            "WIRESPHERE" => Self::WireSphere,
            _ => return None,
        })
    }
}

/// A waypoint's position relative to its parent's centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaypointPos {
    /// Kilometres, in the parent's axes (the port's body frame: y to the north pole).
    pub km: [f64; 3],
    /// `FixedPos*`: turns with the parent's rotation; `StaticPos*`: does not.
    pub fixed: bool,
}

/// `Waypoint "name" { ... }` (FUN_140065f40, FUN_140549e90). Every key is optional: defining
/// a waypoint again by name changes only the keys given; the `DEFAULT_*` constants hold the
/// values of a new one (FUN_14054aeb0).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaypointDef {
    pub name: String,
    pub parent: Option<String>,
    /// `RadiusKm`, or `Radius` in parsecs, as km.
    pub radius_km: Option<f64>,
    /// `Roll`, degrees: turns the shape in the view.
    pub roll_deg: Option<f64>,
    pub color: Option<[f32; 4]>,
    pub color_empty: Option<[f32; 4]>,
    /// Defaults to `Color`.
    pub label_color: Option<[f32; 4]>,
    pub line_width: Option<f32>,
    pub shape: Option<WaypointShape>,
    pub texture: Option<String>,
    pub pos: Option<WaypointPos>,
    pub visible: Option<bool>,
    pub label: Option<bool>,
    pub triggerable: Option<bool>,
}

impl WaypointDef {
    /// A new waypoint's values (FUN_14054aeb0): radius 1 km, white, line width 2, a circle at the
    /// parent's centre, visible, labelled and triggerable.
    pub const DEFAULT_RADIUS_KM: f64 = 1.0;
    pub const DEFAULT_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    pub const DEFAULT_LINE_WIDTH: f32 = 2.0;
}

/// `HighlightWidget "Toolbar|Button" { Time Freq Color }` (FUN_14055df80): the widget blinks in
/// `Color` (default (1, 0.2, 0.2, 1), FUN_14036c000) `Freq` times a second (default 4) for `Time`
/// seconds, or with no `Time` until the next wait ends. An empty name clears the highlight.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Highlight {
    pub widget: String,
    pub time: Option<f64>,
    pub freq: Option<f64>,
    pub color: Option<[f32; 4]>,
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
    /// `TimeRange ("date", "date")` of a `Time` trigger (`Mode "InRange"`/`"OutRange"`), Julian Dates.
    pub time_range: Option<(f64, f64)>,
    /// `Direction (x y z)` of `Control|Move`, in SE's camera axes (x left, y down, z ahead).
    pub direction: Option<[f64; 3]>,
    /// `LonLatRange (lon0 lon1 lat0 lat1)` of `Object|Approach`, degrees: where the camera is
    /// over the object.
    pub lonlat_range: Option<[f64; 4]>,
    /// `ActivationDelay`, seconds: the trigger counts only after this long.
    pub activation_delay: Option<f64>,
    /// `AnimateObject`: an object to point out while waiting.
    pub animate_object: Option<String>,
    /// `Value` of `GUI|ChangeValue`.
    pub value: Option<f64>,
    /// `Button` of `Input|Button` / `Input|JoyButton`.
    pub button: Option<String>,
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
    /// `Horizon`: level the view to the selection's horizon.
    Horizon,
    /// `Fly { Axis Speed|SpeedKm FadeTime }` (FUN_140562dd0): fly along `axis` (SE's camera axes,
    /// x left, y down, z ahead) at `speed` m/s, reached over `fade` s.
    Fly { axis: [f64; 3], speed: f64, fade: f64 },
    /// `StopFly { FadeTime }` (FUN_140562b00).
    StopFly { fade: f64 },
    /// `Turn { Axis AngularSpeed FadeTime }` (FUN_140562670): turn about `axis` at `speed` deg/s.
    Turn { axis: [f64; 3], speed: f64, fade: f64 },
    /// `StopTurn { FadeTime }` (FUN_1405623a0).
    StopTurn { fade: f64 },
    /// `StopRotation`: stop turning at once.
    StopRotation,
    /// `Speed` (pc/s) and `SpeedKm` (km/s), as m/s: the assigned velocity (FUN_140561b00,
    /// FUN_140561a40).
    Speed(f64),
    /// `MoveMode 1..4` (FUN_140561bb0).
    MoveMode(u32),
    /// `UserMoveControl`, `UserRotationControl`, `UserTimeControl`, `UserStereobaseControl`.
    UserControl { kind: InputKind, mode: LockMode, limits: Box<ControlLimits> },
    /// `GotoURL "se://..."`.
    GotoUrl(String),

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

    /// `HighlightWidget`.
    HighlightWidget(Box<Highlight>),
    /// `SetWidgetStyle "Message box" { Pos (x, y) PosPix (x, y) Visible }` (FUN_14055e790): `Pos`
    /// in fractions of the screen, `PosPix` in pixels, of the widget's top-left corner.
    SetWidgetStyle { widget: String, pos: Option<[f64; 2]>, pos_pix: Option<[f64; 2]>, visible: Option<bool> },
    /// `ShowDialog "Settings" { Tab "Graphics" }` (FUN_14055f680); toolbars are dialogs too.
    ShowDialog { dialog: String, tab: Option<String> },
    HideDialog(String),
    HideAllDialogs,
    /// FUN_1402536d0: the four toolbars go into auto-hide and slide away.
    HideAllToolbars,
    /// `OpenMenu "Main|Planetarium|Tutorial"` (FUN_14055d840) and `CloseMenu`.
    OpenMenu(String),
    CloseMenu,
    /// `FadeOut { Time Color }` / `FadeIn { Time }` (FUN_1405554e0, FUN_140554f90): fade the
    /// screen to `color` and back.
    ScreenFade { out: bool, time: Option<f64>, color: Option<[f32; 4]> },

    /// `PlayMusic "file.ogg" { CrossfadeDuration }` (FUN_14055dd30), `PauseMusic` and
    /// `ResumeMusic` (FUN_14055dc00, FUN_14055dad0); `None` takes `MixDurationScript`.
    PlayMusic { file: String, crossfade: Option<f64> },
    PauseMusic { crossfade: Option<f64> },
    ResumeMusic { crossfade: Option<f64> },

    /// `Set Name value`, `SetU`, or `SetForce` (`force`).
    Set { name: String, value: Value, force: bool },
    /// `Get Name`: print a variable's value (FUN_14055a6a0).
    Get(String),
    /// `Reset Name`: back to its default (FUN_14055a8d0).
    Reset(String),
    WaitVar { name: String, wait: VarWait },
    SaveVars,
    RestoreVars,
    Interpolate { name: String, to: f64, time: f64 },

    Waypoint(Box<WaypointDef>),
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

/// `if { A < 7 }`: the `.sc` lexer drops comparison characters, so quote them. The condition
/// block may open on the line after the `if`/`elif` and span lines.
fn preprocess_conditions(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 16);
    // None outside a condition; Some(0) after `if`/`elif` before its `{`; Some(n) n braces deep.
    let mut depth: Option<u32> = None;
    for raw in src.split_inclusive('\n') {
        let mut i = 0;
        if depth.is_none() {
            let Some(k) = cond_keyword_len(raw) else {
                out.push_str(raw);
                continue;
            };
            out.push_str(&raw[..k]);
            i = k;
            depth = Some(0);
        }
        let b = raw.as_bytes();
        let mut in_str = false;
        while i < b.len() {
            let Some(d) = depth else {
                out.push_str(&raw[i..]);
                break;
            };
            let c = b[i];
            // Bytes, not `&raw[i..i + 2]`: that panics when a multi-byte character follows.
            let two = b.get(i..i + 2).unwrap_or(&[]);
            if !in_str && two == b"//" {
                out.push_str(&raw[i..]);
                break;
            }
            if d == 0 && !c.is_ascii_whitespace() && c != b'{' {
                // No condition block follows.
                depth = None;
                continue;
            }
            if !in_str && matches!(two, b"<=" | b">=" | b"==" | b"!=") {
                out.push_str(&format!(" \"{}{}\" ", two[0] as char, two[1] as char));
                i += 2;
            } else if !in_str && matches!(c, b'<' | b'>' | b'=') {
                out.push_str(&format!(" \"{}\" ", c as char));
                i += 1;
            } else {
                match c {
                    b'"' => in_str = !in_str,
                    b'{' if !in_str => depth = Some(d + 1),
                    b'}' if !in_str => depth = (d > 1).then(|| d - 1),
                    _ => {}
                }
                // Copy whole UTF-8 characters.
                let ch = raw[i..].chars().next().unwrap_or(' ');
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Length up to the end of a leading `if`/`elif` keyword when the line starts a condition.
fn cond_keyword_len(line: &str) -> Option<usize> {
    let t = line.trim_start();
    let k = ["elif", "if"].into_iter().find(|k| t.get(..k.len()).is_some_and(|p| p.eq_ignore_ascii_case(k)))?.len();
    let rest = t[k..].trim_start();
    (rest.is_empty() || rest.starts_with('{') || rest.starts_with("//")).then_some(line.len() - t.len() + k)
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
        // Open jump targets point past the last command (the script ends there), not at 0,
        // which would restart the script forever.
        let end = self.ops.len();
        for f in std::mem::take(&mut self.ifs) {
            self.warnings.push(format!("line {}: if without endif", f.line));
            if let Some(p) = f.pending {
                self.set_on_false(p, end);
            }
            for j in f.jumps {
                self.ops[j].op = Op::Jump(end);
            }
        }
        for f in std::mem::take(&mut self.loops) {
            let line = self.ops[f.start].line;
            self.warnings.push(format!("line {line}: loop without endloop"));
            if let Op::Loop { end: e, .. } = &mut self.ops[f.start].op {
                *e = end;
            }
            for b in f.breaks {
                self.ops[b].op = Op::Break { end };
            }
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
            "stopmotion" => Op::StopMotion,
            "horizon" => Op::Horizon,
            "fly" => {
                let axis = blk.and_then(|b| b.vec3("Axis")).unwrap_or([0.0, 0.0, 1.0]);
                let speed = match (blk_f("SpeedKm"), blk_f("Speed")) {
                    (Some(km), _) => km * 1000.0,
                    (None, Some(pc)) => pc * PC_M,
                    // FUN_140562dd0's default, 10 pc/s.
                    (None, None) => FLY_SPEED_PC * PC_M,
                };
                Op::Fly { axis, speed, fade: blk_f("FadeTime").unwrap_or(FADE_TIME_S) }
            }
            "stopfly" => Op::StopFly { fade: blk_f("FadeTime").unwrap_or(FADE_TIME_S) },
            "turn" => Op::Turn {
                axis: blk.and_then(|b| b.vec3("Axis")).unwrap_or([0.0, 1.0, 0.0]),
                speed: blk_f("AngularSpeed").unwrap_or(TURN_SPEED_DEG),
                fade: blk_f("FadeTime").unwrap_or(FADE_TIME_S),
            },
            "stopturn" => Op::StopTurn { fade: blk_f("FadeTime").unwrap_or(FADE_TIME_S) },
            "stoprotation" => Op::StopRotation,
            "speed" | "speedkm" => match item.f64() {
                Some(v) => Op::Speed(if key == "speed" { v * PC_M } else { v * 1000.0 }),
                None => {
                    self.warn(line, "Speed without a value");
                    Op::Other(item.clone())
                }
            },
            "movemode" => match item.f64() {
                Some(m) if (1.0..=4.0).contains(&m) => Op::MoveMode(m as u32),
                _ => {
                    self.warn(line, "MoveMode takes 1 to 4");
                    Op::Other(item.clone())
                }
            },
            "usermovecontrol" | "userrotationcontrol" | "usertimecontrol" | "userstereobasecontrol" => {
                let kind = match key {
                    "usermovecontrol" => InputKind::Move,
                    "userrotationcontrol" => InputKind::Rotation,
                    "usertimecontrol" => InputKind::Time,
                    _ => InputKind::Stereobase,
                };
                let mode = match item.str().map(str::to_ascii_lowercase).as_deref() {
                    Some("free") => LockMode::Free,
                    Some("disabled") => LockMode::Disabled,
                    Some("limited") => LockMode::Limited,
                    _ => {
                        self.warn(line, format!("{} takes \"Free\", \"Disabled\" or \"Limited\"", item.key));
                        return Op::Other(item.clone());
                    }
                };
                Op::UserControl { kind, mode, limits: Box::new(blk.map(parse_limits).unwrap_or_default()) }
            }
            "gotourl" => Op::GotoUrl(item.str().unwrap_or("").to_string()),
            "highlightwidget" => Op::HighlightWidget(Box::new(Highlight {
                widget: item.str().unwrap_or("").to_string(),
                time: blk_f("Time"),
                freq: blk_f("Freq"),
                color: blk.and_then(|b| color4(b, "Color")),
            })),
            "setwidgetstyle" => Op::SetWidgetStyle {
                widget: item.str().unwrap_or("").to_string(),
                pos: blk.and_then(|b| range2(b, "Pos")).map(|(x, y)| [x, y]),
                pos_pix: blk.and_then(|b| range2(b, "PosPix")).map(|(x, y)| [x, y]),
                visible: blk.and_then(|b| b.bool("Visible")),
            },
            "showdialog" => Op::ShowDialog {
                dialog: item.str().unwrap_or("").to_string(),
                tab: blk.and_then(|b| b.str("Tab")).map(str::to_string),
            },
            "hidedialog" => Op::HideDialog(item.str().unwrap_or("").to_string()),
            "hidealldialogs" => Op::HideAllDialogs,
            "hidealltoolbars" => Op::HideAllToolbars,
            "openmenu" => Op::OpenMenu(item.str().unwrap_or("").to_string()),
            "closemenu" => Op::CloseMenu,
            "fadeout" | "fadein" => Op::ScreenFade {
                out: key == "fadeout",
                time: item.f64().or_else(|| blk_f("Time")),
                color: blk.and_then(|b| color4(b, "Color")),
            },
            "playmusic" => Op::PlayMusic { file: item.str().unwrap_or("").to_string(), crossfade: blk_f("CrossfadeDuration") },
            "pausemusic" => Op::PauseMusic { crossfade: item.f64().or_else(|| blk_f("CrossfadeDuration")) },
            "resumemusic" => Op::ResumeMusic { crossfade: item.f64().or_else(|| blk_f("CrossfadeDuration")) },
            "get" | "reset" => match item.args.first().and_then(Value::as_str) {
                Some(n) if key == "get" => Op::Get(n.to_string()),
                Some(n) => Op::Reset(n.to_string()),
                None => {
                    self.warn(line, format!("{} without a variable name", item.key));
                    Op::Other(item.clone())
                }
            },
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
            "waypoint" => match item.str() {
                Some(name) => Op::Waypoint(Box::new(parse_waypoint(name, blk))),
                None => {
                    self.warn(line, "Waypoint without a name");
                    Op::Other(item.clone())
                }
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
        time_range: b.get_last("TimeRange").and_then(|i| {
            let dates: Vec<f64> = flat_values(&i.args).iter().filter_map(|v| value_to_jd(v)).collect();
            (dates.len() >= 2).then(|| (dates[0].min(dates[1]), dates[0].max(dates[1])))
        }),
        direction: b.get_last("Direction").map(lenient_nums).filter(|n| n.len() >= 3).map(|n| [n[0], n[1], n[2]]),
        lonlat_range: b.vec4("LonLatRange"),
        activation_delay: b.f64("ActivationDelay"),
        animate_object: b.str("AnimateObject").map(str::to_string),
        value: b.f64("Value"),
        button: b.str("Button").map(str::to_string),
        raw: b.clone(),
    }
}

/// Defaults of `Fly` / `Turn` (FUN_140562dd0, FUN_140562670): 10 pc/s, 10 deg/s, 2 s fade.
const FLY_SPEED_PC: f64 = 10.0;
const TURN_SPEED_DEG: f64 = 10.0;
const FADE_TIME_S: f64 = 2.0;
const KM_PER_LY: f64 = 9.460_730_472_580_8e12;

/// Values with tuples flattened.
fn flat_values(args: &[Value]) -> Vec<&Value> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Value::Tuple(t) => out.extend(t.iter()),
            v => out.push(v),
        }
    }
    out
}

/// Numbers of a row, reading words with a stray prefix as their number: SE's own
/// tut-02-free-mov.se has `Direction (0.0, 0.0, s-1.0)` for "backward".
fn lenient_nums(item: &Item) -> Vec<f64> {
    flat_values(&item.args)
        .into_iter()
        .filter_map(|v| match v {
            Value::Ident(s) | Value::Str(s) => {
                let t = s.trim_start_matches(|c: char| !(c.is_ascii_digit() || c == '-' || c == '+' || c == '.'));
                t.parse().ok()
            }
            other => other.as_f64(),
        })
        .collect()
}

/// An RGBA colour row (`(r, g, b)` gets alpha 1).
fn color4(b: &Block, key: &str) -> Option<[f32; 4]> {
    let n = b.nums(key)?;
    match n.len() {
        3 => Some([n[0] as f32, n[1] as f32, n[2] as f32, 1.0]),
        l if l >= 4 => Some([n[0] as f32, n[1] as f32, n[2] as f32, n[3] as f32]),
        _ => None,
    }
}

/// Limits of `User*Control "Limited" { ... }`.
fn parse_limits(b: &Block) -> ControlLimits {
    let range = |key: &str| range2(b, key).map(|(a, c)| (a.min(c), a.max(c)));
    ControlLimits {
        center: b.str("Center").map(str::to_string),
        dist_km: b.f64("DistKm").or_else(|| b.f64("DistLy").map(|ly| ly * KM_PER_LY)),
        dist_rad: b.f64("DistRad"),
        speed_kms: b.f64("SpeedKm").or_else(|| b.f64("Speed").map(|pc| pc * PC_M / 1000.0)),
        time_range: b.get_last("TimeRange").and_then(|i| {
            let d: Vec<f64> = flat_values(&i.args).iter().filter_map(|v| value_to_jd(v)).collect();
            (d.len() >= 2).then(|| (d[0].min(d[1]), d[0].max(d[1])))
        }),
        rate_range: range("TimeRateRange"),
        range: range("Range"),
    }
}

/// Unit direction for polar `(lon, lat)` degrees in the port's body frame (as `Goto`'s Lat/Lon).
pub fn polar_dir(lon_deg: f64, lat_deg: f64) -> [f64; 3] {
    let (lon, lat) = (lon_deg.to_radians(), lat_deg.to_radians());
    [lat.cos() * lon.cos(), lat.sin(), -lat.cos() * lon.sin()]
}

/// The keys of a `Waypoint` block (FUN_140549e90).
fn parse_waypoint(name: &str, blk: Option<&Block>) -> WaypointDef {
    let empty = Block::default();
    let b = blk.unwrap_or(&empty);
    let km_per_pc = PC_M / 1000.0;
    // Positions, read in SE's order so the last given wins.
    let mut pos = None;
    for (key, fixed, polar, scale) in [
        ("StaticPosXYZ", false, false, km_per_pc),
        ("StaticPosXYZKm", false, false, 1.0),
        ("StaticPosPolar", false, true, km_per_pc),
        ("StaticPosPolarKm", false, true, 1.0),
        ("FixedPosXYZ", true, false, km_per_pc),
        ("FixedPosXYZKm", true, false, 1.0),
        ("FixedPosPolar", true, true, km_per_pc),
        ("FixedPosPolarKm", true, true, 1.0),
    ] {
        let Some(v) = b.vec3(key) else { continue };
        let km = if polar {
            let d = polar_dir(v[0], v[1]);
            [d[0] * v[2] * scale, d[1] * v[2] * scale, d[2] * v[2] * scale]
        } else {
            [v[0] * scale, v[1] * scale, v[2] * scale]
        };
        pos = Some(WaypointPos { km, fixed });
    }
    WaypointDef {
        name: name.to_string(),
        parent: b.str("Parent").map(str::to_string),
        radius_km: b.f64("RadiusKm").or_else(|| b.f64("Radius").map(|pc| pc * km_per_pc)),
        roll_deg: b.f64("Roll"),
        color: color4(b, "Color"),
        color_empty: color4(b, "ColorEmpty"),
        label_color: color4(b, "LabelColor"),
        line_width: b.f64("LineWidth").map(|w| w as f32),
        shape: b.str("Shape").and_then(WaypointShape::parse),
        texture: b.str("Texture").map(str::to_string),
        pos,
        visible: b.bool("Visible"),
        label: b.bool("Label"),
        triggerable: b.bool("Triggerable"),
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
        accel_time: b.f64("AccelTime"),
        drift_time: b.f64("DriftTime"),
        decel_time: b.f64("DecelTime"),
        accel_power: b.f64("AccelPower"),
        decel_power: b.f64("DecelPower"),
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
        assert!(matches!(ops[16], Op::HighlightWidget(h) if h.widget == "Main toolbar|Wiki" && h.time.is_none()));
    }

    #[test]
    fn waypoints() {
        let src = r#"
Waypoint "WP 1"
{
	Parent	"TRAPPIST-1|TRAPPIST-1 g"
	RadiusKm  500
	Color    (2.0, 2.0, 0.2, 0.5)
	FixedPosPolarKm (2.9 0.62 14000)
	Shape    "Circle"
}
Waypoint "WP" { Radius 1e-10 Shape "WireSphere" StaticPosXYZKm (1 2 3) Label false }
DeleteWaypoint "WP"
ClearWaypoints
"#;
        let s = parse(src);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        let Op::Waypoint(w) = &s.commands[0].op else { panic!("{:?}", s.commands[0]) };
        assert_eq!((w.name.as_str(), w.parent.as_deref()), ("WP 1", Some("TRAPPIST-1|TRAPPIST-1 g")));
        assert_eq!((w.radius_km, w.shape, w.color), (Some(500.0), Some(WaypointShape::Circle), Some([2.0, 2.0, 0.2, 0.5])));
        let p = w.pos.unwrap();
        assert!(p.fixed);
        let r = (p.km[0].powi(2) + p.km[1].powi(2) + p.km[2].powi(2)).sqrt();
        assert!((r - 14000.0).abs() < 1e-6 && (p.km[1] / r - 0.62f64.to_radians().sin()).abs() < 1e-9);
        let Op::Waypoint(w) = &s.commands[1].op else { panic!() };
        // Radius in parsecs, as km.
        assert!((w.radius_km.unwrap() - 1e-10 * PC_M / 1000.0).abs() < 1e-6);
        assert_eq!((w.shape, w.label), (Some(WaypointShape::WireSphere), Some(false)));
        assert_eq!(w.pos, Some(WaypointPos { km: [1.0, 2.0, 3.0], fixed: false }));
        assert!(w.color.is_none() && w.radius_km.is_some());
        assert_eq!(s.commands[2].op, Op::DeleteWaypoint("WP".into()));
        assert_eq!(s.commands[3].op, Op::ClearWaypoints);
    }

    #[test]
    fn gui_music_and_camera_commands() {
        let src = r#"
UserMoveControl "Disabled"
UserTimeControl "Free"
UserMoveControl "Limited" { Center "Earth" DistKm 1e5 SpeedKm 10 }
HighlightWidget "Wiki|Object name" { Time 1 Color (0.5, 0, 0, 1) }
HighlightWidget ""
SetWidgetStyle "Message box" { Pos (0.7, 0.2) }
ShowDialog  "Settings" { Tab "Graphics" }
HideDialog "Wiki"
HideAllToolbars
PlayMusic "Lokijar - Quasar Medusa.ogg"
PauseMusic { CrossfadeDuration 3 }
Fly { Axis (0 0 1) SpeedKm 100 FadeTime 1 }
StopFly
Turn { AngularSpeed 5 }
Speed 0.5
SpeedKm 500
FadeOut { Time 2 Color (0 0 0 1) }
Goto { Time 5 AccelTime 1 DecelTime 2 AccelPower 3 }
"#;
        let s = parse(src);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        let ops: Vec<&Op> = s.commands.iter().map(|c| &c.op).collect();
        assert!(matches!(ops[0], Op::UserControl { kind: InputKind::Move, mode: LockMode::Disabled, .. }));
        assert!(matches!(ops[1], Op::UserControl { kind: InputKind::Time, mode: LockMode::Free, .. }));
        let Op::UserControl { mode: LockMode::Limited, limits, .. } = ops[2] else { panic!("{:?}", ops[2]) };
        assert_eq!((limits.center.as_deref(), limits.dist_km, limits.speed_kms), (Some("Earth"), Some(1e5), Some(10.0)));
        let Op::HighlightWidget(h) = ops[3] else { panic!() };
        assert_eq!((h.widget.as_str(), h.time, h.color), ("Wiki|Object name", Some(1.0), Some([0.5, 0.0, 0.0, 1.0])));
        assert!(matches!(ops[4], Op::HighlightWidget(h) if h.widget.is_empty()));
        assert_eq!(ops[5], &Op::SetWidgetStyle { widget: "Message box".into(), pos: Some([0.7, 0.2]), pos_pix: None, visible: None });
        assert_eq!(ops[6], &Op::ShowDialog { dialog: "Settings".into(), tab: Some("Graphics".into()) });
        assert_eq!(ops[7], &Op::HideDialog("Wiki".into()));
        assert_eq!(ops[8], &Op::HideAllToolbars);
        assert_eq!(ops[9], &Op::PlayMusic { file: "Lokijar - Quasar Medusa.ogg".into(), crossfade: None });
        assert_eq!(ops[10], &Op::PauseMusic { crossfade: Some(3.0) });
        assert_eq!(ops[11], &Op::Fly { axis: [0.0, 0.0, 1.0], speed: 100e3, fade: 1.0 });
        assert_eq!(ops[12], &Op::StopFly { fade: 2.0 });
        assert_eq!(ops[13], &Op::Turn { axis: [0.0, 1.0, 0.0], speed: 5.0, fade: 2.0 });
        assert_eq!(ops[14], &Op::Speed(0.5 * PC_M));
        assert_eq!(ops[15], &Op::Speed(500e3));
        assert_eq!(ops[16], &Op::ScreenFade { out: true, time: Some(2.0), color: Some([0.0, 0.0, 0.0, 1.0]) });
        let Op::Goto(g) = ops[17] else { panic!() };
        assert_eq!((g.accel_time, g.drift_time, g.decel_time, g.accel_power), (Some(1.0), None, Some(2.0), Some(3.0)));
    }

    #[test]
    fn trigger_parameters() {
        let src = r#"
WaitTrigger "Control|Move" { Direction (0.0, 0.0, s-1.0) RangeKm (100, 1.0e9) }
WaitTrigger "Time" { Mode "InRange" TimeRange ("2016.11.17 00:38:00", "2016.11.17 03:04:00") }
WaitTrigger "Object|Approach" { Object "Sun|Jupiter" RangeKm (5e6, 5e7) LonLatRange (-180, 180, 70, 90) }
WaitTrigger "GUI|ChangeValue" { Dialog "Star browser|Filter" Value 3 ActivationDelay 0.5 }
"#;
        let s = parse(src);
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        let t = |i: usize| match &s.commands[i].op {
            Op::WaitTrigger(t) => t.clone(),
            o => panic!("{o:?}"),
        };
        assert_eq!(t(0).direction, Some([0.0, 0.0, -1.0]));
        let (a, b) = t(1).time_range.unwrap();
        assert!(b > a && (b - a - (2.0 + 26.0 / 60.0) / 24.0).abs() < 1e-6, "{a} {b}");
        assert_eq!(t(2).lonlat_range, Some([-180.0, 180.0, 70.0, 90.0]));
        assert_eq!((t(3).value, t(3).activation_delay), (Some(3.0), Some(0.5)));
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
        // Open targets end the script instead of jumping back to the top.
        let s = parse("if { A == 1 }\nWait 1\nelif { A > 5 }\nWait 2\nelse\nWait 3\n");
        assert!(s.commands.iter().all(|c| match c.op {
            Op::If { on_false, .. } | Op::Elif { on_false, .. } => on_false > 0,
            Op::Jump(t) => t > 0,
            _ => true,
        }));
        assert_eq!(s.commands[2].op, Op::Jump(s.commands.len()));
        let s = parse("loop 3\nWait 1\nbreak\n");
        assert_eq!(s.warnings.len(), 1);
        assert_eq!(s.commands[0].op, Op::Loop { count: Some(3), end: 3 });
        assert_eq!(s.commands[2].op, Op::Break { end: 3 });
    }

    #[test]
    fn conditions_with_non_ascii_comments() {
        let s = parse("if { LessonPassed < 0 } // урок\nWait 1\nendif\nif { T > 5 } // 5 °C\nendif\n");
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        let Op::If { cond, .. } = &s.commands[0].op else { panic!() };
        assert_eq!(cond.op, CmpOp::Lt);
        let Op::If { cond, .. } = &s.commands[3].op else { panic!() };
        assert_eq!(cond.op, CmpOp::Gt);
    }

    #[test]
    fn multi_line_conditions() {
        let s = parse("if\n{\n\tVar < 7\n}\nWait 1\nelif\n{ Var >= 9 }\nWait 2\nendif\nWait 3 // x < y\n");
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        let Op::If { cond, on_false } = &s.commands[0].op else { panic!("{:?}", s.commands[0]) };
        assert_eq!((cond.var.as_str(), cond.op, *on_false), ("Var", CmpOp::Lt, 3));
        let Op::Elif { cond, .. } = &s.commands[3].op else { panic!("{:?}", s.commands[3]) };
        assert_eq!((cond.var.as_str(), cond.op), ("Var", CmpOp::Ge));
        assert_eq!(s.commands.len(), 7);
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
