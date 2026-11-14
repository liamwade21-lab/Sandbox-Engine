//! Fuel (step 2.5): the main tank, the reservoir, the burn, the warnings and running dry.
//!
//! Rules (grade C, A20, `docs/elite-flight-model.md`): every consumer's burn comes from the reservoir (`0x14380AE40`); when the
//! reservoir is empty one reservoir's capacity moves over from the main tank, or all of it if less (`0x14380C6E0`, the journal's
//! `ReservoirReplenished`); the warnings fire as the fuel's share of both capacities falls through 25, 10 and 5 %
//! (`0x14380D3C0`); with nothing left the power plant's output drops to what the last fuel pays for, then to zero
//! (`0x1438011E0`).
//!
//! Numbers: the capacities are the journal's (B: main 2 t, reservoir 0.3 t). The burn's size by the plant's load is data not
//! in the exe: the rate is the HUD's own readout at idle, **0.42 t an hour** (D: the user's Elite, 2026-11-14, 2/2/2 pips at
//! rest, the fuel panel's "0.42/h"), the same whatever the load until a measurement of the burn in flight (a stand-in for that
//! part).

/// The HUD's fuel rate at idle, tonnes an hour (D).
pub const IDLE_T_PER_HOUR: f64 = 0.42;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FuelWarning {
    Quarter,
    Tenth,
    Twentieth,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fuel {
    pub main: f64,
    pub reservoir: f64,
    pub main_cap: f64,
    pub reservoir_cap: f64,
}

impl Fuel {
    /// The stock Sidewinder full (the journal's capacities).
    pub fn sidewinder() -> Fuel {
        let main = crate::data::FUEL_TANK_T;
        let reservoir = crate::data::SIDEWINDER.reserve_fuel;
        Fuel { main, reservoir, main_cap: main, reservoir_cap: reservoir }
    }

    pub fn total(&self) -> f64 {
        self.main + self.reservoir
    }

    pub fn share(&self) -> f64 {
        self.total() / (self.main_cap + self.reservoir_cap).max(1e-12)
    }

    /// Burns `t` tonnes from the reservoir, refilling it from the main tank as it empties. Returns the share of the asked
    /// burn that was there (1 unless the ship ran dry: the plant's output scales by it), and any warning crossed.
    pub fn burn(&mut self, t: f64) -> (f64, Option<FuelWarning>) {
        let before = self.share();
        let mut want = t.max(0.0);
        let mut got = 0.0;
        while want > 0.0 {
            let take = want.min(self.reservoir);
            self.reservoir -= take;
            got += take;
            want -= take;
            if want > 0.0 || self.reservoir <= 0.0 {
                if self.main <= 0.0 {
                    break;
                }
                let moved = self.reservoir_cap.min(self.main);
                if moved <= 0.0 {
                    break;
                }
                self.main -= moved;
                self.reservoir += moved;
            }
        }
        let after = self.share();
        let crossed = |x: f64| before > x && after <= x;
        let warning = if crossed(0.05) {
            Some(FuelWarning::Twentieth)
        } else if crossed(0.10) {
            Some(FuelWarning::Tenth)
        } else if crossed(0.25) {
            Some(FuelWarning::Quarter)
        } else {
            None
        };
        (if t > 0.0 { got / t } else { 1.0 }, warning)
    }

    /// A tick at idle's rate (the stand-in for every load).
    pub fn tick(&mut self, dt: f64) -> (f64, Option<FuelWarning>) {
        self.burn(IDLE_T_PER_HOUR / 3600.0 * dt)
    }

    /// The restock button (the spec's D5): full again.
    pub fn refill(&mut self) {
        self.main = self.main_cap;
        self.reservoir = self.reservoir_cap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The burn comes from the reservoir, refilled a reservoir's worth at a time; the warnings fire once each as the share
    /// falls; a dry ship gets less than it asked.
    #[test]
    fn reservoir_refills_and_warnings_fire() {
        let mut f = Fuel::sidewinder();
        let (got, _) = f.burn(0.2);
        assert_eq!(got, 1.0);
        assert!((f.main - 2.0).abs() < 1e-12 && (f.reservoir - 0.1).abs() < 1e-12);
        f.burn(0.15);
        // The reservoir emptied and took 0.3 t from the main tank.
        assert!((f.main - 1.7).abs() < 1e-12 && (f.reservoir - 0.25).abs() < 1e-9);
        let mut warnings = Vec::new();
        for _ in 0..2000 {
            if let (_, Some(w)) = f.burn(0.001) {
                warnings.push(w);
            }
        }
        assert_eq!(warnings, vec![FuelWarning::Quarter, FuelWarning::Tenth, FuelWarning::Twentieth]);
        let (got, _) = f.burn(1.0);
        assert!(got < 1.0);
        assert!(f.total() <= 1e-12);
        f.refill();
        assert!((f.total() - 2.3).abs() < 1e-12);
    }
}
