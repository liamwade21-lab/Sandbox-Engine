//! Where a voice sits and how it reaches the stereo output (T5d-5, T5d-7): Wwise's own pan rules as Elite's exe holds them,
//! as pure maths. Every voice's way to the output is one matrix of gains (`[input channel][output channel]`, two outputs),
//! worked out here for four cases: a voice not positioned in space or one that is, on a route that mixes in stereo or one
//! that mixes in 7.1.4. The mixer makes each sound's [`Placement`] once (its [`Route`]: the layout its output bus mixes in,
//! its 2D panner, centre share and pan values; whether it is spatialised; its attenuation's distance curves) and hands a
//! voice the matrix of a voice at the listener; a voice on an emitter the game placed has its matrix worked out by the
//! player from the emitter's place ([`Placement::at`]), once a block on the audio thread (nothing here allocates; the two
//! speaker meshes are built once, off that thread, the first time a placement is made).
//!
//! The rules are ported step for step from the studies' models, which follow Elite's code (`re/elite/work/t5g/panmodel.py`,
//! T5g; `panmodel3d.py`, T5g-2; the notes `re/elite/notes/sidewinder_sound_events.md`, sections T5g and T5g-2), with the
//! same constants (the exe's addresses beside them) and their test vectors (`place_vectors.rs`):
//! - **Elite's output set-up** (T5g section 0, C): panning rule Speakers, the default speaker angles (front 45, side 90,
//!   rear 135, height 54.7 degrees: `FUN_144a4db50`), the device's own layout, stereo for the user (D: the sessions'
//!   recordings are 2-channel).
//! - **A route's layout** (T5g-2 section 2, C): a bus mixes in its own channel configuration when it has one, else its
//!   parent's, the master in the device's (`FUN_144a58c00`). Four of the ship's buses are 12-channel 7.1.4; the layout
//!   changes only where one of them connects into its stereo parent, by Elite's down-mix table ([`DOWN_MIX_L`]). Panning
//!   into the 12 speakers and the down-mix are both linear, so they fold into the voice's one matrix: the 12-channel mix
//!   itself is never built.
//! - **Not positioned in space** (2D, or 3D with spatialisation None: T5g section 1, T5g-2 section 4, C):
//!   `GetSpeakerVolumes2DPan`, direct speaker assignment or balance-fade by PAN_LR (and PAN_FR, CenterPCT on 7.1.4).
//! - **Positioned in space on a stereo route** (T5g section 2, C): the emitter's direction in the listener's axes, a point
//!   cloud for spread (8 points for mono, 4 a channel for stereo), Wwise's spread and focus transforms, planar VBAP between
//!   virtual speakers at +-45 degrees with mirror copies at +-135, each input channel's column at power 1 / n.
//! - **Positioned in space on a 7.1.4 route** (T5g-2 section 1, C): the speakers on the sphere with a virtual one below,
//!   their Delaunay triangulation in two charts, 3D VBAP in every triangle that holds the direction, the 3D point cloud
//!   (48 points for mono, 32 a channel for stereo), the centre share blending the meshes with and without the centre
//!   speaker, then the down-mix.
//!
//! Kept from the studies where they grade a point E (not settled): the listener's frame is the head's axes taken directly
//! (x right, y up, z ahead: the notes' advice; how Elite builds its listener matrix was not found); the 3D spatialisation
//! mix (`+0x120`) is 100, no blend with the 2D pan (8 layer containers carry a curve on PositioningTypeBlend whose game
//! parameter sits at its default 0, where the curve gives 100); the ray's scaling factor is 1; the emitter's orientation,
//! which only turns the spread cloud, is the listener's (the game hands no emitter orientation; `legs.py` took the ship's,
//! the same with the head straight). A source of more than two channels (none shipped) pans its first two.
//!
//! What the shipped data uses (`re/elite/work/sound/t5d5/positioning.py` over the 775 Sound objects on the 227 shipped
//! events' paths): 774 are 3D (listener-relative routing), 1 is 2D; 626 are on the emitter's position and spatialise by
//! position and orientation (507 with direct speaker assignment, 119 with the balance-fade panner), 92 are on the emitter's
//! position without spatialisation, 56 follow a path automation relative to the listener (they pan as voices at the
//! listener: path automation is not built). Attenuation: the distance curves for volume (dry), low-pass, high-pass,
//! spread and focus are built; not built (listed): the wet (aux) curves, cones (no emitter orientation is known).

use std::sync::LazyLock;

use super::curve;
use super::data::{AttCurve, Attenuation};

/// Added to each pair gain before squaring (`0x14506b228`).
const EPS: f64 = 1.1920928955078125e-07;
/// Float32 pi (`0x144dee488`).
const PI: f64 = 3.1415927410125732;
const DEG: f64 = PI / 180.0;
/// The default speaker angles (`FUN_144a4db50`): front, side, rear (radians, as stored at device+0x40).
const FRONT: f64 = 45.0 * DEG;
const SIDE: f64 = 90.0 * DEG;
const REAR: f64 = 135.0 * DEG;
/// The smallest angle between speakers on the defaults (`FUN_144a51ba0`; its inverse at device+0x4c).
const MIN_ANGLE: f64 = 45.0 * DEG;
/// The height speakers' elevation (`0x425acccd`, device+0x50 in radians).
const HEIGHT: f64 = 54.70000076293945 * DEG;
/// The virtual bottom speaker (`0xb33bbd2e`, `0x80000000`, `0xbf800000`).
const VIRT_BOTTOM: [f64; 3] = [-4.371139e-08, -0.0, -1.0];
/// The triangulation's incircle tolerance (`0x144e299a4`), the triangle search's pad (`0x144de1654`), the 3D VBAP's gain
/// floor (`0x144dee018`) and its smallest determinant (`0x1457e84e8`).
const INCIRCLE_TOL: f64 = 0.009999999776482582;
const SEARCH_PAD: f64 = 0.05000000074505806;
const GAIN_FLOOR: f64 = -9.999999747378752e-06;
const TINY: f64 = 9.999999350456404e-39;
/// The 2D panner's mono gain on a stereo output (a literal in `FUN_144a4e3c0`'s mono branch) and the down-mix's 0.7071.
const SQRT_HALF: f64 = 0.7071067690849304;
/// PAN_LR and PAN_FR to the 0..1 balance (`FUN_144a2e100`: x 0.005 + 0.5) and the balance-fade's third.
const PAN_SCALE: f64 = 0.004999999888241291;
const THIRD: f64 = 0.3333333432674408;
/// The emitter's orientation in the listener's axes (front, top): the listener's own (E, the module's notes).
const EMITTER_FRONT: [f64; 3] = [0.0, 0.0, 1.0];
const EMITTER_TOP: [f64; 3] = [0.0, 1.0, 0.0];

/// Wwise's channel masks of the two layouts the ship's buses use (a channel configuration word's top 20 bits).
pub const MASK_STEREO: u32 = 0x3;
pub const MASK_714: u32 = 0x2d63f;

/// The layout a voice's route mixes in: its output bus's own, else its parent's, the master in the device's (stereo).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Stereo,
    /// 12 channels, FL FR FC LFE BL BR SL SR HFL HFR HBL HBR, down-mixed to stereo where its bus connects into a stereo
    /// parent.
    Height714,
}

impl Layout {
    /// A bus's own layout from its channel configuration word (bank `uChannelConfig`: 8 bits the channel count, 4 the
    /// configuration type, 20 the channel mask): None when the count is 0 ("same as the parent"). A layout the panner does
    /// not build is an error.
    pub fn of_config(word: u32) -> Result<Option<Layout>, String> {
        let (count, kind, mask) = (word & 0xff, (word >> 8) & 0xf, word >> 12);
        match (count, kind, mask) {
            (0, _, _) => Ok(None),
            (2, 1, MASK_STEREO) => Ok(Some(Layout::Stereo)),
            (12, 1, MASK_714) => Ok(Some(Layout::Height714)),
            _ => Err(format!("channel configuration {word:#010x} is not a layout the panner builds (stereo or 7.1.4)")),
        }
    }

