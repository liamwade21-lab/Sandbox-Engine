//! The probe (docs/spec-mods-viltrumite-powers.md F12.1-2, T1): the buildings the scatter has placed round the
//! Viltrumite mod's hero, read back from the GPU, so they can be solid without a CPU copy of the city layout.
//!
//! - **The ask.** Each frame the mod is on, it names a capsule in the universe (`ProbeAsk`, written before
//!   `ask_probe`): from him to where he'll be by the time the list arrives, 40 m round its axis. `ask_probe` turns it
//!   into the camera-relative render frame the cull works in, numbers the frame (`seq`) and keeps that frame's camera,
//!   body and turn in an 8-entry ring, so each list is turned back into the right frame however late it comes.
//! - **On the GPU** (`scatter.wesl` `probe_building`, called from `cull_building` after the range fade and before the
//!   view's planes): every building inside is appended to binding 20 with its id; `gpu.rs` copies the list into one of
//!   four staging buffers made at start (never one made later: C19) and maps it. With the mod off `body.street.z` is 0,
//!   the cull's one uniform branch skips it, and nothing is copied.
//! - **The id** is the building's place in the city layout's own frame: its cube face and its middle's face metres x 4
//!   from the face's corner (i32). A building from a near cell and the same one from a far cell match within a few units
//!   and have the same model; the record's hash differs (it hashes cell-local positions). `urban` and `core` are read
//!   at each cell's own grid point, so the two can differ: a box's front and back setbacks follow them (`city.wesl`
//!   `ct_building`, `down`), which moves its middle along its lot's depth by up to 2.5 x the change of `down` / k face m
//!   (measured: at most one unit; worked out: about 2.3 face m at a cube corner, inside `MATCH_UNITS`). The step 9
//!   measurements and the bound are in the spec's Build log.
//! - **Back on the CPU** (`receive_probe`, in `First`): the newest list, each building in its body's frame (metres, the
//!   frame `Frame::Attached` uses), with its axes and size. `hero probe ...` (viltrumite) logs and measures it.
//!   `BuildingProbe::age_frames` says how old the list is now. When the scatter stops taking the asks the list goes:
//!   at once when it's off or has no body (its buildings are gone), after `UNSERVED_FRAMES` frames otherwise (a frame
//!   or two without drawing keeps it: the boxes stand where they stood).
//! - **Off costs nothing:** `ask_probe` and `receive_probe` run only while the mod asks (or a frame after, to take the
//!   ask out of `ScatterFrame`); the render world's `probe_readback` checks its staging buffers' states and nothing more.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy::render::RenderApp;
use se_core::upos::UPos;

use super::{Scatter, ScatterFrame, update_scatter};
use crate::camera::FlyCamera;
use crate::system::SystemBody;

/// Records a list holds; past this the GPU drops them (its count goes on, so an overflow shows).
pub const PROBE_CAP: u32 = 1024;
pub const PROBE_HEAD_BYTES: u64 = 48;
pub const PROBE_REC_BYTES: u64 = 48;
pub const PROBE_BYTES: u64 = PROBE_HEAD_BYTES + PROBE_CAP as u64 * PROBE_REC_BYTES;
/// The broken table (binding 19, step 10): a 48-byte header and 16-byte entries, made empty.
pub const BROKEN_ENTRIES: u64 = 16_384;
pub const BROKEN_BYTES: u64 = 48 + BROKEN_ENTRIES * 16;
/// Staging buffers for the copies, made once. A buffer is busy from its copy until its list is read, about the lag less
/// one frame (3-5 frames measured, to 5 at 51 fps): six leave a frame idle up to a lag of 7, the ring's last.
pub const STAGING: usize = 6;
/// Frames whose camera, body and turn are kept.
const RING: usize = 8;
/// Frames the scatter may skip the asks (active but not drawing) before the list goes.
pub const UNSERVED_FRAMES: u32 = 3;
/// Face metres x 4 within which a near and a far building are the same one (spec F12.2: lots are 12+ m apart).
pub const MATCH_UNITS: i32 = 16;

/// The broken table's header as `scatter.wesl` `BrokenHead` (step 10): how many entries, the clock their times are on (real
/// seconds), and a capsule left for later.
#[derive(Clone, Copy, Default, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct BrokenHeadGpu {
    pub count: u32,
    pub clock: f32,
    pub pad: [u32; 2],
    pub a: [f32; 4],
    pub b: [f32; 4],
}

/// A broken building as `scatter.wesl` `BrokenEntry`: its id (the probe's: face metres x 4), its face and model, when it broke.
#[derive(Clone, Copy, Default, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct BrokenEntryGpu {
    pub x: i32,
    pub y: i32,
    pub face_model: u32,
    pub t0: f32,
}

