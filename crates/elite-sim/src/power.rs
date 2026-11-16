//! The power distributor (step 2.1): pips, three capacitors, their recharge and what draws on them.
//!
//! Rules (grade C, `docs/elite-flight-model.md`, steps 0.11 and A20): channel 0 is weapons, 1 engines, 2 systems; a press moves
//! `min(most - pips, step)` into its channel, first from unassigned pips, then from the others (`0x1437FF4E0`); each frame a
//! channel recharges `lo + (hi - lo) x s^p` with `s = pips / most` while the power plant gives anything (`0x1437FD920`), up to
//! its capacity (inferred); a draw takes all of what it asks or nothing (`0x143801BC0`), or what there is (`0x1438021C0`).
//!
//! Numbers: the pips are counted in halves, as Elite's `Status.json` counts them; the distributor prototype's
//! PowerDistributorModule template (B, N1: bytes +0x10..+0x12 the start, 4/4/4; +0x13 the most a channel, 8; +0x14 the pool a
//! reset leaves unassigned, 0; +0x16 a press's step, 2; read by 0x1437FEF60 and 0x143801FA0; the prototype chosen by its name)
//! agrees with session 1 (Up twice went 4/4/4, 3/6/3, 2/8/2; grade D). How a press shares its take between the two other
//! channels when one has too little is not measured: here the rest comes from the other (E). The capacitors' sizes and `hi`
//! are the distributor's module record (+0 and +8 per channel, B; coriolis agrees), and so are `lo` (+4) and `p` (+0xC): 0
//! and 1.1 on every channel (B).

use crate::data;

pub const WEP: usize = 0;
pub const ENG: usize = 1;
pub const SYS: usize = 2;

/// Half pips: the most in a channel, all three together (the start's, with no pool), what a press moves, and the start (B).
pub const MOST_HALF: u8 = 8;
pub const TOTAL_HALF: u8 = 12;
pub const STEP_HALF: u8 = 2;
pub const START_HALF: [u8; 3] = [4, 4, 4];

/// The recharge's rate with no pips in the channel, as a share of `hi`, and the curve's power (`lo + (hi - lo) s^p`): the
/// module record's +4 and +0xC on every channel (B).
pub const RECHARGE_LO: f64 = 0.0;
pub const RECHARGE_P: f64 = 1.1;

/// One channel's capacitor: its size (MJ) and its recharge rule's numbers (MW).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capacitor {
    pub capacity: f64,
    pub lo: f64,
    pub hi: f64,
    pub p: f64,
}

/// The distributor's state: the pips (halves) and each capacitor's charge (MJ).
#[derive(Clone, Debug, PartialEq)]
pub struct Distributor {
    pub half: [u8; 3],
    pub charge: [f64; 3],
    pub caps: [Capacitor; 3],
}

impl Distributor {
    /// A module's distributor, its capacitors full and the pips at the start.
    pub fn new(d: &data::Distributor) -> Distributor {
        let cap = |capacity: f64, hi: f64| Capacitor { capacity, lo: RECHARGE_LO * hi, hi, p: RECHARGE_P };
        let caps = [cap(d.wep_cap, d.wep_rate), cap(d.eng_cap, d.eng_rate), cap(d.sys_cap, d.sys_rate)];
        Distributor { half: START_HALF, charge: [caps[0].capacity, caps[1].capacity, caps[2].capacity], caps }
    }

    /// The channel's share of its most pips (0..1): the `t` of the performance blend and the `s` of the recharge.
    pub fn share(&self, c: usize) -> f64 {
        (self.half[c] as f64 / MOST_HALF as f64).clamp(0.0, 1.0)
    }

    /// The pips as whole pips (for the flight's blend: `share x 4`).
    pub fn pips(&self, c: usize) -> f64 {
        self.half[c] as f64 / 2.0
    }

