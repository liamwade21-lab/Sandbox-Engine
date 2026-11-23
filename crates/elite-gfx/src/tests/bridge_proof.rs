//! The bridge's tweener and DataStore driven by a code block of our own (`as3/proof/Proof.as`,
//! compiled to `as3/proof.abc` by `as3/build.sh`) in a movie made here: no export needed, a GPU is
//! (ignored with the proofs).

use super::host::TestHost;
use super::rig::{headless_gpu, target};
use crate::{Movie, MovieFiles, Out, convert_movie};
use elite_ui::Value;
use ruffle_core::swf::{self, Compression, Header, Rectangle, TagCode, Twips};
use std::path::Path;
use std::sync::Arc;

fn tag(code: TagCode, body: &[u8]) -> Vec<u8> {
    let mut out = (((code as u16) << 6) | 0x3f).to_le_bytes().to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// A one-frame ActionScript 3 movie holding the proof block (eager), converted (the bridge goes in
/// before it).
fn proof_movie() -> Vec<u8> {
    let as3 = Path::new(env!("CARGO_MANIFEST_DIR")).join("as3");
    let proof = std::fs::read(as3.join("proof.abc")).expect("as3/proof.abc: run as3/build.sh");
    let mut abc = vec![0, 0, 0, 0];
    abc.extend_from_slice(b"proof");
    abc.push(0);
    abc.extend_from_slice(&proof);
    let tags = [
        tag(TagCode::FileAttributes, &[0x08, 0, 0, 0]),
        tag(TagCode::DoAbc2, &abc),
        tag(TagCode::ShowFrame, &[]),
        tag(TagCode::End, &[]),
    ]
    .concat();
    let header = Header {
        compression: Compression::None,
        version: 15,
        stage_size: Rectangle {
            x_min: Twips::ZERO,
            x_max: Twips::from_pixels(64.0),
            y_min: Twips::ZERO,
            y_max: Twips::from_pixels(64.0),
        },
        frame_rate: swf::Fixed8::from_f64(30.0),
        num_frames: 1,
    };
    let mut file = Vec::new();
    swf::write::write_swf_raw_tags(&header, &tags, &mut file).unwrap();
    convert_movie(&file, &std::fs::read(as3.join("bridge.abc")).unwrap()).unwrap()
}

/// The proof block's lines ("proof: ...") and the bridge's tween reports, in order.
fn lines(outs: &[Out]) -> Vec<String> {
    outs.iter()
        .filter_map(|o| match o {
            Out::Trace(t) if t.starts_with("proof: ") => Some(t["proof: ".len()..].to_string()),
            Out::Trace(t) if t.starts_with("tween dropped: ") => Some("tween dropped".to_string()),
            Out::Trace(t) => Some(format!("other: {t}")),
            _ => None,
        })
        .collect()
}

#[test]
#[ignore]
fn bad_tweens_are_dropped_and_the_store_announces_order_and_changes() {
    let gpu = headless_gpu();
    let host = Arc::new(TestHost::new("{}", r#"{"entries": []}"#));
    let files = MovieFiles { name: "proof".into(), swf: proof_movie(), fonts: Vec::new(), host };
    let mut movie = Movie::load(&gpu, files, target(&gpu, "proof", (64, 64))).unwrap();
    let run = |movie: &mut Movie, seconds: f64| {
        for _ in 0..(seconds * 60.0) as usize {
            movie.step(1.0 / 60.0);
        }
        movie.drain()
    };

    // A null target and a throwing setter: both dropped, the good tween still ends.
    let start = lines(&run(&mut movie, 0.5));
    assert_eq!(start, ["children Complete  -1", "tween dropped", "tween dropped", "good 10"], "{start:#?}");

    // New children are announced at their sorted place; a sort value's change as the new order.
    movie.store(&["r", "b"], "s", Value::from(2));
    movie.store(&["r", "a"], "s", Value::from(1));
    movie.store(&["r", "c"], "s", Value::from(3));
    movie.store(&["r", "c"], "s", Value::from(0));
    // A write that changes nothing is not heard; one that does is.
    movie.store(&["r", "a"], "v", Value::from(5));
    movie.store(&["r", "a"], "v", Value::from(5));
    movie.store(&["r", "a"], "v", Value::object([("x", 7)]));
    movie.store(&["r", "a"], "v", Value::object([("x", 7)]));
    let store = lines(&run(&mut movie, 0.1));
    assert_eq!(
        store,
        [
            "children Added r/b 0",
            "children Added r/a 0",
            "children Added r/c 2",
            "children Complete r/c,r/a,r/b -1",
            "a.v 5",
            "a.v 7",
        ],
        "{store:#?}"
    );
}