/// The broken table for the render world this frame (`viltrumite::breakable::update_broken`): the header every frame, the entries
/// whenever `version` changes.
#[derive(Clone, Default, Debug)]
pub struct BrokenFrame {
    pub head: BrokenHeadGpu,
    pub entries: Arc<Vec<BrokenEntryGpu>>,
    pub version: u32,
}

/// The list's header as `scatter.wesl` `ProbeHead` (the CPU writes it each frame the probe is on, count 0).
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct ProbeHeadGpu {
    pub count: u32,
    pub seq: u32,
    pub cap: u32,
    pub mode: u32,
    pub a: [f32; 4],
    pub b: [f32; 4],
}

/// One building as `scatter.wesl` `ProbeRec`: the draw's `Inst`, then the id.
#[derive(Clone, Copy, Default, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct ProbeRecGpu {
    /// Its foot (the plinth's bottom), camera-relative render frame (m).
    pub pos: [f32; 3],
    /// Width and depth (f16 x 2).
    pub wd: u32,
    /// Its x axis and up (octahedral, snorm16 x 2).
    pub xa: u32,
    pub up: u32,
    /// Its height with the plinth (m): the box is `scale` tall from `pos`.
    pub scale: f32,
    /// The library's model, the fade << 8, the draw's hash << 16.
    pub model: u32,
    /// Face metres x 4 from its face's corner.
    pub id: [i32; 2],
    /// The face, the model from the box << 8, the cell's kind (1 near, 2 far) << 16.
    pub tag: u32,
    /// The cell's metric (true m per face m).
    pub k: f32,
}

/// What the mod asks about this frame (taken by `ask_probe`; nothing asked, nothing probed).
#[derive(Resource, Default)]
pub struct ProbeAsk {
    pub capsule: Option<ProbeCapsule>,
}

#[derive(Clone, Copy, Debug)]
pub struct ProbeCapsule {
    pub from: UPos,
    pub to: UPos,
    /// Round its axis, beyond each building's half diagonal (m).
    pub margin_m: f32,
    /// Measuring: the near and far lists in full, whatever the camera's distance (not as drawn).
    pub both: bool,
    /// The sandbox's solid trees: those within this many metres of the axis are listed too (kind 4), 0 none.
    pub trees_m: f32,
}

/// The render world's part of a frame's ask (`ScatterFrame::probe`).
#[derive(Clone, Copy, Default, Debug)]
pub struct ProbeFrame {
    pub seq: u32,
    pub mode: u32,
    pub a: Vec4,
    pub b: Vec4,
    pub trees_m: f32,
}

/// A list as the GPU wrote it.
pub struct RawList {
    pub seq: u32,
    pub count: u32,
    pub recs: Vec<ProbeRecGpu>,
}

/// Lists read back, from the render world to the main world; `skipped` counts the frames whose list wasn't copied (no
/// staging buffer idle).
#[derive(Resource, Clone, Default)]
pub struct ProbeMail {
    pub lists: Arc<Mutex<Vec<RawList>>>,
    pub skipped: Arc<AtomicU32>,
}

/// A building of the list, in its body's frame.
#[derive(Clone, Copy, Debug)]
pub struct ProbeBuilding {
    pub id: IVec2,
    pub face: u8,
    /// The model from the box (`city.wesl` `M_*`, 1-2 the village's).
    pub model: u8,
    /// 1 near, 2 far; 4 a tree (the sandbox's: `model` is then the library's, `size.z` its scale, `id` its cell's seed and
    /// its number in the cell, `xa` nothing).
    pub kind: u8,
    /// Its foot (the plinth's bottom), from the body's centre, body frame (m).
    pub foot: DVec3,
    /// Its up and x axis (body frame).
    pub up: DVec3,
    pub xa: DVec3,
    /// Width (along `xa`), depth, height from the foot (m).
    pub size: Vec3,
    pub k: f32,
}

impl ProbeBuilding {
    /// Its middle (body frame).
    pub fn middle(&self) -> DVec3 {
        self.foot + self.up * (0.5 * self.size.z as f64)
    }

    /// How far a point (body frame) is outside its box (m; 0 inside): the box as `vertex_building` draws it, `xa` across the
    /// width, `up` from the foot, `cross(xa, up)` across the depth.
    pub fn box_distance(&self, p: DVec3) -> f64 {
        let r = p - self.foot;
        let za = self.xa.cross(self.up);
        let (x, y, z) = (r.dot(self.xa), r.dot(self.up), r.dot(za));
        let h = self.size.z as f64;
        DVec3::new((x.abs() - 0.5 * self.size.x as f64).max(0.0), (-y).max(y - h).max(0.0), (z.abs() - 0.5 * self.size.y as f64).max(0.0)).length()
    }
}

/// The frame a list was asked in.
#[derive(Clone, Copy, Debug)]
struct RingEntry {
    seq: u32,
    frame: u64,
    time: f64,
    body: Entity,
    cam: UPos,
    body_pos: UPos,
    orient: DQuat,
    /// The capsule's start from the camera and its length (m), for the trace.
    reach: f64,
    length: f64,
}

