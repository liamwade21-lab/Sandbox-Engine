//! Facade tests. The retail-data ones are `#[ignore]` and read `SKATE3_ASSET_ROOT` (the folder holding `private/`).
use super::*;
use crate::sim::collision_world_from_mesh;

fn root() -> Option<std::path::PathBuf> { std::env::var_os("SKATE3_ASSET_ROOT").map(Into::into) }

#[test]
fn raw_pad_quantises_like_a_device() {
    let pad = RawPad { buttons: buttons::A, triggers: [1.0, 0.5], left_stick: [-1.0, 1.0], right_stick: [0.0, -0.5] };
    let x = pad.xbox_state();
    assert_eq!(x.buttons, 0x1000);
    assert_eq!(x.triggers, [255, 128]);
    assert_eq!(x.left, [-32768, 32767]);
    assert_eq!(x.right, [0, -16384]);
    assert_eq!(RawPad::from_xinput(0, [0; 2], [-32768, 32767], [0; 2]).left_stick, [-1.0, 1.0]);
}

#[test]
fn flat_mesh_builds_a_valid_world() {
    let material = skate_core::physics::contact::RetailContactMaterial { static_friction: 0.5, dynamic_friction: 0.5, restitution: 0.1 };
    let world = collision_world_from_mesh(
        &[[-10.0, 0.0, -10.0], [10.0, 0.0, -10.0], [10.0, 0.0, 10.0], [-10.0, 0.0, 10.0]],
        &[[0, 3, 2], [0, 2, 1], [0, 0, 0]],
        material,
        true,
    ).unwrap();
    assert_eq!(world.triangles().len(), 2);
    assert!(world.query_metadata().is_ok());
    assert!(collision_world_from_mesh(&[[0.0; 3]], &[[0, 0, 0]], material, false).is_err());
}

/// Load, a flat 200 m square, spawn, push for 3 s then coast, then an ollie (right stick down, then up).
#[test]
#[ignore = "requires private stock data (SKATE3_ASSET_ROOT)"]
fn facade_push_coast_and_ollie_on_a_host_floor() {
    let digest = scenario(false);
    assert_ne!(digest, [0; 3]);
}

/// Swapping in identical geometry mid-push must not change anything the host sees (the world keeps no
/// state between ticks, and nothing of the skater indexes into it).
#[test]
#[ignore = "requires private stock data (SKATE3_ASSET_ROOT)"]
fn swapping_identical_world_mid_roll_changes_nothing() {
    assert_eq!(scenario(false), scenario(true));
}