    /// One press of the channel's key.
    pub fn increase(&mut self, c: usize) {
        let unassigned = TOTAL_HALF.saturating_sub(self.half.iter().sum());
        let mut want = (MOST_HALF - self.half[c].min(MOST_HALF)).min(STEP_HALF);
        let from_pool = want.min(unassigned);
        self.half[c] += from_pool;
        want -= from_pool;
        if want == 0 {
            return;
        }
        // Then from the other two, evenly; what one can't give comes from the other.
        let (a, b) = ((c + 1) % 3, (c + 2) % 3);
        let mut take = [want / 2 + want % 2, want / 2];
        if self.half[a] < self.half[b] {
            take.swap(0, 1);
        }
        let ta = take[0].min(self.half[a]);
        let tb = (want - ta).min(self.half[b]);
        let ta = (want - tb).min(self.half[a]);
        self.half[a] -= ta;
        self.half[b] -= tb;
        self.half[c] += ta + tb;
    }

    /// The reset key: back to the start.
    pub fn reset(&mut self) {
        self.half = START_HALF;
    }

    /// A frame's recharge (`powered`: the power plant gives anything).
    pub fn recharge(&mut self, dt: f64, powered: bool) {
        if !powered {
            return;
        }
        for c in 0..3 {
            let k = self.caps[c];
            let rate = k.lo + (k.hi - k.lo) * self.share(c).powf(k.p);
            self.charge[c] = (self.charge[c] + rate * dt).min(k.capacity);
        }
    }

    /// Draws `e` MJ from the channel, all or nothing: true if it was taken.
    pub fn draw(&mut self, c: usize, e: f64) -> bool {
        if self.charge[c] < e {
            return false;
        }
        self.charge[c] -= e;
        true
    }

    /// Draws up to `e` MJ from the channel: what was taken.
    pub fn draw_some(&mut self, c: usize, e: f64) -> f64 {
        let got = self.charge[c].min(e).max(0.0);
        self.charge[c] -= got;
        got
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Session 1's presses (grade D, read from `Status.json`): Up twice from 4/4/4 gives 3/6/3 (as SYS/ENG/WEP) then 2/8/2,
    /// and a third does nothing; the reset gives 4/4/4 back.
    #[test]
    fn pips_move_as_measured() {
        let mut d = Distributor::new(&data::DISTRIBUTOR_1E);
        d.increase(ENG);
        assert_eq!((d.half[SYS], d.half[ENG], d.half[WEP]), (3, 6, 3));
        d.increase(ENG);
        assert_eq!((d.half[SYS], d.half[ENG], d.half[WEP]), (2, 8, 2));
        d.increase(ENG);
        assert_eq!(d.half, [2, 8, 2]);
        d.reset();
        assert_eq!(d.half, START_HALF);
        // Four presses to all-but-engines: never below 0, the total kept.
        for c in [SYS, SYS, WEP, WEP, WEP] {
            d.increase(c);
            assert_eq!(d.half.iter().sum::<u8>(), TOTAL_HALF);
        }
        assert_eq!(d.half[ENG], 0);
    }

    /// Draws are all or nothing; the recharge stops at the capacity and with no power.
    #[test]
    fn draws_and_recharge() {
        let mut d = Distributor::new(&data::DISTRIBUTOR_1E);
        assert!(d.draw(ENG, 7.0));
        assert!(!d.draw(ENG, 7.0));
        assert!((d.charge[ENG] - 1.0).abs() < 1e-12);
        d.recharge(1.0, false);
        assert!((d.charge[ENG] - 1.0).abs() < 1e-12);
        d.recharge(1000.0, true);
        assert_eq!(d.charge[ENG], d.caps[ENG].capacity);
        assert!((d.draw_some(WEP, 100.0) - d.caps[WEP].capacity).abs() < 1e-12);
    }

    /// The record's recharge curve: `hi x s^1.1` (lo 0): for the ENG's 0.4 MW, 0.4 x 0.5^1.1 = 0.187 MW at 2 pips of 4, the
    /// whole 0.4 at 4 pips, nothing at 0.
    #[test]
    fn the_capacitor_curve() {
        let mut d = Distributor::new(&data::DISTRIBUTOR_1E);
        for (half, want) in [(0u8, 0.0), (4, 0.4 * 0.5f64.powf(1.1)), (8, 0.4)] {
            d.half[ENG] = half;
            d.charge[ENG] = 0.0;
            d.recharge(1.0, true);
            assert!((d.charge[ENG] - want).abs() < 1e-12, "{half}: {}", d.charge[ENG]);
        }
        assert_eq!((RECHARGE_LO, RECHARGE_P), (0.0, 1.1));
    }
}
