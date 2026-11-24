//! What Elite's Full Spectrum System scanner screen (`fsshud`, task UI7) shows that the mod has, and Elite's rule for
//! when the screen may open. Filled once a frame by the game (`sidewinder/ui/screens.rs`), read by the FSS's feeder
//! (`feed::Fss`) and nothing else. The user's scope rule holds: a plain screen fed what the mod knows (this system's
//! bodies); no scanning gameplay, so no body is ever discovered, nothing is zoomed and no probe or honk exists. What the
//! mod cannot feed stays in the movie's own empty state (`re/elite/notes/ui_messages_screens.md` section 7).

/// Everything the FSS screen is fed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FssFacts {
    /// The screen is open (the game decides it by `next_open`): the movie is opened and fed while true, closed when it
    /// turns false.
    pub open: bool,
    /// The star system's name, as shown (`SetSystemName`, the scan summary's line).
    pub system: String,
    /// The system's bodies, stars included (a star has no signal: notes 7.7.2), in any order: the feeder orders them by id.
    pub bodies: Vec<FssBody>,
    /// The system's discovery scan is done: the screen is ACTIVE (`SetState` 2, the spectrum's bodies and the tuning
    /// shown) rather than INACTIVE (0, "Energy pulse required"). The exe's test (`M->vt+0x30()`) is B; the mod has no
    /// discovery scan (the scope rule), so the game's value is its own stand-in (`screens.rs` says which).
    pub discovery_scan_done: bool,
    /// The radio tuning on the spectrum, 0..1 (`SetFrequency`); None: the mod has no tuning value, and nothing that
    /// depends on it is sent (the indicator stays where the movie puts it).
    pub tuning: Option<f64>,
}

/// One body of the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FssBody {
    /// The game's id of the body, unique in the system: it seeds the body's place on the spectrum (Elite seeds it with a
    /// hash of its own body id, which the mod's bodies do not have: notes 7.7.2).
    pub id: u64,
    pub kind: BodyKind,
}

/// What kind of body, as Elite's FSS reads it (notes 7.7.2: body kind 1 star, 2 or 8 planet or moon by its planet class).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// A star: no signal on the spectrum.
    Star,
    /// A planet or moon by Elite's planet class (`FUN_14138db90`: 0 EarthLike, 1 WaterWorld, 2 AmmoniaWorld, 3 MetalRich,
    /// 4 HighMetalContent, 5 Rocky, 6 Icy, 7 RockyIcy, 8 WaterGiant, 9 WaterGiantLife, 10 WaterLife, 11 AmmoniaLife,
    /// 12..16 SudarskyClassI..V, 17 HeliumRich, 18 Helium). A class past 18 has no archetype and no signal.
    Planet(u8),
}

/// Elite's rule for the FSS to work: the ship in supercruise and its throttle at zero (B: Elite's own text says it,
/// `$codex_pilotshandbook_exploration_fss_description_1;` "Ships must be in supercruise ... and throttled down to zero ...
/// for the FSS to function", with the refusals `$INFO_fss_enter_supercruise;` and `$INFO_fss_throttle_warning;`; the exe's
/// test itself was not traced). Elite's third refusal, Supercruise Overcharge, has no cause in the mod.
pub fn may_open(supercruise: bool, throttle: f64) -> bool {
    supercruise && throttle == 0.0
}

/// The screen's next open state from this frame's keys (`ExplorationFSSEnter`, `ExplorationFSSQuit`) and the rule:
/// closed, the enter key opens it when the rule allows (else it stays closed); open, the quit key closes it, and so does
/// the rule failing (ours, B: Elite closes the FSS on its own refusals, `$INFO_fss_warning_speed;` "Too fast, closing FSS",
/// by a test not decoded; the mod closes it when the rule that opened it no longer holds). The enter key does nothing
/// while open (in Elite's FSS state only the FSS's own control set is live: `ui/focus.json` `Exploration_FSS`).
pub fn next_open(open: bool, enter: bool, quit: bool, allowed: bool) -> bool {
    if open {
        !quit && allowed
    } else {
        enter && allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_opens_only_by_the_rule() {
        assert!(may_open(true, 0.0));
        assert!(!may_open(true, 0.25), "throttle not at zero");
        assert!(!may_open(false, 0.0), "not in supercruise");
        assert!(next_open(false, true, false, true));
        assert!(!next_open(false, true, false, false), "refused");
        assert!(!next_open(false, false, false, true), "no key");
        assert!(next_open(true, false, false, true), "stays open");
        assert!(next_open(true, true, false, true), "enter does nothing while open");
        assert!(!next_open(true, false, true, true), "the quit key");
        assert!(!next_open(true, true, true, true), "the shared key quits");
        assert!(!next_open(true, false, false, false), "the rule fails");
    }
}
