//! The toasts, `notifications` (`re/elite/notes/ui_messages_hud.md`, its section). Elite raises a toast for legal status,
//! rank, engineers and bookmarks (`AddNotification`'s senders); the mod has none of them, so nothing is sent: the movie is
//! only opened, shown and hidden (`feed::life`). The feeder is kept so a toast with a cause in the mod has its place.

use super::Outbox;
use crate::UiFacts;

/// The toasts' feeder: no message has a cause in the mod.
#[derive(Clone, Debug, Default)]
pub struct Notifications;

impl Notifications {
    pub fn feed(&mut self, _facts: &UiFacts, _dt: f64, _out: &mut Outbox) {}

    pub fn reset(&mut self) {}
}