/// The probe's main-world state: the ring and the newest list.
#[derive(Resource, Default)]
pub struct BuildingProbe {
    /// This frame's number (counted in `First` while the probe is on) and the last ask's sequence number.
    frame: u64,
    seq: u32,
    ring: [Option<RingEntry>; RING],
    /// The mod asks (from its first ask to `clear`): `receive_probe` runs.
    on: bool,
    /// Frames in a row the mod asked and the scatter couldn't take it.
    unserved: u32,
    /// The frame the newest list was asked in.
    list_frame: u64,
    /// The newest list (body frame), its body, sequence number, and how many frames and real seconds it took from the
    /// ask to here (its age now: `age_frames`); the count as the GPU counted it (over `PROBE_CAP`: some were dropped).
    pub list: Vec<ProbeBuilding>,
    pub body: Option<Entity>,
    pub list_seq: u32,
    pub lag_frames: u32,
    pub lag_s: f64,
    pub count: u32,
    /// A list has arrived since the mod turned the probe on (or since the scatter last stopped taking the asks).
    pub arrived: bool,
    /// Lists the render world couldn't copy (no staging buffer idle), and lists that came back after their frame left
    /// the ring, since the start.
    pub skipped: u32,
    pub lost: u32,
    /// `hero probe log N`: lists left to log.
    pub log: u32,
    /// `hero probe trace FILE`: a line per list.
    pub trace: Option<std::io::BufWriter<std::fs::File>>,
    /// `hero probe stats on`: what the measurements collect.
    pub stats: Option<Box<ProbeStats>>,
    warned_ring: bool,
    /// Lists asked up to this one were asked before the last `clear`: dropped when they come.
    cleared_seq: u32,
}

impl BuildingProbe {
    /// The mod stopped asking: the list goes, and those still on their way are dropped when they come.
    pub fn clear(&mut self) {
        self.drop_list();
        self.on = false;
    }

    /// The list goes, and those still on their way are dropped when they come.
    fn drop_list(&mut self) {
        self.cleared_seq = self.seq;
        self.list.clear();
        self.body = None;
        self.arrived = false;
        self.count = 0;
    }

    /// How old the newest list is now, in frames: from its ask to this frame (its lag when it came, plus the frames
    /// since). `None` without a list.
    pub fn age_frames(&self) -> Option<u32> {
        self.arrived.then(|| (self.frame - self.list_frame) as u32)
    }
}

/// `ask_probe` runs while the mod asks, and the frame after, to take the ask out of `ScatterFrame`.
fn probe_asked(ask: Res<ProbeAsk>, out: Res<ScatterFrame>) -> bool {
    ask.capsule.is_some() || out.probe.is_some()
}

/// `receive_probe` runs while the probe is on.
fn probe_on(probe: Res<BuildingProbe>) -> bool {
    probe.on
}

pub fn build(app: &mut App) {
    let mail = ProbeMail::default();
    app.insert_resource(mail.clone())
        .init_resource::<ProbeAsk>()
        .init_resource::<BuildingProbe>()
        // (After Bevy's clock, so the lag in seconds counts this frame's time.)
        .add_systems(First, receive_probe.run_if(probe_on).after(bevy::time::TimeSystems))
        .add_systems(PostUpdate, ask_probe.run_if(probe_asked).after(update_scatter));
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.insert_resource(mail);
    }
}

/// Turns this frame's ask into the cull's frame and keeps the frame's camera and body (after `update_scatter`, which
/// made the frame's cells and pages from the same camera). An ask the scatter can't take (off, no body, not drawing)
/// drops the list: at once when the scatter is off or has no body, after `UNSERVED_FRAMES` frames otherwise.
pub fn ask_probe(
    mut ask: ResMut<ProbeAsk>,
    mut probe: ResMut<BuildingProbe>,
    mut out: ResMut<ScatterFrame>,
    sc: Res<Scatter>,
    cam: Res<FlyCamera>,
    time: Res<Time<Real>>,
    bodies: Query<&SystemBody>,
) {
    out.probe = None;
    let Some(c) = ask.capsule.take() else { return };
    let probe = &mut *probe;
    probe.on = true;
    let body = sc.body().and_then(|e| bodies.get(e).ok().map(|b| (e, b)));
    let Some((entity, b)) = body.filter(|_| out.active) else {
        probe.unserved = probe.unserved.saturating_add(1);
        if !sc.enabled || body.is_none() || probe.unserved >= UNSERVED_FRAMES {
            probe.drop_list();
        }
        return;
    };
    probe.unserved = 0;
    if let Some(s) = probe.stats.as_mut() {
        s.asked += 1;
    }
    probe.seq = probe.seq.wrapping_add(1).max(1);
    let seq = probe.seq;
    let (a, e) = (c.from.delta_m(cam.pos), c.to.delta_m(cam.pos));
    probe.ring[seq as usize % RING] = Some(RingEntry {
        seq,
        frame: probe.frame,
        time: time.elapsed_secs_f64(),
        body: entity,
        cam: cam.pos,
        body_pos: b.pos,
        orient: b.orient,
        reach: a.length(),
        length: (e - a).length(),
    });
    let (a, e) = (a.as_vec3(), e.as_vec3());
    out.probe = Some(ProbeFrame { seq, mode: if c.both { 2 } else { 1 }, a: a.extend(c.margin_m), b: e.extend(0.0), trees_m: c.trees_m });
}

