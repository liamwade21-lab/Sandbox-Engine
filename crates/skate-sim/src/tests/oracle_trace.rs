//! Sandbox Engine's parity oracle: one scripted controller sequence on flat ground through the production frame, every
//! board and skeleton body's rates hashed by their bits each tick. The same file runs in Sandbox Engine's `skate-sim`;
//! the two traces must be identical line for line.
use super::*;

/// XInput buttons as the recordings hold them.
const A: u16 = 0x1000;
const X: u16 = 0x4000;

/// The script: (first tick, last tick, buttons, left stick, right stick), sticks -1..1 (y up).
const SCRIPT: &[(u32, u32, u16, [f32; 2], [f32; 2])] = &[
    (40, 46, A, [0.0, 0.0], [0.0, 0.0]),
    (90, 96, A, [0.0, 0.0], [0.0, 0.0]),
    (140, 146, A, [0.0, 0.0], [0.0, 0.0]),
    (190, 196, A, [0.0, 0.0], [0.0, 0.0]),
    (260, 300, 0, [-0.8, 0.0], [0.0, 0.0]),
    (330, 350, 0, [0.0, 0.0], [0.0, -1.0]),
    (351, 356, 0, [0.0, 0.0], [0.0, 1.0]),
    (470, 490, 0, [0.0, 0.0], [0.0, -1.0]),
    (491, 496, 0, [0.0, 0.0], [-0.7, 0.7]),
    (600, 606, A, [0.0, 0.0], [0.0, 0.0]),
    (640, 680, 0, [0.7, 0.0], [0.0, 0.0]),
    (720, 740, 0, [0.0, 0.0], [0.0, -1.0]),
    (741, 744, 0, [0.0, 0.0], [-1.0, 0.0]),
    (745, 748, 0, [0.0, 0.0], [0.0, 1.0]),
    (850, 900, X, [0.0, 0.0], [0.0, 0.0]),
];
const TICKS: u32 = 1000;

fn axis(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

pub(crate) fn pad_at(tick: u32) -> skate_core::input::xbox::XboxState {
    let mut s = skate_core::input::xbox::XboxState { buttons: 0, triggers: [0, 0], left: [0, 0], right: [0, 0] };
    for &(a, b, buttons, l, r) in SCRIPT {
        if (a..=b).contains(&tick) {
            s.buttons |= buttons;
            s.left = [axis(l[0]), axis(l[1])];
            s.right = [axis(r[0]), axis(r[1])];
        }
    }
    s
}

fn fnv(h: &mut u64, bits: u32) {
    for byte in bits.to_le_bytes() {
        *h ^= u64::from(byte);
        *h = h.wrapping_mul(0x100000001b3);
    }
}

#[test]
#[ignore = "requires private assets; SKATE3_ASSET_ROOT and SKATE3_TRACE_OUT (and SKATE3_TRACE_EXPECT to compare)"]
fn oracle_trace() {
    let root = std::env::var_os("SKATE3_ASSET_ROOT").expect("set SKATE3_ASSET_ROOT");
    let root = std::path::Path::new(&root);
    let out = std::env::var_os("SKATE3_TRACE_OUT").expect("set SKATE3_TRACE_OUT");
    let assets = skate_data::GameAssets::load(root).unwrap();
    let graphs = crate::graph_runtime::StockGraphs::load(root, &assets).unwrap();
    let mut physics = GamePhysics::load_with_terrain(root, ground::Terrain::Flat).unwrap();
    physics.set_difficulty(crate::difficulty::Difficulty::Easy);
    let mut skater = SkaterRuntime::load(root, &graphs, &physics, "easy").unwrap();
    let mut controls = PlayerControls::load(root).unwrap();
    let mut input = crate::input::ControllerInput::default();
    let mut camera = crate::camera::CameraRuntime::load(root).unwrap();
    let mut text = String::new();
    for tick in 0..TICKS {
        // The production tick, as the game's systems run it: `input::publish_actions` (collect, publish), `controls::sample`
        // (its own copy of the actions), `physics::advance` (a fresh copy).
        input.sample_raw_for_test(pad_at(tick));
        let mut map = input.player_actions();
        controls.update_for_physics(&mut map, &physics, &skater, &camera).unwrap();
        controls.publish_gestures(physics.animation_profile.physics_mode, skater.player_input.physical.state.state_16);
        let mut actions = input.player_actions();
        let result = frame::advance(&mut physics, &mut skater, &mut controls, &graphs, &mut actions, true, &mut camera);
        let mut h = 0xcbf29ce484222325_u64;
        for body in physics.board.bodies().iter().chain(skater.skeleton.bodies().iter()) {
            let r = body.rates;
            for v in [r.position, r.linear_velocity, r.angular_velocity] {
                fnv(&mut h, v.x.to_bits());
                fnv(&mut h, v.y.to_bits());
                fnv(&mut h, v.z.to_bits());
            }
        }
        let deck = physics.board.bodies()[6].rates;
        text.push_str(&format!(
            "{tick} {:?} {:?} {:016x} p=({:.4},{:.4},{:.4}) v=({:.4},{:.4},{:.4}) err={}\n",
            skater.player_state.current(),
            skater.animation.motion.animation.current_name,
            h,
            deck.position.x, deck.position.y, deck.position.z,
            deck.linear_velocity.x, deck.linear_velocity.y, deck.linear_velocity.z,
            result.as_ref().err().map(String::as_str).unwrap_or("-"),
        ));
        if result.is_err() {
            break;
        }
    }
    std::fs::write(out, &text).unwrap();
    // With a reference trace (upstream's, kept as a fixture in Sandbox Engine), the run must match it line for line.
    if let Some(expect) = std::env::var_os("SKATE3_TRACE_EXPECT") {
        let expect = std::fs::read_to_string(expect).unwrap().replace("
", "
");
        for (i, (a, b)) in expect.lines().zip(text.lines()).enumerate() {
            assert_eq!(a, b, "tick {i} differs from the reference");
        }
        assert_eq!(expect.lines().count(), text.lines().count(), "the reference has another length");
    }
}
