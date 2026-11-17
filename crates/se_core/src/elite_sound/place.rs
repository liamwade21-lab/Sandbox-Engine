//! Where a voice sits (T5d-5): Wwise's 3D positioning as the shipped sounds use it, as pure maths. The mixer hands a voice
//! on an emitter the game has placed its [`Placement`] (from the positioning in force on its sound and that positioning's
//! attenuation share set); the player works out [`Placement::at`] from the emitter's place relative to the listener, once
//! a block on the audio thread (nothing here allocates), and applies the distance's gain and filter values and the pan.
//!
//! What the shipped data uses (`re/elite/work/sound/t5d5/positioning.py` over the 775 Sound objects on the 227 shipped
//! events' paths): 774 are 3D (listener-relative routing), 1 is 2D; of the 3D ones 626 are on the emitter's position and
//! spatialise by position and orientation (507 with direct speaker assignment, 119 with the balance-fade panner), 92 are on
//! the emitter's position without spatialisation (20 of them attenuated), 56 follow a path automation relative to the
//! listener (43 of those hold the listener's orientation); none holds the emitter's. 71 attenuation share sets are used,
//! one with a cone (2 sounds). Every cue the state machines post (the gear
//! legs', the doors', the hull's: 71 sounds) is mono, 3D, emitter-positioned, spatialised by position and orientation, with
//! one of four attenuations (`258639ce`, `108d7569`, `2624befd`, `3aa1ac13`) that have volume, low-pass and spread curves
//! and no cone. Built: the distance curves for volume (dry), low-pass and high-pass, the spread curve, and the pan of a
//! mono or stereo source on a stereo output by azimuth and spread. Not built (listed): the wet (aux) curves, Focus, cones
//! (no emitter orientation is known), path automation and listener-held orientation (those voices play centred), the
//! attenuations' own game-parameter curves (none on the four the cues use), and the 2D speaker panning properties
//! (PAN_LR, PAN_FR, CenterPCT).
//!
//! **The pan law.** Elite's ship buses (18 of them) carry Frontier's DTS 3D panner as their mixer plug-in (`03e90c66`,
//! `DtsI3daPannerPlugin`); its `Init` (0x1449c1370) accepts only 4.0, 5.1, 7.1 and three height layouts and on any other
//! bus layout logs "DTS I3DA Panner: unsupported Wwise Bus layout, initialization will fail" and fails, so on the user's
//! stereo output Wwise's own panning places the voices (inferred: what Wwise does with a bus whose mixer plug-in failed
//! is not documented in what could be read). Audiokinetic's documentation could not be fetched (its pages refuse), and
//! the default speaker angles were not found in the exe (`t5d5/anglescan.py`). So, grade E, the plainest reading the brief
//! allows: constant power across the front half (gains `cos` and `sin` of (azimuth / 90 + 1) x 45 degrees, hard left at
//! -90, hard right at +90), mirrored behind (a source at 180 - a degrees sounds as one at a); height is left out (the
//! azimuth of the horizontal direction). In that law the right gain squared is (1 + sin azimuth) / 2, so the mirror
//! needs no fold. **Spread** (Wwise's Attenuation editor: 0 a point source, 100 spread evenly over the whole speaker
//! layout, Audiokinetic's staff answer on its Q&A) spreads the source evenly over an arc of spread / 100 x 360 degrees
//! centred on its azimuth, each direction's power summed: sin azimuth is then averaged over the arc, times
//! sin(w / 2) / (w / 2) (E: the arc's shape). A mono source's gains are scaled by sqrt 2, so a source straight ahead is
//! at full level on both channels, as the player plays a centred mono voice (the T5c note: Wwise's -3 dB centre and the
//! `akd ` chunk's 1.414 on mono recordings cancel). A stereo source's two channels each take half the arc (the left
//! channel the left half), at plain constant power (its `akd ` second gain is 1); at spread 0 both collapse to the point
//! (Audiokinetic's 2018.1 positioning blog: a multichannel source collapses to mono unless the spread is above 0).

use super::curve;
use super::data::{AttCurve, Attenuation};