fn oct_decode(p: u32) -> DVec3 {
    let x = (p & 0xffff) as u16 as i16 as f64 / 32767.0;
    let y = (p >> 16) as u16 as i16 as f64 / 32767.0;
    let (x, y) = (x.max(-1.0), y.max(-1.0));
    let z = 1.0 - x.abs() - y.abs();
    let v = if z < 0.0 { DVec3::new((1.0 - y.abs()) * x.signum_or_one(), (1.0 - x.abs()) * y.signum_or_one(), z) } else { DVec3::new(x, y, z) };
    v.normalize_or_zero()
}

trait SignumOrOne {
    fn signum_or_one(self) -> f64;
}

impl SignumOrOne for f64 {
    /// As WGSL's `select(-1, 1, v >= 0)`.
    fn signum_or_one(self) -> f64 {
        if self >= 0.0 { 1.0 } else { -1.0 }
    }
}

/// A record in the frame it was asked in, turned into its body's frame.
fn to_building(r: &ProbeRecGpu, base: DVec3, inv: DQuat) -> ProbeBuilding {
    let pos = DVec3::new(r.pos[0] as f64, r.pos[1] as f64, r.pos[2] as f64);
    let w = crate::capture::f16_to_f32((r.wd & 0xffff) as u16);
    let d = crate::capture::f16_to_f32((r.wd >> 16) as u16);
    ProbeBuilding {
        id: IVec2::new(r.id[0], r.id[1]),
        face: (r.tag & 0xff) as u8,
        model: ((r.tag >> 8) & 0xff) as u8,
        kind: ((r.tag >> 16) & 0xff) as u8,
        foot: inv * (base + pos),
        up: inv * oct_decode(r.up),
        xa: inv * oct_decode(r.xa),
        size: Vec3::new(w, d, r.scale),
        k: r.k,
    }
}

/// Takes the lists the render world read back: the newest is the list, each building in its body's frame. Runs first
/// in the frame, so the frame's numbers count from here.
pub fn receive_probe(mail: Res<ProbeMail>, mut probe: ResMut<BuildingProbe>, time: Res<Time<Real>>) {
    let probe = &mut *probe;
    probe.frame += 1;
    let skipped = mail.skipped.load(Ordering::Relaxed);
    if skipped != probe.skipped {
        if let Some(s) = probe.stats.as_mut() {
            s.skipped += skipped.wrapping_sub(probe.skipped);
        }
        probe.skipped = skipped;
    }
    let lists: Vec<RawList> = match mail.lists.lock() {
        Ok(mut m) => std::mem::take(&mut *m),
        Err(_) => return,
    };
    if lists.is_empty() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let newest = lists.iter().map(|l| l.seq).max().unwrap_or(0);
    for l in lists.into_iter().filter(|l| l.seq > probe.cleared_seq) {
        let Some(r) = probe.ring[l.seq as usize % RING].filter(|r| r.seq == l.seq) else {
            probe.lost += 1;
            if let Some(s) = probe.stats.as_mut() {
                s.lost += 1;
            }
            if !probe.warned_ring {
                warn!("probe: list {} came back after its frame left the ring ({RING} frames)", l.seq);
                probe.warned_ring = true;
            }
            continue;
        };
        let lag_frames = (probe.frame - r.frame) as u32;
        let lag_s = now - r.time;
        let base = r.cam.delta_m(r.body_pos);
        let inv = r.orient.inverse();
        let list: Vec<ProbeBuilding> = l.recs.iter().map(|rec| to_building(rec, base, inv)).collect();
        if let Some(s) = probe.stats.as_mut() {
            s.add(&list, l.count, lag_frames, lag_s, l.seq != newest);
        }
        if let Some(t) = probe.trace.as_mut() {
            let near = list.iter().filter(|b| b.kind == 1).count();
            let _ = writeln!(
                t,
                "{},{},{},{:.4},{},{},{},{},{:.0},{:.1}",
                probe.frame,
                l.seq,
                lag_frames,
                lag_s,
                l.count,
                near,
                list.len() - near,
                (l.count > PROBE_CAP) as u8,
                r.reach,
                r.length
            );
        }
        if probe.log > 0 {
            probe.log -= 1;
            log_list(&list, &l, lag_frames, lag_s);
        }
        if l.seq == newest {
            probe.list = list;
            probe.body = Some(r.body);
            probe.list_seq = l.seq;
            probe.list_frame = r.frame;
            probe.lag_frames = lag_frames;
            probe.lag_s = lag_s;
            probe.count = l.count;
            probe.arrived = true;
        }
    }
}

