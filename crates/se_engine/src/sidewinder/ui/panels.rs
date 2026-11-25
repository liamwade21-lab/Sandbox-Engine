//! Elite's cockpit panels' data and what they ask of the ship (ARCHITECTURE.md A10 "Data (UI4, UI5)", task UI5b):
//! `UiPanels` owns the four panels' feeders (`elite_ui::feed::Panels`) and the frame's `PanelFacts`.
//!
//! - `hear` (first in `UiSet::Feed`): last frame's events of the four panel movies (`UiReplies`) as `elite_ui::panels::
//!   on_out` reads them; the focus machine takes their pop-ups from here (`focus::step`).
//! - `feed` (after the focus machine and the HUD's facts): the feeders keep the panels' own state (`Panels::hear`) and give
//!   back the ship's actions, which the ship does by the paths its keys and the stand-in panels use (`asks`: the stand-in's
//!   `panels::PanelAction`s, which `fly` does with the keys' rules; Disembark through the seat's leave request and its rule);
//!   then the facts (`facts_of`, a line a field naming its source) and the feeders' messages.
//! - Each feeder is fed only while its panel is on screen (`FocusState::on_screen`). A panel movie is first loaded when it
//!   first comes on screen (`movies.rs`), so what is sent to one not ready is dropped and its feeder alone sends its whole
//!   state again the frame after it becomes ready (`Panels::reset_movie`; all four when a ship's UI is made).
//! - The host's item database (`host::ItemDatabase`, the modules' names, sizes and ratings the right panel asks for) is the
//!   facts' modules, written here.
//! - Comms lines: the messages the pilot is shown while seated (`crate::messages::Messages::said`: the locks, the
//!   refusals, the drive's and the switches' notes that `fly` and `seat` raise), each a local line with a growing id, the
//!   last `COMMS_KEPT` handed to the feeder.

use std::collections::HashMap;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use elite_gfx::Out;
use elite_ui::facts::location_type;
use elite_ui::feed::{Movie, Msg, Outbox, Panels};
use elite_ui::focus::Panel;
use elite_ui::panel_facts::{FireGroups, FireWeapon, FlightPhase, Functions, Location, LogLine, Module, PanelFacts, Power, Ship};
use elite_ui::panels::{on_out, Function, PanelAction};
use elite_ui::Value;
use se_core::catalog::ObjectRef;

use super::super::panels::{self as stand_in, NavRow, MODULES, M_LASERS};
use super::super::pilot::{Dest, Pilot};
use super::super::seat::Seat;
use super::super::sound::ui::UiSounds;
use super::super::Sidewinder;
use super::facts::{time_of_day_s, HudFacts};
use super::focus::UiFocus;
use super::load::PanelFiles;
use super::place::UiDraw;
use super::{UiMessages, UiMovieState, UiReplies, UiSet};

/// The stock Sidewinder's modules under Elite's names, in `panels::MODULES`' order (the journal's loadout, the fitting order):
/// Elite's item name (its texts `$<item>_Name;` are in the shipped string table: a test checks), Elite's module type
/// (`Module::kind`), the size and rating as the outfitting shows them, and the power draw (MW, `elite_sim::data`'s records;
/// the power plant and the fuel tank draw none). The lasers' item is the one `ui/facts.rs` names (session ui1's "1G").
const ITEMS: [(&str, i32, u8, char, f64); 10] = {
    use elite_sim::data as d;
    let laser = ("Hpt_PulseLaser_Gimbal_Small", 7, 1, 'G', d::PULSE_LASER_GIMBAL_SMALL.power);
    [
        ("Int_PowerPlant_Size2_Class1", 10, 2, 'E', 0.0),
        ("Int_Engine_Size2_Class1", 22, 2, 'E', d::THRUSTERS_2E.power),
        ("Int_Hyperdrive_Size2_Class1", 9, 2, 'E', d::FSD_2E.power),
        ("Int_LifeSupport_Size1_Class1", 23, 1, 'E', d::LIFE_SUPPORT_1E_POWER),
        ("Int_PowerDistributor_Size1_Class1", 20, 1, 'E', d::DISTRIBUTOR_1E.power),
        ("Int_Sensors_Size1_Class1", 6, 1, 'E', d::SENSORS_1E_POWER),
        ("Int_FuelTank_Size1_Class3", 11, 1, 'C', 0.0),
        ("Int_ShieldGenerator_Size2_Class1", 4, 2, 'E', d::SHIELD_2E.power),
        laser,
        laser,
    ]
};

