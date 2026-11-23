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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outbox {
    pub msgs: Vec<Msg>,
    pub posts: Vec<&'static str>,
}

impl Outbox {
    pub fn send(&mut self, movie: Movie, name: &'static str, args: Vec<Value>) {
        self.msgs.push(Msg { movie, name, args });
    }
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
