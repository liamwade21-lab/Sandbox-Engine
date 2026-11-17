//! Wwise's curves: the shapes between curve points (and of fades), and the y scalings.
//!
//! The formulas are Wwise's own, as the open tool wwiser transcribes them from the sound engine's code
//! (github.com/bnnm/wwiser, `wwiser/generator/render/bnode_rtpc.py`: `_AkGraph._find` = `CAkConversionTable::ConvertInternal`,
//! `_AkInterpolation__InterpolateNoCheck` = `AkInterpolation::InterpolateNoCheck` with its polynomial sine and cosine
//! approximations, `_scale` = `CAkConversionTable::ApplyCurveScaling` for bank versions 72 and up). Constants are kept as
//! written there.

use super::data::{Interp, Point, Scaling};

/// `AkInterpolation::InterpolateNoCheck`: the value `t` (0..1) of the way from `from` to `to` along `shape`. `Constant`
/// stays at `from` (wwiser handles it in the point search; as a fade it holds the start value until the end).
pub fn interpolate(t: f64, from: f64, to: f64, shape: Interp) -> f64 {
    match shape {
        Interp::Log3 => (1.0 - t) * (1.0 - t) * (1.0 - t) * (from - to) + to,
        Interp::Sine => {
            let a = 1.5707964 * t;
            let v1 = a * a;
            let v2 = (v1 * -0.00018363654 + 0.0083063254) * v1 + -0.16664828;
            let v3 = v2 * v1 + 0.9999966;
            v3 * a * (to - from) + from
        }
        Interp::Log1 => (t - 3.0) * t * 0.5 * (from - to) + from,
        Interp::InvSCurve => {
            if t > 0.5 {
                let v1 = 3.1415927 - 3.1415927 * t;
                let v2 = (v1 * v1 * -0.00009181827 + 0.0041531627) * (v1 * v1) + -0.083324142;
                let v3 = 1.0 - (v2 * (v1 * v1) + 0.4999983) * v1;
                v3 * (to - from) + from
            } else {
                let a = 3.1415927 * t;
                let v1 = a * a;
                let v2 = (v1 * -0.00009181827 + 0.0041531627) * v1 + -0.083324142;
                let v3 = (v2 * v1 + 0.4999983) * a;
                v3 * (to - from) + from
            }
        }
        Interp::Linear => (to - from) * t + from,
        Interp::SCurve => {
            let a = 3.1415927 * t;
            let v1 = a * a;
            let v2 = (v1 * 0.00048483399 + -0.01961384) * v1 + 0.24767479;
            let v3 = v2 * v1 + 0.00069670216;
            v3 * (to - from) + from
        }
        Interp::Exp1 => (t + 1.0) * t * 0.5 * (to - from) + from,
        Interp::SineRecip => {
            let a = 1.5707964 * t;
            let v1 = a * a;
            let v2 = (v1 * -0.0012712094 + 0.04148775) * v1 + -0.49991244;
            let v3 = v2 * v1 + 0.99999332;
            v3 * (from - to) + to
        }
        Interp::Exp3 => t * t * t * (to - from) + from,
        Interp::Constant => from,
    }
}

/// `CAkConversionTable::ConvertInternal` without the scaling: the curve's y at `x` (held flat outside the points).
pub fn find(points: &[Point], x: f64) -> f64 {
    match points.len() {
        0 => return 0.0,
        1 => return points[0].y,
        _ => {}
    }
    for (i, p1) in points.iter().enumerate() {
        if p1.x >= x || i + 1 == points.len() {
            return p1.y;
        }
        let p2 = &points[i + 1];
        if p2.x > x {
            return match p1.interp {
                Interp::Linear => (p2.y - p1.y) * ((x - p1.x) / (p2.x - p1.x)) + p1.y,
                Interp::Constant => p1.y,
                shape => interpolate((x - p1.x) / (p2.x - p1.x), p1.y, p2.y, shape),
            };
        }
    }
    points[points.len() - 1].y
}

/// The silence floor Wwise uses for dB curves (`_VOLUME_MIN` in wwiser: -96.3 dB).
pub const DB_FLOOR: f64 = -96.3;

/// `CAkConversionTable::ApplyCurveScaling` (versions 72 and up): `None` as is; `dB` stores linear gain - 1, clamped to
/// -1..1, -1 being the floor; `Log` is 10^y (the data's reading: [`Scaling`]).
pub fn scale(y: f64, scaling: Scaling) -> f64 {
    match scaling {
        Scaling::None => y,
        Scaling::Log => 10f64.powf(y),
        Scaling::DB => {
            let v = y.clamp(-1.0, 1.0);
            if v == -1.0 {
                DB_FLOOR
            } else {
                (v + 1.0).log10() * 20.0
            }
        }
    }
}

/// A curve's value at `x`, scaled.
pub fn eval(points: &[Point], scaling: Scaling, x: f64) -> f64 {
    scale(find(points, x), scaling)
}

/// dB to a linear factor.
pub fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// A linear factor to dB (a floor for silence, so the sum stays a number).
pub fn gain_to_db(g: f64) -> f64 {
    if g <= 0.0 {
        -200.0
    } else {
        (20.0 * g.log10()).max(-200.0)
    }
}

