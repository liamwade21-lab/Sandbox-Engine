//! The comms panel, `cockpitchatpanel` (`re/elite/notes/ui_messages_panels.md` part 3; the movie's code
//! `code/cockpitchatpanel/`). What is fed: the channel strip's one tab (`UpdateCommChannelList`, after the movie says its
//! tabs exist) and the message log (`PushMessageToLog`): Elite's own first line, the comms system's greeting, then the
//! mod's local lines, each once (by its id), released by Elite's pump; after the movie is loaded again, the log's last 30
//! lines are put back at once (`PushMultipleMessagesToLog`).
//!
//! Left as the movie's own state, and why: the voice and team icons (`SetCommsIcon`, `SetWingIcon`: idle and hidden are
//! the movie's defaults, and the exe's first send would be the same), the alert counters (`SetAlertNumber`: no mail), the
//! inbox, contacts, squadron, history and options pages (none in the mod), the text line's leader (`SetLeaderField`: the
//! mod has no chat input).

use super::{Movie, Outbox};
use crate::panel_facts::PanelFacts;
use crate::Value;
use std::collections::VecDeque;

const MOVIE: Movie = Movie::CockpitChatPanel;

/// The groups (A: notes part 3 4.2): notification 1, local 7.
const FROM_NOTIFICATION: i32 = 1;
const TO_LOCAL: i32 = 7;
/// A message's colour type without its override (A: `FUN_144725fc0`: 2).
const COLOUR_TYPE: i32 = 2;
/// "None" for an id (A: -1, its 32-bit halves as unsigned).
const NO_ID: u32 = u32::MAX;
/// The pump releases one message every 0.5 s, faster with a backlog (A: `FUN_144746830`, the reload 0x3f000000).
const PUMP_S: f64 = 0.5;
/// The game keeps the last 30 released messages of a group (A: `FUN_144746830`: the 31st deletes the oldest).
const HISTORY_MOST: usize = 30;
/// Elite's first line, the first time the comms system runs (A: `FUN_1447078d0`: `AddMessage("$CHAT_Intro;", raw 1, to
/// local)`).
const INTRO: &str = "$CHAT_Intro;";
/// The Main tab (A: `FUN_144759a80`'s item 0: group 0, "$CHAT_group_Main;").
const MAIN_TAB: &str = "$CHAT_group_Main;";

/// One log record (A: `FUN_144725fc0`'s keys): a notification to the local channel, from nobody, with no avatar, ids or
/// colour override. `message` is a string-table key as given, or "&" + literal text (the exe's raw flag, `FUN_144716fc0`).
fn record(message: String, time_of_day_s: Option<f64>) -> Value {
    let time = time_of_day_s.map_or(0, |s| s.rem_euclid(86_400.0).floor() as u32);
    Value::object([
        ("sent", Value::from(false)),
        ("fromGroup", FROM_NOTIFICATION.into()),
        ("toGroup", TO_LOCAL.into()),
        ("target", "".into()),
        ("message", message.into()),
        ("time", time.into()),
        ("avatar", Value::Null),
        ("overrideTextColour", false.into()),
        ("colorType", COLOUR_TYPE.into()),
        ("playerId", (-1).into()),
        ("objectIdHigh", NO_ID.into()),
        ("objectIdLow", NO_ID.into()),
        ("commanderIdHigh", NO_ID.into()),
        ("commanderIdLow", NO_ID.into()),
    ])
}

/// The comms panel's feeder. The log is the game's (Elite's comms manager keeps it, not the movie), so a reload of the
/// movie keeps everything but the channel strip.
#[derive(Clone, Debug, Default)]
pub struct Comms {
    /// The greeting was added (once a run of the comms system).
    greeted: bool,
    /// The highest `LogLine::id` added (ids grow in the order the lines are said).
    taken: Option<u64>,
    /// Added, not yet released.
    queue: VecDeque<Value>,
    timer: f64,
    /// Released, the last `HISTORY_MOST`.
    history: VecDeque<Value>,
    /// The movie was loaded again: its log is to be put back.
    replay: bool,
    /// The movie said its tabs exist (`ChatPanelTabsReady`) and the strip is not sent yet.
    tabs_ready: bool,
}

