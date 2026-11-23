//! The text rules against the notes' own samples (`ui_messages_hud.md` section 8). The table here is a short sample written
//! for the tests in `strings/en.json`'s shape; the ignored test reads the real table from the export.

use super::{Text, TimeFlags};

/// A small table: the symbols the formatters write and the keys the HUD's texts name, in the export's two shapes.
fn sample() -> Text {
    let json = r##"{
        "block_separator": {"text": ",", "why": "sample"},
        "decimal_separator": ".",
        "minus_prefix": "-",
        "plus_prefix": "+",
        "minus_suffix": "",
        "plus_suffix": "",
        "meters_per_second": "m/s",
        "kilometers_per_second": "km/s",
        "megameters_per_second": "Mm/s",
        "speed_of_light": "c",
        "meters_symbol": "m",
        "kilometers_symbol": "km",
        "megameters_symbol": "Mm",
        "light_second_symbol": "Ls",
        "kilo_light_second_symbol": "kLs",
        "light_year_symbol": "Ly",
        "time_separator": ":",
        "hours_symbol": "h",
        "minutes_symbol": "min",
        "seconds_symbol": "s",
        "percent_symbol": "%",
        "unit": "$#value:number:separated=1;",
        "speed": "$#value:speed;",
        "distance": "$#value:distance;",
        "percent": "$#value:number:separated=1;$PERCENT_SYMBOL;",
        "fuel_usage_per_hour": "$#value:number:separated=1:decimalPlaces=2;/$HOURS_SYMBOL;",
        "time_hours_minutes_seconds": "$#value:time:showHours=1:showMinutes=1:showSeconds=1;",
        "time_minutes_seconds": "$#value:time:showMinutes=1:showSeconds=1;",
        "less_than_symbol": "&lt;",
        "gauge_panel_less_than": "$LESS_THAN_SYMBOL; $#value;",
        "firegroups_primary": "PRIMARY $#value;",
        "three": "$#x:number:decimalPlaces=1;$METERS_SYMBOL;,$#y:number:decimalPlaces=1;$METERS_SYMBOL;",
        "group_local": "Local",
        "received": "From $#name;:"
    }"##;
    Text::from_json(json.as_bytes()).expect("the sample table reads")
}

#[test]
fn number_rounds_half_up_and_groups() {
    let t = sample();
    assert_eq!(t.number(160.86, true, 0, 1.0, false), "161");
    assert_eq!(t.number(1234.5, true, 0, 1.0, false), "1,235");
    assert_eq!(t.number(1234.5, false, 0, 1.0, false), "1235");
    assert_eq!(t.number(1_234_567.0, true, 0, 1.0, false), "1,234,567");
    assert_eq!(t.number(0.1234, true, 2, 1.0, false), "0.12");
    assert_eq!(t.number(0.125, true, 2, 1.0, false), "0.13");
    assert_eq!(t.number(-2.5, true, 0, 1.0, false), "-3", "half up on the size");
    assert_eq!(t.number(4.0, true, 0, 1.0, true), "+4");
    assert_eq!(t.number(-4.0, true, 0, 1.0, true), "-4");
    assert_eq!(t.number(0.5, true, 1, 100.0, false), "50.0", "the multiplier");
    // (The notes' scoop sample 12.345 -> "12.35" is worked by hand: the double nearest 12.345 lies under it, so the rule
    // as written gives "12.34" for exactly that input.)
    assert_eq!(t.number(12.346, true, 2, 1.0, false), "12.35");
}

#[test]
fn speed_ladder() {
    let t = sample();
    assert_eq!(t.speed(160.86), "161m/s");
    assert_eq!(t.speed(999.0), "999m/s");
    assert_eq!(t.speed(2500.0), "2.50km/s");
    assert_eq!(t.speed(25_000.0), "25.0km/s");
    assert_eq!(t.speed(250_000.0), "250km/s");
    assert_eq!(t.speed(2_500_000.0), "2.50Mm/s");
    assert_eq!(t.speed(45_000_000.0), "0.15c");
    assert_eq!(t.speed(LIGHT_SPEED * 30.0), "30.0c");
}