    /// The layout a route mixes in from its buses' own layouts, its output bus first and the master last (`FUN_144a58c00`:
    /// the bus's own, else its parent's, the master's the device's). Only the change 7.1.4 to stereo is built (the four
    /// buses' connections into their parents); a stereo bus under a 7.1.4 one would need an up-mix, and a 7.1.4 master a
    /// down-mix to the device that was not read: errors.
    pub fn of_route(own: &[Option<Layout>]) -> Result<Layout, String> {
        let mut parent = Layout::Stereo;
        for (i, o) in own.iter().enumerate().rev() {
            let now = o.unwrap_or(parent);
            if parent == Layout::Height714 && now == Layout::Stereo {
                return Err("a stereo bus under a 7.1.4 one (an up-mix is not built)".into());
            }
            if i + 1 == own.len() && now == Layout::Height714 {
                return Err("a 7.1.4 master bus (its down-mix to the device is not built)".into());
            }
            parent = now;
        }
        Ok(parent)
    }
}

/// Wwise's 2D panner (positioning +0x10c & 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panner {
    DirectSpeakerAssignment,
    BalanceFade,
}

impl Panner {
    pub fn of_name(name: &str) -> Result<Panner, String> {
        match name {
            "DirectSpeakerAssignment" => Ok(Panner::DirectSpeakerAssignment),
            "BalanceFadeHeight" => Ok(Panner::BalanceFade),
            other => Err(format!("panner {other} is not built")),
        }
    }
}

/// How a sound's voices reach the output: fixed for the sound ([`super::SoundBank::route`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    pub layout: Layout,
    pub panner: Panner,
    /// CenterPCT, PAN_LR, PAN_FR (percent, -100..100 for the pans) of the positioning in force.
    pub centre_pct: f64,
    pub pan_lr: f64,
    pub pan_fr: f64,
}

impl Default for Route {
    /// A stereo route, direct speaker assignment, no centre share, centred.
    fn default() -> Route {
        Route { layout: Layout::Stereo, panner: Panner::DirectSpeakerAssignment, centre_pct: 0.0, pan_lr: 0.0, pan_fr: 0.0 }
    }
}

/// A sound's way to the output (the mixer makes it once a sound, [`super::VoiceUpdate::place`] hands it to a placed voice).
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// Positioned in space (listener-relative routing with 3D spatialisation by position, or position and orientation):
    /// Wwise's 3D panner. False: the 2D panner.
    pub pan: bool,
    pub route: Route,
    /// The attenuation's distance curves (x metres): VolumeDry (dB scaling), LowPassFilter, HighPassFilter (0-100 added to
    /// the voice's values), Spread and Focus (percent). None where the share set uses none, or attenuation is off.
    pub volume: Option<AttCurve>,
    pub lowpass: Option<AttCurve>,
    pub highpass: Option<AttCurve>,
    pub spread: Option<AttCurve>,
    pub focus: Option<AttCurve>,
    /// The matrix of a voice at the listener, for a mono and a stereo source (worked out when made).
    at_listener: [[[f32; 2]; 2]; 2],
}

/// What a placement gives at one place: the distance's gain (linear), its low-pass and high-pass shares (0-100, added to
/// the voice's), and the matrix (`matrix[input channel][output channel]`, the distance's gain in it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaceNow {
    pub gain: f32,
    pub lowpass: f32,
    pub highpass: f32,
    pub matrix: [[f32; 2]; 2],
}

impl Placement {
    /// A sound positioned by `pan` with attenuation `att`, on a stereo route with direct speaker assignment; None when
    /// neither applies.
    pub fn new(pan: bool, att: Option<&Attenuation>) -> Option<Placement> {
        (pan || att.is_some()).then(|| Placement::routed(Route::default(), pan, att))
    }

    /// A sound's way to the output: its route, whether it is positioned in space, its attenuation (None when attenuation
    /// is off or the chain names none).
    pub fn routed(route: Route, pan: bool, att: Option<&Attenuation>) -> Placement {
        LazyLock::force(&MESHES);
        let curve = |k: &str| att.and_then(|a| a.curve_to_use.get(k).copied().filter(|i| *i >= 0).and_then(|i| a.curves.get(i as usize)).cloned());
        let mut p = Placement {
            pan,
            route,
            volume: curve("VolumeDry"),
            lowpass: curve("LowPassFilter"),
            highpass: curve("HighPassFilter"),
            spread: curve("Spread"),
            focus: curve("Focus"),
            at_listener: [[[0.0; 2]; 2]; 2],
        };
        p.at_listener = [p.pan_at([0.0; 3], 1), p.pan_at([0.0; 3], 2)];
        p
    }

    /// One curve at a distance (0 without the curve).
    fn read(c: &Option<AttCurve>, d: f64) -> f64 {
        c.as_ref().map_or(0.0, |c| curve::eval(&c.points, c.scaling, d))
    }

    /// The distance's volume (dB), low-pass and high-pass shares, spread and focus (percent) at `d` metres.
    pub fn volume_db(&self, d: f64) -> f64 {
        Self::read(&self.volume, d)
    }

    pub fn lowpass_at(&self, d: f64) -> f64 {
        Self::read(&self.lowpass, d)
    }

    pub fn highpass_at(&self, d: f64) -> f64 {
        Self::read(&self.highpass, d)
    }

    pub fn spread_at(&self, d: f64) -> f64 {
        Self::read(&self.spread, d)
    }

    pub fn focus_at(&self, d: f64) -> f64 {
        Self::read(&self.focus, d)
    }

    /// The matrix of a voice of `channels` channels at the listener (an emitter the game has not placed: Wwise takes an
    /// emitter at the listener as straight ahead, `FUN_144a4ac30`); its distance's share is the mixer's, at 0 m.
    pub fn listener_matrix(&self, channels: usize) -> [[f32; 2]; 2] {
        self.at_listener[channels.clamp(1, 2) - 1]
    }

    /// At `rel`, the emitter's place from the listener in the listener's axes (metres; x right, y up, -z ahead), for a
    /// source of `channels` channels. A place not finite counts as the listener's own (no distance, straight ahead).
    pub fn at(&self, rel: [f32; 3], channels: usize) -> PlaceNow {
        let [x, y, z] = if rel.iter().all(|v| v.is_finite()) { rel.map(f64::from) } else { [0.0; 3] };
        let d = (x * x + y * y + z * z).sqrt();
        let gain = curve::db_to_gain(self.volume_db(d)) as f32;
        let mut matrix = self.pan_at([x, y, -z], channels);
        for row in &mut matrix {
            row.iter_mut().for_each(|g| *g *= gain);
        }
        PlaceNow { gain, lowpass: self.lowpass_at(d) as f32, highpass: self.highpass_at(d) as f32, matrix }
    }

