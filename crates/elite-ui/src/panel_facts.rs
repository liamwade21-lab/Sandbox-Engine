//! What the four cockpit panels show that the mod has (ARCHITECTURE.md A10 "Data (UI4, UI5)", task UI5a): filled once a
//! frame by the game from the mod, read by the panels' feeders (`feed::Panels`) and nothing else. Each field gives its unit
//! and meaning. Where the mod may have nothing a field is an `Option` or an empty list, and the page shows the movie's own
//! empty state (the user's scope rule: nothing is invented to fill a page).
//!
//! What is not here because the mod has nothing for it: other ships and contacts (the left panel's Contacts and Target
//! tabs), missions and transactions, cargo, credits and ranks, crew, fighters and SRVs, the system's economy, government
//! and population, module damage and power priorities. Fuel and hull are not here either: no page in this task's scope
//! shows them (the role panel's Helm page, which does, is shown only to a pilot who is not in his ship).

/// Everything the side panels are fed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelFacts {
    /// The star system's name, as shown (the left panel's tracker "System"); empty with none.
    pub system: String,
    /// The Navigation list's places, in any order: the feeder orders them by Elite's rule (`feed::left`).
    pub locations: Vec<Location>,
    /// The commander's name, as shown (the right panel's home page, the role panel's header); None: the mod has none.
    pub commander: Option<String>,
    pub ship: Ship,
    /// The ship's modules, in fitting order (the order breaks power-draw ties in the Modules list).
    pub modules: Vec<Module>,
    pub power: Power,
    pub fire_groups: FireGroups,
    /// The cargo racks' capacity, tonnes (the Inventory page's "Cargo capacity 0/N": the mod carries no cargo); None: not
    /// known.
    pub cargo_capacity_t: Option<f64>,
    pub functions: Functions,
    /// Where the ship is in its flight (the role panel's reason for refusing to let the pilot out, the right panel's
    /// hyperspace page).
    pub flight: FlightPhase,
    /// The game's rule for B and Disembark allows getting out now (`seat::leave_rule`: since 2026-11-24 anywhere while
    /// seated; not landed, the ship is abandoned).
    pub leave_allowed: bool,
    /// The comms log's local lines the mod has to show, oldest first (none today). The game may hand over any recent
    /// window of them: each is pushed once, by its `LogLine::id`.
    pub comms: Vec<LogLine>,
    /// The game's clock, seconds since midnight (the comms log's times); None: no clock.
    pub time_of_day_s: Option<f64>,
}

impl PanelFacts {
    /// The facts with every NaN or infinite number made harmless (ours): a place with no finite distance is left out of the
    /// list, any other such number is 0. So no cache sees a value that never equals itself (a send every frame).
    pub fn finite(&self) -> PanelFacts {
        let num = |v: f64| if v.is_finite() { v } else { 0.0 };
        let mut f = self.clone();
        f.locations.retain(|l| l.distance_m.is_finite());
        for m in &mut f.modules {
            m.power_mw = num(m.power_mw);
            m.health = num(m.health);
        }
        f.cargo_capacity_t = f.cargo_capacity_t.filter(|c| c.is_finite());
        f.power.capacity_mw = num(f.power.capacity_mw);
        f.power.available_mw = num(f.power.available_mw);
        f.time_of_day_s = f.time_of_day_s.filter(|t| t.is_finite());
        for l in &mut f.comms {
            l.time_of_day_s = num(l.time_of_day_s);
        }
        f
    }
}

/// One place of the Navigation list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location {
    /// An id the game chooses, unique in the list and never 0 (0 is Elite's own "current position" row, which this list
    /// does not have: a place with id 0 is left out, a debug assertion); it comes back in `PanelAction::LockDestination`
    /// and the others.
    pub id: u32,
    /// The name as shown.
    pub name: String,
    /// Elite's `LocationType` (`facts::location_type`: 1 star, 2 planet, 28 landable planet...): the row's icon.
    pub kind: i32,
    /// The distance from the ship, metres.
    pub distance_m: f64,
    /// The place this one belongs to (a moon's planet, a planet's star), by its id; None for a root. The parents must not
    /// run in a circle (places in one are left out, a debug assertion). Ignored for another system's star.
    pub parent: Option<u32>,
    /// The ship is at this place (Elite's "current" marker: the body whose frame the ship flies in; at most one place), and
    /// the tracker's "location".
    pub current: bool,
    /// The locked destination.
    pub locked: bool,
    /// A star of another system: a hyperspace target, not a place of this system. The game lists the ones it can jump to
    /// (Elite lists the other systems within its jump range, notes part 1 7.A.1b); the feeder keeps the nearest 50.
    pub other_system: bool,
}