/// A voice's positioning, made by the mixer from its sound's data (`Mixer` hands it in [`super::VoiceUpdate::place`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// Panned by its direction (3D spatialisation by position, or position and orientation). False: the distance's
    /// curves alone (spatialisation None), the voice centred.
    pub pan: bool,
    /// The attenuation's distance curves (x metres): VolumeDry (dB scaling), LowPassFilter, HighPassFilter (0-100 added to
    /// the voice's values), Spread (percent). None where the share set uses none, or attenuation is off.
    pub volume: Option<AttCurve>,
    pub lowpass: Option<AttCurve>,
    pub highpass: Option<AttCurve>,
    pub spread: Option<AttCurve>,
}

/// What a placement gives at one place: the distance's gain (linear), its low-pass and high-pass shares (0-100, added to
/// the voice's), and the pan matrix (`matrix[input channel][output channel]`, the distance's gain in it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaceNow {
    pub gain: f32,
    pub lowpass: f32,
    pub highpass: f32,
    pub matrix: [[f32; 2]; 2],
}

impl Placement {
    /// From the positioning in force: `pan` (3D spatialisation other than None) and its attenuation (None when attenuation
    /// is off or the chain names none). None when neither applies: the voice plays as an unplaced one.
    pub fn new(pan: bool, att: Option<&Attenuation>) -> Option<Placement> {
        let curve = |k: &str| att.and_then(|a| a.curve_to_use.get(k).copied().filter(|i| *i >= 0).and_then(|i| a.curves.get(i as usize)).cloned());
        let p = Placement { pan, volume: curve("VolumeDry"), lowpass: curve("LowPassFilter"), highpass: curve("HighPassFilter"), spread: curve("Spread") };
        (pan || att.is_some()).then_some(p)
    }

    /// One curve at a distance (0 without the curve).
    fn read(c: &Option<AttCurve>, d: f64) -> f64 {
        c.as_ref().map_or(0.0, |c| curve::eval(&c.points, c.scaling, d))
    }

    /// The distance's volume (dB), low-pass and high-pass shares and spread (percent) at `d` metres.
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

    /// At `rel`, the emitter's place from the listener in the listener's axes (metres; x right, y up, -z ahead), for a
    /// source of `channels` channels. A place not finite counts as the listener's own (no distance, centred).
    pub fn at(&self, rel: [f32; 3], channels: usize) -> PlaceNow {
        let [x, y, z] = if rel.iter().all(|v| v.is_finite()) { rel.map(f64::from) } else { [0.0; 3] };
        let d = (x * x + y * y + z * z).sqrt();
        let gain = curve::db_to_gain(self.volume_db(d)) as f32;
        let h = (x * x + z * z).sqrt();
        // (sin and cos of the azimuth: 0 ahead, +90 degrees right; a source straight above, below or at the listener is
        // ahead.)
        let (s, c) = if h > 1e-6 { (x / h, -z / h) } else { (0.0, 1.0) };
        let mut matrix = if self.pan { pan_gains(s, c, self.spread_at(d).clamp(0.0, 100.0), channels) } else { CENTRED };
        for row in &mut matrix {
            row.iter_mut().for_each(|g| *g *= gain);
        }
        PlaceNow { gain, lowpass: self.lowpass_at(d) as f32, highpass: self.highpass_at(d) as f32, matrix }
    }
}

/// A voice not panned: a mono voice on both channels at full level, a stereo one left to left and right to right (as the
/// player plays an unplaced voice).
pub const CENTRED: [[f32; 2]; 2] = [[1.0, 1.0], [0.0, 0.0]];
const STRAIGHT: [[f32; 2]; 2] = [[1.0, 0.0], [0.0, 1.0]];

/// sin(a) / a, 1 at 0.
fn sinc(a: f64) -> f64 {
    if a.abs() < 1e-9 { 1.0 } else { a.sin() / a }
}

/// The (left, right) gains of a direction whose azimuth's sine is `s`, spread over an arc of `w` radians: constant power
/// over the front half, mirrored behind (the module's notes), the arc's powers summed evenly.
fn pair(s: f64, w: f64) -> [f64; 2] {
    let k = (s * sinc(w / 2.0)).clamp(-1.0, 1.0);
    [((1.0 - k) / 2.0).sqrt(), ((1.0 + k) / 2.0).sqrt()]
}

