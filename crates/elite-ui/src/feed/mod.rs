//! The feeders (ARCHITECTURE.md A10 "Data (UI4, UI5)"): from `UiFacts` to each flight HUD movie's messages, by the exe's
//! own send rules (`re/elite/notes/ui_messages_hud.md`, its sections per movie). Each feeder keeps what it last sent and
//! sends a message only when the exe's sender would. `reset` means "the movies have just been loaded": it forgets what
//! was sent, so the next `feed` sends everything again, and nothing else. It is not called on boarding: the caches carry
//! on while nobody sits in the seat, so a lock or a warning that went meanwhile is cleared by its normal message on the
//! next seated feed. A message whose data the mod has no source for is never sent.

mod gauges;
mod hudring;
mod info;
mod life;
mod notifications;
mod orbital;
mod sights;
#[cfg(test)]
mod tests;

// The cockpit side panels' feeders (task UI5a): `Panels` below.
mod comms;
mod left;
#[cfg(test)]
mod panel_tests;
mod right;
mod role;

pub use right::database_item;

use crate::focus::{FocusState, Panel};
use crate::panel_facts::PanelFacts;
use crate::panels::PanelAction;

use crate::{UiFacts, Value};

pub use gauges::Gauges;
pub use hudring::{weapon_state, HudRing};
pub use info::Info;
pub use life::Life;
pub use notifications::Notifications;
pub use orbital::Orbital;
pub use sights::{Sights, CENTRAL_GUNSIGHT_UID};

/// The movies in A10's scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Movie {
    CockpitGaugePanel,
    CockpitHudRing,
    TargetSights,
    CockpitInfoPanel,
    OrbitalHud,
    Notifications,
    CockpitExternalPanel,
    CockpitInternalPanel2,
    RolePanel3,
    CockpitChatPanel,
    FssHud,
    SaaHud,
}

impl Movie {
    /// The movie's file name (the export's `movies/<name>`).
    pub fn name(self) -> &'static str {
        match self {
            Movie::CockpitGaugePanel => "cockpitgaugepanel",
            Movie::CockpitHudRing => "cockpithudring",
            Movie::TargetSights => "targetsights",
            Movie::CockpitInfoPanel => "cockpitinfopanel",
            Movie::OrbitalHud => "orbitalhud",
            Movie::Notifications => "notifications",
            Movie::CockpitExternalPanel => "cockpitexternalpanel",
            Movie::CockpitInternalPanel2 => "cockpitinternalpanel2",
            Movie::RolePanel3 => "rolepanel3",
            Movie::CockpitChatPanel => "cockpitchatpanel",
            Movie::FssHud => "fsshud",
            Movie::SaaHud => "saahud",
        }
    }
}

/// One message to a movie: its name and its arguments in the order the exe sends them.
#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub movie: Movie,
    pub name: &'static str,
    pub args: Vec<Value>,
}

/// What a frame's feeding produced: the messages in order, and the Wwise events the exe posts itself alongside them (by
/// name; only those the notes prove: none of the messages sent here has one).
/// The side panels' DataStore writes (`stores`, in order) go to their movies BEFORE the frame's messages: a message that
/// hands a movie a DataStore path (`SetModuleContext`, `InitDataStore`) then finds the nodes already written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outbox {
    pub msgs: Vec<Msg>,
    pub posts: Vec<&'static str>,
    pub stores: Vec<Store>,
}

impl Outbox {
    pub fn send(&mut self, movie: Movie, name: &'static str, args: Vec<Value>) {
        self.msgs.push(Msg { movie, name, args });
    }

    /// A DataStore write for `movie` (`elite_gfx::Movie::store`).
    pub fn store(&mut self, movie: Movie, context: &[&str], property: &str, value: Value) {
        let context = context.iter().map(|s| s.to_string()).collect();
        self.stores.push(Store { movie, context, property: property.to_string(), value });
    }
}

/// One DataStore write: the node's path, the property, its value.
#[derive(Clone, Debug, PartialEq)]
pub struct Store {
    pub movie: Movie,
    pub context: Vec<String>,
    pub property: String,
    pub value: Value,
}

/// Every flight HUD feeder, fed together: the movies' life cycle first (so a movie is opened before its data), then the
/// data while someone is seated. A NaN or infinite number in the facts is no new value: the last finite one is used
/// (`UiFacts::keep_finite`), so it can neither poison a cache nor cause a send every frame.
#[derive(Clone, Debug, Default)]
pub struct Hud {
    good: UiFacts,
    life: Life,
    gauges: Gauges,
    ring: HudRing,
    sights: Sights,
    info: Info,
    orbital: Orbital,
    notifications: Notifications,
}

impl Hud {
    /// One frame: `dt` the frame's seconds.
    pub fn feed(&mut self, facts: &UiFacts, dt: f64, out: &mut Outbox) {
        let mut clean = facts.clone();
        clean.keep_finite(&self.good);
        self.good = clean.clone();
        let facts = &clean;
        self.life.feed(facts, dt, out);
        if !facts.status.seated {
            return;
        }
        self.gauges.feed(facts, dt, out);
        self.ring.feed(facts, dt, out);
        self.sights.feed(facts, dt, out);
        self.info.feed(facts, dt, out);
        self.orbital.feed(facts, dt, out);
        self.notifications.feed(facts, dt, out);
    }

