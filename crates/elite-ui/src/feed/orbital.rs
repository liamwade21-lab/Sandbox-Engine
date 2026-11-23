//! Planetary flight, `orbitalhud` (`re/elite/notes/ui_messages_hud.md`, its section; the updater `FUN_1430cb2e0`, the
//! altimeter `FUN_1430dadc0`, the landing guide `FUN_1430da2a0`). What is sent: the gauges' visibility, the altimeter,
//! the heading and pitch ladder, the landing guide.
//!
//! Not sent, and why: `ConfigureAltitudeIndicators` (the minor marks' labels were not decoded, E); `SetAttitudeZones`
//! (the zones' numbers were not decoded, E); `SetRateOfDescent` and its danger zone (the gauge's scale was not decoded,
//! E: its gauge is kept hidden); `SetLandedString` (the text's sender was not traced); the spectrometer's messages (an
//! SRV's, not a lone flight's).

use super::{changed, Movie, Outbox};
use crate::facts::Planet;
use crate::{UiFacts, Value};

const MOVIE: Movie = Movie::OrbitalHud;

/// The altimeter record (B: the game's data, the `StellarForgeManager` template, `messages/altitude_record.json`): the
/// outer radius r1 = A R + B, the inner mark C + (R + max(H, M)) S, the scale's top K r1.
const OUTER_A: f64 = 1.1;
const OUTER_B_M: f64 = 400_000.0;
const INNER_S: f64 = 1.0;
const INNER_C_M: f64 = 0.0;
const INNER_MIN_M: f64 = 25_000.0;
const RANGE_K: f64 = 1.3;
/// A minor mark's state in the normal mode (the body's eligibility code 0, which the mod's bodies always have: stand-in,
/// the reasons for the other codes were not decoded): 1, default (A: the movie; B: the sender gives one state).
const MARK_DEFAULT: i32 = 1;
/// Aligned while the ship's axis is within 5 degrees of the surface normal (A: 5.0 at DAT_144de1678 x 0.0174533).
const ALIGNED_DEG: f64 = 5.0;
const DEG_TO_RAD: f64 = 0.017_453_3;
/// The landing guide's chevron tolerance (degrees): not decoded (E); the alignment limit above stands in for it.
const CHEVRON_TOLERANCE_DEG: f64 = ALIGNED_DEG;

/// The outer radius r1 (m) of a body of radius R: inside it the ship is in planetary flight (the proximity state 1: A,
/// `FUN_142c88f10`, d^2 < r1^2).
fn outer_radius(radius_m: f64) -> f64 {
    radius_m * OUTER_A + OUTER_B_M
}

/// The orbital HUD's feeder.
#[derive(Clone, Debug, Default)]
pub struct Orbital {
    gauges: Option<[bool; 6]>,
}

impl Orbital {
    pub fn feed(&mut self, facts: &UiFacts, _dt: f64, out: &mut Outbox) {
        let near = facts.planet.as_ref().filter(|p| p.centre_distance_m < outer_radius(p.radius_m));
        let guide = near.and_then(|p| p.landing.as_ref()).filter(|l| l.suitable_terrain.is_some());

        // `SetGaugeVisibility(altimeter, attitude, heading, spectrometer, rateOfDescent, landingGuide)` when the regime or
        // a mode changes (B: the trigger "entering or leaving planetary flight"). The order is the movie's own (A:
        // `OrbitalHudApp.setGaugeVisibility` shows the altimeter by the first, the pitch ladder by the second, the heading
        // tape by the third, the spectrometer by the fourth); the notes' table lists heading second and the ladder fourth,
        // which the movie contradicts. Which gauge shows in which part of the regime was not decoded (E): the fed ones show
        // inside the outer radius, the landing guide while it is fed, the spectrometer and the rate of descent stay hidden
        // (stand-in).
        let shown = near.is_some();
        let gauges = [shown, shown, shown, false, false, guide.is_some()];
        if changed(&mut self.gauges, gauges) {
            out.send(MOVIE, "SetGaugeVisibility", gauges.iter().map(|&g| g.into()).collect());
        }
        if let Some(p) = near {
            out.send(MOVIE, "UpdateAltitude", altitude(p));
            // `UpdateHeadingGauges(heading, pitch, roll)` each frame (B). The ladder labels the pitch as given
            // (valueScalar -1); the heading goes through `tape_value`.
            out.send(MOVIE, "UpdateHeadingGauges", vec![tape_value(p.heading_deg).into(), p.pitch_deg.into(), p.roll_deg.into()]);
        }
        if let Some(l) = guide {
            // `UpdateLandingGuide(roll, pitch, tolerance, suitableTerrain, speedOk, aligned, landed)` each frame while shown
            // (A: `FUN_1430cb2e0`): flying, (suitable, |v| <= the limit, the axis within 5 degrees, false); landed, all
            // true. The third argument is a tolerance in degrees: a pitch or roll within it hides that axis's chevrons
            // (A: the movie's `LandingGuide.update`, lines 156 and 162). Its producer was not decoded (E): the exe's own
            // alignment limit, 5 degrees, is sent (stand-in).
            let aligned_cos = (ALIGNED_DEG * DEG_TO_RAD).cos();
            let (suitable, speed_ok, aligned) = if l.landed {
                (true, true, true)
            } else {
                (l.suitable_terrain.unwrap_or(false), l.speed_m_s <= l.speed_limit_m_s, l.axis_dot_normal >= aligned_cos)
            };
            let args = vec![
                l.roll_deg.into(),
                l.pitch_deg.into(),
                CHEVRON_TOLERANCE_DEG.into(),
                suitable.into(),
                speed_ok.into(),
                aligned.into(),
                l.landed.into(),
            ];
            out.send(MOVIE, "UpdateLandingGuide", args);
        }
    }

