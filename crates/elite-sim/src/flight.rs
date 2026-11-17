//! Normal-space flight: thrusters, flight assist on and off, the speed cap, boost's speed, rotation and the blue zone.
//! The rules (time constants, acceleration limits, speed guards, the blue zone, the mass curve, the pips' blend) are Elite's
//! own, read out of its exe (grade C; `docs/elite-flight-model.md` gives each with its address). Two parts are ours (F) until
//! measured: the integration (each tick adds acceleration x dt to the velocity, then the velocity moves the ship: the exe's
//! physics step was not followed that far), and the turn, applied as an angular acceleration directly (the exe's torque rule,
//! f x M x alpha(assist) + (1 - f) x M x alpha(raw), needs the body's inertia M, which is not known).
//! The ship's numbers (`Block`) are mostly not in the exe: their grades are in `data.rs` (`sidewinder_block`).
//!
//! Axes: ship axes are the mod's model axes, x right, y up, z back (the nose is -z). The exe's "linear demands" are
//! (lateral, vertical, forward): `Flight` keeps them so, forward = -z. Rotation axes: x pitch (nose up positive), y yaw
//! (nose left positive), z roll (about the back axis: positive rolls the right wing up).

use glam::{DQuat, DVec3};

/// A speed curve (the blue zone): the factor at a standstill `a`, at full speed `b`, 1 at the speed fraction `p`.
/// Elite's 0x142E50650.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve {
    pub a: f64,
    pub b: f64,
    pub p: f64,
}

impl Curve {
    /// No speed effect (a stand-in until a curve is measured).
    pub const FLAT: Curve = Curve { a: 1.0, b: 1.0, p: 0.5 };

    pub fn at(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        if self.p <= 0.0 {
            return self.b + (1.0 - self.b) * (1.0 - x);
        }
        if x <= self.p {
            self.a + (1.0 - self.a) * (x / self.p)
        } else if self.p >= 1.0 {
            1.0
        } else {
            self.b + (1.0 - self.b) * (1.0 - (x - self.p) / (1.0 - self.p))
        }
    }
}

/// The physics world's rate and its most sub-steps a frame (the `PhysicsWorld` component's template +0x10, +0x14, the same in
/// every location prototype, read by 0x14228D5E0: grade B, `re/elite/notes/sidewinder_numbers.md`, "Component templates").
pub const PHYSICS_HZ: f64 = 60.0;
pub const MAX_SUBSTEPS: u32 = 4;

/// The share of the reverse acceleration left as the forward braking limit while the forward demand is at least
/// `FULL_FORWARD` (0x142E6AF90, C), while the controller's flag +0x5E2 is set. The constructor (0x142E32DD0) sets it to 1;
/// supercruise (0x142EA9810, through virtual +0x180, 0x142E787C0) sets it to `w <= 0.01` in each frame where the object at
/// cruise +0x128 is missing or inactive (`supercruise::ORBITAL_BRAKE_W`); normal flight never writes it, so after leaving
/// orbital cruise it stays as last set. `step` takes it as set; in the exe it is clear in normal flight only after a drop or
/// glide from w over 0.01, until the next supercruise frame (not carried over by the mod).
pub const FULL_FORWARD_BRAKE: f64 = 0.25;
pub const FULL_FORWARD: f64 = 0.999;
/// Rotation-reversal mode 1 acts on an axis whose stick is past this (0x142E6E960, C).
pub const REVERSAL_STICK: f64 = 0.2;

