//! Weapons (step 2.6, first cut): the stock pair of small gimballed pulse lasers, the hardpoints, the WEP capacitor and heat.
//!
//! Numbers: the lasers' fire interval, energy a shot (from the WEP capacitor, all or nothing: the distributor's rule, C), heat a
//! shot, damage, range and falloff are coriolis's and the weapon record's (B, `PULSE_LASER_GIMBAL_SMALL`; the record's fields
//! named by their values only); both guns of the fire group fire together. The hardpoints' deploy time and the beam's showing
//! are stand-ins (E) until N1's W1 run films them (the hull rig's doors take 1.07 s out), and with the hardpoints in nothing
//! fires. Not found in the exe yet: the fire interval's and the beam's own rules (A21's list), the falloff's rule, the gimbals'
//! aim (no targets here: the guns fire along the nose).

use crate::data::{Laser, PULSE_LASER_GIMBAL_SMALL};
use crate::heat::{Heat, HeatSpec};
use crate::power::{Distributor, WEP};

/// Stand-ins (E) until measured: the hardpoints' deploy time (s), and how long a pulse's beam shows (s).
pub mod stand_in {
    pub const DEPLOY_S: f64 = 1.0;
    pub const BEAM_S: f64 = 0.08;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Weapons {
    pub laser: Laser,
    /// Hardpoints asked out, and how far out (0 in .. 1 out).
    pub deployed: bool,
    pub deploy_t: f64,
    /// Seconds until each gun may fire again, and since each last fired (the beam shows for `BEAM_S`).
    cooldown: [f64; 2],
    pub since_shot: [f64; 2],
    pub shots: u64,
}

impl Default for Weapons {
    fn default() -> Self {
        Weapons { laser: PULSE_LASER_GIMBAL_SMALL, deployed: false, deploy_t: 0.0, cooldown: [0.0; 2], since_shot: [f64::MAX; 2], shots: 0 }
    }
}

impl Weapons {
    pub fn ready(&self) -> bool {
        self.deploy_t >= 1.0
    }

    /// Whether gun `i`'s beam shows now.
    pub fn beam(&self, i: usize) -> bool {
        self.since_shot[i] < stand_in::BEAM_S
    }

    /// A tick: the hardpoints move, and with the trigger held each gun fires when its interval has passed and the WEP
    /// capacitor has the shot's energy; each shot adds its heat. Returns the guns that fired.
    pub fn tick(&mut self, dt: f64, trigger: bool, dist: &mut Distributor, heat: &mut Heat, hs: &HeatSpec) -> [bool; 2] {
        self.tick_guns(dt, [trigger; 2], dist, heat, hs)
    }

    /// `tick` with each gun's own trigger (a gun switched off on the Modules panel never fires).
    pub fn tick_guns(&mut self, dt: f64, triggers: [bool; 2], dist: &mut Distributor, heat: &mut Heat, hs: &HeatSpec) -> [bool; 2] {
        let towards = if self.deployed { 1.0 } else { 0.0 };
        self.deploy_t += (towards - self.deploy_t).clamp(-dt / stand_in::DEPLOY_S, dt / stand_in::DEPLOY_S);
        let mut fired = [false; 2];
        for i in 0..2 {
            self.cooldown[i] = (self.cooldown[i] - dt).max(0.0);
            self.since_shot[i] = if self.since_shot[i] == f64::MAX { f64::MAX } else { self.since_shot[i] + dt };
            if triggers[i] && self.ready() && self.cooldown[i] <= 0.0 && dist.draw(WEP, self.laser.dist_draw) {
                heat.add(hs, self.laser.thermal_load, None);
                self.cooldown[i] = self.laser.fire_interval;
                self.since_shot[i] = 0.0;
                self.shots += 1;
                fired[i] = true;
            }
        }
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data;

    /// Nothing fires with the hardpoints in; out, both guns fire at their interval while WEP holds out, each shot drawing
    /// WEP and adding heat; an empty capacitor stops them.
    #[test]
    fn lasers_fire_on_their_interval_and_drain_wep() {
        let hs = HeatSpec::sidewinder();
        let mut heat = Heat::new(&hs);
        let mut d = Distributor::new(&data::DISTRIBUTOR_1E);
        let mut w = Weapons::default();
        let dt = 1.0 / 60.0;
        for _ in 0..30 {
            assert_eq!(w.tick(dt, true, &mut d, &mut heat, &hs), [false, false]);
        }
        w.deployed = true;
        for _ in 0..70 {
            w.tick(dt, false, &mut d, &mut heat, &hs);
        }
        assert!(w.ready());
        let h0 = heat.heat;
        let mut shots = 0;
        for _ in 0..60 {
            shots += w.tick(dt, true, &mut d, &mut heat, &hs).iter().filter(|f| **f).count();
        }
        // One second: each gun fires at 0 and every 0.25 s.
        assert_eq!(shots, 8, "shots in a second");
        assert!(heat.heat > h0);
        assert!(d.charge[WEP] < d.caps[WEP].capacity);
        // Hold the trigger until WEP runs dry: the shots stop.
        for _ in 0..6000 {
            w.tick(dt, true, &mut d, &mut heat, &hs);
        }
        assert!(d.charge[WEP] < w.laser.dist_draw);
    }
}
