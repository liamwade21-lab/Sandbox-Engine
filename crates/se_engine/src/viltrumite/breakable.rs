//! Solid and breakable buildings (F12). So far (step 9's first part) the probe alone: every frame he's on, the capsule
//! the scatter's cull lists the buildings in (`scatter::probe`), from him to where he'll be when the list comes back.
//! Stopping, sliding, roofs and the camera's pull-in (the step's second part) and the broken table (step 10) build on
//! `BuildingProbe::list`.

use bevy::math::DVec3;
use bevy::prelude::*;
use se_core::catalog::ObjectRef;

use super::{Hero, tune};
use crate::scatter::probe::{BuildingProbe, ProbeAsk, ProbeCapsule};
use crate::system::SystemBody;

/// `hero probe anchor` and `both`: measuring (the step 9 numbers), not play.
#[derive(Resource, Default)]
pub struct ProbeDev {
    /// The capsule held at a point of a body (its frame, m) with this radius, instead of following him.
    pub anchor: Option<(ObjectRef, DVec3, f32)>,
    /// The near and far lists in full, whatever the camera's distance.
    pub both: bool,
}

/// The capsule for this frame: from him along his velocity over the ground for as many frames as a list takes to come back
/// and two more, `PROBE_MARGIN_M` round it (after `hero_move`, before `scatter::probe::ask_probe`).
pub fn probe_ask(hero: Res<Hero>, probe: Res<BuildingProbe>, dev: Res<ProbeDev>, time: Res<Time<Real>>, bodies: Query<&SystemBody>, mut ask: ResMut<ProbeAsk>) {
    if let Some((obj, local, radius)) = dev.anchor {
        let Some(b) = bodies.iter().find(|b| b.obj == obj) else { return };
        let at = b.pos.offset_m(b.orient * local);
        ask.capsule = Some(ProbeCapsule { from: at, to: at, margin_m: radius, both: dev.both });
        return;
    }
    let dt = time.delta_secs_f64().min(0.1);
    let lag = if probe.arrived { probe.lag_frames.min(tune::PROBE_LAG_MAX) } else { tune::PROBE_LAG_FRAMES };
    let from = hero.world.0;
    let to = from.offset_m(hero.last_vel * dt * (lag as f64 + tune::PROBE_AHEAD_FRAMES));
    ask.capsule = Some(ProbeCapsule { from, to, margin_m: tune::PROBE_MARGIN_M, both: dev.both });
}

/// Off: no ask (the scatter takes none), and the list is dropped.
pub fn probe_off(probe: &mut BuildingProbe, dev: &mut ProbeDev) {
    probe.clear();
    dev.anchor = None;
    dev.both = false;
}