    /// The matrix without the distance's gain of a source of `channels` channels whose emitter is at `d` in Wwise's
    /// listener space (x right, y up, z ahead; metres). The spread and focus curves read the distance in the listener's
    /// right-ahead plane (`FUN_144a2e2d0`: the up component left out; the scaling factor 1).
    fn pan_at(&self, d: [f64; 3], channels: usize) -> [[f32; 2]; 2] {
        let n = channels.clamp(1, 2);
        let r = &self.route;
        let g = if self.pan {
            let h = (d[0] * d[0] + d[2] * d[2]).sqrt();
            let (spread, focus) = (self.spread_at(h).clamp(0.0, 100.0), self.focus_at(h).clamp(0.0, 100.0));
            match r.layout {
                Layout::Stereo => planar(d, n, spread, focus, EMITTER_FRONT, EMITTER_TOP),
                Layout::Height714 => height(d, n, spread, r.centre_pct, focus, EMITTER_FRONT, EMITTER_TOP).map(down_mix),
            }
        } else {
            unplaced(r, n)
        };
        let m = g.map(|row| row.map(|v| v as f32));
        if m.iter().flatten().all(|v| v.is_finite()) { m } else { unplaced(r, n).map(|row| row.map(|v| v as f32)) }
    }
}

// ------------------------------------------------------------------------------------------------ not positioned in space

/// The balance of a pan value (`clamp(pan x 0.005 + 0.5, 0, 1)`).
fn balance(pan: f64) -> f64 {
    (pan * PAN_SCALE + 0.5).clamp(0.0, 1.0)
}

/// A share's square root (a share the float rounding leaves a hair under 0 is 0: the exe's float32 sums give 0 there).
fn root(v: f64) -> f64 {
    v.max(0.0).sqrt()
}

/// A voice not positioned in space (`GetSpeakerVolumes2DPan` = `FUN_144a4e130`, then the route's down-mix): per input
/// channel (L, R).
fn unplaced(r: &Route, n: usize) -> [[f64; 2]; 2] {
    let x = balance(r.pan_lr);
    match (r.layout, r.panner, n) {
        // T5g section 1 (`FUN_144a4e3c0`): direct assignment, mono 0.7071 on both whatever the pan, stereo 1:1.
        (Layout::Stereo, Panner::DirectSpeakerAssignment, 1) => [[SQRT_HALF, SQRT_HALF], [0.0; 2]],
        (Layout::Stereo, Panner::DirectSpeakerAssignment, _) => [[1.0, 0.0], [0.0, 1.0]],
        (Layout::Stereo, Panner::BalanceFade, 1) => [[root(1.0 - x), root(x)], [0.0; 2]],
        (Layout::Stereo, Panner::BalanceFade, _) => [[root(1.0 - x), 0.0], [0.0, root(x)]],
        (Layout::Height714, panner, n) => unplaced_714(n, panner, r.centre_pct, r.pan_lr, r.pan_fr),
    }
}

/// Not spatialised into a 7.1.4 bus (`FUN_144a4e3c0` into mask 0x2d63f; balance-fade lines 120-149 for mono,
/// `FUN_144a534d0` for stereo), then the down-mix (`panmodel3d.unplaced_714`).
fn unplaced_714(n: usize, panner: Panner, centre_pct: f64, pan_lr: f64, pan_fr: f64) -> [[f64; 2]; 2] {
    let c = centre_pct * 0.01;
    let (x, y) = (balance(pan_lr), balance(pan_fr));
    let mut rows = [[0.0; 11]; 2];
    match panner {
        Panner::DirectSpeakerAssignment if n == 1 => {
            rows[0][0] = ((1.0 - c) * 0.5).sqrt();
            rows[0][1] = rows[0][0];
            rows[0][2] = c.sqrt();
        }
        Panner::DirectSpeakerAssignment => return [[1.0, 0.0], [0.0, 1.0]],
        Panner::BalanceFade => {
            let rear = ((3.0 - y * 4.0) * THIRD).max(0.0);
            let front = ((y * 4.0 - 1.0) * THIRD).max(0.0);
            let side = 1.0 - (rear + front);
            if n == 1 {
                let (f31, f32_) = if 0.5 < x { ((x - 1.0) * c + x, 1.0 - x) } else { ((1.0 - c) * x, x) };
                let f32_ = f32_ * (c + c);
                rows[0][..7].copy_from_slice(&[
                    root(((1.0 - f31) - f32_) * front),
                    root(front * f31),
                    root(front * f32_),
                    root(rear * (1.0 - x)),
                    root(rear * x),
                    root(side * (1.0 - x)),
                    root(side * x),
                ]);
            } else {
                let k = [root((1.0 - x) * front), root(front * x), root(rear * (1.0 - x)), root(rear * x), root(side * (1.0 - x)), root(side * x)];
                rows[0][..7].copy_from_slice(&[k[0], 0.0, 0.0, k[2], 0.0, k[4], 0.0]);
                rows[1][..7].copy_from_slice(&[0.0, k[1], 0.0, 0.0, k[3], 0.0, k[5]]);
            }
        }
    }
    rows.map(down_mix)
}

// ------------------------------------------------------------------------------ the shared steps of the 3D panner (T5g s2)

/// The parabola sine with its 0.225 correction (`FUN_144a4a320`, `FUN_144a49db0`; t in 0..pi).
fn fast_sin(t: f64) -> f64 {
    let y = t * 1.2732394933700562 - t.abs() * t * 0.40528470277786255;
    (y.abs() * y - y) * 0.22499999403953552 + y
}

fn asin_poly(z: f64) -> f64 {
    ((0.7190880179405212 * z * z - 0.3578599989414215) * z * z + 1.0858650207519531) * z
}

fn sin_poly(t: f64) -> f64 {
    ((0.00833216030150652 * t * t - 0.16666655242443085) * t * t + 1.0) * t
}

/// Spread (or focus) up to a half (`FUN_144a4a320`): z + 1 / (sin(pi s) + 1e-4) after holding z at -0.999, normalised.
fn spread_low(p: [f64; 3], s: f64) -> [f64; 3] {
    let k = 1.0 / (fast_sin(s * PI) + 9.999999747378752e-05);
    let [x, y, z] = p;
    let z = z.max(-0.9990000128746033) + k;
    let r = (x * x + y * y + z * z).sqrt();
    [x / r, y / r, z / r]
}

/// Spread (or focus) above a half (`FUN_144a49f10`): latitude asin(z) s + (1 - s) pi/2, the lateral part keeping its
/// direction.
fn spread_high(p: [f64; 3], s: f64) -> [f64; 3] {
    let [x, y, z] = p;
    let r2 = x * x + y * y;
    let mut zc = z.max(-1.0);
    if r2 - 9.999999747378752e-05 < 0.0 {
        zc = if zc >= 0.0 { 1.0 } else { -1.0 };
    }
    let t = asin_poly(zc) * s + (1.0 - s) * 1.5707963705062866;
    let sn = sin_poly(t);
    let c2 = 1.0 - sn * sn;
    let scale = c2 / (c2 * r2).max(9.999999747378752e-05).sqrt();
    [x * scale, y * scale, sn]
}

fn spread_xform(p: [f64; 3], s: f64) -> [f64; 3] {
    if s <= 0.5 { spread_low(p, s) } else { spread_high(p, s) }
}

/// Each input channel's turn about y (degrees, table 0x1457e5770): mono 0; stereo left -90, right +90.
fn channel_angle(n: usize, ch: usize) -> f64 {
    if n == 1 { 0.0 } else if ch == 0 { -90.0 } else { 90.0 }
}

/// A point turned about y (`FUN_144a4ba50`).
fn rot_y(p: [f64; 3], deg: f64) -> [f64; 3] {
    let (s, c) = ((deg * DEG).sin(), (deg * DEG).cos());
    let [x, y, z] = p;
    [x * c + z * s, y, -x * s + z * c]
}