const LIGHT_SPEED: f64 = 299_792_458.0;

#[test]
fn distance_ladder() {
    let t = sample();
    assert_eq!(t.distance(85.0, false), "85m");
    assert_eq!(t.distance(1234.0, false), "1.23km");
    assert_eq!(t.distance(12_500.0, false), "12.5km");
    assert_eq!(t.distance(1_200_000.0, false), "1.20Mm");
    assert_eq!(t.distance(1.35 * LIGHT_SPEED, false), "1.35Ls");
    assert_eq!(t.distance(3.0e11, false), "1,001Ls");
    assert_eq!(t.distance(3.0e11, true), "1.00kLs");
    assert_eq!(t.distance(5.31 * 9_221_136_340_147_200.0, false), "5.31Ly");
}

#[test]
fn time_parts() {
    let t = sample();
    let hms = TimeFlags { hours: true, minutes: true, seconds: true, ..TimeFlags::default() };
    assert_eq!(t.time(3725.0, &hms), "1:02:05");
    let ms = TimeFlags { minutes: true, seconds: true, ..TimeFlags::default() };
    assert_eq!(t.time(125.0, &ms), "2:05");
    assert_eq!(t.time(3725.0, &ms), "62:05", "the first part takes the larger units not shown");
    let limited = TimeFlags { term_limit: 2, ..hms };
    assert_eq!(t.time(3725.0, &limited), "1:02");
    let units = TimeFlags { units: true, ..hms };
    assert_eq!(t.time(3725.0, &units), "1h:02min:05s");
    assert_eq!(t.time(-5.0, &ms), "0:00");
}

#[test]
fn translate_the_hud_formats() {
    let t = sample();
    assert_eq!(t.translate("$UNIT:#value=160.86;"), "161");
    assert_eq!(t.translate("$SPEED:#value=2500;"), "2.50km/s");
    assert_eq!(t.translate("$PERCENT:#value=87;"), "87%");
    assert_eq!(t.translate("$FUEL_USAGE_PER_HOUR:#value=0.1234;"), "0.12/h");
    assert_eq!(t.translate("$DISTANCE:#value=1234;"), "1.23km");
    assert_eq!(t.translate("$TIME_HOURS_MINUTES_SECONDS:#value=3725;"), "1:02:05");
    assert_eq!(t.translate("$TIME_MINUTES_SECONDS:#value=125;"), "2:05");
    assert_eq!(t.translate("$GAUGE_PANEL_less_than:#value=1;"), "&lt; 1");
    assert_eq!(t.translate("$FIREGROUPS_primary:#value=A;"), "PRIMARY A");
}

#[test]
fn translate_grammar() {
    let t = sample();
    assert_eq!(t.translate("plain text, no tokens"), "plain text, no tokens");
    assert_eq!(t.translate("costs $5 and 10$"), "costs $5 and 10$", "a $ that starts no token is text");
    assert_eq!(t.translate("Heat $PERCENT:#value=12; now"), "Heat 12% now");
    assert_eq!(t.translate("$THREE:#x=1.25:#y=35.4;"), "1.3m,35.4m", "several fields");
    assert_eq!(t.translate("$RECEIVED:#name=$GROUP_LOCAL;;"), "From Local:", "a key inside a value");
    assert_eq!(t.translate("$percent:#value=5;"), "5%", "keys match whatever their case");
    assert_eq!(t.translate("$NO_SUCH_KEY;"), "$NO_SUCH_KEY;", "an unknown key is left as written");
    assert_eq!(t.translate("$NO_SUCH_KEY:#value=3;"), "$NO_SUCH_KEY:#value=3;");
    assert_eq!(t.translate("$UNIT:#other=3;"), "$#value:number:separated=1;", "a field not given");
    assert_eq!(t.translate("$UNIT:#value=fast;"), "$#value:number:separated=1;", "a typed value that is no number");
    assert_eq!(t.translate("$#value;"), "$#value;", "no fields at the top");
    assert_eq!(t.translate("$UNIT"), "$UNIT", "no end");
}