/// The comms lines handed to the feeder: the last this many (ours; the feeder pushes each id once, so any recent window
/// does).
const COMMS_KEPT: usize = 32;

/// How deep a place's parents are followed to find a listed one (a catalog's chain is a few links: star, barycentre,
/// planet, moon).
const PARENT_HOPS: usize = 16;

/// A module's id in the panels (`Module::id`): its place in `panels::MODULES`, from 1.
fn module_id(index: usize) -> u32 {
    index as u32 + 1
}

/// The Navigation places' ids, kept for the run (Elite's `Location::id`: unique, never 0): an object keeps its id whenever
/// it is listed again.
#[derive(Default)]
pub struct Ids {
    of: HashMap<ObjectRef, u32>,
    objs: Vec<ObjectRef>,
}

impl Ids {
    /// The object's id, given it the first time (1, 2, ...).
    pub fn id(&mut self, obj: ObjectRef) -> u32 {
        if let Some(&id) = self.of.get(&obj) {
            return id;
        }
        self.objs.push(obj);
        let id = self.objs.len() as u32;
        self.of.insert(obj, id);
        id
    }

    /// The object of an id given out (None for 0 or one never given).
    pub fn obj(&self, id: u32) -> Option<ObjectRef> {
        self.objs.get((id as usize).checked_sub(1)?).copied()
    }
}

/// Elite's `LocationType` for a Navigation row by its class (`panels::NavRow::kind`, as `fly` names it): a star a star
/// (another system's row is its star, as the runtime's proof lists it), a solid planet, moon or dwarf planet a landable
/// planet (the mod sets down on any loaded solid ground; Elite's own rule, which also marks atmospheres, is not the mod's: E),
/// a gas giant and anything else a planet.
pub fn row_type(class: &str) -> i32 {
    match class {
        "STAR" | "SYSTEM" => location_type::STAR,
        "PLANET" | "MOON" | "DWARF PLANET" => location_type::LANDABLE_PLANET,
        _ => location_type::PLANET,
    }
}

/// The Navigation list from `fly`'s rows (`panels::Panels::nav`): each place's id (`ids`), type, distance, its parent (the
/// nearest of its catalog parents, `parent_of`, that is listed in this system), whether the ship is at it (`here`: the body
/// whose frame it flies in) and whether it is the locked destination.
pub fn locations(rows: &[NavRow], ids: &mut Ids, parent_of: impl Fn(ObjectRef) -> Option<ObjectRef>, here: Option<ObjectRef>, locked: Option<ObjectRef>) -> Vec<Location> {
    let in_system = |obj: ObjectRef| rows.iter().any(|r| !r.other_system && r.obj == obj);
    let listed_parent = |obj: ObjectRef| std::iter::successors(parent_of(obj), |&p| parent_of(p)).take(PARENT_HOPS).find(|&p| in_system(p));
    rows.iter()
        .map(|r| Location {
            id: ids.id(r.obj),
            name: r.name.clone(),
            kind: row_type(r.kind),
            distance_m: r.dist_m,
            parent: if r.other_system { None } else { listed_parent(r.obj).map(|p| ids.id(p)) },
            current: !r.other_system && Some(r.obj) == here,
            locked: Some(r.obj) == locked,
            other_system: r.other_system,
        })
        .collect()
}

