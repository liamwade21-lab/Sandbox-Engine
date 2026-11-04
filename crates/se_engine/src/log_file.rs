//! SE's log file (main-def.cfg `Log`). SE rewrites `system/se.log` on every start: a version line,
//! then one line per message, `[MT] ` from the main thread or `[NN] ` from worker NN
//! (LogThreadStamp), after an optional time stamp (LogTimeStamp), with `ERROR: ` / `WARNING: ` in
//! front of problems. The port never writes into the install, so its log is `se.log` in its user
//! folder (`%APPDATA%/SpaceEngine-rs-v2`), next to its settings.

use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use bevy::log::BoxedLayer;
use bevy::log::tracing::{Event, Level, Subscriber, field};
use bevy::log::tracing_subscriber::Layer;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::prelude::*;

/// SE's `Log` settings (main-def.cfg).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogSettings {
    /// `LogTimeStamp`: a time stamp before each line.
    pub time_stamp: bool,
    /// `LogThreadStamp`: the thread before each line.
    pub thread_stamp: bool,
    /// `CatalogLogLevel` 0-4: 0 logs nothing from the catalog loader, 1 and 3 its errors and
    /// warnings, 2 and 4 everything (1 and 2 capped by each catalog's own `LogLevel`).
    pub catalog_level: u8,
    /// `CsvLogLevel` 0-2: the same for the `.csv` catalogs.
    pub csv_level: u8,
}

impl Default for LogSettings {
    /// main-def.cfg's values.
    fn default() -> Self {
        Self { time_stamp: false, thread_stamp: true, catalog_level: 2, csv_level: 1 }
    }
}

impl LogSettings {
    /// Reads the `Log` block of SE's main-def.cfg text; missing keys keep SE's defaults.
    pub fn from_main_def(text: &str) -> Self {
        let doc = se_core::sc::parse(text);
        let d = Self::default();
        let level = |k: &str, def: u8, max: f64| doc.find_deep(k).and_then(|i| i.f64()).map_or(def, |v| v.clamp(0.0, max) as u8);
        Self {
            time_stamp: doc.find_deep("LogTimeStamp").and_then(|i| i.bool()).unwrap_or(d.time_stamp),
            thread_stamp: doc.find_deep("LogThreadStamp").and_then(|i| i.bool()).unwrap_or(d.thread_stamp),
            catalog_level: level("CatalogLogLevel", d.catalog_level, 4.0),
            csv_level: level("CsvLogLevel", d.csv_level, 2.0),
        }
    }

    pub fn load(cfg: &crate::config::EngineConfig) -> Self {
        std::fs::read_to_string(cfg.se_path.join("config/main-def.cfg")).map_or_else(|_| Self::default(), |t| Self::from_main_def(&t))
    }

    /// Whether a catalog loader message about `source` is logged: CSV files by `CsvLogLevel`,
    /// the others by `CatalogLogLevel` (the port's loader only reports problems, which every
    /// level above 0 logs).
    pub fn logs_catalog(&self, source: &str) -> bool {
        if source.to_ascii_lowercase().ends_with(".csv") { self.csv_level > 0 } else { self.catalog_level > 0 }
    }
}

/// The `[MT]` / `[NN]` stamp of the calling thread: SE numbers its workers; the port numbers its
/// threads in the order they first log.
fn thread_stamp() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    thread_local!(static ID: u32 = NEXT.fetch_add(1, Ordering::Relaxed));
    let main = std::thread::current().name() == Some("main");
    if main { "[MT] ".to_string() } else { ID.with(|id| format!("[{:02}] ", id % 100)) }
}

/// One log line as SE writes it.
fn format_line(s: &LogSettings, time: Option<&str>, thread: &str, level: Level, message: &str) -> String {
    let mut line = String::new();
    if let (true, Some(t)) = (s.time_stamp, time) {
        let _ = write!(line, "{t} ");
    }
    if s.thread_stamp {
        line.push_str(thread);
    }
    match level {
        Level::ERROR => line.push_str("ERROR: "),
        Level::WARN => line.push_str("WARNING: "),
        _ => {}
    }
    line.push_str(message);
    line
}

#[derive(Default)]
struct Message(String);

impl field::Visit for Message {
    fn record_debug(&mut self, f: &field::Field, value: &dyn std::fmt::Debug) {
        if f.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else if !f.name().starts_with("log.") {
            // (`log.target` and the like come from crates logging through `log`.)
            let _ = write!(self.0, " {}={value:?}", f.name());
        }
    }
}

struct FileLayer {
    file: Mutex<std::fs::File>,
    settings: LogSettings,
    start: std::time::Instant,
}

impl<S: Subscriber> Layer<S> for FileLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut m = Message::default();
        event.record(&mut m);
        let time = format!("{:10.3}", self.start.elapsed().as_secs_f64());
        let line = format_line(&self.settings, Some(&time), &thread_stamp(), *event.metadata().level(), &m.0);
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// The log file layer for Bevy's `LogPlugin::custom_layer`: `se.log` in the port's user folder,
/// rewritten on every start; scripted test runs write `se-test.log` instead, so the user's own last
/// log stays for looking into what they saw. None when the file can't be created.
pub fn layer(app: &mut App) -> Option<BoxedLayer> {
    let cfg = app.world().get_resource::<crate::config::EngineConfig>();
    let settings = cfg.map(LogSettings::load).unwrap_or_default();
    let name = if cfg.is_some_and(crate::settings::scripted) { "se-test.log" } else { "se.log" };
    app.insert_resource(CatalogLog(settings));
    let path = crate::settings::user_file(name);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut file = std::fs::File::create(&path).ok()?;
    let _ = writeln!(file, "SpaceEngine {} (Rust/Bevy port)\n\nSTARTING", env!("CARGO_PKG_VERSION"));
    Some(Box::new(FileLayer { file: Mutex::new(file), settings, start: std::time::Instant::now() }))
}

/// SE's catalog log levels, for the catalog loader's messages (data.rs).
#[derive(Resource, Debug, Clone, Copy)]
pub struct CatalogLog(pub LogSettings);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_main_def_and_formats_like_se() {
        let s = LogSettings::from_main_def(
            "Log\n{\n\tLogTimeStamp true\n\tLogThreadStamp false\n\tCatalogLogLevel 0\n\tCsvLogLevel 2\n}\n",
        );
        assert_eq!(s, LogSettings { time_stamp: true, thread_stamp: false, catalog_level: 0, csv_level: 2 });
        assert_eq!(LogSettings::from_main_def(""), LogSettings::default());
        assert!(!s.logs_catalog("catalogs/stars/Stars.sc") && s.logs_catalog("catalogs/stars/HIPPARCOS.csv"));
        let d = LogSettings::default();
        assert_eq!(format_line(&d, Some("1.0"), "[MT] ", Level::INFO, "Loading galaxies catalogs"), "[MT] Loading galaxies catalogs");
        assert_eq!(format_line(&d, None, "[04] ", Level::ERROR, "bad"), "[04] ERROR: bad");
        assert_eq!(format_line(&s, Some("1.0"), "[MT] ", Level::WARN, "w"), "1.0 WARNING: w");
    }
}