/// The rotation taking +z to the unit direction of `d` (`FUN_144a4ac30`, Rodrigues), as three columns: the identity for
/// an emitter at the listener or straight ahead, diag(-1, 1, -1) straight behind.
fn dir_matrix(d: [f64; 3]) -> [[f64; 3]; 3] {
    const IDENTITY: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let [x, y, z] = d;
    let r = (x * x + y * y + z * z).sqrt();
    if r <= 0.0 {
        return IDENTITY;
    }
    let (a, b, c) = (x / r, y / r, z / r);
    let ny = -b;
    let s2 = ny * ny + a * a;
    if s2 <= EPS {
        // (0x144ded02c = -0.0, 0x144de16b8 = -1.0.)
        return if c >= 0.0 { IDENTITY } else { [[-1.0, 0.0, -0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]] };
    }
    let s = s2.sqrt();
    let (ax, ay) = (a / s, ny / s);
    let one_c = 1.0 - c;
    let m1 = ax * ay * one_c;
    [[ay * ay * one_c + c, m1, -(ax * s)], [m1, ax * ax * one_c + c, ay * s], [ax * s, -(ay * s), c]]
}

fn apply(cols: &[[f64; 3]; 3], p: [f64; 3]) -> [f64; 3] {
    let [a, b, c] = cols;
    let [x, y, z] = p;
    [x * a[0] + y * b[0] + z * c[0], x * a[1] + y * b[1] + z * c[1], x * a[2] + y * b[2] + z * c[2]]
}

fn dot(u: [f64; 3], v: [f64; 3]) -> f64 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}

/// The most points a cloud has (the 3D set for mono: 6 rings of 8).
const MAX_POINTS: usize = 48;

/// A point cloud (`FUN_144a4c6e0`): `count` points in `at`.
struct Cloud {
    at: [[f64; 3]; MAX_POINTS],
    count: usize,
}

impl Cloud {
    /// The planar set: arc = pi / n, N = int(arc / 45 degrees + 1.01) up to a multiple of 4 (mono 8, stereo 4), h = arc / N,
    /// theta_k = (N - 1) h - 2 k h, the point (sin(-theta), 0, cos(-theta)).
    fn planar(n: usize) -> Cloud {
        let arc = (1.0 / n as f64) * PI;
        let mut count = (arc * (1.0 / MIN_ANGLE) + 1.0099999904632568) as usize;
        if count & 3 != 0 {
            count += 4 - (count & 3);
        }
        let count = count.min(MAX_POINTS);
        let h = arc / count as f64;
        let mut c = Cloud { at: [[0.0; 3]; MAX_POINTS], count };
        let mut th = (count - 1) as f64 * h;
        for p in &mut c.at[..count] {
            *p = [(-th).sin(), 0.0, (-th).cos()];
            th -= 2.0 * h;
        }
        c
    }

    /// The 3D set (bit 31): rings N = int(arc / 45 degrees + 1.01) made even (mono 6, stereo 4) of equal z steps over the cap
    /// of half-angle arc round +z, 8 azimuths each.
    fn sphere(n: usize) -> Cloud {
        const AZ: [f64; 8] = [0.0, 0.7853981852531433, 1.5707963705062866, 2.356194496154785, PI, 3.9269909858703613, 4.71238899230957, 5.4977874755859375];
        let arc = (1.0 / n as f64) * PI;
        let mut rings = (arc * (1.0 / MIN_ANGLE) + 1.0099999904632568) as usize;
        rings += rings & 1;
        let rings = rings.min(MAX_POINTS / 8);
        let c = arc.cos();
        let step = (1.0 - c) / rings as f64;
        let mut z = step * 0.5 + c;
        let mut cloud = Cloud { at: [[0.0; 3]; MAX_POINTS], count: rings * 8 };
        for ring in cloud.at[..rings * 8].chunks_exact_mut(8) {
            let r = (1.0 - z * z).sqrt();
            for (p, a) in ring.iter_mut().zip(AZ) {
                *p = [a.cos() * r, a.sin() * r, z];
            }
            z += step;
        }
        cloud
    }

    /// Focus `focus_pct` applied to the set (`FUN_144a49db0` / `FUN_144a49b90`, the spread transforms with 1 - f / 100).
    fn focused(mut self, focus_pct: f64) -> Cloud {
        let f = 1.0 - focus_pct * 0.01;
        if f != 1.0 {
            for p in &mut self.at[..self.count] {
                *p = spread_xform(*p, f);
            }
        }
        self
    }

    fn points(&self) -> &[[f64; 3]] {
        &self.at[..self.count]
    }
}

/// The emitter's axes in the direction frame, rows by (side, top, front) (`FUN_144a528c0`: side = top x front
/// componentwise, `FUN_144a4a820`).
fn emitter_frame(dm: &[[f64; 3]; 3], front: [f64; 3], top: [f64; 3]) -> [[f64; 3]; 3] {
    let side = [top[1] * front[2] - top[2] * front[1], top[2] * front[0] - top[0] * front[2], top[0] * front[1] - top[1] * front[0]];
    [side, top, front].map(|v| [dot(v, dm[0]), dot(v, dm[1]), dot(v, dm[2])])
}

/// Each point the channel's cloud gives in the listener's axes: the point turned by the channel's angle, into the emitter's
/// axes and the direction frame, spread, back by the direction.
fn spread_points(cloud: &Cloud, n: usize, ch: usize, s: f64, dm: &[[f64; 3]; 3], e: &[[f64; 3]; 3], mut each: impl FnMut([f64; 3])) {
    for p in cloud.points() {
        let p = rot_y(*p, channel_angle(n, ch));
        let pr = [0, 1, 2].map(|j| p[0] * e[0][j] + p[1] * e[1][j] + p[2] * e[2][j]);
        each(apply(dm, spread_xform(pr, s)));
    }
}

// ------------------------------------------------------------------------------------- positioned in space, stereo route

/// The 2.0 ring's pairs (`FUN_144a4bc90`): speakers L(-45), R(+45), R(+135), L(-135) (row 2 of 0x145ee0630 maps them to
/// the outputs), each pair's inverse as (m0, m1, m2, m3) and its two outputs.
fn pairs() -> [([f64; 4], usize, usize); 4] {
    let (s0, c0) = FRONT.sin_cos();
    let (s2, c2) = REAR.sin_cos();
    let ring = [([-s0, c0], 0), ([s0, c0], 1), ([s2, c2], 1), ([-s2, c2], 0)];
    std::array::from_fn(|i| {
        let ([x0, z0], o0) = ring[i];
        let ([x1, z1], o1) = ring[(i + 1) % 4];
        let inv = 1.0 / (x0 * z1 - z0 * x1);
        ([z1 * inv, -x1 * inv, -z0 * inv, x0 * inv], o0, o1)
    })
}

/// `FUN_144a4a730`: the first pair whose two gains are both at least 0; its powers normalised to 1, added to `acc`.
fn vbap_add(pairs: &[([f64; 4], usize, usize); 4], x: f64, z: f64, acc: &mut [f64; 2]) {
    for ([m0, m1, m2, m3], o0, o1) in pairs {
        let g1 = x * m0 + z * m1;
        let g2 = x * m2 + z * m3;
        if g1 >= 0.0 && g2 >= 0.0 {
            let a = (g1 + EPS) * (g1 + EPS);
            let b = (g2 + EPS) * (g2 + EPS);
            let k = 1.0 / (a + b);
            acc[*o0] += k * a;
            acc[*o1] += k * b;
            return;
        }
    }
}