/// What a panel's action is in the mod: a stand-in panel action (`fly` does it with the keys' rules) or the seat's leave.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ask {
    Ship(stand_in::PanelAction),
    Leave,
}

/// A panel's ship action as the mod does it (`facts` the facts the movies saw, whose places' ids came from `ids`; `locked`
/// the destination now; `modules` the switches now). Empty for what the mod has no path for (`CancelJump`: the Navigation
/// page's jump box shows only while the drive counts down, which the facts don't carry, so it is never pressed) and for the
/// panels' own state (the feeders keep it).
pub fn asks(action: &PanelAction, facts: &PanelFacts, ids: &Ids, locked: Option<Dest>, modules: &[bool; 10]) -> Vec<Ask> {
    use stand_in::PanelAction as S;
    // A lock through the stand-in's own pick (which ends in `Pilot::lock`); none if that place is locked already (the
    // stand-in's pick would unlock it).
    let lock = |id: u32| {
        let place = facts.locations.iter().find(|l| l.id == id)?;
        let obj = ids.obj(id)?;
        let dest = if place.other_system { Dest::System(obj) } else { Dest::Body(obj) };
        (locked != Some(dest)).then_some(Ask::Ship(if place.other_system { S::Target(obj) } else { S::NavBody(obj) }))
    };
    match *action {
        PanelAction::LockDestination(id) => lock(id).into_iter().collect(),
        PanelAction::Unlock => vec![Ask::Ship(S::Unlock)],
        // "Lock and engage": the lock, then the drive's own key (J: a jump with a system locked, supercruise otherwise).
        PanelAction::EngageSupercruise { lock: place } | PanelAction::EngageHyperdrive { lock: place } => place.and_then(lock).into_iter().chain([Ask::Ship(S::Jump)]).collect(),
        // The stand-in's switch flips a module: only when it differs and the pilot may switch it.
        PanelAction::SetModule { id, on } => {
            let i = (id as usize).wrapping_sub(1);
            (i < MODULES.len() && MODULES[i].1 && modules[i] != on).then_some(Ask::Ship(S::Module(i))).into_iter().collect()
        }
        PanelAction::Toggle(f) => vec![Ask::Ship(match f {
            Function::FlightAssist => S::FlightAssist,
            Function::LandingGear => S::Gear,
            Function::Lights => S::Lights,
            Function::CargoScoop => S::Scoop,
            Function::SilentRunning => S::Silent,
        })],
        PanelAction::Disembark => vec![Ask::Leave],
        _ => Vec::new(),
    }
}

/// The role panel's Disembark, paid for by a press. The movie sends `OnDisembarkPressed` twice a press, 0.15 s apart (its
/// own code), and the second can come much later: the movie stops running when the pilot gets out and sends it the next
/// time it runs (a boarding later, the role panel shown again). So an event acts only if a select press was sent to the role
/// panel's movie since the last Disembark event, acted on or dropped: each press pays for one action, and the pair's second
/// event finds nothing to pay with, whenever it arrives (across getting out, a change of focus, a reload of the movie or a
/// new ship).
#[derive(Default)]
pub struct Disembark {
    paid: bool,
}

impl Disembark {
    /// A message sent to a movie (`UiMessages`): a select press to the role panel's movie pays.
    pub fn sent(&mut self, msg: &Msg) {
        let press = [Value::from("UI_Select"), Value::from("buttonDown")];
        if msg.movie == Movie::RolePanel3 && msg.name == "OnInputButton" && msg.args == press {
            self.paid = true;
        }
    }

    /// A Disembark event: whether the seat is asked (the payment is spent either way).
    pub fn take(&mut self) -> bool {
        std::mem::take(&mut self.paid)
    }
}

/// The four panels' feeders, the facts they were last fed, the places' ids, and what the movies said last frame.
#[derive(Resource, Default)]
pub struct UiPanels {
    panels: Panels,
    facts: PanelFacts,
    ids: Ids,
    heard: Vec<PanelAction>,
    disembark: Disembark,
    comms: Comms,
}

