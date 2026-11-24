//! The left panel, `cockpitexternalpanel` (`re/elite/notes/ui_messages_panels.md` part 1; the movie's code
//! `code/cockpitexternalpanel/`). What is fed: the tab set (`SetPanelsMode`), the Navigation page and its tracker
//! (`SetDestinationData`), the Contacts page's empty list (`SetContactList`), and the location pop-up (`ShowLocationPopup`,
//! `HidePopup`).
//!
//! Left as the movie's own empty state, and why: Transactions (no missions: the movie's "No outstanding missions" page,
//! read from a DataStore nobody writes), Target (no contacts: "No contact targeted"; the mod's lock is a destination, not a
//! contact, so `SetTarget` has nothing to carry), the Summary's statistics (`SetLocalSystemData`: the mod has no economy,
//! government, security or population), the jump box (`SetJumpingInfo`, and `jumping`, `charging`, `timeLeft` stay
//! off: the drive's countdown is not in `PanelFacts`), the surface map button (`EnablePlanetaryMapButton`: the maps are cut).

use super::{changed, Movie, Outbox};
use crate::panel_facts::{FlightPhase, Location, PanelFacts};
use crate::panels::PanelAction;
use crate::Value;

const MOVIE: Movie = Movie::CockpitExternalPanel;

/// The lone ship's tab set: Navigation, Transactions, Contacts, Target and Summary (A: `FUN_142d2b2e0`, the Sidewinder's
/// performance record +0xC is 1, not a Proving Grounds match: notes part 1, the status section).
const PANELS_MODE: i32 = 0;
/// The Contacts page's id (`Content.as` ctor: Location 1, Missions 2, Contacts 3, Targets 4, Apex 8, Jumping 5, Summary 9).
const CONTACTS_PAGE: i32 = 3;
/// The other systems' list is cut to its nearest 50 (A: `FUN_142d75720`, 0x32).
const OTHER_SYSTEMS_MOST: usize = 50;
/// The tree walk stops below this depth (A: `FUN_142cf67c0`).
const DEPTH_MOST: usize = 10;
/// A parent of these types is skipped when a row looks for its parent (A: `FUN_142d056c0`: 0 System, 5 POIGeneric).
const SKIPPED_PARENTS: [i32; 2] = [0, 5];
/// The location pop-up's state (A: `FUN_144540830(.., state 2, ..)`, the game's popup state 2).
const POPUP_STATE: i32 = 2;
/// Elite's light year in metres: its distance format divides the light-year rung by it (A: notes part 1 7.B.6.4,
/// `FUN_144509790`; the same number as `text/format.rs`'s own, which is private to that module), and that rung's start,
/// a tenth of it.
const ELITE_LIGHT_YEAR_M: f64 = 9_221_136_340_147_200.0;
const LIGHT_YEAR_RUNG_M: f64 = ELITE_LIGHT_YEAR_M / 10.0;
/// The true light year (IAU: 365.25 days of light), the unit of the facts' plain metres.
const TRUE_LIGHT_YEAR_M: f64 = 9_460_730_472_580_800.0;

/// The location pop-up's buttons (A: notes part 1 7.B.6.3): Elite's ids and icon enums.
mod button {
    pub const LOCK: u32 = 0;
    pub const UNLOCK: u32 = 1;
    pub const SUPERCRUISE: u32 = 5;
    pub const HYPERDRIVE: u32 = 6;
    pub const GALAXY_MAP: u32 = 0xb;
    pub const SYSTEM_MAP: u32 = 0xc;
}

/// A row of the Navigation list: its place, and its depth in the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Row<'a> {
    pub location: &'a Location,
    pub depth: usize,
}