/// `hero probe log [N]` | `trace FILE|off` | `stats [on|off|reset]` | `anchor [R]|off` | `both on|off` | `list`.
/// `log`: the next N lists (default 1), each with its first buildings. `trace`: a line per list to FILE (frame, sequence,
/// lag in frames and seconds, count, near, far, overflow, the capsule's start from the camera, its length). `stats on` collects every list's lag and count and every
/// building seen in the near and far lists; `stats` alone logs them (the near-far shift, the ids' collisions). `anchor`:
/// the capsule held where he is now, R m round it (default 300), whatever he does after. `both`: the near and far lists in
/// full, whatever the camera's distance (measuring only: as drawn the far list starts at 85 % of the near range).
pub fn dev_command(world: &mut World, arg: &str) {
    let (verb, rest) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
    match verb.to_ascii_lowercase().as_str() {
        "log" => world.resource_mut::<BuildingProbe>().log = rest.parse().unwrap_or(1),
        "list" => {
            let p = world.resource::<BuildingProbe>();
            let near = p.list.iter().filter(|b| b.kind == 1).count();
            info!(
                "hero probe: list {} of {} buildings ({} near, {} far, {} counted), {} frames ({:.1} ms) old when it came, {} now; {} lists not copied (no staging buffer idle), {} lost from the ring",
                p.list_seq,
                p.list.len(),
                near,
                p.list.len() - near,
                p.count,
                p.lag_frames,
                p.lag_s * 1000.0,
                p.age_frames().map_or("no list".to_string(), |a| format!("{a} frames")),
                p.skipped,
                p.lost
            );
            // The nearest boxes to him now (his middle in the list's body's frame as it is now).
            let (Some(e), pos) = (p.body, world.resource::<Hero>().world.0) else { return };
            let Some(b) = world.get::<SystemBody>(e) else { return };
            let local = b.orient.inverse() * pos.delta_m(b.pos);
            let p = world.resource::<BuildingProbe>();
            let mut by: Vec<_> = p.list.iter().map(|x| (x.box_distance(local), x)).collect();
            by.sort_by(|a, b| a.0.total_cmp(&b.0));
            let cam_fwd = world.resource::<crate::camera::FlyCamera>().rot * DVec3::NEG_Z;
            for (d, x) in by.iter().take(3) {
                // Which way its nearest point lies from the view's forward, about its up (`hero aim`'s sign), on the level.
                let r = local - x.foot;
                let za = x.xa.cross(x.up);
                let (w, h, dp) = (0.5 * x.size.x as f64, x.size.z as f64, 0.5 * x.size.y as f64);
                let q = x.foot + x.xa * r.dot(x.xa).clamp(-w, w) + x.up * r.dot(x.up).clamp(0.0, h) + za * r.dot(za).clamp(-dp, dp);
                let up = b.orient * x.up;
                let v = b.orient * (q - local);
                let (f, v) = (cam_fwd - up * cam_fwd.dot(up), v - up * v.dot(up));
                let yaw = f.cross(v).dot(up).atan2(f.dot(v)).to_degrees();
                info!(
                    "hero probe:   {:.2} m from him (its nearest point {:.0} degrees round from the view): model {} kind {} id ({}, {}), {:.1} x {:.1} x {:.1} m, its foot {:.2} m under his middle",
                    d,
                    yaw,
                    x.model,
                    x.kind,
                    x.id.x,
                    x.id.y,
                    x.size.x,
                    x.size.y,
                    x.size.z,
                    (local - x.foot).dot(x.up)
                );
            }
        }
        "trace" => {
            let mut p = world.resource_mut::<BuildingProbe>();
            p.trace = if rest.eq_ignore_ascii_case("off") {
                None
            } else {
                match std::fs::File::create(rest) {
                    Ok(f) => {
                        let mut w = std::io::BufWriter::new(f);
                        let _ = std::io::Write::write_all(&mut w, b"frame,seq,lag_frames,lag_s,count,near,far,overflow,reach_m,length_m\n");
                        Some(w)
                    }
                    Err(e) => {
                        warn!("hero probe trace: {rest}: {e}");
                        None
                    }
                }
            };
        }
        "stats" => {
            let mut p = world.resource_mut::<BuildingProbe>();
            match rest.to_ascii_lowercase().as_str() {
                "on" => p.stats = Some(Box::default()),
                "off" => p.stats = None,
                "reset" => p.stats = p.stats.as_ref().map(|_| Box::default()),
                _ => match p.stats.as_ref() {
                    Some(s) => s.report(),
                    None => info!("hero probe stats: not collecting (hero probe stats on)"),
                },
            }
        }
        "anchor" => {
            if rest.eq_ignore_ascii_case("off") {
                world.resource_mut::<ProbeDev>().anchor = None;
                return;
            }
            let radius: f32 = rest.parse().unwrap_or(300.0);
            let hero = world.resource::<Hero>();
            let (Some(obj), pos) = (hero.frame.body(), hero.world.0) else {
                warn!("hero probe anchor: he's on no body");
                return;
            };
            let mut q = world.query::<&SystemBody>();
            let Some(local) = q.iter(world).find(|b| b.obj == obj).map(|b| b.orient.inverse() * pos.delta_m(b.pos)) else { return };
            world.resource_mut::<ProbeDev>().anchor = Some((obj, local, radius));
            info!("hero probe: anchored, {radius} m round where he is");
        }
        "both" => world.resource_mut::<ProbeDev>().both = rest.eq_ignore_ascii_case("on"),
        _ => warn!("hero probe: unknown command '{arg}'"),
    }
}