/// The ship's identity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ship {
    /// Elite's item name of the ship model (`sidewinder`): its text is `$<model>_name;`.
    pub model: String,
    /// The ship's own name and ident as shown; None: the mod has none.
    pub name: Option<String>,
    pub ident: Option<String>,
}

/// One module of the ship.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Module {
    /// An id the game chooses, unique among the modules: the Modules list's callback id and database id, echoed back in
    /// `PanelAction::SetModule`.
    pub id: u32,
    /// Elite's item name (`Int_PowerPlant_Size2_Class1`, `Hpt_PulseLaser_Gimbal_Small`): the texts are `$<item>_Name;` and
    /// `$<item>_LongName;`.
    pub item: String,
    /// Elite's module type (`ModuleBase` slot 0x30, the notes' part 2 13.2: 4 shield generator, 6 radar, 7 hardpoint, 9 frame
    /// shift drive, 10 power plant, 11 fuel tank, 20 power distributor, 22 main engine, 23 life support...).
    pub kind: i32,
    /// The size (1..8) and the rating letter ('A'..'I') as the outfitting shows them ("2E", "1G").
    pub size: u8,
    pub rating: char,
    /// The power draw, MW.
    pub power_mw: f64,
    /// The module's switch.
    pub on: bool,
    /// The pilot may switch it (the power plant cannot be).
    pub switchable: bool,
    /// The health, 0..1 (the mod has no module damage: 1).
    pub health: f64,
    /// The power priority group, 0-based; None: the mod has no priorities.
    pub priority: Option<u8>,
    /// For a weapon, its mount (Elite's `WeaponModes`: 0 fixed, 1 gimballed, 2 turret); None for others.
    pub weapon_mode: Option<i32>,
}

/// The power plant's output, MW.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Power {
    /// The full output (the power plant's rating) and what it gives now (0 with it off).
    pub capacity_mw: f64,
    pub available_mw: f64,
}

/// The fire groups.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FireGroups {
    /// How many groups exist (A, B...: the stock ship has one), and the active one, 0 = A.
    pub count: u8,
    pub active: u8,
    /// Each weapon's place in them, in slot order.
    pub weapons: Vec<FireWeapon>,
}

/// One weapon trigger's bindings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FireWeapon {
    /// The module (its `Module::id`).
    pub module: u32,
    /// The hardpoint slot's index and the weapon's trigger index.
    pub slot: u32,
    pub trigger: u32,
    /// Elite's weapon type code (the fire group categories: 0 beam, 2 launcher, 3 and 4 projectile, 10 limpet).
    pub weapon_type: i32,
    /// Per group (`FireGroups::count` of them): 0 none, 1 primary fire, 2 secondary fire.
    pub bindings: Vec<u8>,
}

/// The ship's switches that the right panel's Ship tab shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Functions {
    pub flight_assist: bool,
    /// The landing gear's command (down), and whether it cannot be moved now (landed: the mod's rule).
    pub gear_down: bool,
    pub gear_locked: bool,
    pub cargo_scoop: bool,
    /// The external lights on.
    pub lights: bool,
    pub silent_running: bool,
}

/// Where the ship is in its flight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlightPhase {
    /// Resting on the ground.
    Landed,
    /// Normal space, not landed.
    #[default]
    Normal,
    /// The glide from supercruise into a planet's air.
    Glide,
    Supercruise,
    /// In the hyperspace tunnel.
    Hyperspace,
}

/// One line for the comms log: a notification from the ship's systems (Elite's `[LOCAL] : text`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogLine {
    /// The game's id of the line, growing in the order the lines are said: each id is pushed to the log once.
    pub id: u64,
    /// The text as shown (literal, never a `$KEY;`).
    pub text: String,
    /// When it was said, seconds since midnight.
    pub time_of_day_s: f64,
}
