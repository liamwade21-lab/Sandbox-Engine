//! What the flight HUD shows, in Elite's units (ARCHITECTURE.md A10 "Data (UI4, UI5)"): filled once a frame by the game
//! (`sidewinder/ui/facts.rs`) from the mod, read by the feeders (`feed`) and nothing else. Each field gives its unit, its
//! range and the message that uses it (notes: `re/elite/notes/ui_messages_hud.md`). A field the mod may have no source
//! for is an `Option`: `None` means its message is not sent and the movie shows its own empty state (the user's scope
//! rule: nothing is invented to fill the UI).

/// Everything the flight HUD's movies are fed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiFacts {
    pub status: Status,
    pub speed: Speed,
    pub heat: Heat,
    pub fuel: Fuel,
    pub damage: Damage,
    pub power: Power,
    pub signature: Signature,
    /// The locked destination; None with nothing locked (the compass hidden, the target location cleared).
    pub destination: Option<Destination>,
    pub drive: Drive,
    pub weapons: Weapons,
    pub alerts: Alerts,
    pub clock: Clock,
    /// The planet the ship is near (a planet or moon: the proximity component tracks only those); None far from any.
    pub planet: Option<Planet>,
}

/// The seat, the power and the deployables.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// Someone sits in the seat (the HUD movies are shown only then: `feed::life`).
    pub seated: bool,
    /// The power plant runs (the HUD movies are shown only then: `feed::life`, stand-in).
    pub powered: bool,
    /// The landing gear's command (Elite's request byte +0xBB): `SetStatusIndicators` second plate.
    pub gear_down: bool,
    /// The cargo scoop deployed: `SetStatusIndicators` third plate.
    pub cargo_scoop: bool,
    /// The frame shift drive is mass locked (FSD +0xA0 and not +0xA8): `SetStatusIndicators` first plate.
    pub mass_locked: bool,
}

/// The speed gauge (`SetThrottle`, `SetThrottleMaxSpeed`, `SetThrottleOptimal`, normal flight's branch).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Speed {
    /// In supercruise (the gauge's other branch, whose speed rule was not decoded: nothing is sent then).
    pub supercruise: bool,
    /// The speed (m/s, >= 0): `speedString` and the gauge's bar.
    pub speed_m_s: f64,
    /// The velocity's component along the ship's nose (m/s): Elite's ship axes' +z, forward positive, backwards negative.
    /// At or under -0.05 m/s the bar is sent negative and the movie draws it on the gauge's reverse half (B: the exe's
    /// sign test, DAT_145324cc0).
    pub forward_m_s: f64,
    /// The top speed the engine pips allow now (m/s; the flight component's [0x750], the Sidewinder 100 / 160 / 220 at
    /// 0 / 2 / 4 ENG pips) and at full power ([0x81c], 220).
    pub top_m_s: f64,
    pub full_top_m_s: f64,
    /// The engine module is powered ([0x910] 1, else 0: the bar then reads 0).
    pub engines_on: bool,
    /// The forward/reverse thrust demand, -1..1 (the input record +0x10): the throttle's `desired`.
    pub demand: f64,
    /// Flight assist on: the throttle's `desiredVisible`, and `SetFlightAlert` (assist off) on the info panel.
    pub flight_assist: bool,
}

/// Heat (`SetHeat`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heat {
    /// The heat fraction, heat / capacity (Elite's heatProp; 1.5 is the HeatWarning100 voice, 0..3.5 on the bar).
    pub prop: f64,
}

/// Fuel (`SetFuelLevels`), from the FuelManager's interface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fuel {
    /// The main tank and the reservoir: level and capacity (t).
    pub main_t: f64,
    pub main_capacity_t: f64,
    pub reservoir_t: f64,
    pub reservoir_capacity_t: f64,
    /// The burn rate (t/s, slot +0x98): the usage text, smoothed.
    pub burn_t_per_s: f64,
    /// The fuel the jump to the locked system would use (t); None with no system locked or the drive busy (sent as 0).
    pub jump_cost_t: Option<f64>,
}

/// The shield ring and the hull gauge (`SetDamage`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Damage {
    pub shield: Shield,
    /// The hull's strength, 0..1.
    pub hull: f64,
}

/// The Sidewinder's one shield zone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shield {
    /// Its strength, 0..1.
    pub strength: f64,
    pub state: ShieldState,
}