fn scenario(swap_mid_push: bool) -> [u64; 3] {
    let root = root().expect("set SKATE3_ASSET_ROOT");
    let mut sim = Sim::load(&root, "normal").unwrap();
    let h = 100.0;
    let (floor_vertices, floor_triangles) = ([[-h, 0.0, -h], [h, 0.0, -h], [h, 0.0, h], [-h, 0.0, h]], [[0, 3, 2], [0, 2, 1]]);
    sim.set_world(&floor_vertices, &floor_triangles).unwrap();
    sim.spawn([0.0, 0.0, 0.0], 0.0).unwrap();
    eprintln!("period {:?}, bones {}", sim.period(), sim.skeleton().bone_names.len());
    if swap_mid_push == false {
        let k = sim.skeleton();
        let list: Vec<String> = k.bone_names.iter().zip(&k.parents).enumerate().map(|(i, (n, p))| format!("{i}:{n}<{p}")).collect();
        eprintln!("SKELETON {} board_bones {:?}", list.join(" "), k.board_bones);
    }
    let idle = RawPad::default();
    let push = RawPad { buttons: buttons::A, ..Default::default() };
    let mut states: Vec<String> = Vec::new();
    let mut note = |sim: &Sim, tick: usize, states: &mut Vec<String>| {
        let s = sim.state_name();
        if states.last() != Some(&s) {
            let (p, v) = sim.board();
            eprintln!("tick {tick}: {s} board y={:.3} pos=({:.2},{:.2}) speed={:.2}", p[1], p[0], p[2], (v[0] * v[0] + v[2] * v[2]).sqrt());
            states.push(s);
        }
    };
    let mut tick = 0;
    // Bit-exact fingerprint of everything the host sees each tick: compare builds (e.g. Bevy 0.18 vs 0.20).
    let mut digest = [0xcbf29ce484222325_u64; 3]; // [sim: board + bones, root, camera]
    let mix = |sim: &Sim, digest: &mut [u64; 3]| {
        let feed = |d: &mut u64, v: f32| { *d = (*d ^ u64::from(v.to_bits())).wrapping_mul(0x100000001b3); };
        let (p, v) = sim.board();
        p.into_iter().chain(v).for_each(|x| feed(&mut digest[0], x));
        if let Some(pose) = sim.pose() {
            pose.current.bones.iter().flatten().flatten().copied().for_each(|x| feed(&mut digest[0], x));
            pose.current.root.iter().flatten().copied().for_each(|x| feed(&mut digest[1], x));
            pose.current.camera.position.into_iter().chain(pose.current.camera.rotation).for_each(|x| feed(&mut digest[2], x));
        }
    };
    for _ in 0..60 { sim.step(&idle).unwrap(); mix(&sim, &mut digest); note(&sim, tick, &mut states); tick += 1; }
    let (rest, _) = sim.board();
    eprintln!("settled board y = {:.4}", rest[1]);
    let mut pushes = 0;
    for t in 0..180 {
        if swap_mid_push && t == 90 { sim.set_world(&floor_vertices, &floor_triangles).unwrap(); }
        sim.step(&push).unwrap(); mix(&sim, &mut digest); pushes += u32::from(sim.events().pushed); note(&sim, tick, &mut states); tick += 1;
    }
    let (_, v) = sim.board();
    let speed = (v[0] * v[0] + v[2] * v[2]).sqrt();
    eprintln!("after 3 s of push: speed {speed:.2} m/s, push events {pushes}");
    for _ in 0..30 { sim.step(&idle).unwrap(); mix(&sim, &mut digest); note(&sim, tick, &mut states); tick += 1; }
    let before = sim.board().1;
    let mut left_ground = false;
    let mut max_y = f32::MIN;
    let mut popped = false;
    let mut landed = false;
    for t in 0..150 {
        let stick = if t < 8 { -1.0 } else if t < 14 { 1.0 } else { 0.0 };
        sim.step(&RawPad { right_stick: [0.0, stick], ..Default::default() }).unwrap();
        mix(&sim, &mut digest);
        popped |= sim.events().popped;
        landed |= sim.events().landed;
        left_ground |= sim.events().airborne;
        max_y = max_y.max(sim.board().0[1]);
        note(&sim, tick, &mut states);
        tick += 1;
    }
    eprintln!("ollie: left ground {left_ground}, popped event {popped}, landed event {landed}, peak board y {max_y:.3} (rest {:.3}), speed before {:.2}", rest[1],
        (before[0] * before[0] + before[2] * before[2]).sqrt());
    eprintln!("DIGEST sim={:016x} root={:016x} camera={:016x}", digest[0], digest[1], digest[2]);
    let pose = sim.pose().expect("pose after ticks");
    assert_eq!(pose.current.bones.len(), sim.skeleton().bone_names.len());
    let blended = pose.blend(0.5, sim.skeleton());
    assert_eq!(blended.bones.len(), pose.current.bones.len());
    let c = sim.camera().unwrap();
    eprintln!("camera at {:?} looking {:?} fov {}", c.position, c.forward, c.fov_y_degrees);
    let hud = sim.hud();
    eprintln!("hud: {hud:?}");
    assert!(speed > 2.0, "pushing never got going: {speed}");
    assert!(left_ground, "the ollie never left the ground");
    assert!(popped && landed, "no pop / landing event: popped={popped} landed={landed}");
    assert!(pushes > 0 && pushes < 90, "push plants should be edges, not levels: {pushes}");
    digest
}