/// The ship's performance block (the exe's 51 values; the ones the rules here read). Speeds m/s, accelerations m/s^2,
/// turn rates deg/s (x pitch, y yaw, z roll), angular accelerations deg/s^2, jerks per second, times s. A negative value in a
/// full-power block means "keep the base block's" (the blend's rule).
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// 0x00, 0x0C.
    pub top_speed: f64,
    pub boost_speed: f64,
    /// 0x04: the reverse speed as a share of the cap; 0x08: lateral and vertical.
    pub back_share: f64,
    pub side_share: f64,
    /// 0x10: the boost's ramp to full forward demand.
    pub boost_ramp: f64,
    /// 0x1C, 0x20, 0x24.
    pub accel_fwd: f64,
    pub accel_rev: f64,
    pub accel_side: f64,
    /// 0x28 (0: none).
    pub jerk_lin: f64,
    /// 0x2C: the linear flight assist's time scale D.
    pub fa_d: f64,
    /// 0x30, 0x34: the lateral and vertical upper limits' extra multipliers while boosting.
    pub boost_side: [f64; 2],
    /// 0x38 (x), 0x40 (y), 0x3C (z).
    pub turn: [f64; 3],
    /// 0x44 (x), 0x4C (y), 0x48 (z): the same x, z, y order in the block as the turn rates (0x142E4A820 builds the limits
    /// from controller +0x794, +0x79C, +0x798).
    pub ang_accel: [f64; 3],
    /// 0x50..0x58 (x, y, z in order; 0: none).
    pub ang_jerk: [f64; 3],
    /// 0x5C..0x64 (x, y, z in order): the rotational flight assist's time scales R.
    pub fa_r: [f64; 3],
    /// 0x68.. (angular acceleration), 0x78.. (turn rate, x and y), 0x88.. (turn rate, z).
    pub curve_accel: Curve,
    pub curve_turn: Curve,
    pub curve_roll: Curve,
    /// 0xA8..0xB0: rotation-reversal mode 1's factors (block 0xA4 = 1, the only mode built: the Sidewinder's): with flight
    /// assist on, an axis whose stick is past 0.2 and whose target opposes its spin turns with `alpha x (1 + factor)` (0: none).
    pub reversal: [f64; 3],
    /// 0xC0..0xC8: flight assist off, the damping of a centred axis's spin while another axis is steered (step 0.11).
    pub fa_off_damp: [f64; 3],
    /// From the performance record's boost record (+0x24C), read by the flight step while the boost fires, so carried with the
    /// block: field 4, every upper acceleration limit's multiplier, and field 5, flight assist's D in place of 0x2C.
    pub boost_accel: f64,
    pub boost_fa_d: f64,
}

impl Block {
    /// The distributor's blend (0x142E86E20, 0x141B2E310): base + (full - base) x t, field by field, a negative full-power
    /// value keeping the base's; the reversal factors blend without that rule.
    pub fn blend(base: &Block, full: &Block, t: f64) -> Block {
        let l = |a: f64, b: f64| if b < 0.0 { a } else { a + (b - a) * t };
        let l2 = |a: [f64; 2], b: [f64; 2]| [l(a[0], b[0]), l(a[1], b[1])];
        let l3 = |a: [f64; 3], b: [f64; 3]| [l(a[0], b[0]), l(a[1], b[1]), l(a[2], b[2])];
        let lc = |a: Curve, b: Curve| Curve { a: l(a.a, b.a), b: l(a.b, b.b), p: l(a.p, b.p) };
        Block {
            top_speed: l(base.top_speed, full.top_speed),
            boost_speed: l(base.boost_speed, full.boost_speed),
            back_share: l(base.back_share, full.back_share),
            side_share: l(base.side_share, full.side_share),
            boost_ramp: l(base.boost_ramp, full.boost_ramp),
            accel_fwd: l(base.accel_fwd, full.accel_fwd),
            accel_rev: l(base.accel_rev, full.accel_rev),
            accel_side: l(base.accel_side, full.accel_side),
            jerk_lin: l(base.jerk_lin, full.jerk_lin),
            fa_d: l(base.fa_d, full.fa_d),
            boost_side: l2(base.boost_side, full.boost_side),
            turn: l3(base.turn, full.turn),
            ang_accel: l3(base.ang_accel, full.ang_accel),
            ang_jerk: l3(base.ang_jerk, full.ang_jerk),
            fa_r: l3(base.fa_r, full.fa_r),
            curve_accel: lc(base.curve_accel, full.curve_accel),
            curve_turn: lc(base.curve_turn, full.curve_turn),
            curve_roll: lc(base.curve_roll, full.curve_roll),
            reversal: [0, 1, 2].map(|k| base.reversal[k] + (full.reversal[k] - base.reversal[k]) * t),
            fa_off_damp: l3(base.fa_off_damp, full.fa_off_damp),
            boost_accel: l(base.boost_accel, full.boost_accel),
            boost_fa_d: l(base.boost_fa_d, full.boost_fa_d),
        }
    }