/// A shield zone's state (A: the gauge movie's `Content.SHIELDS_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShieldState {
    None,
    TurnedOff,
    #[default]
    Active,
    Offline,
    Recharging,
    Overcharge,
    /// Just came back.
    Online,
}

impl ShieldState {
    /// The code the movie reads.
    pub fn code(self) -> i32 {
        match self {
            ShieldState::None => -2,
            ShieldState::TurnedOff => -1,
            ShieldState::Active => 0,
            ShieldState::Offline => 1,
            ShieldState::Recharging => 2,
            ShieldState::Overcharge => 3,
            ShieldState::Online => 4,
        }
    }
}

/// A value for each of the distributor's channels, in the messages' order of names.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Channels {
    pub wep: f64,
    pub eng: f64,
    pub sys: f64,
}

/// The power distributor (`SetPowerLevels`, `SetPowerAssignment`, `Indicate*PowerInput`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Power {
    /// Each channel's pips as a share of its most, 0..1 (half-pips / 8).
    pub pips: Channels,
    /// Each capacitor's charge / capacity, 0..1.
    pub capacitors: Channels,
    /// The pip keys pressed this frame.
    pub keys: PipKeys,
}

/// The power distributor's keys pressed this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipKeys {
    pub eng: bool,
    pub sys: bool,
    pub wep: bool,
    pub reset: bool,
}

/// The signature bar (`SetSignature`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Signature {
    /// The emission aggregator's first two channels over their maxima, 0..1 each. What they are was not found (E), so the
    /// mod has no source: None, and the bar is never fed.
    pub channels: Option<[f64; 2]>,
    /// Silent running (the ship record's byte +0xA5): the bar's "silent" clip.
    pub silent: bool,
}

/// The locked destination (`SetCompassData` and the compass's visibility, `SetTargetLocation`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Destination {
    /// Its LocationType (`location_type`): the panel's icon.
    pub kind: i32,
    /// Its name as shown (a `$KEY;` is translated by the movie).
    pub name: String,
    /// Its distance (m), written by the movie as Elite's distance text (`$DISTANCE`: m, km, Mm, Ls, Ly). Whether Elite
    /// measures to a body's centre or its surface was not decoded (E); the filler delivers what the mod's own HUD shows
    /// for the lock (`DestMark::dist_m`: the height over the ground for the body the ship is in, to the mean radius for
    /// another body of the system, to the star for another system).
    pub distance_m: f64,
    /// The unit vector to it in Elite's ship axes (x right, y up, z forward): the compass draws its pointer at (x, -y)
    /// x 32 px from its middle, the "forward" frame for z > 0 and "reverse" otherwise (A: the movie's `Compass.setData`).
    /// The zero vector means no compass (the panel still shows the destination).
    pub dir: [f64; 3],
}

/// The LocationType codes a destination of the mod can have (A: the gauge movie's `LocationType`).
pub mod location_type {
    pub const SYSTEM: i32 = 0;
    pub const STAR: i32 = 1;
    pub const PLANET: i32 = 2;
    pub const BELT: i32 = 3;
    pub const RING: i32 = 11;
    pub const LANDABLE_PLANET: i32 = 28;
    pub const ATMOSPHERE_PLANET: i32 = 36;
    pub const LANDABLE_ATMOSPHERE_PLANET: i32 = 37;
}

/// The frame shift drive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drive {
    /// The charge's share, 0..1, while charging (for supercruise or a jump): the info panel's charge bar.
    pub charge: Option<f64>,
    /// The cooldown's progress, 0..1 (FSD +0x178, "progress": B), while it cools down: the gauge panel's cooldown ring, a
    /// 90-frame clip at frame 1 + int(prop x 90 - 1) whose first frame is a sliver and whose last is the full ring (A: the
    /// clip's shapes, sprite 1773). The filler delivers the share of the cooldown done: 0 just after the drop, 1 when the
    /// drive is ready again, so the ring fills as the drive gets ready (which way Elite runs it was not decoded: E). None
    /// when not cooling (the ring is hidden).
    pub cooldown: Option<f64>,
}

/// The weapons (`cockpithudring`, the central gunsight).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Weapons {
    /// The hardpoints are deployed (the central gunsight shows while armed).
    pub armed: bool,
    /// The active fire group, 0 = A .. 7 = H; None: not sent.
    pub firegroup: Option<u8>,
    /// The fitted weapons, one per slot in slot order; None: the ring is not fed.
    pub hardpoints: Option<Vec<Hardpoint>>,
}

