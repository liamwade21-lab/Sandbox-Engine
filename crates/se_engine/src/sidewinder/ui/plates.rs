//! The dark plates behind the panels (`Gui_*_Background`, ARCHITECTURE.md A10 UI3): shown and hidden by their movie's own
//! events, as Elite's `CockpitPanelComponent` does (`re/elite/notes/ui_messages_hud.md` section 7, A): the movie names
//! the element, the component walks ITS OWN element list for that name (a name it lacks, such as the gauge movie's
//! `Gui_Radar_Docking_Panel_Background` on the Sidewinder, is ignored) and stores the flag, or a time.
//!
//! - `SetBackgroundVisible [name, visible]`: the element's flag.
//! - `SetBackgroundVisibleTime [name, seconds]`: the flag cleared and a time set; the game shows the plate for that long
//!   (the notes' table: the info panel's notifications). Shown while the time runs (B: the reader of the time is not found).
//! - The fade between the states (each element's 0.2 / 0.5 s) is not decoded: a plate shows or goes at once (a labelled
//!   stand-in, E).
//! - Before any event a plate shows if its record's first flag is set (B: the flag is 1 on exactly the plates no movie
//!   switches, such as `Gui_Info_Default_Panel_Background`, "always on", and 0 on every plate a movie switches).

use elite_gfx::Out;
use elite_ui::Value;
use se_core::elite_ui_layout::Layout;

/// One plate's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Plate {
    /// The movie whose component lists it.
    pub movie: String,
    /// Its element name (`Gui_Warning_panel_background`).
    pub name: String,
    pub flag: bool,
    /// Seconds left of a `SetBackgroundVisibleTime`.
    pub time: f32,
}

impl Plate {
    pub fn shown(&self) -> bool {
        self.flag || self.time > 0.0
    }
}

/// Every plate of the layout, in its order (movies, then each movie's backgrounds).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plates {
    pub plates: Vec<Plate>,
}

impl Plates {
    pub fn new(layout: &Layout) -> Plates {
        let plates = layout
            .movies
            .iter()
            .flat_map(|m| m.backgrounds.iter().map(move |b| Plate { movie: m.name.clone(), name: b.name.clone(), flag: b.flag != 0, time: 0.0 }))
            .collect();
        Plates { plates }
    }

    /// The plate `name` of `movie`'s component.
    fn find(&mut self, movie: &str, name: &str) -> Option<&mut Plate> {
        self.plates.iter_mut().find(|p| p.movie == movie && p.name.eq_ignore_ascii_case(name))
    }

    /// A movie's event; whether a plate's state changed.
    pub fn apply(&mut self, movie: &str, out: &Out) -> bool {
        let Out::Event { name, args } = out else { return false };
        let (Some(Value::Str(element)), Some(second)) = (args.first(), args.get(1)) else { return false };
        let Some(plate) = self.find(movie, element) else { return false };
        let before = plate.clone();
        match (name.as_str(), second) {
            ("SetBackgroundVisible", Value::Bool(v)) => {
                plate.flag = *v;
                plate.time = 0.0;
            }
            ("SetBackgroundVisibleTime", Value::Num(s)) if s.is_finite() => {
                plate.flag = false;
                plate.time = s.max(0.0) as f32;
            }
            _ => {}
        }
        *plate != before
    }

    /// A test's `sw ui plate NAME on|off`: every plate of that name; whether one was found.
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        let mut found = false;
        for p in self.plates.iter_mut().filter(|p| p.name.eq_ignore_ascii_case(name)) {
            p.flag = on;
            p.time = 0.0;
            found = true;
        }
        found
    }

    /// The times run on by `dt` seconds; whether a plate went.
    pub fn tick(&mut self, dt: f32) -> bool {
        let mut went = false;
        for p in self.plates.iter_mut().filter(|p| p.time > 0.0) {
            p.time = (p.time - dt).max(0.0);
            went |= p.time == 0.0 && !p.flag;
        }
        went
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plates() -> Plates {
        let p = |movie: &str, name: &str, flag| Plate { movie: movie.into(), name: name.into(), flag, time: 0.0 };
        Plates { plates: vec![p("cockpitinfopanel", "Gui_Warning_panel_background", false), p("cockpitinfopanel", "Gui_Info_Default_Panel_Background", true), p("cockpitgaugepanel", "Gui_Radar_Panel_Background", false)] }
    }

    fn event(name: &str, args: Vec<Value>) -> Out {
        Out::Event { name: name.into(), args }
    }

    #[test]
    fn a_movie_switches_its_own_plates_only() {
        let mut p = plates();
        assert!(!p.plates[2].shown() && p.plates[1].shown(), "the record's flag before any event");
        assert!(p.apply("cockpitgaugepanel", &event("SetBackgroundVisible", vec!["Gui_Radar_Panel_Background".into(), true.into()])));
        assert!(p.plates[2].shown());
        // (Another movie's plate of that name, and a name the component lacks, are ignored.)
        assert!(!p.apply("cockpitinfopanel", &event("SetBackgroundVisible", vec!["Gui_Radar_Panel_Background".into(), false.into()])));
        assert!(!p.apply("cockpitgaugepanel", &event("SetBackgroundVisible", vec!["Gui_Radar_Docking_Panel_Background".into(), true.into()])));
        assert!(p.plates[2].shown());
        // (The same state again is no change; other events and malformed arguments change nothing.)
        assert!(!p.apply("cockpitgaugepanel", &event("SetBackgroundVisible", vec!["Gui_Radar_Panel_Background".into(), true.into()])));
        assert!(!p.apply("cockpitgaugepanel", &event("ShowComplete", vec![])));
        assert!(!p.apply("cockpitgaugepanel", &event("SetBackgroundVisible", vec!["Gui_Radar_Panel_Background".into(), 1.0.into()])));
        assert!(!p.apply("cockpitgaugepanel", &Out::Sound { name: "x".into(), source: None }));
        assert!(p.apply("cockpitgaugepanel", &event("SetBackgroundVisible", vec!["Gui_Radar_Panel_Background".into(), false.into()])));
        assert!(!p.plates[2].shown());
    }

    #[test]
    fn a_timed_plate_shows_for_its_seconds() {
        let mut p = plates();
        assert!(p.apply("cockpitinfopanel", &event("SetBackgroundVisibleTime", vec!["Gui_Warning_panel_background".into(), 2.0.into()])));
        assert!(p.plates[0].shown());
        assert!(!p.tick(1.5));
        assert!(p.plates[0].shown());
        assert!(p.tick(0.6), "it goes when its time is up");
        assert!(!p.plates[0].shown());
        // (A flag set meanwhile ends the time.)
        p.apply("cockpitinfopanel", &event("SetBackgroundVisibleTime", vec!["Gui_Warning_panel_background".into(), 2.0.into()]));
        p.apply("cockpitinfopanel", &event("SetBackgroundVisible", vec!["Gui_Warning_panel_background".into(), false.into()]));
        assert!(!p.plates[0].shown());
        assert!(p.set("gui_warning_panel_background", true) && p.plates[0].shown());
        assert!(!p.set("no such plate", true));
    }
}