impl Comms {
    pub fn feed(&mut self, facts: &PanelFacts, dt: f64, out: &mut Outbox) {
        if self.tabs_ready {
            // B: the strip after the tabs exist; item 0 is the Main tab, the only one the mod's single log needs. Elite also
            // tabs the System channel by default (notes part 3 16), whose name key was not decoded (E): left out. `canChat`
            // false: the mod has no chat input (ours; Elite's Main tab can chat).
            let main = Value::object([
                ("id", Value::from(0)),
                ("type", 0.into()),
                ("group", 0.into()),
                ("name", MAIN_TAB.into()),
                ("notification", false.into()),
                ("isCurrentChannel", true.into()),
                ("canChat", false.into()),
            ]);
            out.send(MOVIE, "UpdateCommChannelList", vec![Value::array([main])]);
            self.tabs_ready = false;
        }
        if self.replay {
            // B: the call Elite makes to rebuild the log (`FUN_1447481a0`, on a tab change and `/clear`: the kept history,
            // added by the movie without fade or typewriter); that Elite makes it after a movie reload was not traced.
            if !self.history.is_empty() {
                out.send(MOVIE, "PushMultipleMessagesToLog", vec![Value::Array(self.history.iter().cloned().collect())]);
            }
            self.replay = false;
        }
        if !self.greeted {
            self.queue.push_back(record(INTRO.to_string(), facts.time_of_day_s));
            self.greeted = true;
        }
        for line in &facts.comms {
            if self.taken.is_none_or(|t| line.id > t) {
                self.queue.push_back(record(format!("&{}", line.text), Some(line.time_of_day_s)));
                self.taken = Some(line.id);
            }
        }
        self.pump(dt, out);
    }

    /// Elite's pump (A: `FUN_144746830`): while the timer is above 0 it falls by max(queue length, 1) x dt; while it is at
    /// or under 0 and messages wait, one goes, joins the history, and the timer reloads.
    fn pump(&mut self, dt: f64, out: &mut Outbox) {
        if self.timer > 0.0 {
            self.timer -= (self.queue.len() as f64).max(1.0) * dt;
        }
        while self.timer <= 0.0 {
            let Some(m) = self.queue.pop_front() else { break };
            out.send(MOVIE, "PushMessageToLog", vec![m.clone()]);
            self.history.push_back(m);
            if self.history.len() > HISTORY_MOST {
                self.history.pop_front();
            }
            self.timer += PUMP_S;
        }
    }

    /// The movie has just been loaded: its log is put back on the next feed; the lines not yet released still wait.
    pub fn reset(&mut self) {
        self.replay = true;
        self.tabs_ready = false;
    }

