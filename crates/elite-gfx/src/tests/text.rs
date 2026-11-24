//! Text fields sized from their own measurement: Elite's FSS legend sets each field's width to its
//! `textWidth + 2` (`fsshud` `components/ControlEntry.as`). Flash Player (and upstream Ruffle since
//! 6429cc886) masks text 2 px inside each side of the field, which cut the last 2 px of each word
//! ("SHOW HELF"); Scaleform does not, and neither does the vendored Ruffle since patch f.

use super::rig::{Rig, STEP, out_dir};
use elite_ui::Value;

fn control(name: &str, binding: &str) -> Value {
    Value::object([("name", Value::from(name)), ("binding", Value::from(binding))])
}

/// The legend's line in the 1920 x 1080 picture: rows, and the columns left of the scanner's label.
const LEGEND_ROWS: std::ops::Range<usize> = 1025..1055;
const LEGEND_COLUMNS: usize = 1000;

/// Each legend word's drawn ink as (first, last) columns, left to right: "LEAVE FSS", "BACKSPACE",
/// "SHOW HELP", "H", "DISCOVERY SCAN", "SPACE". Measured with the text mask removed altogether (a
/// temporary edit of `edit_text.rs`, 2026-11-24): the whole glyphs, which the shown words must equal.
const WHOLE_WORDS: [(usize, usize); 6] = [(101, 184), (198, 291), (325, 421), (435, 444), (479, 615), (629, 679)];

/// The columns of ink (alpha over 40) on the legend's line, grouped into words (gaps over 9 px).
fn words(pixels: &[u8], width: usize) -> Vec<(usize, usize)> {
    let inked = |x: usize| LEGEND_ROWS.clone().any(|y| pixels[(y * width + x) * 4 + 3] > 40);
    let mut words: Vec<(usize, usize)> = Vec::new();
    for x in (0..LEGEND_COLUMNS).filter(|&x| inked(x)) {
        match words.last_mut() {
            Some((_, last)) if x - *last <= 9 => *last = x,
            _ => words.push((x, x)),
        }
    }
    words
}

/// The FSS screen opened with its legend: every word is drawn whole (its ink reaches its last
/// glyph's last column). The picture is `ui1/fss_legend.png`.
#[test]
#[ignore = "reads the UI export (out of git) and needs a Vulkan GPU"]
fn fss_legend_picture() {
    let rig = Rig::new();
    let (mut movie, target) = rig.load("fsshud");
    while !movie.ready() {
        movie.step(STEP);
    }
    movie.send("Open", &[]);
    movie.send("SetSystemName", &[Value::from("Ross 128")]);
    movie.send("SetState", &[Value::from(0)]);
    movie.send(
        "SetControls",
        &[Value::array([
            control("$ExplorationFSSQuit;", "BACKSPACE"),
            control("$ExplorationFSSShowHelp;", "H"),
            control("$ExplorationFSSDiscoveryScan;", "SPACE"),
        ])],
    );
    for _ in 0..90 {
        movie.step(STEP);
    }
    movie.draw();
    rig.shot(&target, &out_dir().join("fss_legend.png"));
    let shown = words(&rig.pixels(&target), target.width() as usize);
    println!("legend words (first, last column): {shown:?}");
    assert_eq!(shown, WHOLE_WORDS, "each legend word is drawn to its last glyph");
}
