//! The tweener's curves (ARCHITECTURE.md A10, stand-in grade B): Robert Penner's standard easing
//! equations, in the form of the caurina Tweener library that also has the out-in variants, under
//! the numbers of Elite's own table `core.tweening.Easing` (0 none, then back, bounce, circ, cubic,
//! elastic, expo, quad, quart, quint, sine, each as in, out, in-out, out-in). Elite's native
//! tweener was not read: the curves are the standard ones its names promise. The bridge's
//! ActionScript tweener asks for them by number (`Ease`), so this is their one definition.

use std::f64::consts::PI;

/// Back's overshoot (Penner's default).
const BACK: f64 = 1.70158;
/// Elastic's period as a share of the duration (Penner's default; in-out uses 1.5 times it).
const PERIOD: f64 = 0.3;

#[derive(Clone, Copy)]
enum Family {
    Back,
    Bounce,
    Circ,
    Cubic,
    Elastic,
    Expo,
    Quad,
    Quart,
    Quint,
    Sine,
}

const FAMILIES: [Family; 10] = [
    Family::Back,
    Family::Bounce,
    Family::Circ,
    Family::Cubic,
    Family::Elastic,
    Family::Expo,
    Family::Quad,
    Family::Quart,
    Family::Quint,
    Family::Sine,
];

/// The number of curves in Elite's table (0 to 40).
pub const CURVES: i32 = 41;

/// The eased share of a tween at `t` (0 at its start, 1 at its end) for curve `kind` of Elite's
/// table. A number outside the table is linear, as 0 (`EASE_NONE`).
pub fn ease(kind: i32, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if !(1..CURVES).contains(&kind) {
        return t;
    }
    let family = FAMILIES[((kind - 1) / 4) as usize];
    match (kind - 1) % 4 {
        0 => ease_in(family, t),
        1 => ease_out(family, t),
        2 => ease_in_out(family, t),
        _ => {
            if t < 0.5 {
                ease_out(family, t * 2.0) / 2.0
            } else {
                0.5 + ease_in(family, t * 2.0 - 1.0) / 2.0
            }
        }
    }
}

fn ease_in(family: Family, t: f64) -> f64 {
    match family {
        Family::Back => t * t * ((BACK + 1.0) * t - BACK),
        Family::Bounce => 1.0 - bounce_out(1.0 - t),
        Family::Circ => 1.0 - (1.0 - t * t).sqrt(),
        Family::Cubic => t.powi(3),
        Family::Elastic => elastic_in(t, PERIOD),
        Family::Expo => {
            if t == 0.0 {
                0.0
            } else {
                2f64.powf(10.0 * (t - 1.0))
            }
        }
        Family::Quad => t * t,
        Family::Quart => t.powi(4),
        Family::Quint => t.powi(5),
        Family::Sine => 1.0 - (t * PI / 2.0).cos(),
    }
}

fn ease_out(family: Family, t: f64) -> f64 {
    1.0 - ease_in(family, 1.0 - t)
}

fn ease_in_out(family: Family, t: f64) -> f64 {
    // Penner's in-out: the in curve over the first half, the out curve over the second, except
    // that back overshoots by 1.525 times as much and elastic's period is 1.5 times as long.
    let half = |f: &dyn Fn(f64) -> f64| {
        if t < 0.5 {
            f(t * 2.0) / 2.0
        } else {
            1.0 - f((1.0 - t) * 2.0) / 2.0
        }
    };
    match family {
        Family::Back => {
            let s = BACK * 1.525;
            half(&|x| x * x * ((s + 1.0) * x - s))
        }
        Family::Elastic => half(&|x| elastic_in(x, PERIOD * 1.5)),
        _ => half(&|x| ease_in(family, x)),
    }
}

fn elastic_in(t: f64, period: f64) -> f64 {
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let s = period / 4.0;
    -(2f64.powf(10.0 * (t - 1.0))) * ((t - 1.0 - s) * 2.0 * PI / period).sin()
}

fn bounce_out(t: f64) -> f64 {
    const K: f64 = 7.5625;
    if t < 1.0 / 2.75 {
        K * t * t
    } else if t < 2.0 / 2.75 {
        let t = t - 1.5 / 2.75;
        K * t * t + 0.75
    } else if t < 2.5 / 2.75 {
        let t = t - 2.25 / 2.75;
        K * t * t + 0.9375
    } else {
        let t = t - 2.625 / 2.75;
        K * t * t + 0.984375
    }
}

#[cfg(test)]
mod tests {
    use super::{CURVES, ease};

    #[test]
    fn every_curve_starts_at_0_and_ends_at_1() {
        for kind in 0..CURVES {
            assert!(ease(kind, 0.0).abs() < 1e-9, "curve {kind} at 0: {}", ease(kind, 0.0));
            assert!((ease(kind, 1.0) - 1.0).abs() < 1e-9, "curve {kind} at 1: {}", ease(kind, 1.0));
        }
    }

    #[test]
    fn in_out_and_out_in_pass_the_middle_at_half() {
        for kind in (1..CURVES).filter(|k| (k - 1) % 4 >= 2) {
            assert!((ease(kind, 0.5) - 0.5).abs() < 1e-9, "curve {kind} at 0.5: {}", ease(kind, 0.5));
        }
    }

    #[test]
    fn known_values() {
        assert_eq!(ease(0, 0.25), 0.25); // EASE_NONE
        assert!((ease(13, 0.5) - 0.125).abs() < 1e-12); // EASE_IN_CUBIC
        assert!((ease(26, 0.5) - 0.75).abs() < 1e-12); // EASE_OUT_QUAD
        assert!(ease(1, 0.2) < 0.0); // EASE_IN_BACK dips below 0
        assert!(ease(2, 0.8) > 1.0); // EASE_OUT_BACK overshoots 1
        assert_eq!(ease(99, 0.3), 0.3); // outside the table: linear
    }
}