/// Elite's order of the Navigation list (A: notes part 1 7.A.1b, `FUN_142d75720` and the walk `FUN_142cf67c0`): this
/// system's places (list A) and the other systems' stars (list B, its nearest 50) each sorted by distance, nearest first
/// (ties keep the facts' order: Elite's are arbitrary); the nodes are list A then list B; a place of this system hangs under
/// its nearest listed ancestor that is not a System or a generic point of interest (a parent of those types is passed over
/// unless it ends the chain); every node without one is a root; then a pre-order walk of the roots in node order, children
/// in node order, depth the recursion's, nothing deeper than 10. A place with id 0 (reserved: `Location::id`) is left out,
/// and so is a place whose parents run in a circle (it can never reach a root); both are the game's mistakes (debug
/// assertions).
pub fn rows(locations: &[Location]) -> Vec<Row<'_>> {
    debug_assert!(locations.iter().all(|l| l.id != 0), "a Navigation place with the reserved id 0");
    let by_distance = |other: bool| {
        let mut list: Vec<&Location> = locations.iter().filter(|l| l.other_system == other && l.id != 0).collect();
        list.sort_by(|a, b| a.distance_m.total_cmp(&b.distance_m));
        list
    };
    let mut nodes = by_distance(false);
    let mut others = by_distance(true);
    others.truncate(OTHER_SYSTEMS_MOST);
    nodes.extend(others);
    let find = |id: u32| locations.iter().find(|l| l.id == id);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut has_parent = vec![false; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        if node.other_system {
            continue;
        }
        let mut parent = node.parent.and_then(find);
        let mut hops = 0;
        while let Some(p) = parent {
            let next = p.parent.and_then(find);
            if !SKIPPED_PARENTS.contains(&p.kind) || next.is_none() || hops > locations.len() {
                break;
            }
            parent = next;
            hops += 1;
        }
        if let Some(p) = parent
            && let Some(at) = nodes.iter().position(|n| n.id == p.id)
            && at != i
        {
            children[at].push(i);
            has_parent[i] = true;
        }
    }
    fn walk<'a>(at: usize, depth: usize, nodes: &[&'a Location], children: &[Vec<usize>], out: &mut Vec<Row<'a>>) {
        if depth > DEPTH_MOST {
            return;
        }
        out.push(Row { location: nodes[at], depth });
        for &c in &children[at] {
            walk(c, depth + 1, nodes, children, out);
        }
    }
    let mut out = Vec::new();
    for root in (0..nodes.len()).filter(|&i| !has_parent[i]) {
        walk(root, 0, &nodes, &children, &mut out);
    }
    // Every node hangs from a root unless its parents run in a circle (the walk's depth limit aside).
    debug_assert!(
        {
            let mut parent_of = vec![None; nodes.len()];
            for (at, kids) in children.iter().enumerate() {
                kids.iter().for_each(|&k| parent_of[k] = Some(at));
            }
            (0..nodes.len()).all(|i| {
                let mut at = i;
                (0..=nodes.len()).any(|_| match parent_of[at] {
                    Some(p) => {
                        at = p;
                        false
                    }
                    None => true,
                })
            })
        },
        "the Navigation places' parents run in a circle"
    );
    out
}

/// A row's distance text (A: `FUN_144648d00(&text, 1, node+0x10)`, the float at the node, in metres: Elite's own distance
/// format then picks m, km, Mm, Ls or ly). The facts' distance is the true one; the format divides its light-year rung by
/// Elite's own light year (2.6 % shorter than the true one) because Elite's galaxy is measured in it, so from a tenth of a
/// light year up the value handed over is the true distance in light years times Elite's light year, and the panel shows
/// the true figure (B: Elite's own values for other systems are in its own metres; below the rung the units, m, km, Mm and
/// Ls, are exact and the value passes as it is).
pub fn distance_text(m: f64) -> String {
    format!("$DISTANCE:#value={};", elite_metres(m) as f32)
}

/// The facts' true metres as the value Elite's distance format takes (see `distance_text`).
fn elite_metres(m: f64) -> f64 {
    if m >= LIGHT_YEAR_RUNG_M { (m / TRUE_LIGHT_YEAR_M * ELITE_LIGHT_YEAR_M).max(LIGHT_YEAR_RUNG_M) } else { m }
}

/// What a row's distance shows (Elite's own format, without its unit texts): the list is sent again only when this, or
/// anything else a row or the tracker shows, changes.
fn shown_distance(m: f64) -> String {
    crate::Text::default().distance(f64::from(elite_metres(m) as f32), false)
}