/// A source of `n` channels at the direction `d` (listener space) panned to a stereo output (`FUN_144a4f1a0`'s planar
/// branch, `panmodel.gains`): per input channel (L, R).
fn planar(d: [f64; 3], n: usize, spread_pct: f64, focus_pct: f64, front: [f64; 3], top: [f64; 3]) -> [[f64; 2]; 2] {
    let pr = pairs();
    let dm = dir_matrix(d);
    let s = spread_pct * 0.01;
    let mut out = [[0.0; 2]; 2];
    let cloud = (s > 0.0).then(|| Cloud::planar(n).focused(focus_pct));
    let e = emitter_frame(&dm, front, top);
    for (ch, row) in out.iter_mut().enumerate().take(n) {
        let mut acc = [0.0; 2];
        let points = match &cloud {
            Some(c) => {
                spread_points(c, n, ch, s, &dm, &e, |q| vbap_add(&pr, q[0], q[2], &mut acc));
                c.count
            }
            None => {
                let q = apply(&dm, [0.0, 0.0, 1.0]);
                vbap_add(&pr, q[0], q[2], &mut acc);
                1
            }
        };
        let k = 1.0 / n as f64 / points as f64;
        *row = acc.map(|a| (a * k).sqrt());
    }
    out
}

// ------------------------------------------------------------------------------------ positioned in space, 7.1.4 route

/// Elite's down-mix of a 7.1.4 bus into its stereo parent (`FUN_144a4b050`, table 0x1457e4f50 by channel bit; the LFE row
/// taken out first and dropped): FL FR FC BL BR SL SR HFL HFR HBL HBR to the left and to the right. Not power-normalised.
pub const DOWN_MIX_L: [f64; 11] = [1.0, 0.0, SQRT_HALF, SQRT_HALF, 0.0, SQRT_HALF, 0.0, SQRT_HALF, 0.0, 0.5, 0.0];
pub const DOWN_MIX_R: [f64; 11] = [0.0, 1.0, SQRT_HALF, 0.0, SQRT_HALF, 0.0, SQRT_HALF, 0.0, SQRT_HALF, 0.0, 0.5];

fn down_mix(row: [f64; 11]) -> [f64; 2] {
    [row.iter().zip(DOWN_MIX_L).map(|(g, k)| g * k).sum(), row.iter().zip(DOWN_MIX_R).map(|(g, k)| g * k).sum()]
}

/// Most points of a mesh (11 speakers and the virtual bottom) and most triangles of one list.
const MESH_POINTS: usize = 12;
const MAX_TRIS: usize = 16;

/// A kept triangle: its points' indices (ascending) and its circumcircle in its chart (centre, radius).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Tri {
    idx: [usize; 3],
    cx: f64,
    cy: f64,
    r: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Tris {
    at: [Tri; MAX_TRIS],
    len: usize,
}

impl Tris {
    fn push(&mut self, t: Tri) {
        if self.len < MAX_TRIS {
            self.at[self.len] = t;
            self.len += 1;
        }
    }

    fn all(&self) -> &[Tri] {
        &self.at[..self.len]
    }

    fn has(&self, idx: [usize; 3]) -> bool {
        self.all().iter().any(|t| t.idx == idx)
    }
}

/// An azimuth and elevation wrapped as the exe wraps them.
fn wrap(mut az: f64, mut el: f64) -> [f64; 2] {
    if az > PI {
        az -= 2.0 * PI;
    }
    if az < -PI {
        az += 2.0 * PI;
    }
    if el > PI / 2.0 {
        el -= PI;
    }
    if el < -PI / 2.0 {
        el += PI;
    }
    [az, el]
}

fn asin_clamped(v: f64) -> f64 {
    v.clamp(-1.0, 1.0).asin()
}

/// The second chart (poles left and right): az2 = atan2(Z, -X), el2 = asin(-Y / |p|).
fn chart2(p: [f64; 3]) -> [f64; 2] {
    let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    wrap(p[2].atan2(-p[0]), asin_clamped(-p[1] / r))
}

/// `FUN_144aa4970`: the triangle is degenerate in the chart, or another point lies inside its circumcircle (beyond the
/// tolerance), the determinant exactly as written there.
fn incircle_rejects(pts: &[[f64; 2]], i: usize, j: usize, k: usize) -> bool {
    let [ax, ay] = pts[i];
    let o = (pts[k][1] - ay) * (pts[j][0] - ax) - (pts[j][1] - ay) * (pts[k][0] - ax);
    if o == 0.0 {
        return true;
    }
    let (b, c) = if o >= 0.0 { (j, k) } else { (k, j) };
    let ([bx, by], [cx, cy]) = (pts[b], pts[c]);
    pts.iter().enumerate().filter(|(d, _)| ![i, b, c].contains(d)).any(|(_, &[dx, dy])| {
        let f16 = ax * ax - dx * dx + ay * ay - dy * dy;
        let f17 = bx * bx - dx * dx + by * by - dy * dy;
        let f15 = cx * cx - dx * dx + cy * cy - dy * dy;
        let det = (f17 * (ay - dy) * (cx - dx) + (by - dy) * (ax - dx) * f15 + (bx - dx) * f16 * (cy - dy))
            - ((by - dy) * f16 * (cx - dx) + (bx - dx) * (ay - dy) * f15 + f17 * (ax - dx) * (cy - dy));
        INCIRCLE_TOL < det
    })
}

/// `FUN_144aa53e0`'s circumcircle, as written (the points in index order).
fn circumcircle(pts: &[[f64; 2]], i: usize, j: usize, k: usize) -> (f64, f64, f64) {
    let ([az_i, el_i], [az_j, el_j], [az_k, el_k]) = (pts[i], pts[j], pts[k]);
    let f28 = el_k * el_k + az_k * az_k;
    let f24 = el_j * el_j + az_j * az_j;
    let f25 = el_i * el_i + az_i * az_i;
    let f23 = (el_i * az_k + az_i * el_j + az_j * el_k) - (el_j * az_k + el_i * az_j + az_i * el_k);
    let f27 = (f28 * el_j + f24 * el_i + f25 * el_k) - (f28 * el_i + f25 * el_j + f24 * el_k);
    let f26 = (f28 * az_i + f25 * az_j + f24 * az_k) - (f28 * az_j + az_i * f24 + f25 * az_k);
    let f22 = -0.5 / f23;
    let inner = (f26 * f26 + f27 * f27) - ((el_k * az_i * f24 + el_i * az_j * f28 + f25 * el_j * az_k) - (el_k * f25 * az_j + az_i * el_j * f28 + f24 * el_i * az_k)) * f23 * 4.0;
    (f27 * f22, f22 * f26, inner.max(0.0).sqrt() * (0.5 / f23.abs()))
}

/// The speakers of a 7.1.4 bus with or without its centre (`FUN_144a4c3e0`, mask 0x2d63f): (azimuth, elevation) in
/// channel order without the LFE, the azimuth positive to the LEFT.
fn speakers_714(with_fc: bool) -> ([[f64; 2]; MESH_POINTS - 1], usize) {
    let all = [
        [FRONT, 0.0],
        [-FRONT, 0.0],
        [0.0, 0.0],
        [REAR, 0.0],
        [-REAR, 0.0],
        [SIDE, 0.0],
        [-SIDE, 0.0],
        [FRONT, HEIGHT],
        [-FRONT, HEIGHT],
        [REAR, HEIGHT],
        [-REAR, HEIGHT],
    ];
    let mut out = [[0.0; 2]; MESH_POINTS - 1];
    let mut n = 0;
    for (i, s) in all.iter().enumerate() {
        if i == 2 && !with_fc {
            continue;
        }
        out[n] = *s;
        n += 1;
    }
    (out, n)
}

/// The speaker mesh of the height panner (`FUN_144aa3ee0` / `FUN_144aa5030` / `FUN_144aa53e0`): `n` real speakers and
/// the virtual bottom at index `n`, in mesh space (X ahead, Y left, Z up); triangle lists A (upper, chart 1), B (with the
/// bottom, chart 1), C (upper, chart 2), D (with the bottom, chart 2; chart 2 keeps only triples chart 1 lacks).
#[derive(Debug)]
struct Mesh {
    n: usize,
    xyz: [[f64; 3]; MESH_POINTS],
    lists: [Tris; 4],
}