    /// The movies have just been loaded: forget everything sent, so the next `feed` sends it all again. Not for boarding.
    pub fn reset(&mut self) {
        self.life.reset();
        self.gauges.reset();
        self.ring.reset();
        self.sights.reset();
        self.info.reset();
        self.orbital.reset();
        self.notifications.reset();
    }
}

/// The cockpit side panels' feeders, fed together (task UI5a; `re/elite/notes/ui_messages_panels.md`): the left, right,
/// role and comms panels, by their exe's send rules (each feeder's notes). The movies' life cycle (`Open`, `Show`, the
/// focus) is the focus machine's alone (`focus::Focus`, which sends it in order); each panel is fed only while it is on
/// screen (`FocusState::on_screen`), and a page is sent whole when the movie says it is entered (`hear`). `hear` takes what the movies sent back (`panels::on_out`) and returns the ship's actions,
/// the pop-ups' buttons resolved against the pop-up open (as Elite's controllers keep the selected place and module).
///
/// A panel's feeder produces nothing while its panel is not on screen, so nothing piles up for a movie the game has not
/// loaded yet (it loads each the first time it is wanted), however long the flight. While a wanted movie is still
/// loading, the game drops what its feeder gives; once it is ready the game calls `reset_movie` for it (and
/// `Focus::reload_movie`), and that one feeder sends its whole state again while the others carry on.
#[derive(Clone, Debug, Default)]
pub struct Panels {
    left: left::Left,
    right: right::Right,
    role: role::Role,
    comms: comms::Comms,
}

impl Panels {
    /// One frame: `dt` the frame's seconds.
    pub fn feed(&mut self, facts: &PanelFacts, focus: &FocusState, dt: f64, out: &mut Outbox) {
        if focus.on_screen.is_empty() {
            return;
        }
        let facts = &facts.finite();
        let on = |p: Panel| focus.on_screen.contains(&p);
        if on(Panel::Left) {
            self.left.feed(facts, out);
        }
        if on(Panel::Right) {
            self.right.feed(facts, out);
        }
        if on(Panel::Role) {
            self.role.feed(facts, out);
        }
        if on(Panel::Comms) {
            self.comms.feed(facts, dt, out);
        }
    }

    /// The movies have just been loaded: everything is sent again (with `Focus::reload` for their life cycle).
    pub fn reset(&mut self) {
        for p in Panel::ALL {
            self.reset_movie(p.movie());
        }
    }

    /// One panel's movie has just been loaded (or loaded again): its feeder sends its whole state on its next feed; the
    /// others carry on. For the comms movie this puts its log back (`PushMultipleMessagesToLog`): call it only when that
    /// movie was really loaded. Not a panel movie: nothing.
    pub fn reset_movie(&mut self, movie: Movie) {
        match movie {
            Movie::CockpitExternalPanel => self.left.reset(),
            Movie::CockpitInternalPanel2 => self.right.reset(),
            Movie::RolePanel3 => self.role.reset(),
            Movie::CockpitChatPanel => self.comms.reset(),
            _ => {}
        }
    }

    /// What the movies sent back (`panels::on_out`'s actions): the panels' own state is kept here; the ship's actions come
    /// back (the pop-ups' buttons as the actions they mean).
    pub fn hear(&mut self, actions: &[PanelAction], facts: &PanelFacts) -> Vec<PanelAction> {
        let mut out = Vec::new();
        for action in actions {
            let resolved = match action {
                PanelAction::Page { movie: Movie::CockpitExternalPanel, .. }
                | PanelAction::Popup { movie: Movie::CockpitExternalPanel, .. }
                | PanelAction::LocationSelected(_)
                | PanelAction::LocationButton(_)
                | PanelAction::LocationBack => self.left.hear(action, facts),
                PanelAction::Page { movie: Movie::CockpitInternalPanel2, .. }
                | PanelAction::Popup { movie: Movie::CockpitInternalPanel2, .. }
                | PanelAction::ModuleSelected(_)
                | PanelAction::ModuleOption(_)
                | PanelAction::ModuleRow(_) => self.right.hear(action, facts),
                PanelAction::TabsReady => {
                    self.comms.tabs_ready();
                    None
                }
                PanelAction::Page { .. }
                | PanelAction::Popup { .. }
                | PanelAction::ShowComplete(_)
                | PanelAction::HideComplete(_)
                | PanelAction::RoleScreen { .. } => None,
                ship => Some(ship.clone()),
            };
            out.extend(resolved);
        }
        out
    }
}

/// The exe's change-driven cache: true (and `now` kept) when `now` differs from what was last sent or nothing was.
fn changed<T: PartialEq>(last: &mut Option<T>, now: T) -> bool {
    if last.as_ref() == Some(&now) {
        return false;
    }
    *last = Some(now);
    true
}

/// The exe's threshold cache: true (and `now` kept) when nothing was sent or `now` moved from it by more than `threshold`.
fn moved(last: &mut Option<f64>, now: f64, threshold: f64) -> bool {
    if last.is_some_and(|l| (now - l).abs() <= threshold) {
        return false;
    }
    *last = Some(now);
    true
}