/// One row's record (A: `FUN_144503d20`'s keys in its order; `selectable` always 1, the reason empty: the mod has no
/// permits, fuel or mass refusals for a place of this system, notes part 1 7.B.6.4; no mission, bookmark, carrier,
/// security or status icons: 0 and false).
fn row_record(row: &Row, distance: String) -> Value {
    let l = row.location;
    Value::object([
        ("id", Value::from(l.id)),
        ("depth", (row.depth as u32).into()),
        ("type", l.kind.into()),
        ("stationStatus", 0.into()),
        ("label", l.name.as_str().into()),
        ("distance", distance.into()),
        ("selectable", true.into()),
        ("current", l.current.into()),
        ("disabledReason", "".into()),
        ("isMissionObjective", false.into()),
        ("statusIconA", 0.into()),
        ("statusIconB", 0.into()),
        ("statusIconC", 0.into()),
        ("statusIconD", 0.into()),
        ("isTargetLocation", l.locked.into()),
        ("hasBeenBookmarked", false.into()),
        ("isOwnCarrier", false.into()),
        ("securityLevel", 0.into()),
        ("showHatchesOverride", false.into()),
    ])
}

/// `SetDestinationData`'s object (A: the builder `FUN_1445039d0`'s keys; every key is sent, the panel reads them on its
/// paths). The mod has no route plotting: a locked star of another system is shown as a route of one jump to it (ours, B:
/// the tracker's "Destination" and its count), anything else as "No Route Set" (`numJumps` 0). `location` is the place the
/// ship is at; no jurisdiction, no filters, no jump in progress. `distance` makes each row's distance: the text sent, or
/// what it shows (the record then is the change test's key).
fn destination_data(facts: &PanelFacts, distance: fn(f64) -> String) -> Value {
    let current = facts.locations.iter().find(|l| l.current).map_or("", |l| l.name.as_str());
    let target = facts.locations.iter().find(|l| l.locked && l.other_system);
    let target_name = target.map_or("", |l| l.name.as_str());
    let list: Vec<Value> = rows(&facts.locations).iter().map(|r| row_record(r, distance(r.location.distance_m))).collect();
    Value::object([
        ("system", Value::from(facts.system.as_str())),
        ("location", current.into()),
        ("jurisdiction", "".into()),
        ("destinationList", list.into()),
        ("destination", target_name.into()),
        ("isOnRoute", false.into()),
        ("distance", "".into()),
        ("numJumps", (if target.is_some() { 1 } else { 0 }).into()),
        ("finalDestination", target_name.into()),
        ("jumping", false.into()),
        ("charging", false.into()),
        ("timeLeft", 0.into()),
        ("totalTime", 0.into()),
        ("filtersActive", false.into()),
        ("isPlanningRouteInProgress", false.into()),
        ("routeProgress", 0.into()),
        ("hasPendingDestination", false.into()),
        ("isTargetingOffRouteSystem", false.into()),
    ])
}

/// A pop-up button (A: `FUN_144540570`'s keys; `selectable` always 1 in this builder).
fn popup_button(id: u32, label: &str, icon: i32) -> Value {
    Value::object([
        ("id", Value::from(id)),
        ("label", label.into()),
        ("iconEnum", icon.into()),
        ("isCodexButton", false.into()),
        ("tutorialHighlightKey", "".into()),
        ("isApex", false.into()),
        ("selectable", true.into()),
    ])
}

/// The location pop-up's buttons in Elite's order (A: notes part 1 7.B.6.3): Lock or Unlock destination; for a place that
/// is not the current one, Engage supercruise ("Lock and supercruise" when not locked) for a place of this system while not
/// in supercruise, or Engage hyperdrive ("Lock and engage hyperdrive") for another system; then the Galaxy map button for a
/// System, the System map button for the rest (the maps are cut: these do nothing). Left out: Supercruise Assist (the mod
/// has none), the obscured variants (the mod has no obscuring test), the surface map (`EnablePlanetaryMapButton` is never
/// sent), Apex and the codex.
fn popup_buttons(l: &Location, flight: FlightPhase) -> Vec<Value> {
    let mut out = vec![if l.locked {
        popup_button(button::UNLOCK, "$EXT_PANEL_deselect_destination;", 2)
    } else {
        popup_button(button::LOCK, "$EXT_PANEL_select_destination;", 1)
    }];
    if !l.current {
        if !l.other_system && flight != FlightPhase::Supercruise {
            let label = if l.locked { "$EXT_PANEL_engage_supercruise;" } else { "$EXT_PANEL_select_and_engage_supercruise;" };
            out.push(popup_button(button::SUPERCRUISE, label, 3));
        } else if l.other_system {
            let label = if l.locked { "$EXT_PANEL_engage_hyperdrive;" } else { "$EXT_PANEL_select_and_engage_hyperdrive;" };
            out.push(popup_button(button::HYPERDRIVE, label, 4));
        }
    }
    out.push(if l.kind == 0 {
        popup_button(button::GALAXY_MAP, "$EXT_PANEL_location_galaxy_map;", 20)
    } else {
        popup_button(button::SYSTEM_MAP, "$EXT_PANEL_location_system_map;", 21)
    });
    out
}