    /// The movie has just been loaded: everything is sent again.
    pub fn reset(&mut self) {
        *self = Orbital::default();
    }
}

/// The heading tape's value for the compass heading `h` (degrees clockwise from north). The tape labels its value negated,
/// adding 360 to a negative label (A: `HeadingGauge`, valueScalar 1, `getLabel(-value)`), and draws its ticks round the
/// negated value, skipping any under its `limitMin` of -260 (A: `OrbitalHudApp`); so the value is -h for h in [0, 180) and
/// 360 - h for h in [180, 360), which keeps the ticks' centre in [-180, 180) and the label h. What the exe itself sends was
/// not decoded (B, ours).
fn tape_value(h: f64) -> f64 {
    let h = h.rem_euclid(360.0);
    if h < 180.0 { 0.0 - h } else { 360.0 - h }
}

/// `UpdateAltitude(altitude, label, minor0, state0, minor1, state1, location)` (A: `FUN_1430dadc0`; numbers B): the
/// scale is linear in metres over range = K r1 - R; altitude = (d - R) / range; the label `$DISTANCE:#value=<d - R>;`;
/// minor1 = (r1 - R) / range; minor0 (normal mode) = (C + (R + max(H, M)) S - R) / range. H has no source (E): without it
/// max(H, M) is M (stand-in). The marks' state is the sender's one state, given to both (B); the text under the altimeter
/// was not decoded (E): sent empty (stand-in).
fn altitude(p: &Planet) -> Vec<Value> {
    let r = p.radius_m;
    let r1 = outer_radius(r);
    let range = RANGE_K * r1 - r;
    let height = p.centre_distance_m - r;
    let inner = INNER_C_M + (r + p.inner_height_m.unwrap_or(INNER_MIN_M).max(INNER_MIN_M)) * INNER_S - r;
    vec![
        (height / range).into(),
        format!("$DISTANCE:#value={height};").into(),
        (inner / range).into(),
        MARK_DEFAULT.into(),
        ((r1 - r) / range).into(),
        MARK_DEFAULT.into(),
        "".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::Landing;

    const EARTH_R: f64 = 6_371_000.0;

    fn over_earth(height_m: f64) -> UiFacts {
        UiFacts {
            planet: Some(Planet {
                radius_m: EARTH_R,
                centre_distance_m: EARTH_R + height_m,
                inner_height_m: None,
                heading_deg: 90.0,
                pitch_deg: -10.0,
                roll_deg: 5.0,
                landing: None,
            }),
            ..UiFacts::default()
        }
    }

    fn close(v: &Value, b: f64) {
        let a = v.as_f64().expect("a number");
        assert!((a - b).abs() < 1e-9, "{a} against {b}");
    }

    #[test]
    fn altimeter_from_the_record() {
        let mut o = Orbital::default();
        let mut out = Outbox::default();
        o.feed(&over_earth(100_000.0), 0.0, &mut out);
        let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
        assert_eq!(names, ["SetGaugeVisibility", "UpdateAltitude", "UpdateHeadingGauges"]);
        let vis: Vec<bool> = out.msgs[0].args.iter().map(|v| v.as_bool().unwrap()).collect();
        assert_eq!(vis, [true, true, true, false, false, false], "altimeter, ladder, heading; no spectrometer");
        // The notes' planet: the top 0.43 R + 520 km over the surface, mark 1 at 0.1 R + 400 km, mark 0 at 25 km.
        let range = 0.43 * EARTH_R + 520_000.0;
        let a = &out.msgs[1].args;
        close(&a[0], 100_000.0 / range);
        assert_eq!(a[1].as_str(), Some("$DISTANCE:#value=100000;"));
        close(&a[2], 25_000.0 / range);
        close(&a[4], (0.1 * EARTH_R + 400_000.0) / range);
        assert_eq!((a[3].as_f64(), a[5].as_f64(), a[6].as_str()), (Some(1.0), Some(1.0), Some("")));
        assert_eq!(out.msgs[2].args, vec![Value::from(-90.0), (-10.0).into(), 5.0.into()], "the tape reads -(-90) = 90");
    }

    /// The tape's label for a value: the movie's `getLabel(-value)` (a negative label gets 360 added).
    fn tape_label(value: f64) -> f64 {
        let l = (-value).trunc();
        if l >= 0.0 { l } else { 360.0 + l }
    }

    #[test]
    fn heading_tape_values() {
        for (h, value) in [(0.0, 0.0), (50.0, -50.0), (179.9, -179.9), (180.0, 180.0), (245.0, 115.0), (359.9, 0.1)] {
            let v = tape_value(h);
            assert!((v - value).abs() < 1e-9, "{h}: {v}");
            assert!((-180.0..180.0).contains(&-v), "{h}: the ticks' centre {} in [-180, 180)", -v);
            assert!((-v).trunc() >= -260.0, "{h}: inside the tape's limit");
            // The movie truncates its label toward zero: below 180 the label is the floor of h, from 180 its ceiling
            // (mod 360), so a fraction of a degree reads either way by half the circle.
            let want = if h < 180.0 { h.floor() } else { h.ceil().rem_euclid(360.0) };
            assert_eq!(tape_label(v), want, "{h}");
        }
    }

    #[test]
    fn regime_ends_at_the_outer_radius() {
        let mut o = Orbital::default();
        let mut out = Outbox::default();
        let edge = 0.1 * EARTH_R + 400_000.0;
        o.feed(&over_earth(edge - 1.0), 0.0, &mut out);
        assert_eq!(out.msgs.len(), 3);
        let mut out = Outbox::default();
        o.feed(&over_earth(edge + 1.0), 0.0, &mut out);
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetGaugeVisibility"]);
        assert!(out.msgs[0].args.iter().all(|v| v.as_bool() == Some(false)));
    }

    #[test]
    fn landing_guide() {
        let mut o = Orbital::default();
        let mut f = over_earth(50.0);
        let deg = |d: f64| (d * DEG_TO_RAD).cos();
        let mut landing = Landing {
            roll_deg: 2.0,
            pitch_deg: -3.0,
            axis_dot_normal: deg(4.0),
            speed_m_s: 14.0,
            speed_limit_m_s: 15.0,
            suitable_terrain: None,
            landed: false,
        };
        f.planet.as_mut().unwrap().landing = Some(landing.clone());
        let mut out = Outbox::default();
        o.feed(&f, 0.0, &mut out);
        assert!(out.msgs.iter().all(|m| m.name != "UpdateLandingGuide"), "no terrain rule: not fed");
        landing.suitable_terrain = Some(true);
        f.planet.as_mut().unwrap().landing = Some(landing.clone());
        let mut out = Outbox::default();
        o.feed(&f, 0.0, &mut out);
        assert_eq!(out.msgs[0].name, "SetGaugeVisibility", "the guide's gauge shown");
        let g = &out.msgs.iter().find(|m| m.name == "UpdateLandingGuide").unwrap().args;
        let flags: Vec<bool> = g[3..].iter().map(|v| v.as_bool().unwrap()).collect();
        assert_eq!(flags, [true, true, true, false]);
        assert_eq!(g[2].as_f64(), Some(5.0), "the chevrons' tolerance");
        landing.axis_dot_normal = deg(6.0);
        landing.speed_m_s = 15.5;
        f.planet.as_mut().unwrap().landing = Some(landing.clone());
        let mut out = Outbox::default();
        o.feed(&f, 0.0, &mut out);
        let g = &out.msgs.iter().find(|m| m.name == "UpdateLandingGuide").unwrap().args;
        let flags: Vec<bool> = g[3..].iter().map(|v| v.as_bool().unwrap()).collect();
        assert_eq!(flags, [true, false, false, false]);
        landing.landed = true;
        landing.suitable_terrain = Some(false);
        f.planet.as_mut().unwrap().landing = Some(landing);
        let mut out = Outbox::default();
        o.feed(&f, 0.0, &mut out);
        let g = &out.msgs.iter().find(|m| m.name == "UpdateLandingGuide").unwrap().args;
        let flags: Vec<bool> = g[3..].iter().map(|v| v.as_bool().unwrap()).collect();
        assert_eq!(flags, [true, true, true, true], "landed: all true");
    }
}