/// The pan matrix (`[input channel][output channel]`) of a source at the azimuth whose sine and cosine are `s`, `c`, with
/// `spread` percent, on a stereo output: a mono source scaled by sqrt 2 (straight ahead at full level on both), a stereo
/// source's channels each on its half of the arc. A source of more channels than two is not in the data: its first two
/// are taken as a stereo pair.
pub fn pan_gains(s: f64, c: f64, spread: f64, channels: usize) -> [[f32; 2]; 2] {
    let w = spread.clamp(0.0, 100.0) / 100.0 * std::f64::consts::TAU;
    if channels <= 1 {
        let [l, r] = pair(s, w);
        return [[(l * std::f64::consts::SQRT_2) as f32, (r * std::f64::consts::SQRT_2) as f32], [0.0, 0.0]];
    }
    // (Each channel's half arc: its middle a quarter of the arc to its side, its width half the arc.)
    let (sa, ca) = (w / 4.0).sin_cos();
    let left = pair(s * ca - c * sa, w / 2.0);
    let right = pair(s * ca + c * sa, w / 2.0);
    let m = [[left[0] as f32, left[1] as f32], [right[0] as f32, right[1] as f32]];
    if m.iter().flatten().all(|g| g.is_finite()) { m } else { STRAIGHT }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(deg: f64, spread: f64) -> [f32; 2] {
        let a = deg.to_radians();
        pan_gains(a.sin(), a.cos(), spread, 1)[0]
    }

    fn close(a: [f32; 2], b: [f64; 2]) -> bool {
        (f64::from(a[0]) - b[0]).abs() < 1e-5 && (f64::from(a[1]) - b[1]).abs() < 1e-5
    }

    /// The pan law's gains (mono, scaled by sqrt 2) at 0, +-45, +-90 and 180 degrees, at spread 0 and 100: constant power,
    /// straight ahead and straight behind full on both, hard to the side at +3 dB on one, everything even at spread 100.
    #[test]
    fn the_pan_law_at_its_corners() {
        let r2 = std::f64::consts::SQRT_2;
        let side = |k: f64| [(1.0 - k).sqrt(), (1.0 + k).sqrt()];
        let h = std::f64::consts::FRAC_1_SQRT_2;
        for (deg, want) in [(0.0, [1.0, 1.0]), (45.0, side(h)), (-45.0, side(-h)), (90.0, [0.0, r2]), (-90.0, [r2, 0.0]), (180.0, [1.0, 1.0])] {
            let g = at(deg, 0.0);
            assert!(close(g, want), "{deg} degrees: {g:?} against {want:?}");
            assert!((f64::from(g[0]).powi(2) + f64::from(g[1]).powi(2) - 2.0).abs() < 1e-5, "constant power at {deg}");
            let wide = at(deg, 100.0);
            assert!(close(wide, [1.0, 1.0]), "{deg} degrees at spread 100: {wide:?}");
        }
        // Mirrored behind: 135 degrees sounds as 45.
        assert!(close(at(135.0, 0.0), side(h)));
        // A stereo source at spread 0 straight ahead: both channels to the middle at plain constant power; at spread 100
        // the left channel leans left.
        let st = pan_gains(0.0, 1.0, 0.0, 2);
        assert!(st.iter().flatten().all(|g| (f64::from(*g) - h).abs() < 1e-6), "{st:?}");
        let wide = pan_gains(0.0, 1.0, 100.0, 2);
        assert!(wide[0][0] > wide[0][1] && wide[1][1] > wide[1][0], "{wide:?}");
    }

    /// Spread between: the gains move smoothly from the point source's to even ones.
    #[test]
    fn spread_widens_smoothly() {
        let mut last = at(90.0, 0.0);
        for k in 1..=100 {
            let g = at(90.0, f64::from(k));
            assert!((g[0] - last[0]).abs() < 0.05 && (g[1] - last[1]).abs() < 0.05, "spread {k}: {g:?} after {last:?}");
            last = g;
        }
    }
}