    pub fn tabs_ready(&mut self) {
        self.tabs_ready = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel_facts::LogLine;

    fn line(id: u64, text: &str) -> LogLine {
        LogLine { id, text: text.into(), time_of_day_s: 10.0 + id as f64 }
    }

    fn pushed(out: &Outbox) -> Vec<String> {
        out.msgs
            .iter()
            .filter(|m| m.name == "PushMessageToLog")
            .filter_map(|m| m.args[0].get("message").and_then(Value::as_str).map(str::to_string))
            .collect()
    }

    #[test]
    fn the_greeting_then_lines_at_the_pumps_pace() {
        let mut c = Comms::default();
        let mut f = PanelFacts { time_of_day_s: Some(3_723.9), ..PanelFacts::default() };
        f.comms = vec![line(1, "Gear down"), line(2, "Gear up")];
        let mut out = Outbox::default();
        c.feed(&f, 0.1, &mut out);
        assert_eq!(out.msgs.len(), 1, "one at once");
        let r = &out.msgs[0].args[0];
        assert_eq!(r.get("message").and_then(Value::as_str), Some("$CHAT_Intro;"));
        assert_eq!(r.get("time").and_then(Value::as_f64), Some(3_723.0));
        assert_eq!(r.get("fromGroup").and_then(Value::as_f64), Some(1.0));
        assert_eq!(r.get("toGroup").and_then(Value::as_f64), Some(7.0));
        let Value::Object(fields) = r else { panic!() };
        assert_eq!(fields.len(), 14, "every key of Elite's record");
        // Two wait: the timer falls twice as fast; 0.5 s / 2 = 0.25 s, four frames of 1/16 s.
        let mut sent = Vec::new();
        for _ in 0..30 {
            let mut out = Outbox::default();
            c.feed(&f, 0.0625, &mut out);
            sent.push(out.msgs.len());
        }
        assert_eq!(sent.iter().position(|&n| n == 1), Some(3), "after 0.25 s (two waiting)");
        assert_eq!(sent.iter().rposition(|&n| n == 1), Some(11), "then 0.5 s (one waiting)");
        c.tabs_ready();
        let mut out = Outbox::default();
        c.feed(&f, 0.05, &mut out);
        assert_eq!(out.msgs[0].name, "UpdateCommChannelList");
        let Value::Array(items) = &out.msgs[0].args[0] else { panic!() };
        assert_eq!(items[0].get("name").and_then(Value::as_str), Some("$CHAT_group_Main;"));
    }

    /// The game's lines come as a window that slides: every line is pushed once, whatever the window shows.
    #[test]
    fn a_sliding_window_loses_no_line() {
        let mut c = Comms::default();
        let mut all = Vec::new();
        for n in 1..=9u64 {
            let window = (n.saturating_sub(1).max(1)..=n).map(|k| line(k, &format!("line {k}"))).collect();
            let f = PanelFacts { comms: window, ..PanelFacts::default() };
            for _ in 0..40 {
                let mut out = Outbox::default();
                c.feed(&f, 0.05, &mut out);
                all.extend(pushed(&out));
            }
        }
        let want: Vec<String> = std::iter::once("$CHAT_Intro;".to_string()).chain((1..=9).map(|n| format!("&line {n}"))).collect();
        assert_eq!(all, want);
    }

    /// After a reload the released lines come back at once, the waiting ones go on at the pump's pace, nothing twice.
    #[test]
    fn a_reload_puts_the_log_back() {
        let mut c = Comms::default();
        let f = PanelFacts { comms: vec![line(1, "one"), line(2, "two"), line(3, "three")], ..PanelFacts::default() };
        c.feed(&f, 0.0, &mut Outbox::default());
        c.reset();
        let mut out = Outbox::default();
        c.feed(&f, 0.0, &mut out);
        let replay = out.msgs.iter().find(|m| m.name == "PushMultipleMessagesToLog").expect("the log put back");
        let Value::Array(lines) = &replay.args[0] else { panic!() };
        assert_eq!(lines.len(), 1, "the greeting, the one released");
        let mut rest = Vec::new();
        for _ in 0..60 {
            let mut out = Outbox::default();
            c.feed(&f, 0.05, &mut out);
            assert!(out.msgs.iter().all(|m| m.name != "PushMultipleMessagesToLog"), "once");
            rest.extend(pushed(&out));
        }
        assert_eq!(rest, ["&one", "&two", "&three"]);
        for n in 4..40u64 {
            let f = PanelFacts { comms: vec![line(n, "more")], ..PanelFacts::default() };
            for _ in 0..12 {
                c.feed(&f, 0.05, &mut Outbox::default());
            }
        }
        c.reset();
        let mut out = Outbox::default();
        c.feed(&PanelFacts::default(), 0.0, &mut out);
        let replay = out.msgs.iter().find(|m| m.name == "PushMultipleMessagesToLog").unwrap();
        let Value::Array(lines) = &replay.args[0] else { panic!() };
        assert_eq!(lines.len(), 30, "the last 30");
    }
}