/// The meshes without and with the centre speaker, built once.
static MESHES: LazyLock<[Mesh; 2]> = LazyLock::new(|| [Mesh::build(false), Mesh::build(true)]);

impl Mesh {
    fn build(with_fc: bool) -> Mesh {
        let (spk, n) = speakers_714(with_fc);
        let mut xyz = [[0.0; 3]; MESH_POINTS];
        let mut c1 = [[0.0; 2]; MESH_POINTS];
        for i in 0..n {
            let [az, el] = wrap(spk[i][0], spk[i][1]);
            xyz[i] = [az.cos() * el.cos(), az.sin() * el.cos(), el.sin()];
            c1[i] = [az, el];
        }
        xyz[n] = VIRT_BOTTOM;
        c1[n] = [0.0, -PI / 2.0];
        let c2: Vec<[f64; 2]> = xyz[..=n].iter().map(|p| chart2(*p)).collect();
        let (a, b) = Self::triangulate(&c1[..=n], n, None);
        let (c, d) = Self::triangulate(&c2, n, Some((&a, &b)));
        Mesh { n, xyz, lists: [a, b, c, d] }
    }

    /// Every triple i < j < k of the chart's points that no other point's incircle test rejects (and that the first chart
    /// does not already hold), with its circumcircle; those holding the bottom apart.
    fn triangulate(pts: &[[f64; 2]], bottom: usize, prev: Option<(&Tris, &Tris)>) -> (Tris, Tris) {
        let (mut top, mut bot) = (Tris::default(), Tris::default());
        let m = pts.len();
        for i in 0..m.saturating_sub(2) {
            for j in i + 1..m - 1 {
                for k in j + 1..m {
                    let has_b = [i, j, k].contains(&bottom);
                    if incircle_rejects(pts, i, j, k) || prev.is_some_and(|(a, b)| if has_b { b } else { a }.has([i, j, k])) {
                        continue;
                    }
                    let (cx, cy, r) = circumcircle(pts, i, j, k);
                    let t = Tri { idx: [i, j, k], cx, cy, r };
                    if has_b { bot.push(t) } else { top.push(t) }
                }
            }
        }
        (top, bot)
    }

    /// `FUN_144aa4760`: the powers on the real speakers of a direction in mesh space, summed over every triangle that holds
    /// it (not normalised over triangles): at or below the ear plane only the bottom lists, above only the upper ones, each
    /// in its own chart.
    fn gains(&self, p: [f64; 3]) -> [f64; MESH_POINTS - 1] {
        let [x, y, z] = p;
        let r = (x * x + y * y + z * z).sqrt();
        let c1 = wrap(y.atan2(x), asin_clamped(z / r));
        let c2 = chart2(p);
        let lists = if c1[1] <= 0.0 { [(1, c1), (3, c2)] } else { [(0, c1), (2, c2)] };
        let mut out = [0.0; MESH_POINTS - 1];
        for (l, [px, py]) in lists {
            for t in self.lists[l].all() {
                let (dy, dx) = ((t.cy - py).abs(), (t.cx - px).abs());
                if dx + dy <= t.r || (dx <= t.r && dy <= t.r && dx * dx + dy * dy <= (t.r + SEARCH_PAD) * (t.r + SEARCH_PAD)) {
                    if t.idx.contains(&self.n) {
                        self.bottom(t.idx, p, &mut out);
                    } else {
                        self.vbap(t.idx, p, &mut out);
                    }
                }
            }
        }
        out
    }

