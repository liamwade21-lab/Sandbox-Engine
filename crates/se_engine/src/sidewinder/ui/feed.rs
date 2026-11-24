//! The flight HUD's feeders in the game (ARCHITECTURE.md A10 "Data (UI4, UI5)", task UI4b): `HudFeed` owns
//! `elite_ui::feed::Hud` and runs it once a frame while seated, after `facts::fill`, on the frame's `HudFacts` and real
//! seconds; its messages go to `UiMessages` in order (the movies' step sends them the same frame), its posts to `UiSounds`.
//!
//! - It runs only while the movies exist (`place::UiDraw`, made with them): otherwise nothing would drain the messages.
//! - Reset: `Hud::reset` when the movies have just been made (`UiDraw` added since the feed last ran: the first spawn's, and
//!   a new spawn's after the ship went, whose movies are new). The feed's next messages then carry everything, and they reach
//!   each new movie in order: kept while it loads (`movies.rs`, one movie a frame) and queued by the movie until it is ready.
//!   So the first boarding is no special case, and no movie ever gets a change before the state it changes, which a reset at
//!   `UiMovieState::fresh` (the movie ready) would do: the messages sent between the new movies' making and their readiness
//!   would be changes only.
//! - The six HUD movies are wanted (`UiWanted`) from the first seated frame.
//! - Leaving the seat: nothing here runs (`UiSet::Feed` runs only while seated); the movies stay loaded and the feeders' caches
//!   stand, so the next boarding sends only what changed meanwhile (`elite_ui::feed`'s rule).

use bevy::prelude::*;
use elite_ui::feed::{Hud, Outbox};

use super::super::sound::ui::UiSounds;
use super::facts::HudFacts;
use super::place::UiDraw;
use super::{UiMessages, UiSet, UiWanted, HUD};

/// The HUD's feeders and what they last sent.
#[derive(Resource, Default)]
pub struct HudFeed {
    hud: Hud,
}

/// Once a frame while seated and the movies exist: the feeders on the frame's facts.
fn feed(
    time: Res<Time<Real>>,
    facts: Res<HudFacts>,
    draw: Res<UiDraw>,
    mut feed: ResMut<HudFeed>,
    mut messages: ResMut<UiMessages>,
    mut wanted: ResMut<UiWanted>,
    sounds: Option<ResMut<UiSounds>>,
) {
    if draw.is_added() {
        feed.hud.reset();
    }
    for m in HUD {
        if !wanted.movies.contains(&m) {
            wanted.movies.push(m);
        }
    }
    let mut out = Outbox::default();
    feed.hud.feed(&facts.facts, time.delta_secs_f64(), &mut out);
    // (The DataStore writes and the messages, each in order: the movies' step writes the stores first.)
    messages.stores.append(&mut out.stores);
    messages.msgs.append(&mut out.msgs);
    if let Some(mut sounds) = sounds {
        for event in out.posts {
            sounds.post(event);
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<HudFeed>().add_systems(PostUpdate, feed.in_set(UiSet::Feed).after(super::facts::fill).run_if(resource_exists::<UiDraw>));
}