/// One weapon slot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardpoint {
    /// The module's item id, its name as shown (already translated), its weapon type and mode (WeaponModes: 0 forward
    /// fire, 1 gimballed, 2 turret...), as the item data gives them (`SetHardpoints`).
    pub module_item_id: u32,
    pub name: String,
    pub weapon_type: i32,
    pub weapon_mode: i32,
    /// The deploy and retract animations' lengths (s).
    pub deploy_s: f64,
    pub retract_s: f64,
    /// Where its deployment is.
    pub deploy: Deploy,
    /// The module is switched on (ModuleIsOn +0x18) and powered (+0x70).
    pub module_on: bool,
    pub powered: bool,
    /// The fire group's answer for it (the fire-group interface +0x30): 1 primary, 2 secondary, 3 and 4 as the exe numbers
    /// them, 0 none.
    pub trigger: u8,
}

/// A hardpoint's deployment (the module's deploy state MB +0x88: 0 in, 1 and 2 deploying, 3 out, 4 retracting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Deploy {
    #[default]
    Retracted,
    Deploying,
    Deployed,
    Retracting,
}

/// The info panel's alarms the mod has a cause for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Alerts {
    /// The life support alarm (`SetLifeSupportAlarm`), and the emergency oxygen left while it runs (s:
    /// `SetLifeSupportFailureIn`).
    pub life_support: LifeAlarm,
    pub oxygen_s: f64,
}

/// The life support alarm's three states (the exe's `FUN_142d59590`: critical at 30 s or less).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LifeAlarm {
    #[default]
    Off,
    On,
    Critical,
}

/// The game clock (`SetTime`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clock {
    /// Seconds since midnight of the game's day; None: not sent.
    pub time_of_day_s: Option<f64>,
}

/// Planetary flight (`orbitalhud`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Planet {
    /// The body's radius R (m) and the ship's distance from its centre (m): the altimeter is radius based (d - R).
    pub radius_m: f64,
    pub centre_distance_m: f64,
    /// The body record's float H that the altimeter's inner mark uses (`[(body+0x78)+0x288]`, m): not found (E), None.
    pub inner_height_m: Option<f64>,
    /// The gauges' angles (degrees, `UpdateHeadingGauges`). What the movie does with them (A: `OrbitalHudApp`,
    /// `HeadingGauge`, `AttitudeGauge`): the heading moves the top tape (0..360); the pitch moves the ladder and is what
    /// its sweet-spot and danger zones are compared with; the roll is the angle the whole ladder is turned clockwise on the
    /// screen (Flash's rotation), its labels flipped past 90 degrees either way (the feeder sends the heading as the tape's value: -h
    /// under 180, 360 - h from 180; the tape labels its value negated). The convention the filler delivers (B: the
    /// exe works them out from the pose against the ground; the signs are our reading of a ladder that stays with the
    /// horizon): heading clockwise from the body's north, 0..360; pitch the nose over the horizon, nose up positive; roll
    /// positive with the LEFT wing down (the horizon then appears turned clockwise), i.e. the aircraft's roll negated.
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
    /// The landing guide's readings while it is active; None: not fed.
    pub landing: Option<Landing>,
}

/// The landing guide (`UpdateLandingGuide`), from the planetary landing component.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Landing {
    /// The ship's roll and pitch against the landing surface's normal (degrees, the same signs as `Planet`'s): they turn and
    /// move the guide's lines, and the chevrons show which way to correct while either is beyond the tolerance sent.
    pub roll_deg: f64,
    pub pitch_deg: f64,
    /// The dot product of the ship's up axis and the landing surface's normal, both unit: 1 when the ship sits level on
    /// the surface. Aligned from cos 5 degrees (A: the landing component's +0x60; that the axis the exe takes, the pose's
    /// row at +0x10, is the up axis is our reading, B).
    pub axis_dot_normal: f64,
    /// The speed (m/s) and the mod's landing speed limit (m/s: `elite_sim::landing::TOUCHDOWN_MPS`, 15, the landing
    /// module's P10, which the exe's landing component returns at +0x70). Over it the guide shows "Slow Down" (the movie's
    /// TOO_FAST state).
    pub speed_m_s: f64,
    pub speed_limit_m_s: f64,
    /// The terrain is suitable (+0x28): its writer was not found (E); None: the guide is not fed.
    pub suitable_terrain: Option<bool>,
    /// Touched down (+0x30).
    pub landed: bool,
}