    /// Supercruise's "accel mode 2" (0x141B2D8F0) with the speed multiplier `k` and the ship's speed `v`: the top speed and the
    /// forward and lateral accelerations times k; the reverse one `max(r k, r (v / V0) max(1, sqrt(v / (V0 k))))`, V0 the
    /// block's own top speed: braking grows with the speed over the cap.
    pub fn mode2(&self, k: f64, v: f64) -> Block {
        let mut b = self.clone();
        let (v0, r) = (self.top_speed.max(1e-9), self.accel_rev);
        let over = (v / (v0 * k).max(1e-300)).max(0.0).sqrt().max(1.0);
        b.accel_rev = (r * k).max(over * (v / v0) * r);
        b.top_speed *= k;
        b.accel_fwd *= k;
        b.accel_side *= k;
        b
    }

    /// The thrusters' mass multipliers (0x142EBDF60): `speed` scales the two speeds, `accel` the three accelerations,
    /// `turn` the three turn rates.
    pub fn scaled(&self, speed: f64, accel: f64, turn: f64) -> Block {
        let mut b = self.clone();
        b.top_speed *= speed;
        b.boost_speed *= speed;
        b.accel_fwd *= accel;
        b.accel_rev *= accel;
        b.accel_side *= accel;
        for k in 0..3 {
            b.turn[k] *= turn;
        }
        b
    }

    /// The block refresh's speed multiplier (0x142E86E20 step 4, 0x141B2DA50): the top speed and the boost speed times `k`,
    /// nothing else.
    pub fn speeds_times(&self, k: f64) -> Block {
        Block { top_speed: self.top_speed * k, boost_speed: self.boost_speed * k, ..self.clone() }
    }
}

/// The thrusters' mass curve (0x142CAC400): the multiplier at mass `m` from (min, optimal, max mass) and (min, optimal, max
/// multiplier). The max multiplier at min mass, the optimal at the optimal mass, the min at max mass.
pub fn mass_curve(m: f64, mass: [f64; 3], mul: [f64; 3]) -> f64 {
    let ([m0, mopt, m1], [k0, kopt, k1]) = (mass, mul);
    if mopt <= 0.0 {
        return 1.0;
    }
    if k0 >= k1 {
        return k0;
    }
    let n = (mopt - m0) / (m1 - m0);
    let e = if n >= 1.0 { 1.0 } else { ((kopt - k0) / (k1 - k0)).ln() / (1.0 - n).ln() };
    let x = ((m - m0) / (m1 - m0)).clamp(0.0, 1.0);
    k0 + (k1 - k0) * (1.0 - x).powf(e)
}

/// The controls this tick: demands in -1..1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// Rotation demands: pitch (nose up +), yaw (nose left +), roll (right wing up +).
    pub pitch: f64,
    pub yaw: f64,
    pub roll: f64,
    /// Linear demands: lateral (right +), vertical (up +), forward (the throttle: forward +, reverse -).
    pub lateral: f64,
    pub vertical: f64,
    pub forward: f64,
    pub flight_assist: bool,
}

/// The ship's motion, in a rest frame the game gives (`vel` m/s, `rot` ship axes to the rest frame) and its own spin
/// (`omega` rad/s in ship axes), with what the rules carry from one tick to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub vel: DVec3,
    pub rot: DQuat,
    pub omega: DVec3,
    /// What the jerk limits remember. `last_acc`: the linear limit's (its length is read), the thrust acceleration as the jerk
    /// limit left it, before the speed guard, written only on a tick the limit runs (flight assist on, not boosting): the exe's
    /// +0x630 (0x142E6E960), so after a boost the limit goes on from where it stood before the boost. `last_alpha`: the
    /// angular acceleration of last tick (+0x640, written every tick).
    pub last_acc: DVec3,
    pub last_alpha: DVec3,
}