/// The pilot's messages as comms lines: the last ones, and the messages' count last seen with the frame it was seen in.
#[derive(Default)]
pub struct Comms {
    lines: Vec<LogLine>,
    seen: Option<(u64, u32)>,
}

impl Comms {
    /// A frame's look at the messages (`said`: their count and the last text; `frame` this frame's number; `time_of_day_s`
    /// the clock): a new message since the last seated frame becomes a line. Out of the seat in between, or the first look,
    /// only takes the count (what was said meanwhile is not the cockpit's).
    pub fn hear(&mut self, said: (u64, &str), frame: u32, time_of_day_s: f64) {
        let new = matches!(self.seen, Some((count, at)) if at == frame.wrapping_sub(1) && count != said.0);
        self.seen = Some((said.0, frame));
        if !new || said.1.is_empty() {
            return;
        }
        let id = self.lines.last().map_or(1, |l| l.id + 1);
        self.lines.push(LogLine { id, text: said.1.to_string(), time_of_day_s });
        if self.lines.len() > COMMS_KEPT {
            self.lines.remove(0);
        }
    }
}

impl UiPanels {
    /// What the panel movies said last frame (`on_out`'s actions).
    pub fn heard(&self) -> &[PanelAction] {
        &self.heard
    }
}

/// Everything `facts_of` reads, borrowed for the frame.
pub struct Sources<'a> {
    pub pilot: &'a Pilot,
    pub hud: &'a HudFacts,
    /// `fly`'s Navigation rows (`panels::Panels::nav`, made while the left panel has the keys).
    pub rows: &'a [NavRow],
    /// The body whose frame the ship flies in (`Parked::obj`).
    pub here: Option<ObjectRef>,
    /// The system's name (`ActiveSystem::name`).
    pub system: &'a str,
    /// A catalog object's parent (`Ephemeris::parent_of`).
    pub parent_of: &'a dyn Fn(ObjectRef) -> Option<ObjectRef>,
    /// The game's date (Julian day).
    pub jd: f64,
    /// The comms lines (`Comms`).
    pub comms: &'a [LogLine],
}