/// A number that is not finite is no new value: the last one stands.
fn keep(now: &mut f64, last: f64) {
    if !now.is_finite() {
        *now = last;
    }
}

fn keep_opt(now: &mut Option<f64>, last: Option<f64>) {
    if now.is_some_and(|v| !v.is_finite()) {
        *now = last;
    }
}

fn keep_channels(now: &mut Channels, last: Channels) {
    keep(&mut now.wep, last.wep);
    keep(&mut now.eng, last.eng);
    keep(&mut now.sys, last.sys);
}

impl UiFacts {
    /// Every NaN or infinite number replaced by `last`'s (the last facts fed, whose numbers are all finite), so a bad input
    /// is no new value. A number with no earlier value to keep falls back to the default's (0; for a direction the zero
    /// vector: no compass), except a planet whose radius or distance is not finite with none before: it is dropped.
    pub fn keep_finite(&mut self, last: &UiFacts) {
        let (s, l) = (&mut self.speed, &last.speed);
        keep(&mut s.speed_m_s, l.speed_m_s);
        keep(&mut s.forward_m_s, l.forward_m_s);
        keep(&mut s.top_m_s, l.top_m_s);
        keep(&mut s.full_top_m_s, l.full_top_m_s);
        keep(&mut s.demand, l.demand);
        keep(&mut self.heat.prop, last.heat.prop);
        let (f, l) = (&mut self.fuel, &last.fuel);
        keep(&mut f.main_t, l.main_t);
        keep(&mut f.main_capacity_t, l.main_capacity_t);
        keep(&mut f.reservoir_t, l.reservoir_t);
        keep(&mut f.reservoir_capacity_t, l.reservoir_capacity_t);
        keep(&mut f.burn_t_per_s, l.burn_t_per_s);
        keep_opt(&mut f.jump_cost_t, l.jump_cost_t);
        keep(&mut self.damage.shield.strength, last.damage.shield.strength);
        keep(&mut self.damage.hull, last.damage.hull);
        keep_channels(&mut self.power.pips, last.power.pips);
        keep_channels(&mut self.power.capacitors, last.power.capacitors);
        if self.signature.channels.is_some_and(|c| c.iter().any(|v| !v.is_finite())) {
            self.signature.channels = last.signature.channels;
        }
        if let Some(d) = &mut self.destination {
            let l = last.destination.clone().unwrap_or_default();
            keep(&mut d.distance_m, l.distance_m);
            if d.dir.iter().any(|v| !v.is_finite()) {
                d.dir = l.dir;
            }
        }
        keep_opt(&mut self.drive.charge, last.drive.charge);
        keep_opt(&mut self.drive.cooldown, last.drive.cooldown);
        if let Some(hardpoints) = &mut self.weapons.hardpoints {
            for (k, hp) in hardpoints.iter_mut().enumerate() {
                let l = last.weapons.hardpoints.as_ref().and_then(|h| h.get(k)).cloned().unwrap_or_default();
                keep(&mut hp.deploy_s, l.deploy_s);
                keep(&mut hp.retract_s, l.retract_s);
            }
        }
        keep(&mut self.alerts.oxygen_s, last.alerts.oxygen_s);
        keep_opt(&mut self.clock.time_of_day_s, last.clock.time_of_day_s);
        if let Some(p) = &mut self.planet {
            let place_bad = !p.radius_m.is_finite() || !p.centre_distance_m.is_finite();
            if place_bad && last.planet.is_none() {
                self.planet = None;
                return;
            }
            let l = last.planet.clone().unwrap_or_default();
            keep(&mut p.radius_m, l.radius_m);
            keep(&mut p.centre_distance_m, l.centre_distance_m);
            keep_opt(&mut p.inner_height_m, l.inner_height_m);
            keep(&mut p.heading_deg, l.heading_deg);
            keep(&mut p.pitch_deg, l.pitch_deg);
            keep(&mut p.roll_deg, l.roll_deg);
            if let Some(g) = &mut p.landing {
                let lg = l.landing.unwrap_or_default();
                keep(&mut g.roll_deg, lg.roll_deg);
                keep(&mut g.pitch_deg, lg.pitch_deg);
                keep(&mut g.axis_dot_normal, lg.axis_dot_normal);
                keep(&mut g.speed_m_s, lg.speed_m_s);
                keep(&mut g.speed_limit_m_s, lg.speed_limit_m_s);
            }
        }
    }
}