fn log_list(list: &[ProbeBuilding], l: &RawList, lag_frames: u32, lag_s: f64) {
    let near = list.iter().filter(|b| b.kind == 1).count();
    info!(
        "probe: list {}: {} buildings ({} near, {} far){}, {} frames ({:.1} ms) after its ask",
        l.seq,
        l.count,
        near,
        list.len() - near,
        if l.count > PROBE_CAP { " OVERFLOW" } else { "" },
        lag_frames,
        lag_s * 1000.0
    );
    for b in list.iter().take(12) {
        info!(
            "probe:   model {:2} kind {} face {} id ({}, {}) size {:.1} x {:.1} x {:.1} m, metric {:.3}, middle {:.1} m from the body's centre, x axis {:.2}",
            b.model,
            b.kind,
            b.face,
            b.id.x,
            b.id.y,
            b.size.x,
            b.size.y,
            b.size.z,
            b.k,
            b.middle().length(),
            b.xa
        );
    }
}

/// One building as the measurements keep it (the last time it was seen in its list).
#[derive(Clone, Copy, Debug)]
struct Seen {
    b: ProbeBuilding,
}

/// What `hero probe stats` measures: the lists' lag and counts, every building seen in the near and in the far lists
/// (by its exact id and model), matched at the report.
#[derive(Default)]
pub struct ProbeStats {
    /// Asks the scatter took, lists that came back, those superseded in their frame, not copied (no staging buffer
    /// idle), and lost (back after their frame left the ring).
    asked: u32,
    lists: u32,
    superseded: u32,
    skipped: u32,
    lost: u32,
    lag_frames: [u32; 16],
    lag_s_sum: f64,
    lag_s_max: f64,
    count_sum: u64,
    count_max: u32,
    overflows: u32,
    /// Lists with buildings in them, and those lists' counts.
    nonempty: u32,
    near: HashMap<(u8, u8, i32, i32), Seen>,
    far: HashMap<(u8, u8, i32, i32), Seen>,
    /// Within one list: two buildings of one model and face whose ids are within `MATCH_UNITS`.
    list_collisions: u32,
    collision_examples: Vec<String>,
}