impl Default for Motion {
    fn default() -> Self {
        Motion { vel: DVec3::ZERO, rot: DQuat::IDENTITY, omega: DVec3::ZERO, last_acc: DVec3::ZERO, last_alpha: DVec3::ZERO }
    }
}

/// What a tick worked out: the thrusters' acceleration (ship axes, without the outside pull) and the angular acceleration.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    pub thrust: DVec3,
    pub alpha: DVec3,
    pub cap: f64,
}

/// (lateral, vertical, forward) from ship axes (x right, y up, z back), and back.
fn to_flight(v: DVec3) -> DVec3 {
    DVec3::new(v.x, v.y, -v.z)
}

fn from_flight(f: DVec3) -> DVec3 {
    DVec3::new(f.x, f.y, -f.z)
}

/// One tick of the flight step (0x142E6E960) and its integration. `block` is this tick's current block (after the pips'
/// blend and the mass curve), `full_top` the full-power block's top speed, `boosting` whether boost is firing (the cap is the
/// boost speed, the upper limits take the boost's multiplier, D the boost's, and the linear jerk limit rests), `outside` the
/// outside acceleration in the rest frame (gravity: the flight assist holds against it). Returns what the thrusters did.
pub fn step(block: &Block, full_top: f64, boosting: bool, input: &Input, outside: DVec3, m: &mut Motion, dt: f64) -> Step {
    let inv = m.rot.inverse();
    let v = to_flight(inv * m.vel);
    let g = to_flight(inv * outside);
    let s = m.vel.length();
    let cap = if boosting { block.boost_speed } else { block.top_speed };
    let d = DVec3::new(input.lateral, input.vertical, input.forward).clamp(DVec3::splat(-1.0), DVec3::splat(1.0));
    // The flight assist's time constant (both modes use tau): the speed's share of the full-power top speed is held to 1
    // first (0x142E6E960), so T is at most D, over the cap after a boost too; D is the boost record's while boosting.
    let fa_d = if boosting { block.boost_fa_d } else { block.fa_d };
    let t_lin = (fa_d * (4.0 / 3.0 * (s / full_top.max(1e-9)).clamp(0.0, 1.0) - 1.0 / 3.0).max(0.0)).max(0.5);
    let tau = t_lin.max(dt);
    // The limits (0x142E6AF90, 0x142E6E960): braking a full forward demand only at a quarter of the reverse acceleration;
    // while boosting every upper limit times the boost's multiplier, lateral and vertical also times 0x30, 0x34.
    let mut upper = DVec3::new(block.accel_side, block.accel_side, block.accel_fwd);
    if boosting {
        upper *= DVec3::new(block.boost_side[0], block.boost_side[1], 1.0) * block.boost_accel;
    }
    let brake = if d.z >= FULL_FORWARD { FULL_FORWARD_BRAKE * block.accel_rev } else { block.accel_rev };
    let lower = DVec3::new(-block.accel_side, -block.accel_side, -brake);
    let share_cap = DVec3::new(block.side_share * cap, block.side_share * cap, cap);
    let mut a;
    if input.flight_assist {
        let mut want = d * cap;
        if want.length() > cap {
            want = want.normalize() * cap;
        }
        want.x = want.x.clamp(-block.side_share * cap, block.side_share * cap);
        want.y = want.y.clamp(-block.side_share * cap, block.side_share * cap);
        want.z = want.z.clamp(-block.back_share * cap, cap);
        a = (want - v) / tau - g;
        a = a.clamp(lower, upper);
        if block.jerk_lin > 0.0 && !boosting {
            let most = m.last_acc.length() + block.jerk_lin * dt;
            if a.length() > most {
                a = a.normalize_or_zero() * most;
            }
            m.last_acc = a;
        }
        // The speed guard: never past the larger of the speed now and the cap.
        a = guard(v, a + g, s.max(cap), dt) - g;
    } else {
        a = DVec3::ZERO;
        for k in 0..3 {
            let dk = d[k];
            if dk == 0.0 {
                continue;
            }
            let raw = dk.abs() * if dk >= 0.0 { upper[k] } else { lower[k] };
            let room = (dk.signum() * share_cap[k] - v[k]).abs() / tau;
            let at_cap = (dk > 0.0 && v[k] >= share_cap[k]) || (dk < 0.0 && v[k] <= -share_cap[k] * if k == 2 { block.back_share } else { 1.0 });
            a[k] = if at_cap { 0.0 } else { raw.clamp(-room, room) };
        }
        if s > full_top && s > 1e-9 {
            a -= v.normalize() * 0.25 * block.accel_rev;
        }
        let new = v + (a + g) * dt;
        let u = if g.length() > 1e-9 && new.length() > 1e-9 { new.normalize().dot(g.normalize()).max(0.0) } else { 0.0 };
        let base = s.max(cap);
        let limit = base + u * (500.0 - base).max(0.0);
        a = guard(v, a + g, limit, dt) - g;
    }
    // Rotation.
    let x = s / cap.max(1e-9);
    let turn_factor = [block.curve_turn.at(x), block.curve_turn.at(x), block.curve_roll.at(x)];
    let accel_factor = block.curve_accel.at(x);
    // (0x142E6E960, `rotation-clamp.md`) The stick scales the raw turn rate; only the clamp's bound carries the speed curve:
    // target = clamp(stick x W_raw, +-F x W_raw), F the turn curve for pitch and yaw and the roll curve for roll.
    let w_raw = DVec3::from_array([0, 1, 2].map(|k| block.turn[k].to_radians()));
    let w = DVec3::from_array([0, 1, 2].map(|k| w_raw[k] * turn_factor[k]));
    let amax = DVec3::from_array([0, 1, 2].map(|k| block.ang_accel[k].to_radians() * accel_factor));
    let stick = DVec3::new(input.pitch, input.yaw, input.roll).clamp(DVec3::splat(-1.0), DVec3::splat(1.0));
    let mut alpha = DVec3::ZERO;
    let mut remembered = DVec3::ZERO;
    for k in 0..3 {
        if input.flight_assist {
            let target = (stick[k] * w_raw[k]).clamp(-w[k], w[k]);
            let tr = (block.fa_r[k] * (10.0 / 9.0 * s / block.top_speed.max(1e-9) - 1.0 / 9.0).max(0.0)).max(dt);
            // tau_r can be 0 (at rest, R x 0, with dt 0): no error then asks for nothing, any other for the limit (not 0 / 0).
            let err = target - m.omega[k];
            let mut al = if err == 0.0 { 0.0 } else { (err / tr).clamp(-amax[k], amax[k]) };
            if block.ang_jerk[k] > 0.0 {
                let most = m.last_alpha[k].abs() + block.ang_jerk[k].to_radians() * dt;
                al = al.clamp(-most, most);
            }
            remembered[k] = al;
            // Reversal mode 1: the stick past 0.2 and the target against the spin (the spin's sign taken as +1 at 0, as the
            // exe's copysign does) turns harder; the jerk limit remembers the angular acceleration before this factor.
            if stick[k].abs() > REVERSAL_STICK && target.signum() * m.omega[k].signum() <= 0.0 {
                al *= 1.0 + block.reversal[k];
            }
            alpha[k] = al;
        } else {
            let spinning_that_way = (stick[k] > 0.0 && m.omega[k] >= w[k]) || (stick[k] < 0.0 && m.omega[k] <= -w[k]);
            alpha[k] = if spinning_that_way { 0.0 } else { amax[k] * stick[k] };
            // (0x142E6E960, step 0.11) A centred axis's spin is damped while another axis is steered, up to the cap's speed.
            let others = (0..3).any(|j| j != k && stick[j].abs() > 0.001);
            if stick[k].abs() <= 0.001 && others && s <= cap + 0.1 {
                alpha[k] += block.fa_off_damp[k] * -m.omega[k];
            }
            remembered[k] = alpha[k];
        }
    }
    // Integration (semi-implicit): the velocity with this tick's thrust and pull, then the turn.
    m.vel += m.rot * from_flight(a) * dt + outside * dt;
    m.omega += alpha * dt;
    let spin = DVec3::new(m.omega.x, m.omega.y, m.omega.z) * dt;
    if spin.length() > 0.0 {
        m.rot = (m.rot * DQuat::from_scaled_axis(spin)).normalize();
    }
    m.last_alpha = remembered;
    Step { thrust: from_flight(a), alpha, cap }
}