/// Cents to a playback rate ratio.
pub fn cents_to_ratio(c: f64) -> f64 {
    2f64.powf(c / 1200.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each shape at its mid-point against the closed form it approximates (Wwise's polynomial approximations are good
    /// to 1e-5 for the sines, 7e-4 for the S-curve): Log3 1 - (1-t)^3, Sine sin(pi t / 2), Log1 (3 - t) t / 2, InvSCurve
    /// sin(pi t) / 2 below the middle, Linear t, SCurve (1 - cos(pi t)) / 2, Exp1 (t + 1) t / 2, SineRecip
    /// 1 - cos(pi t / 2), Exp3 t^3, Constant the start value.
    #[test]
    fn shapes_at_their_mid_points() {
        use std::f64::consts::PI;
        let closed: [(Interp, fn(f64) -> f64); 10] = [
            (Interp::Log3, |t| 1.0 - (1.0 - t).powi(3)),
            (Interp::Sine, |t| (PI * t / 2.0).sin()),
            (Interp::Log1, |t| (3.0 - t) * t / 2.0),
            (Interp::InvSCurve, |t| if t <= 0.5 { (PI * t).sin() / 2.0 } else { 1.0 - (PI * (1.0 - t)).sin() / 2.0 }),
            (Interp::Linear, |t| t),
            (Interp::SCurve, |t| (1.0 - (PI * t).cos()) / 2.0),
            (Interp::Exp1, |t| (t + 1.0) * t / 2.0),
            (Interp::SineRecip, |t| 1.0 - (PI * t / 2.0).cos()),
            (Interp::Exp3, |t| t * t * t),
            (Interp::Constant, |_| 0.0),
        ];
        for (shape, f) in closed {
            for t in [0.25, 0.5, 0.75] {
                let v = interpolate(t, 0.0, 1.0, shape);
                assert!((v - f(t)).abs() < 1e-3, "{shape:?} at {t}: {v} against {}", f(t));
            }
            // Ends: every shape but Constant runs from the start to the target.
            assert!(interpolate(0.0, 0.0, 1.0, shape).abs() < 1e-3, "{shape:?} at 0");
            if shape != Interp::Constant {
                assert!((interpolate(1.0, 0.0, 1.0, shape) - 1.0).abs() < 1e-3, "{shape:?} at 1");
            }
        }
        // Documented mid-points: Log3 0.875, Log1 0.625, Exp1 0.375, Exp3 0.125, Sine and SineRecip 1/sqrt2 and 1 - 1/sqrt2.
        assert!((interpolate(0.5, 0.0, 1.0, Interp::Log3) - 0.875).abs() < 1e-9);
        assert!((interpolate(0.5, 0.0, 1.0, Interp::Log1) - 0.625).abs() < 1e-9);
        assert!((interpolate(0.5, 0.0, 1.0, Interp::Exp1) - 0.375).abs() < 1e-9);
        assert!((interpolate(0.5, 0.0, 1.0, Interp::Exp3) - 0.125).abs() < 1e-9);
        assert!((interpolate(0.5, 0.0, 1.0, Interp::Sine) - 0.5f64.sqrt()).abs() < 1e-5);
        assert!((interpolate(0.5, 0.0, 1.0, Interp::SineRecip) - (1.0 - 0.5f64.sqrt())).abs() < 1e-5);
        assert!((interpolate(0.5, 0.0, 1.0, Interp::SCurve) - 0.5).abs() < 1e-3);
        assert!((interpolate(0.5, 0.0, 1.0, Interp::InvSCurve) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn db_scaling() {
        assert_eq!(scale(-1.0, Scaling::DB), DB_FLOOR);
        assert!(scale(0.0, Scaling::DB).abs() < 1e-12);
        assert!((scale(1.0, Scaling::DB) - 20.0 * 2f64.log10()).abs() < 1e-12);
        assert!((scale(-0.5, Scaling::DB) - 20.0 * 0.5f64.log10()).abs() < 1e-12);
        assert_eq!(scale(5.0, Scaling::DB), scale(1.0, Scaling::DB));
        assert_eq!(scale(7.5, Scaling::None), 7.5);
        // Log: the EQ frequencies of the drives' bus (11338178) at the default 3 of their game parameters.
        assert!((scale(2.0, Scaling::Log) - 100.0).abs() < 1e-9);
        assert!((scale(3.0, Scaling::Log) - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn point_search() {
        let p = |x, y, interp| Point { x, y, interp };
        let c = [p(0.0, 0.0, Interp::Linear), p(10.0, 10.0, Interp::Constant), p(20.0, 0.0, Interp::Exp3), p(30.0, 30.0, Interp::Linear)];
        assert_eq!(find(&c, -5.0), 0.0);
        assert_eq!(find(&c, 5.0), 5.0);
        assert_eq!(find(&c, 15.0), 10.0);
        assert!((find(&c, 25.0) - 0.125 * 30.0).abs() < 1e-9);
        assert_eq!(find(&c, 99.0), 30.0);
        assert_eq!(find(&c, 10.0), 10.0);
    }
}