#[test]
fn literal_marker() {
    let t = sample();
    assert_eq!(t.translate("&0m/s"), "0m/s");
    assert_eq!(t.translate("&$UNIT:#value=5;"), "$UNIT:#value=5;", "literal: not expanded");
    assert_eq!(t.translate("&"), "");
    assert_eq!(t.translate("a & b $PERCENT:#value=5;"), "a & b 5%", "only a leading & marks it");
}

#[test]
fn deep_nesting_is_capped() {
    let t = sample();
    // A value nested 20,000 deep: parsed only to the cap, no stack overflow, and the text comes back.
    let deep = format!("{}{}", "$UNIT:#value=".repeat(20_000), ";".repeat(20_000));
    let out = t.translate(&deep);
    assert!(!out.is_empty());
    let deep_fields = format!("{}x{}", "$RECEIVED:#name=".repeat(20_000), ";".repeat(20_000));
    assert!(!t.translate(&deep_fields).is_empty());
}

#[test]
fn string_calls() {
    let t = sample();
    assert_eq!(t.get_string("UNIT"), "$#value:number:separated=1;");
    assert_eq!(t.get_string("missing_key"), "missing_key");
    assert!(t.contains("Speed") && !t.contains("missing_key"));
    assert_eq!(t.localise_number(0.0, "", None), "0");
    assert_eq!(t.localise_number(1234.0, "", None), "1,234");
    assert_eq!(t.localise_number(1_234_567.5, "", None), "1,234,567.5");
    assert_eq!(t.localise_number(12.345, "", None), "12.34", "the fraction's first two characters, not rounded");
}

#[test]
fn table_errors() {
    assert!(Text::from_json(b"[1, 2]").is_err());
    assert!(Text::from_json(br#"{"a": 3}"#).is_err());
    assert!(Text::from_json(br#"{"a": {"why": "no text"}}"#).is_err());
    assert!(Text::from_json(b"not json").is_err());
}

/// The real table from the export gives the notes' samples (`ELITE_UI_EXPORT`, default the export in `re/`).
#[test]
#[ignore = "reads Elite's string table from the export (out of git)"]
fn real_table_gives_the_notes_samples() {
    let dir = std::env::var("ELITE_UI_EXPORT")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/elite/frea_export/sidewinder_ui/decoded").into());
    let bytes = std::fs::read(format!("{dir}/strings/en.json")).expect("strings/en.json in the export");
    let t = Text::from_json(&bytes).expect("the real table reads");
    assert_eq!(t.translate("$UNIT:#value=160.86;"), "161");
    assert_eq!(t.translate("$UNIT:#value=1234.5;"), "1,235");
    assert_eq!(t.translate("$SPEED:#value=160.86;"), "161m/s");
    assert_eq!(t.translate("$SPEED:#value=2500;"), "2.50km/s");
    assert_eq!(t.translate("$SPEED:#value=45000000;"), "0.15c");
    assert_eq!(t.translate("$PERCENT:#value=87;"), "87%");
    assert_eq!(t.translate("$FUEL_USAGE_PER_HOUR:#value=0.1234;"), "0.12/h");
    assert_eq!(t.translate("$DISTANCE:#value=1234;"), "1.23km");
    assert_eq!(t.translate("$DISTANCE:#value=85;"), "85m");
    assert_eq!(t.translate("$TIME_HOURS_MINUTES_SECONDS:#value=3725;"), "1:02:05");
    assert_eq!(t.translate("$TIME_MINUTES_SECONDS:#value=125;"), "2:05");
    assert_eq!(t.translate("$GAUGE_PANEL_less_than:#value=1;"), "&lt; 1");
    assert_eq!(t.translate("$FIREGROUPS_primary:#value=A;"), "PRIMARY A");
    assert_eq!(t.translate("$ALARM_flightAssist;"), "Assist off");
}