impl ProbeStats {
    fn add(&mut self, list: &[ProbeBuilding], count: u32, lag_frames: u32, lag_s: f64, superseded: bool) {
        self.lists += 1;
        self.superseded += superseded as u32;
        self.lag_frames[(lag_frames as usize).min(15)] += 1;
        self.lag_s_sum += lag_s;
        self.lag_s_max = self.lag_s_max.max(lag_s);
        self.count_sum += count as u64;
        self.count_max = self.count_max.max(count);
        self.overflows += (count > PROBE_CAP) as u32;
        self.nonempty += (count > 0) as u32;
        for b in list {
            let key = (b.face, b.model, b.id.x, b.id.y);
            let map = if b.kind == 2 { &mut self.far } else { &mut self.near };
            map.insert(key, Seen { b: *b });
        }
        // Collisions inside the list (a grid of MATCH_UNITS squares).
        let mut grid: HashMap<(u8, u8, u8, i32, i32), Vec<usize>> = HashMap::new();
        for (i, b) in list.iter().enumerate() {
            grid.entry((b.kind, b.face, b.model, b.id.x.div_euclid(MATCH_UNITS), b.id.y.div_euclid(MATCH_UNITS))).or_default().push(i);
        }
        for (i, b) in list.iter().enumerate() {
            let (gx, gy) = (b.id.x.div_euclid(MATCH_UNITS), b.id.y.div_euclid(MATCH_UNITS));
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(v) = grid.get(&(b.kind, b.face, b.model, gx + dx, gy + dy)) else { continue };
                    for &j in v {
                        let o = &list[j];
                        if j > i && o.id != b.id && (o.id - b.id).abs().max_element() <= MATCH_UNITS {
                            self.list_collisions += 1;
                            if self.collision_examples.len() < 8 {
                                self.collision_examples.push(format!(
                                    "model {} kind {} ids ({}, {}) and ({}, {}), sizes {:.1}x{:.1}x{:.1} and {:.1}x{:.1}x{:.1}",
                                    b.model, b.kind, b.id.x, b.id.y, o.id.x, o.id.y, b.size.x, b.size.y, b.size.z, o.size.x, o.size.y, o.size.z
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    /// The report: lag, counts, and every far building against the near one nearest it of the same model and face.
    pub fn report(&self) {
        let lists = self.lists.max(1);
        let lag: Vec<String> = self.lag_frames.iter().enumerate().filter(|(_, n)| **n > 0).map(|(f, n)| format!("{f}: {n}")).collect();
        info!(
            "probe stats: {} asks, {} lists back ({} superseded in their frame, {} not copied: no staging buffer idle, {} lost from the ring), lag in frames [{}], mean {:.1} ms, max {:.1} ms; buildings a list mean {:.1}, max {}, {} overflows, {} lists not empty",
            self.asked,
            self.lists,
            self.superseded,
            self.skipped,
            self.lost,
            lag.join(", "),
            self.lag_s_sum / lists as f64 * 1000.0,
            self.lag_s_max * 1000.0,
            self.count_sum as f64 / lists as f64,
            self.count_max,
            self.overflows,
            self.nonempty
        );
        let per_model = |m: &HashMap<(u8, u8, i32, i32), Seen>| {
            let mut n: std::collections::BTreeMap<u8, u32> = default();
            for k in m.keys() {
                *n.entry(k.1).or_default() += 1;
            }
            n.iter().map(|(m, c)| format!("{m}: {c}")).collect::<Vec<_>>().join(", ")
        };
        info!(
            "probe stats: seen {} near buildings and {} far ones (distinct ids); by model near [{}], far [{}]",
            self.near.len(),
            self.far.len(),
            per_model(&self.near),
            per_model(&self.far)
        );
        // The near ones by face, model and MATCH_UNITS square.
        let mut grid: HashMap<(u8, u8, i32, i32), Vec<&Seen>> = HashMap::new();
        for s in self.near.values() {
            grid.entry((s.b.face, s.b.model, s.b.id.x.div_euclid(MATCH_UNITS), s.b.id.y.div_euclid(MATCH_UNITS))).or_default().push(s);
        }
        let nearest = |b: &ProbeBuilding, reach: i32| -> Option<&Seen> {
            let (gx, gy) = (b.id.x.div_euclid(MATCH_UNITS), b.id.y.div_euclid(MATCH_UNITS));
            let r = reach.div_euclid(MATCH_UNITS) + 1;
            let mut best: Option<(&Seen, i64)> = None;
            for dx in -r..=r {
                for dy in -r..=r {
                    for s in grid.get(&(b.face, b.model, gx + dx, gy + dy)).into_iter().flatten() {
                        let d = s.b.id - b.id;
                        if d.abs().max_element() > reach {
                            continue;
                        }
                        let d2 = d.x as i64 * d.x as i64 + d.y as i64 * d.y as i64;
                        if best.is_none_or(|(_, e)| d2 < e) {
                            best = Some((s, d2));
                        }
                    }
                }
            }
            best.map(|b| b.0)
        };
        // Pairs within the match: id shift (face m, the larger axis), true shift along the ground and up (m), size
        // differences. A far one whose nearest near one of its model is farther (to 4 times the match) is another
        // building (lots are 12+ m apart), its own near one missing: listed apart, with any near one of another model
        // within the match.
        let mut face_m = Vec::new();
        let mut true_m = Vec::new();
        let mut vert_m = Vec::new();
        let mut dw: f32 = 0.0;
        let mut dd: f32 = 0.0;
        let mut dh: Vec<f64> = Vec::new();
        let mut by_model: HashMap<u8, (u32, f64, f32)> = HashMap::new();
        let mut neighbour = Vec::new();
        let mut unmatched = Vec::new();
        for s in self.far.values() {
            let Some(n) = nearest(&s.b, 4 * MATCH_UNITS) else {
                unmatched.push(s.b);
                continue;
            };
            let d = (n.b.id - s.b.id).abs().max_element();
            if d > MATCH_UNITS {
                neighbour.push((s.b, n.b));
                continue;
            }
            let d = d as f64 / 4.0;
            face_m.push(d);
            let delta = n.b.middle() - s.b.middle();
            let up = s.b.up;
            let along = (delta - up * delta.dot(up)).length();
            true_m.push(along);
            vert_m.push((n.b.foot - s.b.foot).dot(up));
            dw = dw.max((n.b.size.x - s.b.size.x).abs());
            dd = dd.max((n.b.size.y - s.b.size.y).abs());
            let h = (n.b.size.z - s.b.size.z).abs();
            dh.push(h as f64);
            let e = by_model.entry(s.b.model).or_insert((0, 0.0, 0.0));
            e.0 += 1;
            e.1 = e.1.max(d);
            e.2 = e.2.max(h);
        }
        let pct = |v: &mut Vec<f64>, p: f64| -> f64 {
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(|a, b| a.total_cmp(b));
            v[((v.len() - 1) as f64 * p).round() as usize]
        };
        let hist = |v: &[f64]| -> String {
            let edges = [0.05, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
            let mut out = Vec::new();
            let mut lo = 0.0;
            for e in edges {
                out.push(format!("<{e}: {}", v.iter().filter(|&&x| x >= lo && x < e).count()));
                lo = e;
            }
            out.push(format!(">=8: {}", v.iter().filter(|&&x| x >= 8.0).count()));
            out.join(", ")
        };
        info!(
            "probe shift: {} far buildings with a near one of their model within {} face m, {} whose nearest is another building (4-16 face m), {} with none within 16; id shift (face m, larger axis) max {:.2}, p50 {:.2}, p90 {:.2}, p99 {:.2} [{}]",
            face_m.len(),
            MATCH_UNITS / 4,
            neighbour.len(),
            unmatched.len(),
            pct(&mut face_m.clone(), 1.0),
            pct(&mut face_m.clone(), 0.5),
            pct(&mut face_m.clone(), 0.9),
            pct(&mut face_m.clone(), 0.99),
            hist(&face_m)
        );
        info!(
            "probe shift: true metres along the ground max {:.2}, p50 {:.2}, p90 {:.2}, p99 {:.2} [{}]; foot height near minus far (the drawn ground then) max {:.2}, min {:.2}; size differences max width {:.2}, depth {:.2}, height {:.2} m ({} of {} differ in height, p90 {:.2})",
            pct(&mut true_m.clone(), 1.0),
            pct(&mut true_m.clone(), 0.5),
            pct(&mut true_m.clone(), 0.9),
            pct(&mut true_m.clone(), 0.99),
            hist(&true_m),
            vert_m.iter().copied().reduce(f64::max).unwrap_or(0.0),
            vert_m.iter().copied().reduce(f64::min).unwrap_or(0.0),
            dw,
            dd,
            pct(&mut dh.clone(), 1.0),
            dh.iter().filter(|&&h| h > 0.01).count(),
            dh.len(),
            pct(&mut dh.clone(), 0.9)
        );
        let mut models: Vec<_> = by_model.into_iter().collect();
        models.sort_by_key(|m| m.0);
        info!(
            "probe shift by model (model: pairs, max face m, max height difference m): {}",
            models.iter().map(|(m, (n, d, h))| format!("{m}: {n}, {d:.2}, {h:.1}")).collect::<Vec<_>>().join("; ")
        );
        // Any near building, whatever its model, within the match of a far one.
        let any_model = |b: &ProbeBuilding| -> Option<ProbeBuilding> {
            self.near.values().map(|s| s.b).filter(|n| n.face == b.face && (n.id - b.id).abs().max_element() <= MATCH_UNITS).min_by_key(|n| (n.id - b.id).abs().max_element())
        };
        for (f, n) in neighbour.iter().take(6) {
            info!(
                "probe shift: far model {} id ({}, {}) size {:.1} x {:.1} x {:.1}: nearest near of its model {:.2} face m off, size {:.1} x {:.1} x {:.1}; near of any model within {} face m: {:?}",
                f.model,
                f.id.x,
                f.id.y,
                f.size.x,
                f.size.y,
                f.size.z,
                (n.id - f.id).abs().max_element() as f64 / 4.0,
                n.size.x,
                n.size.y,
                n.size.z,
                MATCH_UNITS / 4,
                any_model(f).map(|o| (o.model, o.size.z))
            );
        }
        for b in unmatched.iter().take(6) {
            info!(
                "probe shift: far without a near: model {} id ({}, {}) face {} size {:.1} x {:.1} x {:.1}; near of any model within {} face m: {:?}",
                b.model,
                b.id.x,
                b.id.y,
                b.face,
                b.size.x,
                b.size.y,
                b.size.z,
                MATCH_UNITS / 4,
                any_model(b).map(|o| (o.model, o.size.z))
            );
        }
        // Near tall ones (25 m and over, as the far cells hold) with no far one: only meaningful where the far list
        // covered the same ground (measuring mode at a fixed anchor).
        let mut far_grid: HashMap<(u8, u8, i32, i32), Vec<&Seen>> = HashMap::new();
        for s in self.far.values() {
            far_grid.entry((s.b.face, s.b.model, s.b.id.x.div_euclid(MATCH_UNITS), s.b.id.y.div_euclid(MATCH_UNITS))).or_default().push(s);
        }
        let near_tall: Vec<&Seen> = self.near.values().filter(|s| s.b.size.z >= 25.0).collect();
        let near_alone: Vec<&&Seen> = near_tall
            .iter()
            .filter(|s| {
                let (gx, gy) = (s.b.id.x.div_euclid(MATCH_UNITS), s.b.id.y.div_euclid(MATCH_UNITS));
                !(-4..=4).any(|dx| {
                    (-4..=4).any(|dy| {
                        far_grid.get(&(s.b.face, s.b.model, gx + dx, gy + dy)).is_some_and(|v| v.iter().any(|f| (f.b.id - s.b.id).abs().max_element() <= 4 * MATCH_UNITS))
                    })
                })
            })
            .collect();
        info!("probe shift: {} near buildings 25 m or taller, {} of them without a far one within 16 face m", near_tall.len(), near_alone.len());
        for s in near_alone.iter().take(4) {
            info!("probe shift: near without a far: model {} id ({}, {}) size {:.1} x {:.1} x {:.1}", s.b.model, s.b.id.x, s.b.id.y, s.b.size.x, s.b.size.y, s.b.size.z);
        }
        // Collisions: two distinct ids of one model within MATCH_UNITS, in the lists and among all seen.
        let mut seen_collisions = 0;
        let mut examples = Vec::new();
        for (map, name) in [(&self.near, "near"), (&self.far, "far")] {
            let mut g: HashMap<(u8, u8, i32, i32), Vec<&Seen>> = HashMap::new();
            for s in map.values() {
                g.entry((s.b.face, s.b.model, s.b.id.x.div_euclid(MATCH_UNITS), s.b.id.y.div_euclid(MATCH_UNITS))).or_default().push(s);
            }
            for s in map.values() {
                let (gx, gy) = (s.b.id.x.div_euclid(MATCH_UNITS), s.b.id.y.div_euclid(MATCH_UNITS));
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for o in g.get(&(s.b.face, s.b.model, gx + dx, gy + dy)).into_iter().flatten() {
                            let d = o.b.id - s.b.id;
                            if (d.x, d.y) > (0, 0) && d.abs().max_element() <= MATCH_UNITS {
                                seen_collisions += 1;
                                if examples.len() < 6 {
                                    examples.push(format!("{name} model {} ({}, {}) and ({}, {})", s.b.model, s.b.id.x, s.b.id.y, o.b.id.x, o.b.id.y));
                                }
                            }
                        }
                    }
                }
            }
        }
        info!(
            "probe ids: {} collisions within one list, {} among all buildings seen (two ids of one model within {} face m){}{}",
            self.list_collisions,
            seen_collisions,
            MATCH_UNITS / 4,
            if examples.is_empty() { String::new() } else { format!(": {}", examples.join("; ")) },
            if self.collision_examples.is_empty() { String::new() } else { format!("; in lists: {}", self.collision_examples.join("; ")) }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_shader() {
        // scatter.wesl ProbeHead and ProbeRec (Inst 32 bytes, then the id, meta, k).
        assert_eq!(std::mem::size_of::<ProbeHeadGpu>() as u64, PROBE_HEAD_BYTES);
        assert_eq!(std::mem::size_of::<ProbeRecGpu>() as u64, PROBE_REC_BYTES);
        assert_eq!(std::mem::size_of::<super::super::CellGpu>(), 64);
    }

    #[test]
    fn oct_round_trip() {
        // The shader's oct_encode, in f64.
        fn enc(n: DVec3) -> u32 {
            let a = n / (n.x.abs() + n.y.abs() + n.z.abs());
            let mut e = (a.x, a.y);
            if a.z < 0.0 {
                e = ((1.0 - a.y.abs()) * a.x.signum_or_one(), (1.0 - a.x.abs()) * a.y.signum_or_one());
            }
            let q = |v: f64| ((v.clamp(-1.0, 1.0) * 32767.0).round() as i16) as u16 as u32;
            q(e.0) | (q(e.1) << 16)
        }
        for n in [DVec3::X, DVec3::NEG_Y, DVec3::new(0.3, -0.5, -0.8).normalize(), DVec3::new(-0.7, 0.1, 0.7).normalize(), DVec3::NEG_Z] {
            assert!((oct_decode(enc(n)) - n).length() < 1e-4, "{n}");
        }
    }

    #[test]
    fn stats_match_near_and_far() {
        let b = |kind: u8, x: i32, y: i32| ProbeBuilding {
            id: IVec2::new(x, y),
            face: 2,
            model: 4,
            kind,
            foot: DVec3::new(x as f64 / 4.0, y as f64 / 4.0, 6.4e6),
            up: DVec3::Z,
            xa: DVec3::X,
            size: Vec3::new(30.0, 30.0, 120.0),
            k: 1.0,
        };
        let mut s = ProbeStats::default();
        s.add(&[b(1, 1000, 1000), b(1, 4000, 4000)], 2, 3, 0.05, false);
        s.add(&[b(2, 1006, 998), b(2, 9000, 9000)], 2, 4, 0.06, false);
        assert_eq!(s.near.len(), 2);
        assert_eq!(s.far.len(), 2);
        // Two of one model 3 face m apart in one list: a collision.
        s.add(&[b(1, 2000, 2000), b(1, 2012, 2000)], 2, 3, 0.05, false);
        assert_eq!(s.list_collisions, 1);
        s.report();
    }
}