/// The pop-up's record (A: `FUN_144540830`'s keys in its order). The mod has no scan, threat, local activities or stat
/// rows for a place: the description is empty, the threat's level 0 (the threat box hidden), the icons and stats empty
/// lists (the activities box hidden), no notification.
fn popup_record(icon: i32, title: &str, buttons: Vec<Value>) -> Value {
    let threat = Value::object([("level", Value::from(0)), ("description", "".into()), ("type", 0.into())]);
    Value::object([
        ("state", Value::from(POPUP_STATE)),
        ("icon", icon.into()),
        ("stationStatus", 0.into()),
        ("title", title.into()),
        ("titleDescription", "".into()),
        ("icons", Value::Array(Vec::new())),
        ("description", "".into()),
        ("stats", Value::Array(Vec::new())),
        ("threat", threat),
        ("buttons", buttons.into()),
        ("notification", "".into()),
    ])
}

/// The pop-up the panel asked for or the one open.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Popup {
    /// Asked for (`LocationSelected`), not sent yet.
    Asked(u32),
    Open(u32),
    /// To be closed (`HidePopup`) on the next feed.
    Closing,
}

/// The left panel's feeder.
#[derive(Clone, Debug, Default)]
pub struct Left {
    configured: bool,
    /// What the list and tracker show, as last sent (the change test's key: `destination_data` with shown distances).
    shown: Option<Value>,
    page: i32,
    contacts_sent: bool,
    popup: Option<Popup>,
}

impl Left {
    pub fn feed(&mut self, facts: &PanelFacts, out: &mut Outbox) {
        if !self.configured {
            out.send(MOVIE, "SetPanelsMode", vec![PANELS_MODE.into()]);
            self.configured = true;
        }
        // The exe sends the list whenever its data changed (B), which with the ship moving is every frame; here it goes when
        // what a row or the tracker SHOWS changes (ours): the distances as the format writes them, the order, the lock.
        if changed(&mut self.shown, destination_data(facts, shown_distance)) {
            out.send(MOVIE, "SetDestinationData", vec![destination_data(facts, distance_text)]);
        }
        if self.page == CONTACTS_PAGE && !self.contacts_sent {
            // B: the contacts' sender runs only on this page and sends when its hash changes or the page is entered (the
            // resend flags `ExternalOnPanel` sets); the mod has no contacts, so the list is empty.
            out.send(MOVIE, "SetContactList", vec![Value::Array(Vec::new())]);
            self.contacts_sent = true;
        }
        match self.popup {
            Some(Popup::Asked(id)) => {
                // Elite's id 0 is its "current position" row, which this list never has (`Location::id`): no pop-up.
                let record = facts
                    .locations
                    .iter()
                    .find(|l| l.id == id && id != 0)
                    .map(|l| popup_record(l.kind, &l.name, popup_buttons(l, facts.flight)));
                match record {
                    Some(r) => {
                        out.send(MOVIE, "ShowLocationPopup", vec![r]);
                        self.popup = Some(Popup::Open(id));
                    }
                    None => self.popup = None,
                }
            }
            Some(Popup::Closing) => {
                // B: the game closes the pop-up after a button or Back (`FUN_142175020`); the movie closes it on `HidePopup`.
                out.send(MOVIE, "HidePopup", vec![]);
                self.popup = None;
            }
            _ => {}
        }
    }

    /// The movie has just been loaded.
    pub fn reset(&mut self) {
        *self = Left::default();
    }