/// The panels' facts from the mod's state.
pub fn facts_of(s: &Sources, ids: &mut Ids) -> PanelFacts {
    let p = s.pilot;
    let hardpoints = s.hud.facts.weapons.hardpoints.as_deref().unwrap_or_default();
    let modules = ITEMS
        .iter()
        .enumerate()
        .map(|(i, &(item, kind, size, rating, power_mw))| Module {
            id: module_id(i),                                   // its place in `panels::MODULES`
            item: item.to_string(),                             // `ITEMS`: Elite's name of the stand-in's module
            kind,
            size,
            rating,
            power_mw,                                           // `elite_sim::data`'s draw
            on: p.modules[i],                                   // `Pilot::modules`, the Modules tab's switches
            switchable: MODULES[i].1,                           // `panels::MODULES`: the plant, distributor and tank are not
            health: 1.0,                                        // no source: the mod has no module damage
            priority: None,                                     // no source: the mod has no power priorities
            weapon_mode: M_LASERS.iter().position(|&m| m == i).and_then(|k| hardpoints.get(k)).map(|h| h.weapon_mode), // the HUD's facts' laser
        })
        .collect();
    PanelFacts {
        system: s.system.to_string(),                           // `ActiveSystem::name`
        locations: locations(s.rows, ids, s.parent_of, s.here, p.locked().map(Dest::obj)), // `fly`'s rows, `Pilot::locked`
        commander: None,                                        // no source: the mod has no commander
        ship: Ship { model: "sidewinder".to_string(), name: None, ident: None }, // the stock ship; no name or ident in the mod
        modules,
        power: Power {
            capacity_mw: elite_sim::data::POWER_PLANT_2E_MW,    // the plant's rating
            available_mw: if p.powered { elite_sim::data::POWER_PLANT_2E_MW } else { 0.0 }, // `Pilot::powered`: fuel aboard
        },
        fire_groups: FireGroups {
            count: 1,                                           // the stock ship's one group (`ui/facts.rs`: group A)
            active: s.hud.facts.weapons.firegroup.unwrap_or(0), // the HUD's facts' group
            weapons: hardpoints
                .iter()
                .zip(M_LASERS)
                .enumerate()
                .map(|(slot, (h, m))| FireWeapon { module: module_id(m), slot: slot as u32, trigger: 0, weapon_type: h.weapon_type, bindings: vec![h.trigger] })
                .collect(),                                     // the HUD's facts' lasers: their type and trigger
        },
        cargo_capacity_t: Some(elite_sim::data::CARGO_T),       // the loadout's cargo rack (the mod carries no cargo)
        functions: Functions {
            flight_assist: p.flight_assist,                     // `Pilot::flight_assist`
            gear_down: p.gear_down,                             // `Pilot::gear_down`, the gear's command
            gear_locked: p.landed,                              // `pilot::gear_toggle` refuses while landed
            cargo_scoop: p.scoop,                               // `Pilot::scoop`
            lights: p.lights,                                   // `Pilot::lights`
            silent_running: p.silent,                           // `Pilot::silent`
        },
        flight: phase(p),                                       // `Pilot`'s tunnel, landing, supercruise and glide
        leave_allowed: true,                                    // the seat's rule for B and Disembark: nothing refuses (A12)
        comms: s.comms.to_vec(),                                // the pilot's messages while seated (`Comms`)
        time_of_day_s: Some(time_of_day_s(s.jd)),               // the game's date (`SimTime::jd`, UTC), as the HUD's clock
    }
}

/// Where the ship is in its flight.
fn phase(p: &Pilot) -> FlightPhase {
    if p.jump.is_some() {
        FlightPhase::Hyperspace
    } else if p.landed {
        FlightPhase::Landed
    } else if p.cruise.is_some() {
        FlightPhase::Supercruise
    } else if p.glide.is_some() {
        FlightPhase::Glide
    } else {
        FlightPhase::Normal
    }
}

fn is_panel(movie: Movie) -> bool {
    Panel::ALL.iter().any(|p| p.movie() == movie)
}

/// Last frame's events of the panel movies. Only if this ran last frame too: the replies a seated period ended with are
/// not heard at the next boarding.
fn hear(frame: Res<FrameCount>, replies: Res<UiReplies>, mut ui: ResMut<UiPanels>, mut last: Local<Option<u32>>) {
    ui.heard.clear();
    let fresh = *last == Some(frame.0.wrapping_sub(1));
    *last = Some(frame.0);
    if !fresh {
        return;
    }
    for (movie, out) in &replies.outs {
        if let Out::Event { name, args } = out
            && is_panel(*movie)
        {
            ui.heard.extend(on_out(*movie, name, args));
        }
    }
}