    /// `FUN_144aa41f0`: 3D VBAP in one triangle (skipped when a gain is under the floor), powers normalised and added.
    fn vbap(&self, [i, j, k]: [usize; 3], p: [f64; 3], out: &mut [f64; MESH_POINTS - 1]) {
        let cross = |u: [f64; 3], v: [f64; 3]| [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let (pi, pj, pk) = (self.xyz[i], self.xyz[j], self.xyz[k]);
        let det = pi[0] * (pj[1] * pk[2] - pj[2] * pk[1]) - pi[1] * (pj[0] * pk[2] - pj[2] * pk[0]) + pi[2] * (pj[0] * pk[1] - pj[1] * pk[0]);
        let (gi, gj, gk) = if TINY <= det || det <= -TINY {
            let g = (dot(p, cross(pj, pk)) / det, dot(p, cross(pk, pi)) / det, dot(p, cross(pi, pj)) / det);
            if g.0 < GAIN_FLOOR || g.1 < GAIN_FLOOR || g.2 < GAIN_FLOOR {
                return;
            }
            g
        } else {
            (0.0, 0.0, 0.0)
        };
        let s = gi * gi + gj * gj + gk * gk;
        if TINY <= s || s <= -TINY {
            out[i] += gi * gi / s;
            out[j] += gj * gj / s;
            out[k] += gk * gk / s;
        }
    }

    /// `FUN_144aa40c0`: a triangle with the bottom speaker by 2D VBAP of its two real speakers on (X, Y) alone, the
    /// bottom's share dropped and the powers renormalised.
    fn bottom(&self, t: [usize; 3], p: [f64; 3], out: &mut [f64; MESH_POINTS - 1]) {
        let u = if t[0] == self.n { t[1] } else { t[0] };
        let v = if t[2] == self.n { t[1] } else { t[2] };
        let ([xu, yu, _], [xv, yv, _]) = (self.xyz[u], self.xyz[v]);
        let det = yv * xu - xv * yu;
        let (gu, gv) = if TINY <= det || det <= -TINY {
            let g = ((yv * p[0] - xv * p[1]) / det, (xu * p[1] - yu * p[0]) / det);
            if g.0 < GAIN_FLOOR || g.1 < GAIN_FLOOR {
                return;
            }
            g
        } else {
            (0.0, 0.0)
        };
        let s = gu * gu + gv * gv;
        if s <= EPS && -EPS <= s {
            return;
        }
        out[u] += gu * gu / s;
        out[v] += gv * gv / s;
    }
}

/// A source of `n` channels at `d` (listener space) panned to the 11 speakers of a 7.1.4 bus (`FUN_144a4f1a0`'s 3D branch,
/// `panmodel3d.gains_714`): per input channel the gains of FL FR FC BL BR SL SR HFL HFR HBL HBR (the LFE gets nothing).
/// `centre_pct` blends the mesh without the centre (0) and with it (100); each channel's powers are scaled to 1 / n.
fn height(d: [f64; 3], n: usize, spread_pct: f64, centre_pct: f64, focus_pct: f64, front: [f64; 3], top: [f64; 3]) -> [[f64; 11]; 2] {
    let [no_fc, fc] = &*MESHES;
    let dm = dir_matrix(d);
    let (s, c) = (spread_pct * 0.01, centre_pct * 0.01);
    let cloud = (s > 0.0).then(|| Cloud::sphere(n).focused(focus_pct));
    let e = emitter_frame(&dm, front, top);
    let mut rows = [[0.0; 11]; 2];
    for (ch, row) in rows.iter_mut().enumerate().take(n) {
        let mut acc = [0.0; 11];
        let mut add = |q: [f64; 3]| {
            let p = [q[2], -q[0], q[1]];
            if c != 1.0 {
                let g = no_fc.gains(p);
                let w = 1.0 - c;
                acc[0] += w * g[0];
                acc[1] += w * g[1];
                for m in 2..10 {
                    acc[m + 1] += w * g[m];
                }
            }
            if c != 0.0 {
                let g = fc.gains(p);
                for m in 0..11 {
                    acc[m] += c * g[m];
                }
            }
        };
        match &cloud {
            Some(cl) => spread_points(cl, n, ch, s, &dm, &e, &mut add),
            None => add(apply(&dm, [0.0, 0.0, 1.0])),
        }
        let tot: f64 = acc.iter().sum();
        if tot > 0.0 {
            *row = acc.map(|a| (a * (1.0 / n as f64) / tot).sqrt());
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::super::place_vectors as v;
    use super::*;
    use crate::elite_sound::data::{Interp, Point, Scaling};

    /// A direction in listener space by azimuth (+ right) and elevation (+ up), degrees (`panmodel.az`).
    fn dir(az: f64, el: f64) -> [f64; 3] {
        let (p, e) = (az * DEG, el * DEG);
        [p.sin() * e.cos(), e.sin(), p.cos() * e.cos()]
    }

    fn near(a: [[f64; 2]; 2], b: [[f64; 2]; 2], tol: f64) -> bool {
        a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - y).abs() <= tol)
    }

    const TOL: f64 = 1e-9;

    /// A constant curve (`value` at every distance).
    fn flat(value: f64) -> AttCurve {
        AttCurve { scaling: Scaling::None, points: vec![Point { x: 0.0, y: value, interp: Interp::Linear }, Point { x: 1000.0, y: value, interp: Interp::Linear }] }
    }

    fn placed(layout: Layout, spread: f64) -> Placement {
        let mut p = Placement::routed(Route { layout, ..Route::default() }, true, None);
        p.spread = Some(flat(spread));
        p.at_listener = [p.pan_at([0.0; 3], 1), p.pan_at([0.0; 3], 2)];
        p
    }

    /// The 2.0 route's planar panner gives the model's gains at every azimuth, elevation and spread of the table, mono and
    /// stereo; constant power for mono.
    #[test]
    fn the_stereo_route_is_the_models() {
        for (n, table) in [(1, v::PLANAR_MONO), (2, v::PLANAR_STEREO)] {
            for &(az, el, s, want) in table {
                let got = planar(dir(az, el), n, s, 0.0, EMITTER_FRONT, EMITTER_TOP);
                assert!(near(got, want, TOL), "{n} ch at {az} / {el}, spread {s}: {got:?} against {want:?}");
                if n == 1 {
                    assert!((got[0][0].powi(2) + got[0][1].powi(2) - 1.0).abs() < 1e-9, "constant power");
                }
            }
        }
        // The corners as the notes give them: ahead 0.7071 each, hard right from 45 to 135, behind mirrored.
        let at = |az: f64| planar(dir(az, 0.0), 1, 0.0, 0.0, EMITTER_FRONT, EMITTER_TOP)[0];
        assert!((at(0.0)[0] - 0.5f64.sqrt()).abs() < 1e-9 && (at(90.0)[1] - 1.0).abs() < 1e-9, "{:?} {:?}", at(0.0), at(90.0));
        assert!((at(150.0)[0] - at(30.0)[0]).abs() < 1e-6, "{:?} {:?}", at(150.0), at(30.0));
    }

    /// The 7.1.4 route (the mesh panner, the centre share, the down-mix) gives the model's gains over the table, mono and
    /// stereo, centre 0, 50 and 100; and the speakers' own gains before the down-mix.
    #[test]
    fn the_height_route_is_the_models() {
        for (n, table) in [(1, v::HEIGHT_MONO), (2, v::HEIGHT_STEREO)] {
            for &(az, el, s, c, want) in table {
                let got = height(dir(az, el), n, s, c, 0.0, EMITTER_FRONT, EMITTER_TOP).map(down_mix);
                assert!(near(got, want, TOL), "{n} ch at {az} / {el}, spread {s}, centre {c}: {got:?} against {want:?}");
            }
        }
        for &(az, el, c, want) in v::SPEAKERS_MONO {
            let got = height(dir(az, el), 1, 0.0, c, 0.0, EMITTER_FRONT, EMITTER_TOP)[0];
            assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() <= TOL), "{az} / {el}, centre {c}: {got:?} against {want:?}");
        }
    }

    /// Focus narrows the cloud as the model does, on both routes.
    #[test]
    fn focus_is_the_models() {
        for &(layout, n, az, el, s, f, want) in v::FOCUS {
            let got = if layout == 0 { planar(dir(az, el), n, s, f, EMITTER_FRONT, EMITTER_TOP) } else { height(dir(az, el), n, s, 0.0, f, EMITTER_FRONT, EMITTER_TOP).map(down_mix) };
            assert!(near(got, want, TOL), "layout {layout}, {n} ch, {az} / {el}, spread {s}, focus {f}: {got:?} against {want:?}");
        }
    }

    /// Voices not positioned in space: on a stereo route mono 0.7071 on both with direct assignment, stereo 1:1, balance-
    /// fade by PAN_LR; on a 7.1.4 route the model's table (direct and balance-fade, centre, both pans).
    #[test]
    fn unplaced_voices_are_the_models() {
        let route = |layout, panner, pan_lr| Route { layout, panner, pan_lr, ..Route::default() };
        let (d, f) = (Panner::DirectSpeakerAssignment, Panner::BalanceFade);
        assert_eq!(unplaced(&route(Layout::Stereo, d, 60.0), 1), [[SQRT_HALF, SQRT_HALF], [0.0, 0.0]]);
        assert_eq!(unplaced(&route(Layout::Stereo, d, 60.0), 2), [[1.0, 0.0], [0.0, 1.0]]);
        let x: f64 = 0.8;
        assert!(near(unplaced(&route(Layout::Stereo, f, 60.0), 1), [[(1.0 - x).sqrt(), x.sqrt()], [0.0, 0.0]], 1e-7));
        assert!(near(unplaced(&route(Layout::Stereo, f, 60.0), 2), [[(1.0 - x).sqrt(), 0.0], [0.0, x.sqrt()]], 1e-7));
        assert!(near(unplaced(&route(Layout::Stereo, f, -300.0), 1), [[1.0, 0.0], [0.0, 0.0]], 0.0), "the balance held to 0..1");
        for &(n, fade, c, lr, fr, want) in v::UNPLACED_HEIGHT {
            let r = Route { layout: Layout::Height714, panner: if fade { f } else { d }, centre_pct: c, pan_lr: lr, pan_fr: fr };
            let got = unplaced(&r, n);
            assert!(near(got, want, TOL), "{n} ch, fade {fade}, centre {c}, pans {lr} / {fr}: {got:?} against {want:?}");
        }
        // PAN_FR at its ends (41 shipped objects carry -100 or 100): the shares sum to 1 with no float hair under 0.
        for fr in [-100.0, 100.0] {
            let r = Route { layout: Layout::Height714, panner: f, centre_pct: 0.0, pan_lr: 0.0, pan_fr: fr };
            assert!(unplaced(&r, 1).iter().flatten().chain(unplaced(&r, 2).iter().flatten()).all(|g| g.is_finite()), "{fr}");
        }
    }

    /// The meshes are the model's: the same triangles in the same lists, with the centre speaker and without.
    #[test]
    fn the_meshes_are_the_models() {
        const NAMES_FC: [&str; 12] = ["FL", "FR", "FC", "BL", "BR", "SL", "SR", "HFL", "HFR", "HBL", "HBR", "BOT"];
        const NAMES: [&str; 11] = ["FL", "FR", "BL", "BR", "SL", "SR", "HFL", "HFR", "HBL", "HBR", "BOT"];
        for (mesh, names, want) in [(&MESHES[1], &NAMES_FC[..], v::MESH_FC), (&MESHES[0], &NAMES[..], v::MESH_NOFC)] {
            for (list, want) in mesh.lists.iter().zip(want) {
                let got: Vec<String> = list.all().iter().map(|t| t.idx.map(|i| names[i]).join("-")).collect();
                assert_eq!(got, want.to_vec());
            }
        }
    }

    /// Elite's down-mix table (T5g-2 section 3): a voice on one speaker alone comes out by that speaker's row.
    #[test]
    fn the_down_mix_is_elites_table() {
        let h = 0.70710677f32;
        let want: [[f32; 2]; 11] = [[1.0, 0.0], [0.0, 1.0], [h, h], [h, 0.0], [0.0, h], [h, 0.0], [0.0, h], [h, 0.0], [0.0, h], [0.5, 0.0], [0.0, 0.5]];
        for (i, w) in want.iter().enumerate() {
            let mut row = [0.0; 11];
            row[i] = 1.0;
            assert_eq!(down_mix(row).map(|g| g as f32), *w, "speaker {i}");
        }
        // Hard right at ear level is SR alone: 0.7071 on the right; straight above the four heights at 0.5 each.
        let right = height(dir(90.0, 0.0), 1, 0.0, 0.0, 0.0, EMITTER_FRONT, EMITTER_TOP)[0];
        assert!((right[6] - 1.0).abs() < 1e-9 && near([down_mix(right), [0.0; 2]], [[0.0, SQRT_HALF], [0.0; 2]], 1e-9));
    }

    /// A route's layout: a bus's own, else its parent's, the master's the device's (stereo); only 7.1.4 into stereo.
    #[test]
    fn a_bus_takes_its_parents_layout() {
        let (s, h) = (Some(Layout::Stereo), Some(Layout::Height714));
        assert_eq!(Layout::of_config(0x2d63_f10c), Ok(h));
        assert_eq!(Layout::of_config(0), Ok(None));
        assert_eq!(Layout::of_config(0x0000_3102), Ok(s));
        assert!(Layout::of_config(0x0006_3f06).is_err(), "5.1 is not built");
        assert_eq!(Layout::of_route(&[None, None]), Ok(Layout::Stereo), "nothing of their own: the device's");
        assert_eq!(Layout::of_route(&[None, h, None, None]), Ok(Layout::Height714), "the parent's 7.1.4, down-mixed above");
        assert!(Layout::of_route(&[None, None, h]).is_err(), "a 7.1.4 master");
        assert_eq!(Layout::of_route(&[h, s]), Ok(Layout::Height714));
        assert_eq!(Layout::of_route(&[]), Ok(Layout::Stereo), "no bus: the device's");
        assert!(Layout::of_route(&[s, h]).is_err(), "an up-mix");
        assert!(Layout::of_route(&[None, h, s, None]).is_ok() && Layout::of_route(&[None, s, h]).is_err());
    }

    /// The gear legs at their places from the head (T5d-5), head straight, on their 7.1.4 route at spread 49.7: the model's
    /// gains, the left leg 10.3 dB to the left and the right one 11.4 dB to the right (T5g-2 section 4).
    #[test]
    fn the_legs_sit_where_the_model_puts_them() {
        let p = placed(Layout::Height714, 49.7);
        for &(x, y, z, l, r) in v::LEGS {
            let m = p.at([x as f32, y as f32, z as f32], 1).matrix[0];
            assert!((f64::from(m[0]) - l).abs() < 1e-6 && (f64::from(m[1]) - r).abs() < 1e-6, "({x}, {y}, {z}): {m:?} against {l} / {r}");
        }
        let db = |i: usize| 20.0 * (v::LEGS[i].3 / v::LEGS[i].4).log10();
        assert!((db(0) - 10.3).abs() < 0.05 && (db(2) + 11.4).abs() < 0.05, "{} {}", db(0), db(2));
    }

    /// An emitter at the listener sounds straight ahead; straight above and straight behind as the model has them; a
    /// place not finite counts as the listener's own, and a matrix is always finite.
    #[test]
    fn the_listener_above_behind_and_places_not_finite() {
        for layout in [Layout::Stereo, Layout::Height714] {
            let p = placed(layout, 50.0);
            for n in [1, 2] {
                let ahead = p.pan_at(dir(0.0, 0.0), n);
                assert_eq!(p.at([0.0; 3], n).matrix, ahead, "{layout:?} {n} ch: at the listener");
                assert_eq!(p.listener_matrix(n), ahead);
                for bad in [[f32::NAN, 0.0, 0.0], [0.0, f32::INFINITY, 1.0], [0.0, 0.0, f32::NEG_INFINITY]] {
                    let m = p.at(bad, n).matrix;
                    assert_eq!(m, ahead, "{layout:?} {n} ch: {bad:?}");
                }
                // Straight above (mod axes: +y) and straight behind (+z).
                let above = p.at([0.0, 3.0, 0.0], n).matrix;
                let behind = p.at([0.0, 0.0, 3.0], n).matrix;
                assert!(above.iter().chain(&behind).flatten().all(|g| g.is_finite()));
            }
        }
        // Straight above at spread 0: the model's 0.7071 each on the stereo route, 0.604 each on the 7.1.4 one; behind
        // 0.7071 / 0.5.
        let pt = |layout, rel: [f32; 3]| placed(layout, 0.0).at(rel, 1).matrix[0];
        let c = |a: [f32; 2], b: f32| (a[0] - b).abs() < 1e-3 && (a[1] - b).abs() < 1e-3;
        assert!(c(pt(Layout::Stereo, [0.0, 2.0, 0.0]), 0.70711) && c(pt(Layout::Height714, [0.0, 2.0, 0.0]), 0.604));
        assert!(c(pt(Layout::Stereo, [0.0, 0.0, 2.0]), 0.70711) && c(pt(Layout::Height714, [0.0, 0.0, 2.0]), 0.5));
    }

    /// A stereo voice with a place but no spatialisation (attenuation alone) keeps both its channels, left to left and
    /// right to right (the T5d-5 review: the old matrix dropped its right channel).
    #[test]
    fn a_placed_stereo_voice_not_spatialised_keeps_both_channels() {
        for layout in [Layout::Stereo, Layout::Height714] {
            let mut p = Placement::routed(Route { layout, ..Route::default() }, false, None);
            p.volume = Some(flat(-6.0));
            let now = p.at([-4.0, 0.0, -1.0], 2);
            let g = now.gain;
            assert!((g - 0.5012).abs() < 1e-3);
            assert_eq!(now.matrix, [[g, 0.0], [0.0, g]], "{layout:?}");
        }
    }

    /// What one matrix of a placed voice costs (the player works one out at a block's start when the place changed): a
    /// mono and a stereo voice at spread 50 on each route, microseconds a matrix.
    #[test]
    #[ignore]
    fn what_a_placed_matrix_costs() {
        for layout in [Layout::Stereo, Layout::Height714] {
            let p = placed(layout, 50.0);
            for n in [1, 2] {
                let t = std::time::Instant::now();
                let mut sum = 0.0f32;
                for k in 0..2000 {
                    let a = k as f32 * 0.01;
                    sum += p.at([3.0 * a.sin(), -1.0, -3.0 * a.cos()], n).matrix[0][0];
                }
                eprintln!("{layout:?}, {n} ch: {:.1} us a matrix ({sum:.1})", t.elapsed().as_secs_f64() * 1e6 / 2000.0);
            }
        }
    }

    /// The matrix is a function of the place alone: the same place gives the same bits however often it is asked for (a
    /// game writing it every frame at 30, 60 or 144 frames a second changes nothing but how often it is written).
    #[test]
    fn the_matrix_depends_on_the_place_alone() {
        let p = placed(Layout::Height714, 30.0);
        let rel = [2.0, -1.0, -3.0];
        let first = p.at(rel, 1);
        for _ in 0..5 {
            assert_eq!(p.at(rel, 1), first);
        }
        assert_eq!(p.clone().at(rel, 1), first);
    }
}