    /// The panel's own events; the ship's actions a pop-up button means come back.
    pub fn hear(&mut self, action: &PanelAction, facts: &PanelFacts) -> Option<PanelAction> {
        match *action {
            PanelAction::Page { page, .. } => {
                // A page is sent anew when entered (the controllers' resend flags, notes part 1 0.).
                if page != self.page {
                    self.page = page;
                    self.contacts_sent = false;
                    self.shown = None;
                }
                None
            }
            PanelAction::LocationSelected(id) => {
                self.popup = Some(Popup::Asked(id));
                None
            }
            PanelAction::LocationBack => {
                self.close_popup();
                None
            }
            PanelAction::Popup { state: 0, .. } => {
                // The movie closed its pop-up itself (its `hide` closes pop-ups and says `ExternalOnPopup [0]`).
                if matches!(self.popup, Some(Popup::Open(_))) {
                    self.popup = None;
                }
                None
            }
            PanelAction::LocationButton(b) => {
                let Some(Popup::Open(id)) = self.popup else { return None };
                self.close_popup();
                let l = facts.locations.iter().find(|l| l.id == id)?;
                let lock = (!l.locked).then_some(id);
                match b {
                    button::LOCK => Some(PanelAction::LockDestination(id)),
                    button::UNLOCK => Some(PanelAction::Unlock),
                    button::SUPERCRUISE => Some(PanelAction::EngageSupercruise { lock }),
                    button::HYPERDRIVE => Some(PanelAction::EngageHyperdrive { lock }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn close_popup(&mut self) {
        if self.popup.is_some() {
            self.popup = Some(Popup::Closing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::location_type as kind;

    fn place(id: u32, name: &str, kind: i32, distance_m: f64, parent: Option<u32>) -> Location {
        Location { id, name: name.into(), kind, distance_m, parent, ..Location::default() }
    }

    /// A small system: a star with two planets, one with a moon nearer than its planet, a far second star, a System
    /// entry the bodies hang from, and two other systems' stars.
    fn system() -> Vec<Location> {
        let mut far = place(8, "Far star", kind::STAR, 9.0e12, Some(1));
        far.locked = false;
        vec![
            place(1, "The system", kind::SYSTEM, 5.0e11, None),
            place(2, "Star", kind::STAR, 3.0e11, Some(1)),
            place(3, "Planet b", kind::PLANET, 2.0e8, Some(2)),
            place(4, "Moon b1", kind::PLANET, 1.5e8, Some(3)),
            place(5, "Planet c", kind::PLANET, 1.0e9, Some(2)),
            place(6, "Neighbour", kind::STAR, 4.0e16, None).other(),
            place(7, "Nearer neighbour", kind::STAR, 2.0e16, None).other(),
            far,
        ]
    }

    trait Other {
        fn other(self) -> Self;
    }
    impl Other for Location {
        fn other(mut self) -> Self {
            self.other_system = true;
            self
        }
    }

    #[test]
    fn rows_are_a_depth_first_walk_by_distance() {
        let locs = system();
        let got: Vec<(&str, usize)> = rows(&locs).iter().map(|r| (r.location.name.as_str(), r.depth)).collect();
        // List A by distance: Moon b1, Planet b, Planet c, Star, The system, Far star. The moon hangs under Planet b, the
        // planets under the Star; the Star's parent is the System, which is passed over but ends its chain, so the Star
        // hangs under it; the far star too. Roots in node order: The system (the only root of list A), then list B.
        assert_eq!(
            got,
            [
                ("The system", 0),
                ("Star", 1),
                ("Planet b", 2),
                ("Moon b1", 3),
                ("Planet c", 2),
                ("Far star", 1),
                ("Nearer neighbour", 0),
                ("Neighbour", 0),
            ]
        );
    }

    #[test]
    fn a_system_parent_in_the_middle_is_passed_over() {
        // A point of interest between a moon and its planet: the moon hangs under the planet, not the point.
        let locs = vec![
            place(1, "Planet", kind::PLANET, 10.0, None),
            place(2, "Point", 5, 20.0, Some(1)),
            place(3, "Moon", kind::PLANET, 30.0, Some(2)),
        ];
        let got: Vec<(&str, usize)> = rows(&locs).iter().map(|r| (r.location.name.as_str(), r.depth)).collect();
        assert_eq!(got, [("Planet", 0), ("Point", 1), ("Moon", 1)]);
    }

    #[test]
    fn other_systems_are_cut_to_fifty_and_depth_to_ten() {
        let mut locs: Vec<Location> = (0..60).map(|i| place(100 + i, "x", kind::STAR, 1.0e16 + f64::from(i), None).other()).collect();
        assert_eq!(rows(&locs).len(), 50);
        assert_eq!(rows(&locs)[0].location.id, 100, "nearest first");
        locs.clear();
        for i in 0..14u32 {
            locs.push(place(i + 1, "d", kind::PLANET, f64::from(i), (i > 0).then_some(i)));
        }
        let depths: Vec<usize> = rows(&locs).iter().map(|r| r.depth).collect();
        assert_eq!(depths, (0..=10).collect::<Vec<_>>(), "nothing deeper than 10");
    }

    #[test]
    fn row_and_destination_records() {
        let mut facts = PanelFacts { system: "Sol".into(), locations: system(), ..PanelFacts::default() };
        facts.locations[2].current = true;
        facts.locations[6].locked = true;
        let d = destination_data(&facts, distance_text);
        assert_eq!(d.get("system").and_then(Value::as_str), Some("Sol"));
        assert_eq!(d.get("location").and_then(Value::as_str), Some("Planet b"));
        assert_eq!(d.get("destination").and_then(Value::as_str), Some("Nearer neighbour"));
        assert_eq!(d.get("numJumps").and_then(Value::as_f64), Some(1.0));
        let Some(Value::Array(list)) = d.get("destinationList") else { panic!("the list") };
        let planet = &list[2];
        assert_eq!(planet.get("label").and_then(Value::as_str), Some("Planet b"));
        assert_eq!(planet.get("depth").and_then(Value::as_f64), Some(2.0));
        assert_eq!(planet.get("distance").and_then(Value::as_str), Some("$DISTANCE:#value=200000000;"));
        assert_eq!(planet.get("current").and_then(Value::as_bool), Some(true));
        assert_eq!(list[6].get("isTargetLocation").and_then(Value::as_bool), Some(true));
        let Value::Object(fields) = planet else { panic!() };
        assert_eq!(fields.len(), 19, "every key of Elite's row builder");
        assert_eq!(distance_text(1234.5678), "$DISTANCE:#value=1234.5677;", "the node's float");
        facts.locations[6].locked = false;
        let d = destination_data(&facts, distance_text);
        assert_eq!(d.get("numJumps").and_then(Value::as_f64), Some(0.0), "no route");
    }

    /// Wolf 359 at 3.8e16 true metres is 4.02 true light years: the panel must show 4.02, not Elite's 4.12 of the raw metres.
    #[test]
    fn other_systems_show_true_light_years() {
        let token = distance_text(3.8e16);
        let value: f64 = token.trim_start_matches("$DISTANCE:#value=").trim_end_matches(';').parse().unwrap();
        assert!((value / ELITE_LIGHT_YEAR_M - 3.8e16 / TRUE_LIGHT_YEAR_M).abs() < 1e-6, "{token}");
        // (The change test's format has no string table, so no decimal point: 4.02 reads "402".)
        assert!(shown_distance(3.8e16).starts_with("402"), "{}", shown_distance(3.8e16));
        assert_eq!(distance_text(2.0e8), "$DISTANCE:#value=200000000;", "inside the system the metres pass as they are");
        assert!(shown_distance(9.3e14).starts_with("010"), "just under a tenth of a true light year: the rung's floor");
    }

    /// A place with the reserved id 0 is the game's mistake: a debug assertion, and in a release build it is left out.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "reserved id 0"))]
    fn the_reserved_id_is_left_out() {
        let locs = vec![place(0, "Zero", kind::PLANET, 1.0, None), place(1, "One", kind::PLANET, 2.0, None)];
        let got: Vec<u32> = rows(&locs).iter().map(|r| r.location.id).collect();
        assert_eq!(got, [1]);
    }

    /// Places whose parents run in a circle never reach a root: a debug assertion, and in a release build they are left out.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "circle"))]
    fn a_circle_of_parents_is_left_out() {
        let locs = vec![place(1, "A", kind::PLANET, 1.0, Some(2)), place(2, "B", kind::PLANET, 2.0, Some(1)), place(3, "C", kind::STAR, 3.0, None)];
        let got: Vec<u32> = rows(&locs).iter().map(|r| r.location.id).collect();
        assert_eq!(got, [3]);
    }

    /// The list goes again only when what it shows changes: a few metres at 200 Mm do not change "0.67Ls".
    #[test]
    fn the_list_is_sent_when_what_it_shows_changes() {
        let mut facts = PanelFacts { system: "Sol".into(), locations: system(), ..PanelFacts::default() };
        let mut left = Left::default();
        left.feed(&facts, &mut Outbox::default());
        facts.locations[2].distance_m += 3.0;
        let mut out = Outbox::default();
        left.feed(&facts, &mut out);
        assert!(out.msgs.is_empty(), "the same text shown");
        facts.locations[2].distance_m = 2.5e8;
        let mut out = Outbox::default();
        left.feed(&facts, &mut out);
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetDestinationData"]);
    }

    #[test]
    fn feed_sends_on_change_and_the_popup_flow() {
        let mut facts = PanelFacts { system: "Sol".into(), locations: system(), ..PanelFacts::default() };
        let mut left = Left::default();
        let mut out = Outbox::default();
        left.feed(&facts, &mut out);
        let names: Vec<&str> = out.msgs.iter().map(|m| m.name).collect();
        assert_eq!(names, ["SetPanelsMode", "SetDestinationData"]);
        assert_eq!(out.msgs[0].args, vec![Value::from(0)]);
        let mut out = Outbox::default();
        left.feed(&facts, &mut out);
        assert!(out.msgs.is_empty(), "nothing changed");
        // The Contacts page: an empty list once per entry.
        left.hear(&PanelAction::Page { movie: MOVIE, page: 3 }, &facts);
        let mut out = Outbox::default();
        left.feed(&facts, &mut out);
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["SetDestinationData", "SetContactList"]);
        // A row selected: the pop-up; its Lock button locks that place and closes it.
        left.hear(&PanelAction::LocationSelected(3), &facts);
        let mut out = Outbox::default();
        left.feed(&facts, &mut out);
        let popup = out.msgs.iter().find(|m| m.name == "ShowLocationPopup").expect("the pop-up");
        let r = &popup.args[0];
        assert_eq!(r.get("title").and_then(Value::as_str), Some("Planet b"));
        assert_eq!(r.get("state").and_then(Value::as_f64), Some(2.0));
        let Some(Value::Array(buttons)) = r.get("buttons") else { panic!() };
        let ids: Vec<f64> = buttons.iter().filter_map(|b| b.get("id").and_then(Value::as_f64)).collect();
        assert_eq!(ids, [0.0, 5.0, 12.0], "Lock, Lock and supercruise, System map");
        assert_eq!(left.hear(&PanelAction::LocationButton(0), &facts), Some(PanelAction::LockDestination(3)));
        let mut out = Outbox::default();
        left.feed(&facts, &mut out);
        assert_eq!(out.msgs.iter().map(|m| m.name).collect::<Vec<_>>(), ["HidePopup"]);
        assert_eq!(left.hear(&PanelAction::LocationButton(0), &facts), None, "no pop-up open");
        // Locked now: Unlock and Engage supercruise; in supercruise no engage button; another system: the hyperdrive.
        facts.locations[2].locked = true;
        let l = &facts.locations[2];
        let ids = |l: &Location, f| popup_buttons(l, f).iter().filter_map(|b| b.get("id").and_then(Value::as_f64)).collect::<Vec<_>>();
        assert_eq!(ids(l, FlightPhase::Normal), [1.0, 5.0, 12.0]);
        assert_eq!(ids(l, FlightPhase::Supercruise), [1.0, 12.0]);
        assert_eq!(ids(&facts.locations[5], FlightPhase::Normal), [0.0, 6.0, 12.0]);
        assert_eq!(ids(&facts.locations[0], FlightPhase::Normal), [0.0, 5.0, 11.0], "a System: the galaxy map");
        left.hear(&PanelAction::LocationSelected(3), &facts);
        left.feed(&facts, &mut Outbox::default());
        assert_eq!(left.hear(&PanelAction::LocationButton(5), &facts), Some(PanelAction::EngageSupercruise { lock: None }));
        left.hear(&PanelAction::LocationSelected(6), &facts);
        left.feed(&facts, &mut Outbox::default());
        assert_eq!(left.hear(&PanelAction::LocationButton(6), &facts), Some(PanelAction::EngageHyperdrive { lock: Some(6) }));
        left.hear(&PanelAction::LocationSelected(6), &facts);
        left.feed(&facts, &mut Outbox::default());
        assert_eq!(left.hear(&PanelAction::LocationButton(12), &facts), None, "the maps are cut");
        left.reset();
        let mut out = Outbox::default();
        left.feed(&facts, &mut out);
        assert_eq!(out.msgs[0].name, "SetPanelsMode", "all again after a reload");
    }
}