/// Each seated frame: what the movies asked done, the facts, the feeders' messages.
#[allow(clippy::too_many_arguments)]
fn feed(
    (time, frame, said, files): (Res<Time<Real>>, Res<FrameCount>, Res<crate::messages::Messages>, Res<PanelFiles>),
    focus: Res<UiFocus>,
    pilot: Res<Pilot>,
    hud: Res<HudFacts>,
    (ship, data, system, sim): (Res<Sidewinder>, Res<crate::data::SeData>, Res<crate::system::ActiveSystem>, Res<crate::universe::SimTime>),
    (movies, draw): (Res<UiMovieState>, Res<UiDraw>),
    mut carrier: ResMut<stand_in::Panels>,
    mut seat: ResMut<Seat>,
    mut messages: ResMut<UiMessages>,
    sounds: Option<ResMut<UiSounds>>,
    mut ui: ResMut<UiPanels>,
) {
    let Some(state) = focus.state() else { return };
    let ui = &mut *ui;
    let heard = std::mem::take(&mut ui.heard);
    // (A backing plate's switch is the plates' own: `place.rs` applies it from the same replies.)
    for action in ui.panels.hear(&heard, &ui.facts).into_iter().filter(|a| !matches!(a, PanelAction::Background { .. })) {
        let asked = asks(&action, &ui.facts, &ui.ids, pilot.locked(), &pilot.modules);
        info!("sidewinder ui: the panels ask {action:?}: {}", if asked.is_empty() { "nothing the mod does".to_string() } else { format!("{asked:?}") });
        for ask in asked {
            match ask {
                Ask::Ship(a) => carrier.actions.push(a),
                Ask::Leave if ui.disembark.take() => seat.request_leave(),
                Ask::Leave => info!("sidewinder ui: Disembark dropped: no select press sent to the role panel since the last"),
            }
        }
    }
    // (This frame's presses, sent after the events above were made: a select press to the role panel pays for the next.)
    for m in &messages.msgs {
        ui.disembark.sent(m);
    }
    ui.comms.hear(said.said(), frame.0, time_of_day_s(sim.jd));
    let eph = se_core::ephemeris::Ephemeris::new(&data.catalog);
    let sources = Sources {
        pilot: &pilot,
        hud: &hud,
        rows: &carrier.nav,
        here: ship.parked.as_ref().map(|p| p.obj),
        system: &system.name,
        parent_of: &|o| eph.parent_of(o),
        jd: sim.jd,
        comms: &ui.comms.lines,
    };
    ui.facts = facts_of(&sources, &mut ui.ids);
    files.items.set(&ui.facts.modules);
    if draw.is_added() {
        ui.panels.reset();
    }
    for m in movies.fresh.iter().filter(|m| is_panel(**m)) {
        ui.panels.reset_movie(*m);
    }
    let mut out = Outbox::default();
    ui.panels.feed(&ui.facts, &state, time.delta_secs_f64(), &mut out);
    let ready = |m: Movie| movies.ready.contains(&m);
    messages.stores.extend(out.stores.into_iter().filter(|s| ready(s.movie)));
    messages.msgs.extend(out.msgs.into_iter().filter(|m| ready(m.movie)));
    if let Some(mut sounds) = sounds {
        for event in out.posts {
            sounds.post(event);
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<UiPanels>().add_systems(
        PostUpdate,
        (hear.before(super::focus::step), feed.after(super::focus::step).after(super::facts::fill)).in_set(UiSet::Feed).run_if(resource_exists::<UiDraw>),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(obj: ObjectRef, kind: &'static str, other_system: bool) -> NavRow {
        NavRow { obj, name: format!("{obj:?}"), kind, dist_m: 1.0e9, other_system }
    }

    /// Ids start at 1, never change for an object, and come back to it; 0 and unknown ids have none.
    #[test]
    fn ids_are_stable_and_start_at_one() {
        let mut ids = Ids::default();
        let (a, b) = (ObjectRef::Body(7), ObjectRef::Star(3));
        assert_eq!((ids.id(a), ids.id(b), ids.id(a)), (1, 2, 1));
        assert_eq!((ids.obj(1), ids.obj(2), ids.obj(0), ids.obj(3)), (Some(a), Some(b), None, None));
    }

    #[test]
    fn a_rows_type_comes_from_its_class() {
        assert_eq!(row_type("STAR"), location_type::STAR);
        assert_eq!(row_type("SYSTEM"), location_type::STAR);
        assert_eq!(row_type("GAS GIANT"), location_type::PLANET);
        for solid in ["PLANET", "MOON", "DWARF PLANET"] {
            assert_eq!(row_type(solid), location_type::LANDABLE_PLANET, "{solid}");
        }
    }

    /// A moon hangs under its planet past an unlisted barycentre, the planet under the star; another system's star has no
    /// parent; the ship's body is current; the lock is flagged.
    #[test]
    fn the_list_hangs_each_place_under_its_listed_parent() {
        let (star, bary, planet, moon, far) = (ObjectRef::Star(1), ObjectRef::Body(10), ObjectRef::Body(11), ObjectRef::Body(12), ObjectRef::Star(2));
        let parent_of = |o: ObjectRef| match o {
            ObjectRef::Body(12) => Some(bary),
            ObjectRef::Body(10) => Some(planet),
            ObjectRef::Body(11) => Some(star),
            ObjectRef::Star(2) => Some(star),
            _ => None,
        };
        let rows = [row(planet, "GAS GIANT", false), row(star, "STAR", false), row(moon, "MOON", false), row(far, "SYSTEM", true)];
        let mut ids = Ids::default();
        let list = locations(&rows, &mut ids, parent_of, Some(planet), Some(moon));
        let of = |o: ObjectRef| list.iter().find(|l| ids.obj(l.id) == Some(o)).unwrap();
        assert_eq!(of(moon).parent, Some(of(planet).id));
        assert_eq!(of(planet).parent, Some(of(star).id));
        assert_eq!((of(star).parent, of(far).parent), (None, None));
        assert!(of(planet).current && !of(moon).current && !of(far).current);
        assert!(of(moon).locked && !of(planet).locked);
        assert!(of(far).other_system && of(far).kind == location_type::STAR);
        assert!(list.iter().all(|l| l.id != 0));
    }

    /// The panels' ship actions become the stand-in's actions and the keys' presses; a lock of what is locked does nothing
    /// (the stand-in's pick would unlock it); a module switch only for one the pilot may switch, when it differs.
    #[test]
    fn the_actions_take_the_ships_own_paths() {
        use stand_in::PanelAction as S;
        let (body, star) = (ObjectRef::Body(5), ObjectRef::Star(9));
        let mut ids = Ids::default();
        let rows = [row(body, "PLANET", false), row(star, "SYSTEM", true)];
        let facts = PanelFacts { locations: locations(&rows, &mut ids, |_| None, None, None), ..PanelFacts::default() };
        let (b, s) = (ids.id(body), ids.id(star));
        let on = [true; 10];
        let ask = |a: PanelAction, locked| asks(&a, &facts, &ids, locked, &on);
        assert_eq!(ask(PanelAction::LockDestination(b), None), [Ask::Ship(S::NavBody(body))]);
        assert_eq!(ask(PanelAction::LockDestination(s), None), [Ask::Ship(S::Target(star))]);
        assert_eq!(ask(PanelAction::LockDestination(b), Some(Dest::Body(body))), []);
        assert_eq!(ask(PanelAction::LockDestination(99), None), []);
        assert_eq!(ask(PanelAction::Unlock, Some(Dest::Body(body))), [Ask::Ship(S::Unlock)]);
        assert_eq!(ask(PanelAction::EngageSupercruise { lock: Some(b) }, None), [Ask::Ship(S::NavBody(body)), Ask::Ship(S::Jump)]);
        assert_eq!(ask(PanelAction::EngageSupercruise { lock: None }, Some(Dest::Body(body))), [Ask::Ship(S::Jump)]);
        assert_eq!(ask(PanelAction::EngageHyperdrive { lock: Some(s) }, Some(Dest::System(star))), [Ask::Ship(S::Jump)]);
        assert_eq!(ask(PanelAction::SetModule { id: module_id(stand_in::M_THRUSTERS), on: false }, None), [Ask::Ship(S::Module(stand_in::M_THRUSTERS))]);
        assert_eq!(ask(PanelAction::SetModule { id: module_id(stand_in::M_THRUSTERS), on: true }, None), [], "already on");
        assert_eq!(ask(PanelAction::SetModule { id: module_id(0), on: false }, None), [], "the power plant can't be switched");
        assert_eq!(ask(PanelAction::SetModule { id: 0, on: false }, None), []);
        assert_eq!(ask(PanelAction::Toggle(Function::LandingGear), None), [Ask::Ship(S::Gear)]);
        assert_eq!(ask(PanelAction::Toggle(Function::Lights), None), [Ask::Ship(S::Lights)]);
        assert_eq!(ask(PanelAction::Toggle(Function::SilentRunning), None), [Ask::Ship(S::Silent)]);
        assert_eq!(ask(PanelAction::Toggle(Function::CargoScoop), None), [Ask::Ship(S::Scoop)]);
        assert_eq!(ask(PanelAction::Toggle(Function::FlightAssist), None), [Ask::Ship(S::FlightAssist)]);
        assert_eq!(ask(PanelAction::Disembark, None), [Ask::Leave]);
        assert_eq!(ask(PanelAction::CancelJump, None), []);
    }

    /// Each select press sent to the role panel pays for one Disembark: the pair's second event, late (after getting out, a
    /// boarding, the role panel shown again), finds nothing; a press to another movie, or a release, pays nothing.
    #[test]
    fn a_late_disembark_finds_nothing_to_pay_with() {
        let msg = |movie, name, args: &[&str]| Msg { movie, name, args: args.iter().map(|a| Value::from(*a)).collect() };
        let press = msg(Movie::RolePanel3, "OnInputButton", &["UI_Select", "buttonDown"]);
        let mut d = Disembark::default();
        assert!(!d.take(), "an event with no press");
        d.sent(&press);
        d.sent(&msg(Movie::RolePanel3, "OnInputButton", &["UI_Select", "buttonUp"]));
        assert!(d.take(), "the press's first event");
        assert!(!d.take(), "its second, 0.15 s later");
        // Out, boarded again, the role panel shown: the late second event of an earlier pair, then a real press.
        d.sent(&msg(Movie::CockpitExternalPanel, "OnInputButton", &["UI_Select", "buttonDown"]));
        d.sent(&msg(Movie::RolePanel3, "OnInputButton", &["UI_Down", "buttonDown"]));
        assert!(!d.take(), "late: nothing paid since");
        d.sent(&press);
        assert!(d.take() && !d.take(), "a new press, one action");
    }

    /// A message said while seated becomes a line with the next id; the first look and a look after a gap (out of the seat)
    /// only take the count; the window keeps the last `COMMS_KEPT`.
    #[test]
    fn the_pilots_messages_become_comms_lines() {
        let mut c = Comms::default();
        c.hear((4, "old"), 10, 0.0);
        c.hear((5, "Destination: Sun"), 11, 1.0);
        c.hear((5, "Destination: Sun"), 12, 1.0);
        c.hear((6, "Landing gear retracted"), 13, 2.0);
        c.hear((8, "said while out"), 20, 3.0);
        assert_eq!(c.lines.iter().map(|l| (l.id, l.text.as_str())).collect::<Vec<_>>(), [(1, "Destination: Sun"), (2, "Landing gear retracted")]);
        for k in 0..40u32 {
            c.hear((9 + u64::from(k), "again"), 21 + k, 4.0);
        }
        assert_eq!(c.lines.len(), COMMS_KEPT);
        assert_eq!(c.lines.last().map(|l| l.id), Some(2 + 40));
    }

    /// Every module's item has its name in the shipped string table, and the list follows `panels::MODULES`.
    #[test]
    fn the_modules_have_elites_names() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/ui/strings/en.json");
        let text = elite_ui::Text::from_json(&std::fs::read(path).unwrap()).unwrap();
        for (item, ..) in ITEMS {
            assert!(text.contains(&format!("{item}_Name")), "{item}_Name is not in the string table");
        }
        assert_eq!(ITEMS.len(), MODULES.len());
        for m in M_LASERS {
            assert_eq!(ITEMS[m].1, 7, "a laser is a hardpoint");
        }
    }
}