/// The speed guard: the acceleration shortened so that |v + a dt| is at most `limit` (only when it would pass it).
fn guard(v: DVec3, a: DVec3, limit: f64, dt: f64) -> DVec3 {
    let new = v + a * dt;
    if new.length() <= limit || a.length() < 1e-12 {
        return a;
    }
    // Solve |v + k a dt| = limit for k in [0, 1].
    let (p, q) = (a * dt, v);
    let (aa, bb, cc) = (p.dot(p), 2.0 * p.dot(q), q.dot(q) - limit * limit);
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < 0.0 || aa <= 0.0 {
        return DVec3::ZERO;
    }
    let k = ((-bb + disc.sqrt()) / (2.0 * aa)).clamp(0.0, 1.0);
    a * k
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block() -> Block {
        Block {
            top_speed: 200.0,
            boost_speed: 300.0,
            back_share: 0.5,
            side_share: 0.4,
            boost_ramp: 1.0,
            accel_fwd: 40.0,
            accel_rev: 30.0,
            accel_side: 30.0,
            jerk_lin: 0.0,
            fa_d: 0.0,
            boost_side: [1.0, 1.0],
            turn: [40.0, 20.0, 100.0],
            ang_accel: [200.0, 100.0, 240.0],
            ang_jerk: [0.0; 3],
            fa_r: [0.0; 3],
            curve_accel: Curve::FLAT,
            curve_turn: Curve::FLAT,
            curve_roll: Curve::FLAT,
            reversal: [0.0; 3],
            fa_off_damp: [0.0; 3],
            boost_accel: 1.0,
            boost_fa_d: 0.0,
        }
    }

    /// Boosting: every upper limit times the boost's multiplier (from rest the speed climbs at 4 x 40 m/s^2), the cap the
    /// boost speed; braking a full forward demand only at a quarter of the reverse acceleration.
    #[test]
    fn boost_multiplies_the_limits_and_full_forward_brakes_softly() {
        let b = Block { boost_accel: 4.0, ..block() };
        let dt = 1.0 / 60.0;
        let mut m = Motion::default();
        let full = Input { forward: 1.0, flight_assist: true, ..default_input() };
        step(&b, b.top_speed, true, &full, DVec3::ZERO, &mut m, dt);
        assert!((m.vel.length() - 160.0 * dt).abs() < 1e-9, "{:?}", m.vel);
        for _ in 0..600 {
            step(&b, b.top_speed, true, &full, DVec3::ZERO, &mut m, dt);
        }
        assert!((m.vel.length() - b.boost_speed).abs() < 0.01);
        // Over the cap with the throttle full: 0.25 x 30 m/s^2; with it at 0.9: the whole 30.
        let was = m.vel.length();
        step(&b, b.top_speed, false, &full, DVec3::ZERO, &mut m, dt);
        assert!(((was - m.vel.length()) / dt - 7.5).abs() < 1e-6);
        let was = m.vel.length();
        step(&b, b.top_speed, false, &Input { forward: 0.9, ..full }, DVec3::ZERO, &mut m, dt);
        assert!(((was - m.vel.length()) / dt - 30.0).abs() < 1e-6);
    }

    /// The linear jerk limit remembers only what it limited: a boost leaves its memory as it was, so braking after a boost
    /// starts again from that, not from the boost's acceleration; and T is at most D over the cap.
    #[test]
    fn the_jerk_limit_rests_while_boosting() {
        let b = Block { jerk_lin: 25.0, fa_d: 1.0, boost_fa_d: 1.0, boost_accel: 4.0, ..block() };
        let dt = 1.0 / 60.0;
        let mut m = Motion::default();
        for _ in 0..120 {
            step(&b, b.top_speed, true, &Input { forward: 1.0, flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, dt);
        }
        assert_eq!(m.last_acc, DVec3::ZERO);
        let was = m.vel.length();
        step(&b, b.top_speed, false, &Input { flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, dt);
        assert!(((was - m.vel.length()) / dt - 25.0 * dt).abs() < 1e-6, "{}", (was - m.vel.length()) / dt);
        assert!((m.last_acc.length() - 25.0 * dt).abs() < 1e-9);
    }

    /// Reversal mode 1: a stick against the spin turns with alpha x (1 + factor); with the spin, as before.
    #[test]
    fn reversal_mode_one_doubles_a_reversing_turn() {
        let b = Block { reversal: [1.0; 3], ..block() };
        let dt = 1.0 / 60.0;
        let mut m = Motion { omega: DVec3::new(0.3, 0.0, 0.0), ..Motion::default() };
        let s = step(&b, b.top_speed, false, &Input { pitch: -1.0, flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, dt);
        assert!((s.alpha.x + 2.0 * 200f64.to_radians()).abs() < 1e-9, "{:?}", s.alpha);
        assert!((m.last_alpha.x + 200f64.to_radians()).abs() < 1e-9);
        let mut m = Motion { omega: DVec3::new(0.3, 0.0, 0.0), ..Motion::default() };
        let s = step(&b, b.top_speed, false, &Input { pitch: 1.0, flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, dt);
        assert!((s.alpha.x - 200f64.to_radians()).abs() < 1e-9);
        // Under 0.2 of stick: none.
        let mut m = Motion { omega: DVec3::new(0.3, 0.0, 0.0), ..Motion::default() };
        let s = step(&b, b.top_speed, false, &Input { pitch: -0.1, flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, dt);
        assert!(s.alpha.x.abs() <= 200f64.to_radians() + 1e-9);
    }

    /// Accel mode 2: the top speed and the forward and lateral accelerations times k; braking `r k` at or under the cap and
    /// growing as `r (v / V0) sqrt(v / (V0 k))` over it.
    #[test]
    fn accel_mode_two() {
        let b = Block { top_speed: 1000.0, accel_fwd: 40.0, accel_rev: 90.0, accel_side: 30.0, ..block() };
        let at = b.mode2(0.5, 500.0);
        assert_eq!((at.top_speed, at.accel_fwd, at.accel_side), (500.0, 20.0, 15.0));
        assert!((at.accel_rev - 45.0).abs() < 1e-12);
        let over = b.mode2(0.5, 2000.0);
        assert!((over.accel_rev - 90.0 * 2.0 * 2.0).abs() < 1e-9, "{}", over.accel_rev);
        assert!((b.mode2(0.5, 100.0).accel_rev - 45.0).abs() < 1e-12);
    }

    fn run(b: &Block, input: Input, outside: DVec3, m: &mut Motion, secs: f64) {
        let dt = 1.0 / 60.0;
        for _ in 0..(secs / dt).round() as usize {
            step(b, b.top_speed, false, &input, outside, m, dt);
        }
    }

    #[test]
    fn blue_zone_curve() {
        let c = Curve { a: 0.6, b: 0.8, p: 0.5 };
        assert_eq!(c.at(0.0), 0.6);
        assert_eq!(c.at(0.5), 1.0);
        assert!((c.at(1.0) - 0.8).abs() < 1e-12);
        assert!((c.at(0.25) - 0.8).abs() < 1e-12);
        assert_eq!(Curve::FLAT.at(0.3), 1.0);
    }

    #[test]
    fn mass_curve_hits_its_three_points() {
        let (mass, mul) = ([24.0, 48.0, 72.0], [0.83, 1.0, 1.03]);
        assert!((mass_curve(24.0, mass, mul) - 1.03).abs() < 1e-12);
        assert!((mass_curve(48.0, mass, mul) - 1.0).abs() < 1e-9);
        assert!((mass_curve(72.0, mass, mul) - 0.83).abs() < 1e-12);
        assert!((mass_curve(100.0, mass, mul) - 0.83).abs() < 1e-12);
    }

    /// Flight assist on: full throttle reaches the cap without passing it, at most the forward acceleration on the way.
    #[test]
    fn assist_on_reaches_the_cap() {
        let b = block();
        let mut m = Motion::default();
        let dt = 1.0 / 60.0;
        let mut was = 0.0;
        for _ in 0..(10.0 / dt) as usize {
            step(&b, b.top_speed, false, &Input { forward: 1.0, flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, dt);
            let s = m.vel.length();
            assert!(s <= b.top_speed + 1e-9, "{s}");
            assert!((s - was) / dt <= b.accel_fwd + 1e-6);
            was = s;
        }
        assert!((m.vel.z + b.top_speed).abs() < 0.01, "{:?}", m.vel); // forward is -z
        // Throttle to zero: it stops.
        run(&b, Input { flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, 15.0);
        assert!(m.vel.length() < 0.01, "{:?}", m.vel);
    }

    /// Flight assist on holds against gravity (hovers); off, gravity wins.
    #[test]
    fn assist_holds_against_gravity() {
        let b = block();
        let g = DVec3::new(0.0, -9.81, 0.0);
        let mut m = Motion::default();
        run(&b, Input { flight_assist: true, ..default_input() }, g, &mut m, 5.0);
        assert!(m.vel.length() < 1e-6, "{:?}", m.vel);
        let mut m = Motion::default();
        run(&b, Input { flight_assist: false, ..default_input() }, g, &mut m, 1.0);
        assert!((m.vel.y + 9.81).abs() < 0.02, "{:?}", m.vel);
    }

    /// Flight assist off: the ship drifts at the speed it had; the reverse and lateral caps are the block's shares.
    #[test]
    fn assist_off_drifts_and_caps() {
        let b = block();
        let mut m = Motion::default();
        run(&b, Input { forward: 1.0, flight_assist: false, ..default_input() }, DVec3::ZERO, &mut m, 10.0);
        assert!((m.vel.length() - b.top_speed).abs() < 0.5, "{:?}", m.vel);
        let v = m.vel;
        run(&b, Input { flight_assist: false, ..default_input() }, DVec3::ZERO, &mut m, 5.0);
        assert!((m.vel - v).length() < 1e-9);
        let mut m = Motion::default();
        run(&b, Input { lateral: 1.0, flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, 10.0);
        assert!((m.vel.x - b.side_share * b.top_speed).abs() < 0.01, "{:?}", m.vel);
    }

    /// Rotation: assist on settles at the turn rate; off keeps spinning when the stick lets go.
    #[test]
    fn rotation_rates() {
        let b = block();
        let mut m = Motion::default();
        run(&b, Input { pitch: 1.0, flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, 3.0);
        assert!((m.omega.x - 40f64.to_radians()).abs() < 1e-6, "{:?}", m.omega);
        run(&b, Input { flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, 3.0);
        assert!(m.omega.length() < 1e-6);
        let mut m = Motion::default();
        run(&b, Input { roll: 1.0, flight_assist: false, ..default_input() }, DVec3::ZERO, &mut m, 3.0);
        let w = m.omega.z;
        assert!((w - 100f64.to_radians()).abs() < 0.1, "{w}");
        run(&b, Input { flight_assist: false, ..default_input() }, DVec3::ZERO, &mut m, 1.0);
        assert!((m.omega.z - w).abs() < 1e-9);
    }

    /// A tick of no time at rest (tau_r = R x 0 = dt = 0) leaves the spin 0, not NaN; a stick then asks for the limit.
    #[test]
    fn a_tick_of_no_time_at_rest_keeps_the_spin() {
        let b = Block { fa_r: [0.1; 3], ..block() };
        let mut m = Motion::default();
        let s = step(&b, b.top_speed, false, &Input { flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, 0.0);
        assert_eq!((s.alpha, m.omega, m.last_alpha), (DVec3::ZERO, DVec3::ZERO, DVec3::ZERO));
        assert!(m.rot.is_finite() && m.vel.is_finite());
        let s = step(&b, b.top_speed, false, &Input { pitch: 1.0, flight_assist: true, ..default_input() }, DVec3::ZERO, &mut m, 0.0);
        assert!((s.alpha.x - 200f64.to_radians()).abs() < 1e-9 && m.omega == DVec3::ZERO, "{:?}", s.alpha);
    }

    fn default_input() -> Input {
        Input::default()
    }
}
