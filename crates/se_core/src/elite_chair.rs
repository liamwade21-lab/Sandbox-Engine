//! Elite Dangerous's pilot chair animation as pure code (ARCHITECTURE.md A5): sampling its clips, the blend tree's
//! nodes, and the motion graph `CockpitChairMotionGraph` for a seated solo pilot. The references are the export's
//! `manipose.py` (clips) and `blend_ref.py` (the nodes, matched bit for bit against the exe), in
//! `re/elite/frea_export/sidewinder/decoded/animation/`; the proofs are `re/elite/notes/sidewinder_chair.md` and
//! `agent_motiongraph.md` (sections cited as "MG 2.3"). The data is `chair/clips.json`, `chair/clips.bin` and
//! `chair/graph.json` (`crate::elite_ship`); the skeleton is `models/cockpit_chair.edm`'s bones. Nothing here draws,
//! reads a file or keeps a clock: the caller steps it once a frame with the frame's seconds and its inputs, and skins
//! the chair with the matrices it returns.
//!
//! Conventions:
//! - Elite's model space (+X starboard, +Y up, +Z nose, left-handed), metres, seconds; nothing is converted to the mod's
//!   space here.
//! - Matrices are glam's with column vectors. Elite stores row vectors (row-major, translation in row 3, a point mapping
//!   as p' = p.M, world = local.parent world); a matrix here is Elite's transposed, and the one place that converts is
//!   `elite_matrix` (the stored inverse binds). `bone_matrices` are the posed world matrices, `skin_matrices` posed world
//!   x inverse bind (Elite's `inverse_bind . world`), so a vertex skins as `skin * v`.
//! - Clips are sampled as `manipose.py` does: rotations in float64 (cast to float32 for the pose), positions in float32.
//!   The pose and the blend nodes are float32 in the exe's operation order (`blend_ref.py`); world matrices are worked
//!   out in float64 from the float32 pose, as `manipose.world_matrices` does, and handed out as float32.
//! - The exe normalises with rsqrtps and one Newton step; rsqrtps is approximate on real CPUs, exact in the emulator the
//!   reference was proven in. This uses the exact `1 / sqrt` (the emulator's), so it matches the reference bit for bit.
//! - Bones and clips are matched by name without case (the graph writes clip names in mixed case, the clip file in
//!   lower case). A channel's bone must be the skeleton's by name and index; the one exception is the clips' two
//!   centre-screen bones (`CHAIR_LACKS`), which the Sidewinder's chair does not have and which drive nothing.
//! - Damaged data is an `Err` from `Chair::new` naming its file, never a panic later: it runs the files' own checks
//!   (`ChairClips::check`, `ChairGraph::check`: key offsets in checked arithmetic, no activity starting itself, nesting
//!   at most `CHAIR_MAX_NESTING`, every number finite) and refuses what this module does not implement. `step` takes a
//!   negative or non-finite `dt` as 0 and non-finite inputs as 0.
//!
//! Implemented (all proven unless listed below):
//! - Clips: phase to frames (`frames_at`), positions linear in float32, rotations normalised-lerped on the first key's
//!   side (`sample_clip`).
//! - Blend nodes: `additive_node` (AdditiveBlendNode), `normal_node` (NormalBlendNode), `leftover_node`
//!   (LeftoverBlendNode), their fast slerp (`fslerp`). The tree is MG 3.1's: one AdditiveBlend over a LeftoverBlend per
//!   layer, layers chained in number order (a layer persists once made), leaves at priority 1 masked to their clip's
//!   bones before the layer below (priority 0, weight 1, masked to the bones animated below), additive edges evaluated
//!   on the identity pose, every Leftover's input the rest pose. So a non-additive clip is an override on its own bones
//!   (the throttle hand, the buffets), an additive one a delta (the stick, the yaw, the trigger).
//! - The graph: the root handler picks the state from the loop value (`CockpitChairAction_Loop`; key = enum index + 1);
//!   the chair template writes `Mode_Ship` when the pilot sits down and `Empty_Ship` when he gets up, and the graph starts
//!   at `Uninitialised`. Activities: Animation (phase from a variable, clamped or wrapped, or free running at the rate
//!   ParamBlock; weight from the weight ParamBlock x fade, clamped), Grouped, Triggered (restarted by its event, off when
//!   its clip completes), JoystickButton (In, Loop, Out). State changes take the current state's listed transition, or
//!   the default block, which routes along the cheapest path; the fade is the block's (linear over its time, 0.2 s when
//!   negative, a cut at 0, or its curve); outgoing activities fade by 1 - c, incoming by c; an activity both states
//!   start keeps running (as AnimationActivitySmoothTransition keeps a matching clip); a block's wait flag waits for the
//!   state's completion or its LoopEvent (a looping base clip's wrap).
//! - Inputs (MG 2.6): throttle -> `ThrottleControl` = the 10-tap average of clamp((throttle + 1) / 2) (one tap per
//!   update, as the exe: its frame time is unused); roll = x, pitch = y, smoothed together by the 7-tap average ->
//!   `JoystickWeight` = min(r, 1), `JoystickProp` = the angle round the stick (0 at +y, 0.25 at +x, 0.5 at -y, 0.75 at
//!   -x); yaw -> `JoystickYawProp` = (its 7-tap average + 1) / 2; fire -> `PrimaryTriggerPress` (seated only); while
//!   nobody sits the controls are 0. Buffets: the velocity change's largest component over its threshold (ties x, y, z)
//!   picks the event (x < 0 Left, x >= 0 Right, z >= 0 Back, z < 0 Fwd, y < 0 Above, y >= 0 Under); intensity
//!   clamp(max(5, mass ratio) x |dv|^2 / 200^2, 0, 1), x 0.3 for the pilot's seat; it fires only when above the last
//!   buffet's intensity or 0.5 s after it; speed in [1, 1.25).
//!
//! Not traced in the exe (chosen here, with the reason):
//! - The buffets' mass ratio (body +0x164 / event +0x70) is taken as at most 5, so the exe's own floor max(5, ratio)
//!   gives 5: the ship has no collision event here to take it from. The intensity gate compares the raw intensity (before
//!   the seat's 0.3), as the collision component keeps it; the speed's random draw is a fixed-seed xorshift, so a run
//!   repeats exactly. Camera modes 3 and 4 (no buffet for the seat) are the caller's to skip.
//! - Within one step: inputs, buffets, the decision, the cross-fade timer, a new transition, then the activities; a
//!   transition started this step and a clip started this step begin at time 0 and advance from the next step (the
//!   exe's interleaving of the animator update with the graph update was not traced).
//! - A request made while a cross-fade runs waits until it ends; a route through an intermediate state takes one hop at a
//!   time, each with its own listed block, the next hop once the last cross-fade ended (the default block's own
//!   0.2 s applies only when no path exists). For the shipped data this only matters for the first sitting down
//!   (`Uninitialised` -> `Empty_Ship` -> `Mode_Ship`, 0.5 s each).
//! - An activity running in both states during a change is kept at full weight; for AnimationActivity that is
//!   SmoothTransition's rule, for the Grouped buffets (the same record in both states) it is a choice.
//! - A triggered clip or a button stage is taken off at the update after it completes, so its last frame shows once.
//! - Leaves of one layer at the same priority keep the order they started in (the exe's heap sort is not stable for
//!   ties; it matters only when two buffets overlap on a bone).
//! - A phase-driven clip whose phase went down this step is sampled backwards (the exe sets the leaf's backwards flag
//!   from the sign of the rate it works out); the leaf's stored phase is taken as counted forwards.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use glam::{DMat4, DQuat, DVec3, Mat4};

use crate::elite_asset::Edm;
use crate::elite_ship::{ChairActivity, ChairBlock, ChairClip, ChairClips, ChairGraph, ChairKey, ChairParam, ShipDataError};

/// What `Chair::new` refuses, each naming its file.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ChairError {
    /// What the files' own checks refuse (the error names `chair/clips.json`, `chair/clips.bin` or `chair/graph.json`).
    #[error("{0}")]
    Data(#[from] ShipDataError),
    #[error("the chair skeleton (models/cockpit_chair.edm's bones): {0}")]
    Skeleton(String),
    #[error("chair/clips.json: {0}")]
    Clips(String),
    #[error("chair/graph.json: {0}")]
    Graph(String),
}

/// 1 / 32767 as the exe stores it (0x145110e80).
const INV_32767: f32 = 3.051_851e-5;
/// The frame fraction under which one frame is used alone, and over which the next alone (0x145111f30 / 0x145111f34).
const SNAP_LO: f32 = 0.001;
const SNAP_HI: f32 = 0.999;
/// Every blend weight threshold (strict >).
const T: f32 = 0.001;
/// A transition block's negative fade (DAT_144e29770).
const NEGATIVE_FADE_S: f32 = 0.2;
/// The throttle's 10-tap average, newest first (0x145310320).
const THROTTLE_TAPS: [f32; 10] = [0.2, 0.2, 0.2, 0.1, 0.1, 0.1, 0.025, 0.025, 0.025, 0.025];
/// The stick's and the yaw's 7-tap average, newest first (0x1453102c0).
const STICK_TAPS: [f32; 7] = [0.3, 0.3, 0.3, 0.05, 0.025, 0.0125, 0.0125];
/// 2 pi as the exe stores it (0x144def63c).
const TWO_PI: f32 = 6.283_185_5;
/// Buffets (CockpitCollisionAnimationComponent, exe globals at 0x145e89c74..): a new one only above the last one's
/// intensity or this long after it; the pilot's seat gets this share; the speed is 1 + this x [0, 1).
const BUFFET_REFIRE_S: f32 = 0.5;
const BUFFET_SEAT_SHARE: f32 = 0.3;
const BUFFET_SPEED_SPREAD: f32 = 0.25;
/// I = clamp(max(BUFFET_MASS_FLOOR, mass ratio) x |dv|^2 / BUFFET_DV^2, 0, 1); the ratio is not traced (module docs).
const BUFFET_MASS_FLOOR: f32 = 5.0;
const BUFFET_MASS_RATIO: f32 = BUFFET_MASS_FLOOR;
const BUFFET_DV: f32 = 200.0;
/// The activity flags implemented: bit 0 loop, bit 1 additive edge, bit 4 LoopEvent at the wrap (MG 2.3). Any other is
/// refused: mirror (2), edge event weight (3), re-evaluate (5) and no stream argument (6) appear in no shipped activity.
const SUPPORTED_FLAGS: u32 = 0b1_0011;
/// The clips' bones the Sidewinder's chair lacks (`sidewinder_chair.md`: the `_activatechair` and `_idles` sets' arm).
const CHAIR_LACKS: [&str; 2] = ["BN_CentreScreenLowerArm_joint", "BN_CentreScreen_joint"];

// ------------------------------------------------------------------------------------------------ clips

/// One bone's local rotation (x, y, z, w) and position, relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BonePose {
    pub q: [f32; 4],
    pub p: [f32; 3],
}

const IDENTITY: BonePose = BonePose { q: [0.0, 0.0, 0.0, 1.0], p: [0.0; 3] };

/// A clip sampled at a phase: one value per channel, in the clip's channel order.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipSample {
    pub rotations: Vec<[f32; 4]>,
    pub positions: Vec<[f32; 3]>,
}

/// Phase (0..1) to (frame a, frame b, fraction), 0x14074bca0: f = (frames - 1) x phase (x (1 - phase) backwards); a
/// fraction under 0.001 uses frame a alone, over 0.999 frame a + 1 alone; backwards the frames count from the end.
pub fn frames_at(phase: f32, frames: u32, reverse: bool) -> (usize, usize, f32) {
    let p = if reverse { 1.0 - phase } else { phase };
    let f = (frames.max(1) - 1) as f32 * p;
    let a = f.floor();
    let frac = f - a;
    let (fa, fb) = if frac > SNAP_HI {
        (a + 1.0, a + 1.0)
    } else if frac < SNAP_LO {
        (a, a)
    } else {
        (a, a + 1.0)
    };
    let last = frames.max(1) as i64 - 1;
    let (fa, fb) = ((fa as i64).clamp(0, last), (fb as i64).clamp(0, last));
    if reverse {
        ((last - fa) as usize, (last - fb) as usize, frac)
    } else {
        (fa as usize, fb as usize, frac)
    }
}

fn rot_key(k: &[i16]) -> [f64; 4] {
    let q = [0, 1, 2, 3].map(|i| (k[i] as f32 * INV_32767) as f64);
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    q.map(|v| v / n)
}

/// 0x1407329a0: the second key on the first's side (t takes the dot's sign), (1 - t) q0 + t q1, normalised.
fn nlerp(q0: [f64; 4], q1: [f64; 4], t: f64) -> [f64; 4] {
    let d: f64 = (0..4).map(|i| q0[i] * q1[i]).sum();
    let tt = if d < 0.0 { -t } else { t };
    let r = [0, 1, 2, 3].map(|i| (1.0 - t) * q0[i] + tt * q1[i]);
    let n = r.iter().map(|v| v * v).sum::<f64>().sqrt();
    r.map(|v| v / n)
}

fn pos_key(k: &[i16], offset: [f32; 3], scale: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|i| (k[i] as f32 * INV_32767) * scale[i] + offset[i])
}

/// A clip at a phase, as `manipose.sample`, reading its keys from `clips`. None when a key or table entry lies outside
/// what `clips` holds (never for clips that passed `ChairClips::check`).
pub fn sample_clip(clips: &ChairClips, clip: &ChairClip, phase: f32, reverse: bool) -> Option<ClipSample> {
    let (a, b, t) = frames_at(phase, clip.frames, reverse);
    let key = |at: u64, count: usize, frame: usize, ch: usize| -> Option<&[i16]> {
        let i = usize::try_from(at / 2).ok()?.checked_add(frame.checked_mul(count)?.checked_add(ch)?.checked_mul(4)?)?;
        clips.keys.get(i..i.checked_add(4)?)
    };
    let nr = clip.rot_channels.len();
    let rotations = (0..nr)
        .map(|c| Some(nlerp(rot_key(key(clip.rot_keys_at, nr, a, c)?), rot_key(key(clip.rot_keys_at, nr, b, c)?), t as f64).map(|v| v as f32)))
        .collect::<Option<_>>()?;
    let np = clip.pos_channels.len();
    let positions = clip
        .pos_channels
        .iter()
        .enumerate()
        .map(|(c, ch)| {
            let e = ch.table? as usize;
            let (offset, scale) = (*clip.pos_offset.get(e)?, *clip.pos_scale.get(e)?);
            let pa = pos_key(key(clip.pos_keys_at, np, a, c)?, offset, scale);
            let pb = pos_key(key(clip.pos_keys_at, np, b, c)?, offset, scale);
            Some([0, 1, 2].map(|i| (pb[i] - pa[i]) * t + pa[i]))
        })
        .collect::<Option<_>>()?;
    Some(ClipSample { rotations, positions })
}

// ------------------------------------------------------------------------------------------------ blend nodes

/// An edge of a blend node: the child's pose, the edge weight (+0x10), active (+0x1e), the bone mask (+0x30, None:
/// every bone) and the priority (+0x1c, LeftoverBlendNode only).
#[derive(Clone, Copy, Debug)]
pub struct BlendChild<'a> {
    pub pose: &'a [BonePose],
    pub weight: f32,
    pub active: bool,
    pub mask: Option<&'a [bool]>,
    pub priority: u16,
}

impl BlendChild<'_> {
    fn masked(&self, bone: usize) -> bool {
        self.mask.map_or(true, |m| m[bone])
    }
}

fn norm_factor(len2: f32) -> f32 {
    let r = 1.0 / len2.sqrt();
    (3.0 - (r * r) * len2) * (r * 0.5)
}

fn neg_if(sign_src: f32, v: f32) -> f32 {
    if sign_src.is_sign_negative() {
        -v
    } else {
        v
    }
}

/// The exe's slerp approximation: t' = t (1 + k + t (2kt - 3k)), k = 0.45 (1 - 0.9 |dot|)^2 (0x145112050/60).
fn fast_t(t: f32, dot: f32) -> f32 {
    let k = 1.0 - dot.abs() * 0.9;
    let k = (k * k) * 0.45;
    ((-3.0 * k + (k + k) * t) * t + (1.0 + k)) * t
}

/// Blend quaternion a towards b by t (b taken on a's side), normalised: the exe's fast slerp.
pub fn fslerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot = ((a[2] * b[2] + a[3] * b[3]) + a[1] * b[1]) + a[0] * b[0];
    let tp = fast_t(t, dot);
    let s0 = 1.0 - tp;
    let s1 = neg_if(dot, tp);
    let r = [0, 1, 2, 3].map(|i| a[i] * s0 + b[i] * s1);
    let len2 = ((r[3] * r[3] + r[2] * r[2]) + r[1] * r[1]) + r[0] * r[0];
    let n = norm_factor(len2);
    r.map(|v| v * n)
}

/// AdditiveBlendNode's last step: fslerp(identity, d, t) with the exe's literal dot.
fn scale_from_identity(d: [f32; 4], t: f32) -> [f32; 4] {
    let dot = ((d[3] * 1.0 + d[2] * 0.0) + d[1] * 0.0) + d[0] * 0.0;
    let tp = fast_t(t, dot);
    let s0 = 1.0 - tp;
    let s1 = neg_if(dot, tp);
    let w = 1.0 * s0 + d[3] * s1;
    let z = d[2] * s1 + s0 * 0.0;
    let x = d[0] * s1 + s0 * 0.0;
    let y = d[1] * s1 + s0 * 0.0;
    let len2 = ((w * w + z * z) + y * y) + x * x;
    let n = norm_factor(len2);
    [x * n, y * n, z * n, w * n]
}

/// Hamilton product b (x) d in the exe's order (base first).
fn qmul(b: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    let [bx, by, bz, bw] = b;
    let [dx, dy, dz, dw] = d;
    [
        ((bx * dw + bw * dx) + by * dz) - bz * dy,
        ((by * dw + bw * dy) + bz * dx) - bx * dz,
        ((bz * dw + bw * dz) + bx * dy) - by * dx,
        ((bw * dw - bx * dx) - by * dy) - bz * dz,
    ]
}

/// NormalBlendNode (0x14073f7e0): the children mixed by weight over the input pose; what their weights leave goes to
/// the input. Positions are never renormalised.
pub fn normal_node(input: &[BonePose], children: &[BlendChild]) -> Vec<BonePose> {
    let live: Vec<&BlendChild> = children.iter().filter(|c| c.active && c.weight > T).collect();
    if live.is_empty() {
        return input.to_vec();
    }
    (0..input.len())
        .map(|b| {
            let (mut q, mut p) = ([0.0f32; 4], [0.0f32; 3]);
            let (mut rem, mut acc) = (1.0f32, 0.0f32);
            for c in &live {
                let w = if c.masked(b) { c.weight } else { 0.0 };
                let w = if w > T { w } else { 0.0 };
                let first = rem == 1.0;
                rem -= w;
                acc += w;
                let BonePose { q: qc, p: pc } = c.pose[b];
                if first {
                    q = qc;
                    p = [0, 1, 2].map(|i| 0.0 + pc[i] * w);
                } else {
                    q = fslerp(q, qc, w / acc);
                    p = [0, 1, 2].map(|i| p[i] + pc[i] * w);
                }
            }
            if rem > T {
                let BonePose { q: qi, p: pi } = input[b];
                q = fslerp(q, qi, rem);
                p = [0, 1, 2].map(|i| pi[i] * rem + p[i]);
            }
            BonePose { q, p }
        })
        .collect()
}

/// AdditiveBlendNode (0x14073d330): `base` is what its base child gave; each child is a delta (evaluated on the
/// identity pose), laid on as rotation base (x) fslerp(identity, delta, weight), position base + delta x weight.
pub fn additive_node(base: &[BonePose], children: &[BlendChild]) -> Vec<BonePose> {
    let live: Vec<&BlendChild> = children.iter().filter(|c| (if c.active { c.weight } else { 0.0 }) > T).collect();
    if live.is_empty() {
        return base.to_vec();
    }
    (0..base.len())
        .map(|b| {
            let (mut aq, mut ap, mut aw) = (IDENTITY.q, [0.0f32; 3], 0.0f32);
            for (n, c) in live.iter().enumerate() {
                let w = if c.masked(b) { c.weight } else { 0.0 };
                let m = w > T;
                let w = if m { w } else { 0.0 };
                let BonePose { q: qc, p: pc } = c.pose[b];
                if n == 0 {
                    if m {
                        aw = w + aw;
                        aq = qc;
                        ap = pc.map(|v| v * w);
                    } else {
                        aq = IDENTITY.q;
                        ap = [0.0; 3];
                    }
                } else {
                    aw = w + aw;
                    if m {
                        aq = fslerp(aq, qc, w / aw);
                    }
                    ap = [0, 1, 2].map(|i| pc[i] * w + ap[i]);
                }
            }
            let BonePose { q: bq, p: bp } = base[b];
            BonePose { q: qmul(bq, scale_from_identity(aq, aw)), p: [0, 1, 2].map(|i| bp[i] + ap[i]) }
        })
        .collect()
}

/// LeftoverBlendNode (0x14073eb00): children in array order, a new priority group whenever the priority drops; per
/// bone a group's weights are scaled down to 1 when they sum above it (or to their sum when no later child weighs the
/// bone), what a group leaves goes to the later groups, then to the input pose. One child: a hard switch per bone.
pub fn leftover_node(input: &[BonePose], children: &[BlendChild]) -> Vec<BonePose> {
    let bones = input.len();
    if let [c] = children {
        if !(c.active && c.weight > T) {
            return input.to_vec();
        }
        return (0..bones).map(|b| if c.masked(b) && c.weight > T { c.pose[b] } else { input[b] }).collect();
    }
    let k = children.len();
    let w: Vec<Vec<f32>> = children
        .iter()
        .map(|c| {
            let we = if c.active { c.weight } else { 0.0 };
            (0..bones).map(|b| if we > T && c.masked(b) { we } else { 0.0 }).collect()
        })
        .collect();
    let last: Vec<usize> = (0..bones).map(|b| (0..k).filter(|&i| w[i][b] > T).map(|i| i + 1).max().unwrap_or(0)).collect();
    let (mut rem, mut acc, mut norm) = (vec![1.0f32; bones], vec![0.0f32; bones], vec![1.0f32; bones]);
    let mut out = input.to_vec();
    let mut cur: u32 = u32::MAX;
    let mut count = 0;
    for (i, c) in children.iter().enumerate() {
        let prio = c.priority as u32;
        if prio < cur {
            for b in 0..bones {
                let (mut s, mut more) = (0.0f32, 0.0f32);
                for j in i..k {
                    if j >= last[b] {
                        break;
                    }
                    if children[j].priority as u32 != prio {
                        more = 1.0;
                        break;
                    }
                    s += w[j][b];
                }
                norm[b] = if more < s { 1.0 / s } else { 1.0 };
                rem[b] -= acc[b];
                acc[b] = 0.0;
            }
            cur = prio;
        }
        if !(0..bones).any(|b| w[i][b] > T && rem[b] > T) {
            continue;
        }
        for b in 0..bones {
            let BonePose { q: qc, p: pc } = c.pose[b];
            let e = (w[i][b] * norm[b]) * rem[b];
            if count == 0 {
                out[b] = BonePose { q: qc, p: pc.map(|v| v * e) };
                acc[b] = e + acc[b];
            } else {
                acc[b] = e + acc[b];
                let den = (1.0 - rem[b]) + acc[b];
                let q = if den > T { fslerp(out[b].q, qc, e / den) } else { qc };
                out[b] = BonePose { q, p: [0, 1, 2].map(|n| pc[n] * e + out[b].p[n]) };
            }
        }
        count += 1;
    }
    if count == 0 {
        return input.to_vec();
    }
    for b in 0..bones {
        let left = rem[b] - acc[b];
        if left > T {
            let BonePose { q: qi, p: pi } = input[b];
            out[b] = BonePose { q: fslerp(out[b].q, qi, left), p: [0, 1, 2].map(|n| pi[n] * left + out[b].p[n]) };
        }
    }
    out
}

// ------------------------------------------------------------------------------------------------ curves

/// A ParamBlock's or a fade's keys at v (0x140601760): the first key's value below it, the last's above it, between
/// keys the left key's interpolation.
fn eval_keys(keys: &[ChairKey], v: f32) -> f32 {
    let Some(first) = keys.first() else { return 0.0 };
    if v <= first.x {
        return first.y;
    }
    let mut prev = first;
    for k in &keys[1..] {
        if v <= k.x {
            let dx = k.x - prev.x;
            let dy = k.y - prev.y;
            let t = (v - prev.x) / dx;
            let f = match prev.interp {
                1 => t,
                2 => t.powf(prev.a),
                3 => (prev.a.powf(t) - 1.0) / (prev.a - 1.0),
                4 => t * t * 3.0 - (t * t * t + t * t * t),
                5 => {
                    let m0 = dx * prev.a;
                    return ((dy * 3.0 - (m0 + m0)) - dx * prev.b) * t * t + t * m0 + ((m0 - (dy + dy)) + dx * prev.b) * t * t * t + prev.y;
                }
                _ => 0.0,
            };
            return f * dy + prev.y;
        }
        prev = k;
    }
    prev.y
}

// ------------------------------------------------------------------------------------------------ the skeleton

/// One bone of the chair model (`models/cockpit_chair.edm`): local values relative to its parent, Elite's stored inverse
/// bind (row-major, translation in the last row).
#[derive(Clone, Debug, PartialEq)]
pub struct ChairBone {
    pub name: String,
    /// -1 for a root; always before the bone.
    pub parent: i16,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: f32,
    pub inverse_bind: [f32; 16],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChairSkeleton {
    pub bones: Vec<ChairBone>,
}

impl ChairSkeleton {
    /// The chair model's bones, which must carry their inverse binds.
    pub fn from_edm(edm: &Edm) -> Result<ChairSkeleton, ChairError> {
        let bones = edm
            .parts
            .iter()
            .map(|p| {
                Ok(ChairBone {
                    name: p.name.clone(),
                    parent: p.parent,
                    translation: p.translation,
                    rotation: p.rotation,
                    scale: p.scale,
                    inverse_bind: p.inverse_bind.ok_or_else(|| ChairError::Skeleton(format!("bone {} has no inverse bind", p.name)))?,
                })
            })
            .collect::<Result<Vec<_>, ChairError>>()?;
        let s = ChairSkeleton { bones };
        s.check()?;
        Ok(s)
    }

    fn check(&self) -> Result<(), ChairError> {
        if self.bones.is_empty() {
            return Err(ChairError::Skeleton("no bones".into()));
        }
        if let Some((i, b)) = self.bones.iter().enumerate().find(|(i, b)| b.parent < -1 || b.parent as i64 >= *i as i64) {
            return Err(ChairError::Skeleton(format!("bone {i} ({}): its parent {} does not come before it", b.name, b.parent)));
        }
        Ok(())
    }

    fn rest(&self) -> Vec<BonePose> {
        self.bones.iter().map(|b| BonePose { q: b.rotation, p: b.translation }).collect()
    }

    /// Posed world matrices (float64, column vectors): world = parent world x local.
    fn world(&self, pose: &[BonePose]) -> Vec<DMat4> {
        let mut w: Vec<DMat4> = Vec::with_capacity(pose.len());
        for (b, bp) in self.bones.iter().zip(pose) {
            let q = DQuat::from_xyzw(bp.q[0] as f64, bp.q[1] as f64, bp.q[2] as f64, bp.q[3] as f64);
            let t = DVec3::new(bp.p[0] as f64, bp.p[1] as f64, bp.p[2] as f64);
            let local = DMat4::from_scale_rotation_translation(DVec3::splat(b.scale as f64), q, t);
            w.push(if b.parent < 0 { local } else { w[b.parent as usize] * local });
        }
        w
    }
}

/// Elite's row-major, row-vector matrix as glam's column-vector one (the same numbers read as columns).
fn elite_matrix(m: &[f32; 16]) -> DMat4 {
    DMat4::from_cols_array(&m.map(|v| v as f64))
}

// ------------------------------------------------------------------------------------------------ the graph

/// What the chair is fed each step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChairInputs {
    /// Someone sits in it (the local pilot).
    pub seated: bool,
    /// Elite's `ForwardReverseThrust`, -1 (full reverse) ..1.
    pub throttle: f32,
    /// Elite's `Roll` (+1 the stick to the left), `Pitch` (+1 the stick forward), `Yaw`, each -1..1.
    pub roll: f32,
    pub pitch: f32,
    pub yaw: f32,
    /// `PrimaryFire` held.
    pub fire: bool,
    /// The cockpit point's change of velocity this step (m/s), ship space, Elite's axes (+X starboard, +Y up, +Z nose).
    pub velocity_change: [f32; 3],
}

/// A running cross-fade between two states.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrossFade {
    pub from: u32,
    pub to: u32,
    pub elapsed_s: f32,
    pub duration_s: f32,
}

/// A clip playing now (for tests and logs).
#[derive(Clone, Debug, PartialEq)]
pub struct Playing {
    pub clip: String,
    pub phase: f32,
    /// The edge weight it enters the tree with.
    pub weight: f32,
}

/// An AnimationActivity, resolved: its clip, the skeleton bone of each channel (None only for the clips' centre-screen
/// bones the chair lacks) and the bones it animates.
struct Anim {
    /// Its clip's index in `ChairClips::clips`.
    clip: usize,
    rot: Vec<Option<usize>>,
    pos: Vec<Option<usize>>,
    mask: Vec<bool>,
    looping: bool,
    additive_edge: bool,
    loop_event: bool,
    layer: u32,
    weight: ChairParam,
    rate: ChairParam,
    start_phase: f32,
    phase_variable: Option<String>,
    phase_offset: f32,
}

/// An activity, resolved; the numbers are indices into `Chair::anims` (clips) or `Chair::acts`.
enum Act {
    Anim(usize),
    Grouped(Vec<usize>),
    Triggered { trigger: String, child: usize },
    Button { variable: String, stages: [usize; 3] },
}

/// A clip instance (a leaf of the blend tree).
#[derive(Clone, Debug)]
struct Leaf {
    anim: usize,
    phase: f32,
    rate: f32,
    backwards: bool,
    weight: f32,
    /// Started this step: not advanced yet.
    fresh: bool,
    serial: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Idle,
    In,
    Loop,
    Out,
}

/// A running activity. Its depth is the graph's nesting, at most `CHAIR_MAX_NESTING` (checked), so walking it recursively
/// is bounded.
#[derive(Clone, Debug)]
enum Node {
    Anim(Leaf),
    Grouped(Vec<Node>),
    Triggered { trigger: String, child: usize, leaf: Option<Leaf> },
    Button { variable: String, stages: [usize; 3], stage: Stage, leaf: Option<Leaf> },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Fade {
    Steady,
    In,
    Out,
}

#[derive(Clone, Debug)]
struct Running {
    act: usize,
    node: Node,
    fade: Fade,
}

#[derive(Clone, Debug)]
struct Fading {
    from: u32,
    to: u32,
    keys: Vec<ChairKey>,
    elapsed: f32,
}

impl Fading {
    fn duration(&self) -> f32 {
        self.keys.last().map_or(0.0, |k| k.x)
    }
    fn c(&self) -> f32 {
        eval_keys(&self.keys, self.elapsed)
    }
}

/// The chair's motion graph, running.
pub struct Chair {
    skeleton: ChairSkeleton,
    clips: Arc<ChairClips>,
    graph: Arc<ChairGraph>,
    anims: Vec<Anim>,
    acts: Vec<Act>,
    act_index: HashMap<String, usize>,
    state_names: HashMap<u32, String>,
    rest: Vec<BonePose>,
    inverse_binds: Vec<DMat4>,
    // running state
    state: u32,
    loop_value: String,
    seated: bool,
    running: Vec<Running>,
    fading: Option<Fading>,
    layers: BTreeSet<u32>,
    vars: HashMap<String, f32>,
    events: HashSet<String>,
    throttle_taps: VecDeque<f32>,
    stick_taps: VecDeque<[f32; 2]>,
    yaw_taps: VecDeque<f32>,
    buffet_since: f32,
    buffet_last: f32,
    rng: u32,
    serial: u64,
    /// The blend tree's output: each bone's local pose.
    pose: Vec<BonePose>,
    world: Vec<Mat4>,
    skin: Vec<Mat4>,
}

fn graph_err(why: impl Into<String>) -> ChairError {
    ChairError::Graph(why.into())
}

/// A clip's channels' skeleton bones, matched by name and the clip's skeleton index. A channel whose bone the skeleton
/// lacks is an error, with one exception the notes document: the `_activatechair` and `_idles` sets were made for a
/// 144-bone chair with a centre-screen arm, and their two centre-screen channels (`in_chair` false) drive nothing in the
/// Sidewinder's 142-bone chair.
fn channel_bones(clip: &ChairClip, rotations: bool, bone_of: &HashMap<String, usize>) -> Result<Vec<Option<usize>>, ChairError> {
    let (kind, channels) = if rotations { ("rotation", &clip.rot_channels) } else { ("position", &clip.pos_channels) };
    channels
        .iter()
        .enumerate()
        .map(|(i, ch)| match bone_of.get(&ch.bone.to_lowercase()) {
            Some(&b) if ch.in_chair && b == ch.skeleton_index as usize => Ok(Some(b)),
            None if !ch.in_chair && CHAIR_LACKS.iter().any(|n| n.eq_ignore_ascii_case(&ch.bone)) => Ok(None),
            found => Err(ChairError::Clips(format!(
                "clip {}, {kind} channel {i}: bone {} (index {}, in the chair: {}) is {} in the skeleton",
                clip.name,
                ch.bone,
                ch.skeleton_index,
                ch.in_chair,
                found.map_or_else(|| "not".to_string(), |b| format!("bone {b}")),
            ))),
        })
        .collect()
}

/// An input the rules can use: a non-finite one is 0.
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

impl Chair {
    /// A chair at the graph's initial state (the template's `Uninitialised`: no clip, the rest pose). Refuses whatever
    /// the files' parsers refuse (`ChairClips::check`, `ChairGraph::check`), a skeleton out of order, a clip the graph
    /// names and `clips` lacks, a channel whose bone the skeleton lacks (`channel_bones`), a graph whose template values
    /// select no state, and the features this does not implement (activity flags other than loop, additive edge and
    /// LoopEvent, random start phases, phase output variables, triggered fades, minimum times or output variables, a
    /// trigger or button whose child is not a clip).
    pub fn new(skeleton: &ChairSkeleton, clips: Arc<ChairClips>, graph: Arc<ChairGraph>) -> Result<Chair, ChairError> {
        skeleton.check()?;
        clips.check()?;
        graph.check()?;
        let bone_of: HashMap<String, usize> = skeleton.bones.iter().enumerate().map(|(i, b)| (b.name.to_lowercase(), i)).collect();
        let act_index: HashMap<String, usize> = graph.activities.keys().enumerate().map(|(i, id)| (id.clone(), i)).collect();
        // the clips first, so a trigger or button can name its clip's entry
        let mut anims = Vec::new();
        let mut anim_of: HashMap<&str, usize> = HashMap::new();
        for (id, a) in &graph.activities {
            let ChairActivity::AnimationActivity { clip, flags, layer, weight, rate, start_phase, phase_variable, phase_offset, phase_output_variable } = a else {
                continue;
            };
            let at = |why: String| graph_err(format!("activity {id}: {why}"));
            let ci = clips.clips.iter().position(|c| c.name.eq_ignore_ascii_case(clip)).ok_or_else(|| at(format!("clip {clip} is not in chair/clips.json")))?;
            if flags & !SUPPORTED_FLAGS != 0 || *start_phase < 0.0 || phase_output_variable.as_deref().is_some_and(|v| !v.is_empty()) {
                return Err(at(format!("flags {flags:#x}, start phase {start_phase} or a phase output: not implemented")));
            }
            let c = &clips.clips[ci];
            let (rot, pos) = (channel_bones(c, true, &bone_of)?, channel_bones(c, false, &bone_of)?);
            let mut mask = vec![false; skeleton.bones.len()];
            rot.iter().chain(&pos).flatten().for_each(|&b| mask[b] = true);
            anim_of.insert(id.as_str(), anims.len());
            anims.push(Anim {
                clip: ci,
                rot,
                pos,
                mask,
                looping: flags & 1 != 0,
                additive_edge: flags & 2 != 0,
                loop_event: flags & 0x10 != 0,
                layer: *layer,
                weight: weight.clone(),
                rate: rate.clone(),
                start_phase: *start_phase,
                phase_variable: phase_variable.clone().filter(|v| !v.is_empty()),
                phase_offset: *phase_offset,
            });
        }
        let mut acts = Vec::new();
        for (id, a) in &graph.activities {
            let at = |why: String| graph_err(format!("activity {id}: {why}"));
            let act_of = |n: &String| act_index.get(n).copied().ok_or_else(|| at(format!("no activity {n}")));
            let clip_of = |n: &String| anim_of.get(n.as_str()).copied().ok_or_else(|| at(format!("activity {n} is not a clip (AnimationActivity)")));
            acts.push(match a {
                ChairActivity::AnimationActivity { .. } => Act::Anim(clip_of(id)?),
                ChairActivity::GroupedActivityActivity { children } => Act::Grouped(children.iter().map(act_of).collect::<Result<_, _>>()?),
                ChairActivity::TriggeredActivityActivity { trigger_variable, activity, fade_in_s, fade_out_s, hold_min_flag, min_time_s, output_variable } => {
                    if *fade_in_s != 0.0 || *fade_out_s != 0.0 || *hold_min_flag != 0 || *min_time_s != 0.0 || output_variable.as_deref().is_some_and(|v| !v.is_empty()) {
                        return Err(at("triggered fades, minimum times or an output variable: not implemented".into()));
                    }
                    Act::Triggered { trigger: trigger_variable.clone(), child: clip_of(activity)? }
                }
                ChairActivity::JoystickButtonActivity { button_variable, in_activity, loop_activity, out_activity } => {
                    Act::Button { variable: button_variable.clone(), stages: [clip_of(in_activity)?, clip_of(loop_activity)?, clip_of(out_activity)?] }
                }
            });
        }
        let t = &graph.templates;
        let initial = graph.output_state(&t.initial_loop).ok_or_else(|| graph_err(format!("the initial value {} selects no state", t.initial_loop)))?;
        for v in [&t.occupied_loop, &t.vacated_loop] {
            graph.output_state(v).ok_or_else(|| graph_err(format!("the loop value {v} selects no state")))?;
        }
        let state_names = graph
            .states
            .iter()
            .map(|s| (s.id, graph.root_outputs.iter().find(|o| o.state == s.id).map_or_else(|| format!("state {}", s.id), |o| o.enum_name.clone())))
            .collect();
        let vars = graph.variables.iter().filter(|v| v.kind == 1).map(|v| (v.name.clone(), v.default_i32 as f32)).collect();
        let loop_value = t.initial_loop.clone();
        let mut chair = Chair {
            rest: skeleton.rest(),
            inverse_binds: skeleton.bones.iter().map(|b| elite_matrix(&b.inverse_bind)).collect(),
            skeleton: skeleton.clone(),
            clips,
            graph,
            anims,
            acts,
            act_index,
            state_names,
            state: initial,
            loop_value,
            seated: false,
            running: Vec::new(),
            fading: None,
            layers: BTreeSet::new(),
            vars,
            events: HashSet::new(),
            throttle_taps: VecDeque::new(),
            stick_taps: VecDeque::new(),
            yaw_taps: VecDeque::new(),
            buffet_since: 0.0,
            buffet_last: 0.0,
            rng: 0x2545_f491,
            serial: 0,
            pose: Vec::new(),
            world: Vec::new(),
            skin: Vec::new(),
        };
        for a in chair.state_acts(initial) {
            let node = chair.start(a);
            chair.running.push(Running { act: a, node, fade: Fade::Steady });
        }
        chair.evaluate();
        Ok(chair)
    }

    /// One step of `dt` seconds (a negative or non-finite `dt` is 0; non-finite inputs are 0).
    pub fn step(&mut self, dt: f32, inputs: &ChairInputs) {
        let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        let inputs = ChairInputs {
            throttle: finite(inputs.throttle),
            roll: finite(inputs.roll),
            pitch: finite(inputs.pitch),
            yaw: finite(inputs.yaw),
            velocity_change: inputs.velocity_change.map(finite),
            ..*inputs
        };
        self.feed(dt, &inputs);
        let target = self.graph.output_state(&self.loop_value);
        let loop_event = self.loop_event(dt);
        if let Some(f) = &mut self.fading {
            f.elapsed += dt;
            if f.elapsed >= f.duration() {
                self.finish_fade();
            }
        }
        if let Some(target) = target.filter(|&t| self.fading.is_none() && t != self.state) {
            self.change_toward(target, loop_event);
        }
        let c = self.fading.as_ref().map_or(1.0, Fading::c);
        let mut running = std::mem::take(&mut self.running);
        for r in &mut running {
            let fade = match r.fade {
                Fade::Steady => 1.0,
                Fade::Out => (1.0 - c).clamp(0.0, 1.0),
                Fade::In => 1.0 - (1.0 - c).clamp(0.0, 1.0),
            };
            self.update(&mut r.node, fade, dt);
        }
        self.running = running;
        self.events.clear();
        self.evaluate();
    }

    /// Posed world matrices, one per skeleton bone, in the chair model's space (Elite's axes), column vectors.
    pub fn bone_matrices(&self) -> &[Mat4] {
        &self.world
    }

    /// Posed world x inverse bind per bone: a skinning palette.
    pub fn skin_matrices(&self) -> &[Mat4] {
        &self.skin
    }

    /// The current state's name (its loop enum value, e.g. `Mode_Ship`); during a cross-fade the state being entered.
    pub fn state(&self) -> &str {
        self.state_names.get(&self.state).map_or("", String::as_str)
    }

    pub fn crossfade(&self) -> Option<CrossFade> {
        self.fading.as_ref().map(|f| CrossFade { from: f.from, to: f.to, elapsed_s: f.elapsed, duration_s: f.duration() })
    }

    /// A graph variable's value (floats; events and enums are not kept as values).
    pub fn variable(&self, name: &str) -> Option<f32> {
        self.vars.get(name).copied()
    }

    /// The clips playing, in the order they started.
    pub fn playing(&self) -> Vec<Playing> {
        let mut leaves = Vec::new();
        self.running.iter().for_each(|r| collect(&r.node, &mut leaves));
        leaves.sort_by_key(|l| l.serial);
        leaves.into_iter().map(|l| Playing { clip: self.clip(&self.anims[l.anim]).name.clone(), phase: l.phase, weight: l.weight }).collect()
    }

    // -------------------------------------------------------------------------------------------- inputs

    fn set(&mut self, name: &str, v: f32) {
        self.vars.insert(name.to_string(), v);
    }

    fn feed(&mut self, dt: f32, inputs: &ChairInputs) {
        if inputs.seated != self.seated {
            self.seated = inputs.seated;
            let t = &self.graph.templates;
            self.loop_value = if self.seated { t.occupied_loop.clone() } else { t.vacated_loop.clone() };
        }
        let i = if self.seated { *inputs } else { ChairInputs::default() };
        let graph = Arc::clone(&self.graph);
        let t = &graph.templates;
        // FUN_142aa7190
        let v = (i.throttle + 1.0) * 0.5;
        let v = if 0.0 <= v { v.min(1.0) } else { 0.0 };
        self.throttle_taps.push_front(v);
        self.throttle_taps.truncate(THROTTLE_TAPS.len());
        let throttle = self.throttle_taps.iter().zip(THROTTLE_TAPS).fold(0.0f32, |s, (h, w)| s + h * w);
        self.set(&t.throttle_variable, throttle);
        // FUN_142aa6b40 (x = roll, y = pitch) and FUN_142abb750
        self.stick_taps.push_front([i.roll, i.pitch]);
        self.stick_taps.truncate(STICK_TAPS.len());
        let (sx, sy) = self.stick_taps.iter().zip(STICK_TAPS).fold((0.0f32, 0.0f32), |(sx, sy), (h, w)| (sx + w * h[0], sy + w * h[1]));
        let r = (sy * sy + sx * sx).sqrt();
        let mut prop = 0.0;
        if r > 0.0 {
            let a = (sy / r).acos();
            prop = if sx < 0.0 { TWO_PI - a } else { a } / TWO_PI;
        }
        self.set(&t.joystick_weight_variable, if r <= 1.0 { r } else { 1.0 });
        self.set(&t.joystick_prop_variable, prop);
        self.yaw_taps.push_front(i.yaw);
        self.yaw_taps.truncate(STICK_TAPS.len());
        let yaw = self.yaw_taps.iter().zip(STICK_TAPS).fold(0.0f32, |s, (h, w)| s + h * w);
        self.set(&t.joystick_yaw_variable, (yaw + 1.0) * 0.5);
        if self.seated {
            self.set(&t.trigger_variable, if i.fire { 1.0 } else { 0.0 });
        }
        self.buffet(dt, inputs.velocity_change);
    }

    /// FUN_143075ea0 and the collision component: the event of the velocity change's largest component.
    fn buffet(&mut self, dt: f32, dv: [f32; 3]) {
        self.buffet_since += dt;
        let graph = Arc::clone(&self.graph);
        let t = &graph.templates;
        let a = dv.map(f32::abs);
        let Some(axis) = (0..3).filter(|&k| a[k] > t.buffet_thresholds[k]).fold(None, |best: Option<usize>, k| match best {
            Some(b) if a[b] >= a[k] => Some(b),
            _ => Some(k),
        }) else {
            return;
        };
        let slot = match axis {
            0 => usize::from(dv[0] >= 0.0),
            1 => 4 + usize::from(dv[1] >= 0.0),
            _ => 2 + usize::from(dv[2] < 0.0),
        };
        let dv2 = dv[0] * dv[0] + dv[1] * dv[1] + dv[2] * dv[2];
        let intensity = (BUFFET_MASS_RATIO.max(BUFFET_MASS_FLOOR) * dv2 / (BUFFET_DV * BUFFET_DV)).clamp(0.0, 1.0);
        if !(intensity > self.buffet_last || self.buffet_since >= BUFFET_REFIRE_S) {
            return;
        }
        let Some(event) = t.buffet_events.get(slot) else { return };
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let speed = 1.0 + BUFFET_SPEED_SPREAD * ((self.rng >> 8) as f32 / 16_777_216.0);
        self.events.insert(event.clone());
        self.set(&t.buffet_intensity_variable, intensity * BUFFET_SEAT_SHARE);
        self.set(&t.buffet_speed_variable, speed);
        self.buffet_last = intensity;
        self.buffet_since = 0.0;
    }

    // -------------------------------------------------------------------------------------------- the state machine

    fn state_acts(&self, state: u32) -> Vec<usize> {
        self.graph.state(state).map_or_else(Vec::new, |s| s.activities.iter().filter_map(|a| self.act_index.get(a).copied()).collect())
    }

    /// The listed transition from `from` to `to`, if any: its block.
    fn listed(&self, from: u32, to: u32) -> Option<&str> {
        self.graph.state(from)?.transitions.iter().find(|t| t.targets.contains(&to)).map(|t| t.block.as_str())
    }

    /// The first hop of the cheapest route (Dijkstra over the blocks' path costs, 0xffff unusable).
    fn first_hop(&self, from: u32, to: u32) -> Option<u32> {
        let mut dist: HashMap<u32, u32> = HashMap::from([(from, 0)]);
        let mut first: HashMap<u32, u32> = HashMap::new();
        let mut open: BTreeSet<(u32, u32)> = BTreeSet::from([(0, from)]);
        while let Some((d, s)) = open.pop_first() {
            if s == to {
                return first.get(&to).copied();
            }
            for t in self.graph.state(s).map_or(&[][..], |s| &s.transitions[..]) {
                let cost = self.graph.transition_blocks.get(&t.block).map_or(0xffff, |b| b.path_cost);
                if cost == 0xffff {
                    continue;
                }
                for &n in &t.targets {
                    let nd = d + cost as u32;
                    let hop = if s == from { Some(n) } else { first.get(&s).copied() };
                    if let Some(h) = hop.filter(|_| dist.get(&n).map_or(true, |&o| nd < o)) {
                        dist.insert(n, nd);
                        first.insert(n, h);
                        open.insert((nd, n));
                    }
                }
            }
        }
        None
    }

    fn change_toward(&mut self, target: u32, loop_event: bool) {
        let (to, block_id) = match self.listed(self.state, target) {
            Some(b) => (target, b.to_string()),
            None => {
                let routes = self.graph.transition_blocks.get(&self.graph.default_block).is_some_and(|b| b.route_by_path_table != 0);
                match self.first_hop(self.state, target).filter(|_| routes) {
                    Some(hop) => match self.listed(self.state, hop) {
                        Some(b) => (hop, b.to_string()),
                        None => return,
                    },
                    None => (target, self.graph.default_block.clone()),
                }
            }
        };
        let Some(block) = self.graph.transition_blocks.get(&block_id).cloned() else { return };
        if block.wait_flag != 0 && !self.may_leave(&block, loop_event) {
            return;
        }
        let keys = fade_keys(&block);
        let from = self.state;
        let keep = self.state_acts(to);
        for r in &mut self.running {
            r.fade = if keep.contains(&r.act) { Fade::Steady } else { Fade::Out };
        }
        for a in keep {
            if !self.running.iter().any(|r| r.act == a) {
                let node = self.start(a);
                self.running.push(Running { act: a, node, fade: Fade::In });
            }
        }
        self.state = to;
        self.fading = Some(Fading { from, to, keys, elapsed: 0.0 });
        if self.fading.as_ref().is_some_and(|f| f.duration() <= 0.0) {
            self.finish_fade();
        }
    }

    /// A block's wait (0x140602788): a looping state leaves at its LoopEvent, a mortal one once complete or within the
    /// block's slack of completing.
    fn may_leave(&self, block: &ChairBlock, loop_event: bool) -> bool {
        let mut leaves = Vec::new();
        self.running.iter().filter(|r| r.fade != Fade::Out).for_each(|r| collect(&r.node, &mut leaves));
        let mortal: Vec<&Leaf> = leaves.into_iter().filter(|l| !self.anims[l.anim].looping).collect();
        if mortal.is_empty() {
            return loop_event;
        }
        let to_go = mortal
            .iter()
            .map(|l| {
                let d = self.clip(&self.anims[l.anim]).duration_s;
                if l.rate > 0.0 { (1.0 - l.phase) * d / l.rate } else { 0.0 }
            })
            .fold(0.0f32, f32::max);
        to_go <= 0.0 || to_go < block.wait_slack_s
    }

    fn finish_fade(&mut self) {
        self.running.retain(|r| r.fade != Fade::Out);
        self.running.iter_mut().for_each(|r| r.fade = Fade::Steady);
        self.fading = None;
    }

    /// LoopEvent (FUN_14060f450): a looping clip with flag bit 4 wraps during this step.
    fn loop_event(&self, dt: f32) -> bool {
        let mut leaves = Vec::new();
        self.running.iter().for_each(|r| collect(&r.node, &mut leaves));
        leaves.iter().any(|l| {
            let a = &self.anims[l.anim];
            a.looping && a.loop_event && a.phase_variable.is_none() && l.phase + dt * self.param(&a.rate) / self.clip(a).duration_s >= 1.0
        })
    }

    // -------------------------------------------------------------------------------------------- activities

    fn clip(&self, a: &Anim) -> &ChairClip {
        &self.clips.clips[a.clip]
    }

    fn param(&self, p: &ChairParam) -> f32 {
        match &p.variable {
            Some(v) => eval_keys(&p.keys, self.vars.get(v).copied().unwrap_or(0.0)),
            None => p.constant,
        }
    }

    /// The phase a phase-driven clip is told (FUN_1405f7a00): variable + offset, wrapped when it loops, else clamped.
    fn phase_target(&self, a: &Anim, var: &str) -> f32 {
        let x = self.vars.get(var).copied().unwrap_or(0.0) + a.phase_offset;
        if a.looping {
            (x as f64 % 1.0) as f32
        } else if 0.0 <= x {
            x.min(1.0)
        } else {
            0.0
        }
    }

    fn start(&mut self, act: usize) -> Node {
        let start = match self.acts.get(act) {
            Some(Act::Anim(a)) => Ok(*a),
            Some(Act::Grouped(children)) => Err(children.clone()),
            Some(Act::Triggered { trigger, child }) => return Node::Triggered { trigger: trigger.clone(), child: *child, leaf: None },
            Some(Act::Button { variable, stages }) => return Node::Button { variable: variable.clone(), stages: *stages, stage: Stage::Idle, leaf: None },
            None => return Node::Grouped(Vec::new()),
        };
        match start {
            Ok(anim) => Node::Anim(self.new_leaf(anim)),
            Err(children) => Node::Grouped(children.into_iter().map(|c| self.start(c)).collect()),
        }
    }

    fn new_leaf(&mut self, anim: usize) -> Leaf {
        let a = &self.anims[anim];
        let (layer, phase, rate) = (a.layer, a.phase_variable.as_deref().map_or(a.start_phase, |v| self.phase_target(a, v)), self.param(&a.rate));
        self.layers.insert(layer);
        self.serial += 1;
        Leaf { anim, phase, rate, backwards: false, weight: 0.0, fresh: true, serial: self.serial }
    }

    fn complete(&self, l: &Leaf) -> bool {
        !self.anims[l.anim].looping && if l.rate >= 0.0 { l.phase >= 1.0 } else { l.phase <= 0.0 }
    }

    fn update(&mut self, node: &mut Node, fade: f32, dt: f32) {
        match node {
            Node::Anim(l) => self.update_leaf(l, fade, dt),
            Node::Grouped(children) => children.iter_mut().for_each(|c| self.update(c, fade, dt)),
            Node::Triggered { trigger, child, leaf } => {
                if self.events.contains(trigger.as_str()) {
                    *leaf = Some(self.new_leaf(*child));
                } else if leaf.as_ref().is_some_and(|l| self.complete(l)) {
                    *leaf = None;
                }
                if let Some(l) = leaf {
                    self.update_leaf(l, fade, dt);
                }
            }
            Node::Button { variable, stages, stage, leaf } => {
                let button = self.vars.get(variable.as_str()).copied().unwrap_or(0.0);
                let done = leaf.as_ref().is_some_and(|l| self.complete(l));
                let next = match *stage {
                    Stage::Idle if button > 0.0 => Some(Stage::In),
                    Stage::In if done => Some(if button <= 0.0 { Stage::Out } else { Stage::Loop }),
                    Stage::Loop if button == 0.0 => Some(Stage::Out),
                    Stage::Out if done => Some(Stage::Idle),
                    _ => None,
                };
                if let Some(n) = next {
                    *stage = n;
                    *leaf = match n {
                        Stage::Idle => None,
                        Stage::In => Some(self.new_leaf(stages[0])),
                        Stage::Loop => Some(self.new_leaf(stages[1])),
                        Stage::Out => Some(self.new_leaf(stages[2])),
                    };
                }
                if let Some(l) = leaf {
                    self.update_leaf(l, fade, dt);
                }
            }
        }
    }

    /// FUN_1405f7a00: the edge weight clamp(W x fade, 0, 1); the phase from its variable or advanced at its rate (a
    /// phase that would not be finite stays where it was).
    fn update_leaf(&self, l: &mut Leaf, fade: f32, dt: f32) {
        let a = &self.anims[l.anim];
        l.weight = (self.param(&a.weight) * fade).clamp(0.0, 1.0);
        if let Some(var) = &a.phase_variable {
            let target = self.phase_target(a, var);
            if !l.fresh && dt > 0.0 {
                l.backwards = target < l.phase;
            }
            l.phase = target;
        } else {
            l.rate = self.param(&a.rate);
            l.backwards = l.rate < 0.0;
            let p = l.phase + dt * l.rate / self.clip(a).duration_s;
            if !l.fresh && p.is_finite() {
                l.phase = if a.looping { p.rem_euclid(1.0) } else { p.clamp(0.0, 1.0) };
            }
        }
        l.fresh = false;
    }

    // -------------------------------------------------------------------------------------------- the pose

    /// A leaf's pose over `base` (rest for a Leftover's leaf, identity for an additive edge or an additive clip).
    fn leaf_pose(&self, l: &Leaf, base: &[BonePose]) -> Vec<BonePose> {
        let a = &self.anims[l.anim];
        let mut pose = base.to_vec();
        let Some(s) = sample_clip(&self.clips, self.clip(a), l.phase, l.backwards) else { return pose };
        for (q, b) in s.rotations.iter().zip(&a.rot) {
            if let Some(p) = b.and_then(|b| pose.get_mut(b)) {
                p.q = *q;
            }
        }
        for (v, b) in s.positions.iter().zip(&a.pos) {
            if let Some(p) = b.and_then(|b| pose.get_mut(b)) {
                p.p = *v;
            }
        }
        pose
    }

    /// The blend tree of MG 3.1, then the matrices.
    fn evaluate(&mut self) {
        let mut leaves = Vec::new();
        self.running.iter().for_each(|r| collect(&r.node, &mut leaves));
        leaves.sort_by_key(|l| l.serial);
        let identity = vec![IDENTITY; self.rest.len()];
        let mut below: Option<Vec<BonePose>> = None;
        let mut below_mask = vec![false; self.rest.len()];
        for &layer in &self.layers {
            let mine: Vec<&Leaf> = leaves.iter().copied().filter(|l| self.anims[l.anim].layer == layer).collect();
            let (adds, overs): (Vec<&Leaf>, Vec<&Leaf>) = mine.iter().copied().partition(|l| self.anims[l.anim].additive_edge);
            let over_poses: Vec<Vec<BonePose>> = overs
                .iter()
                .map(|l| {
                    let a = &self.anims[l.anim];
                    self.leaf_pose(l, if self.clip(a).additive { &identity } else { &self.rest })
                })
                .collect();
            let mut children: Vec<BlendChild> = overs
                .iter()
                .zip(&over_poses)
                .map(|(l, pose)| BlendChild { pose, weight: l.weight, active: l.weight > T, mask: Some(&self.anims[l.anim].mask), priority: 1 })
                .collect();
            if let Some(b) = &below {
                children.push(BlendChild { pose: b, weight: 1.0, active: true, mask: Some(&below_mask), priority: 0 });
            }
            let left = if children.is_empty() { self.rest.clone() } else { leftover_node(&self.rest, &children) };
            let add_poses: Vec<Vec<BonePose>> = adds.iter().map(|l| self.leaf_pose(l, &identity)).collect();
            let add_children: Vec<BlendChild> = adds
                .iter()
                .zip(&add_poses)
                .map(|(l, pose)| BlendChild { pose, weight: l.weight, active: l.weight > T, mask: None, priority: 0 })
                .collect();
            let out = additive_node(&left, &add_children);
            for l in &mine {
                for (m, &on) in below_mask.iter_mut().zip(&self.anims[l.anim].mask) {
                    *m |= on;
                }
            }
            below = Some(out);
        }
        self.pose = below.unwrap_or_else(|| self.rest.clone());
        let world = self.skeleton.world(&self.pose);
        self.skin = world.iter().zip(&self.inverse_binds).map(|(w, ib)| (*w * *ib).as_mat4()).collect();
        self.world = world.iter().map(DMat4::as_mat4).collect();
    }
}

/// A block's fade as keys (FUN_1405fbd00): its curve, else linear 0 -> 1 over its time (0.2 s when negative), a single
/// key of 1 for a cut.
fn fade_keys(block: &ChairBlock) -> Vec<ChairKey> {
    if let Some(c) = &block.curve {
        return c.clone();
    }
    let d = if block.fade_time_s < 0.0 { NEGATIVE_FADE_S } else { block.fade_time_s };
    if d > 0.0 {
        vec![ChairKey { x: 0.0, y: 0.0, interp: 1, a: 0.0, b: 0.0 }, ChairKey { x: d, y: 1.0, interp: 1, a: 0.0, b: 0.0 }]
    } else {
        vec![ChairKey { x: 0.0, y: 1.0, interp: 1, a: 0.0, b: 0.0 }]
    }
}

fn collect<'a>(node: &'a Node, out: &mut Vec<&'a Leaf>) {
    match node {
        Node::Anim(l) => out.push(l),
        Node::Grouped(children) => children.iter().for_each(|c| collect(c, out)),
        Node::Triggered { leaf, .. } | Node::Button { leaf, .. } => out.extend(leaf.iter()),
    }
}

#[cfg(test)]
mod tests {
    //! Against `tests/data/elite/chair/` (made by `re/elite/tools/make_chair_vectors.py` from the decoded export: the
    //! shipped clips and graph, the skeleton, and manipose.py / blend_ref.py's outputs).
    use super::*;
    use crate::elite_ship::{parse_chair_clips, parse_chair_graph, CHAIR_MAX_NESTING};
    use serde_json::Value;

    fn data(name: &str) -> Vec<u8> {
        let path = format!("{}/tests/data/elite/chair/{name}", env!("CARGO_MANIFEST_DIR"));
        let b = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        miniz_oxide::inflate::decompress_to_vec_zlib(&b).expect(name)
    }

    fn json(name: &str) -> Value {
        serde_json::from_slice(&data(name)).expect(name)
    }

    fn clips() -> ChairClips {
        parse_chair_clips(&data("clips.json.z"), &data("clips.bin.z")).unwrap()
    }

    fn graph() -> ChairGraph {
        parse_chair_graph(&data("graph.json.z")).unwrap()
    }

    fn floats<const N: usize>(v: &Value) -> [f64; N] {
        let a = v.as_array().expect("a list of numbers");
        assert_eq!(a.len(), N);
        std::array::from_fn(|i| a[i].as_f64().unwrap())
    }

    fn skeleton() -> ChairSkeleton {
        let v = json("skeleton.json.z");
        let bones = v["bones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| ChairBone {
                name: b["name"].as_str().unwrap().to_string(),
                parent: b["parent"].as_i64().unwrap() as i16,
                translation: floats::<3>(&b["local_position"]).map(|x| x as f32),
                rotation: floats::<4>(&b["local_rotation"]).map(|x| x as f32),
                scale: b["local_scale"].as_f64().unwrap() as f32,
                inverse_bind: floats::<16>(&b["inverse_bind"]).map(|x| x as f32),
            })
            .collect();
        ChairSkeleton { bones }
    }

    fn chair() -> Chair {
        Chair::new(&skeleton(), Arc::new(clips()), Arc::new(graph())).unwrap()
    }

    fn pose_of(v: &Value) -> Vec<BonePose> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|b| {
                let x = floats::<7>(b).map(|x| x as f32);
                BonePose { q: [x[0], x[1], x[2], x[3]], p: [x[4], x[5], x[6]] }
            })
            .collect()
    }

    fn bits(p: &[BonePose]) -> Vec<u32> {
        p.iter().flat_map(|b| b.q.iter().chain(&b.p).map(|x| x.to_bits())).collect()
    }

    #[test]
    fn clips_match_manipose() {
        let (c, v) = (clips(), json("vectors.json.z"));
        let (mut worst, mut n) = (0.0f64, 0);
        for s in v["samples"].as_array().unwrap() {
            let name = s["clip"].as_str().unwrap();
            let clip = c.clip(name).unwrap();
            let (phase, reverse) = (s["phase"].as_f64().unwrap() as f32, s["reverse"].as_bool().unwrap());
            let at = format!("{name} at {phase} (backwards {reverse})");
            let (a, b, t) = frames_at(phase, clip.frames, reverse);
            // (the fraction is a float32: serde_json's reading of a float64 may be an ulp off, its float32 is exact)
            assert_eq!((a, b, t), (s["frames"][0].as_u64().unwrap() as usize, s["frames"][1].as_u64().unwrap() as usize, s["frac"].as_f64().unwrap() as f32), "{at}");
            let got = sample_clip(&c, clip, phase, reverse).unwrap();
            for (q, r) in got.rotations.iter().zip(s["rot"].as_array().unwrap()) {
                let r = floats::<4>(r);
                worst = (0..4).map(|i| (q[i] as f64 - r[i]).abs()).fold(worst, f64::max);
            }
            for (p, r) in got.positions.iter().zip(s["pos"].as_array().unwrap()) {
                assert_eq!(p.map(f32::to_bits), floats::<3>(r).map(|x| (x as f32).to_bits()), "{at}: a position");
            }
            assert_eq!((got.rotations.len(), got.positions.len()), (s["rot"].as_array().unwrap().len(), s["pos"].as_array().unwrap().len()));
            n += 1;
        }
        println!("{n} samples: positions bit-exact, rotations within {worst:e}");
        assert!(n >= 13 * 18 && worst <= 2e-7, "{worst}");
    }

    #[test]
    fn blend_nodes_match_blend_ref() {
        let v = json("vectors.json.z");
        let mut n = 0;
        for kind in ["normal", "additive", "leftover"] {
            for (k, case) in v["blend"][kind].as_array().unwrap().iter().enumerate() {
                let input = pose_of(&case["input"]);
                let kids: Vec<(Vec<BonePose>, f32, bool, Option<Vec<bool>>, u16)> = case["children"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| {
                        let mask = c["mask"].as_array().map(|m| {
                            let mut on = vec![false; input.len()];
                            m.iter().for_each(|b| on[b.as_u64().unwrap() as usize] = true);
                            on
                        });
                        (pose_of(&c["pose"]), c["w"].as_f64().unwrap() as f32, c["active"].as_bool().unwrap(), mask, c["prio"].as_u64().unwrap() as u16)
                    })
                    .collect();
                let children: Vec<BlendChild> =
                    kids.iter().map(|(pose, weight, active, mask, priority)| BlendChild { pose, weight: *weight, active: *active, mask: mask.as_deref(), priority: *priority }).collect();
                let out = match kind {
                    "normal" => normal_node(&input, &children),
                    "additive" => additive_node(&input, &children),
                    _ => leftover_node(&input, &children),
                };
                assert_eq!(bits(&out), bits(&pose_of(&case["out"])), "{kind} case {k}");
                n += 1;
            }
        }
        assert_eq!(n, 72);
    }

    /// The rest pose with one clip's channels (manipose.pose).
    fn clip_pose(s: &ChairSkeleton, c: &ChairClips, name: &str, phase: f32) -> Vec<BonePose> {
        let clip = c.clip(name).unwrap();
        let sample = sample_clip(c, clip, phase, false).unwrap();
        let bone = |n: &str| s.bones.iter().position(|b| b.name.eq_ignore_ascii_case(n));
        let mut pose = s.rest();
        for (ch, q) in clip.rot_channels.iter().zip(&sample.rotations) {
            if let Some(b) = bone(&ch.bone) {
                pose[b].q = *q;
            }
        }
        for (ch, p) in clip.pos_channels.iter().zip(&sample.positions) {
            if let Some(b) = bone(&ch.bone) {
                pose[b].p = *p;
            }
        }
        pose
    }

    fn worst(world: &[DMat4], reference: &[f64]) -> f64 {
        world.iter().flat_map(|m| m.to_cols_array()).zip(reference).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max)
    }

    #[test]
    fn world_matrices_match_manipose() {
        let (s, c, v) = (skeleton(), clips(), json("vectors.json.z"));
        let mut most = 0.0f64;
        for w in v["world"].as_array().unwrap() {
            let name = w["clip"].as_str().unwrap();
            let world = s.world(&clip_pose(&s, &c, name, w["phase"].as_f64().unwrap() as f32));
            let reference: Vec<f64> = w["m"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
            assert_eq!(reference.len(), 16 * s.bones.len());
            most = most.max(worst(&world, &reference));
        }
        let rest: Vec<f64> = v["rest"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let rest_err = worst(&s.world(&s.rest()), &rest);
        println!("world matrices within {most:e} of manipose's (rest pose {rest_err:e})");
        assert!(most < 1e-5 && rest_err < 1e-12, "{most} {rest_err}");
    }

    #[test]
    fn rest_pose_rebuilds_the_stored_binds() {
        let c = chair();
        let mut most = 0.0f32;
        for m in c.skin_matrices() {
            most = m.to_cols_array().iter().zip(Mat4::IDENTITY.to_cols_array()).map(|(a, b)| (a - b).abs()).fold(most, f32::max);
        }
        assert_eq!(c.state(), "Uninitialised");
        assert!(c.playing().is_empty());
        assert!(most < 1e-5, "{most}");
    }

    const DT: f32 = 0.125;

    fn seated() -> ChairInputs {
        ChairInputs { seated: true, ..ChairInputs::default() }
    }

    /// A chair the pilot sat in, settled in Mode_Ship with every smoothing history full.
    fn seated_chair() -> Chair {
        let mut c = chair();
        for _ in 0..20 {
            c.step(DT, &seated());
        }
        assert_eq!((c.state(), c.crossfade()), ("Mode_Ship", None));
        c
    }

    fn playing(c: &Chair, clip: &str) -> Option<Playing> {
        c.playing().into_iter().find(|p| p.clip.eq_ignore_ascii_case(clip))
    }

    #[test]
    fn sitting_down_and_getting_up() {
        let g = graph();
        assert_eq!(g.states.iter().map(|s| s.id).collect::<Vec<_>>(), [1, 8, 40]);
        let mut c = chair();
        c.step(DT, &seated());
        // Uninitialised lists no transition to Mode_Ship: the cheapest route goes through Empty_Ship, 0.5 s each.
        assert_eq!(c.state(), "Empty_Ship");
        assert_eq!(c.crossfade().map(|f| (f.from, f.to, f.duration_s)), Some((40, 1, 0.5)));
        for _ in 0..4 {
            c.step(DT, &seated());
        }
        assert_eq!(c.state(), "Mode_Ship");
        assert_eq!(c.crossfade().map(|f| (f.from, f.to, f.duration_s, f.elapsed_s)), Some((1, 8, 0.5, 0.0)));
        // half way: the incoming stick clip at half weight (JoystickWeight 0: its weight is 0 anyway), the throttle at 0.5
        c.step(DT, &seated());
        c.step(DT, &seated());
        assert_eq!(playing(&c, "chairmale_leftstickmove").map(|p| p.weight), Some(0.5));
        c.step(DT, &seated());
        c.step(DT, &seated());
        assert_eq!((c.state(), c.crossfade()), ("Mode_Ship", None));
        assert_eq!(playing(&c, "chairmale_leftstickmove").map(|p| p.weight), Some(1.0));
        assert_eq!(playing(&c, "chairmale_baseloop").map(|p| p.weight), Some(1.0));
        // getting up: Empty_Ship over 0.5 s, the pilot's clips fading out
        c.step(DT, &ChairInputs::default());
        assert_eq!(c.state(), "Empty_Ship");
        assert_eq!(c.crossfade().map(|f| (f.from, f.to, f.duration_s)), Some((8, 1, 0.5)));
        for _ in 0..4 {
            c.step(DT, &ChairInputs::default());
        }
        assert_eq!((c.state(), c.crossfade()), ("Empty_Ship", None));
        let left: Vec<String> = c.playing().into_iter().map(|p| p.clip).collect();
        assert_eq!(left, ["chairmale_baseloop"]);
        // sitting down again: one 0.5 s cross-fade
        c.step(DT, &seated());
        assert_eq!(c.crossfade().map(|f| (f.from, f.to, f.duration_s)), Some((1, 8, 0.5)));
    }

    #[test]
    fn throttle_drives_the_throttle_clip() {
        let mut c = seated_chair();
        let name = c.graph.templates.throttle_variable.clone();
        for (throttle, phase) in [(0.5f32, 0.75f32), (-1.0, 0.0), (1.0, 1.0), (0.0, 0.5)] {
            for _ in 0..10 {
                c.step(DT, &ChairInputs { throttle, ..seated() });
            }
            let v = c.variable(&name).unwrap();
            assert!((v - phase).abs() < 1e-6, "throttle {throttle}: {v}");
            assert_eq!(playing(&c, "chairmale_leftstickmove").unwrap().phase, v);
        }
        // the 10-tap average, newest first: a step from 0.5 to 1 climbs by the taps' weights
        c.step(DT, &ChairInputs { throttle: 1.0, ..seated() });
        assert!((c.variable(&name).unwrap() - (0.5 + 0.5 * 0.2)).abs() < 1e-6);
    }

    #[test]
    fn stick_smoothing_step_response() {
        let mut c = seated_chair();
        let name = c.graph.templates.joystick_weight_variable.clone();
        // full pitch from rest: each step adds the next tap, newest first
        for want in [0.3f32, 0.6, 0.9, 0.95, 0.975, 0.9875, 1.0] {
            c.step(DT, &ChairInputs { pitch: 1.0, ..seated() });
            let got = c.variable(&name).unwrap();
            assert!((got - want).abs() < 1e-6, "{got} for {want}");
        }
    }

    #[test]
    fn full_stick_poses_the_stick_clip_round_its_four_directions() {
        // Elite's Pitch +1 is the stick forward (phase 0), Roll +1 to the left (0.25), Pitch -1 back (0.5), Roll -1 right (0.75)
        let stick = skeleton().bones.iter().position(|b| b.name == "BN_JoyStick_joint").unwrap();
        for (roll, pitch, phase) in [(0.0f32, 1.0f32, 0.0f32), (1.0, 0.0, 0.25), (0.0, -1.0, 0.5), (-1.0, 0.0, 0.75)] {
            let mut c = seated_chair();
            for _ in 0..8 {
                c.step(DT, &ChairInputs { roll, pitch, ..seated() });
            }
            let t = &c.graph.templates;
            let (w, prop) = (c.variable(&t.joystick_weight_variable).unwrap(), c.variable(&t.joystick_prop_variable).unwrap());
            assert!((w - 1.0).abs() < 1e-6 && (prop - phase).abs() < 1e-6, "roll {roll} pitch {pitch}: weight {w}, prop {prop}");
            let p = playing(&c, "chairmale_rightstickmove").unwrap();
            assert_eq!((p.phase, p.weight), (prop, w));
            // the stick bone: the base loop's pose with the stick clip's delta at that phase, weight 1
            let clips = clips();
            let base = sample_clip(&clips, clips.clip("chairmale_baseloop").unwrap(), playing(&c, "chairmale_baseloop").unwrap().phase, false).unwrap();
            let base_ch = clips.clip("chairmale_baseloop").unwrap().rot_channels.iter().position(|ch| ch.bone == "BN_JoyStick_joint").unwrap();
            let delta = sample_clip(&clips, clips.clip("chairmale_rightstickmove").unwrap(), phase, false).unwrap();
            let want = qmul(base.rotations[base_ch], scale_from_identity(delta.rotations[0], w));
            let got = c.pose[stick].q;
            assert!((0..4).all(|i| (got[i] - want[i]).abs() < 1e-6), "phase {phase}: {got:?} vs {want:?}");
        }
    }

    #[test]
    fn trigger_plays_in_loop_out() {
        let mut c = seated_chair();
        let mut seen: Vec<String> = Vec::new();
        let mut note = |c: &Chair| {
            for p in c.playing() {
                if p.clip.contains("trigger") && seen.last() != Some(&p.clip) {
                    seen.push(p.clip);
                }
            }
        };
        for _ in 0..30 {
            c.step(1.0 / 60.0, &ChairInputs { fire: true, ..seated() });
            note(&c);
        }
        assert!(playing(&c, "chairmale_righttriggerupper_loop").is_some());
        for _ in 0..40 {
            c.step(1.0 / 60.0, &seated());
            note(&c);
        }
        assert_eq!(seen, ["chairmale_righttriggerupper_in", "chairmale_righttriggerupper_loop", "chairmale_righttriggerupper_out"]);
        assert!(c.playing().iter().all(|p| !p.clip.contains("trigger")), "the Out clip ends and leaves");
    }

    #[test]
    fn buffets_by_axis_and_sign() {
        for (dv, clip) in [
            ([-100.0f32, 0.0, 0.0], "chairmale_buffetleftsmall"),
            ([100.0, 0.0, 0.0], "chairmale_buffetrightsmall"),
            ([0.0, -100.0, 0.0], "chair_buffetabovesmall"),
            ([0.0, 100.0, 0.0], "chair_buffetundersmall"),
            ([0.0, 0.0, 100.0], "chair_buffetbacksmall"),
            ([0.0, 0.0, -100.0], "chair_buffetfwdsmall"),
        ] {
            let mut c = seated_chair();
            c.step(DT, &ChairInputs { velocity_change: dv, ..seated() });
            let buffets: Vec<Playing> = c.playing().into_iter().filter(|p| p.clip.contains("buffet")).collect();
            // intensity clamp(5 x 100^2 / 200^2) = 1, x 0.3 for the pilot's seat
            assert_eq!(buffets.len(), 1, "{dv:?}");
            assert_eq!((buffets[0].clip.as_str(), buffets[0].phase, buffets[0].weight), (clip, 0.0, 0.3), "{dv:?}");
            let speed = c.variable(&c.graph.templates.buffet_speed_variable).unwrap();
            assert!((1.0..1.25).contains(&speed));
            // the clip runs to its end and leaves
            for _ in 0..40 {
                c.step(DT, &seated());
            }
            assert!(c.playing().iter().all(|p| !p.clip.contains("buffet")), "{dv:?}");
        }
        // under the 1e-5 threshold nothing fires; ties go to x; a weaker one within 0.5 s doesn't fire
        let mut c = seated_chair();
        c.step(DT, &ChairInputs { velocity_change: [0.0, 9e-6, 0.0], ..seated() });
        assert!(c.playing().iter().all(|p| !p.clip.contains("buffet")));
        c.step(DT, &ChairInputs { velocity_change: [50.0, 50.0, -50.0], ..seated() });
        assert!(playing(&c, "chairmale_buffetrightsmall").is_some());
        c.step(DT, &ChairInputs { velocity_change: [0.0, 10.0, 0.0], ..seated() });
        assert!(playing(&c, "chair_buffetundersmall").is_none());
    }

    #[test]
    fn deterministic() {
        let run = || {
            let mut c = chair();
            let mut out = Vec::new();
            for k in 0..200 {
                let x = (k as f32 * 0.37).sin();
                let i = ChairInputs {
                    seated: (20..150).contains(&k),
                    throttle: x,
                    roll: (k as f32 * 0.11).cos(),
                    pitch: x * 0.5,
                    yaw: -x,
                    fire: k % 37 < 10,
                    velocity_change: [x * 30.0, 0.0, if k % 23 == 0 { -80.0 } else { 0.0 }],
                };
                c.step(1.0 / 60.0, &i);
                out.extend(c.bone_matrices().iter().flat_map(|m| m.to_cols_array().map(f32::to_bits)));
            }
            out
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn refusals() {
        let (s, c, g) = (skeleton(), clips(), graph());
        let mut g2 = g.clone();
        if let Some(ChairActivity::AnimationActivity { clip, .. }) = g2.activities.values_mut().find(|a| matches!(a, ChairActivity::AnimationActivity { .. })) {
            *clip = "ChairMale_NoSuchClip".into();
        }
        assert!(matches!(Chair::new(&s, Arc::new(c.clone()), Arc::new(g2)), Err(ChairError::Graph(e)) if e.contains("ChairMale_NoSuchClip")));
        let mut g3 = g.clone();
        if let Some(ChairActivity::AnimationActivity { flags, .. }) = g3.activities.values_mut().find(|a| matches!(a, ChairActivity::AnimationActivity { .. })) {
            *flags |= 4;
        }
        assert!(matches!(Chair::new(&s, Arc::new(c.clone()), Arc::new(g3)), Err(ChairError::Graph(e)) if e.contains("not implemented")));
        let mut g4 = g.clone();
        g4.templates.initial_loop = "Mode_Buggy".into();
        assert!(matches!(Chair::new(&s, Arc::new(c.clone()), Arc::new(g4)), Err(ChairError::Graph(e)) if e.contains("Mode_Buggy")));
        let mut s2 = s.clone();
        s2.bones[3].parent = 7;
        assert!(matches!(Chair::new(&s2, Arc::new(c), Arc::new(g)), Err(ChairError::Skeleton(_))));
        assert!(ChairSkeleton::from_edm(&Edm {
            metres_per_unit: 1.0,
            vertex_decode: crate::elite_asset::VertexDecode::Elite,
            parts: vec![crate::elite_asset::EdmPart { name: "root".into(), parent: -1, translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.0, inverse_bind: None }],
            materials: vec![],
            draws: vec![],
            animations: vec![],
        })
        .is_err());
    }

    #[test]
    fn damaged_data_is_refused_not_a_crash() {
        let (s, c, g) = (skeleton(), clips(), graph());
        let new = |s: &ChairSkeleton, c: ChairClips, g: ChairGraph| Chair::new(s, Arc::new(c), Arc::new(g)).err().map(|e| e.to_string()).unwrap_or_default();
        // a key offset whose end overflows, in clips that never went through the parser
        let mut c2 = c.clone();
        c2.clips[0].rot_keys_at = u64::MAX - 7;
        let e = new(&s, c2, g.clone());
        assert!(e.starts_with("chair/clips.json: clip ") && e.contains("rotation keys") && e.contains("overflow"), "{e}");
        // a group that starts itself, directly and through another group
        let gid = g.activities.iter().find(|(_, a)| matches!(a, ChairActivity::GroupedActivityActivity { .. })).map(|(k, _)| k.clone()).unwrap();
        let push = |g: &mut ChairGraph, child: &str| {
            if let Some(ChairActivity::GroupedActivityActivity { children }) = g.activities.get_mut(&gid) {
                children.push(child.to_string());
            }
        };
        let mut g2 = g.clone();
        push(&mut g2, &gid);
        let e = new(&s, c.clone(), g2);
        assert!(e.starts_with("chair/graph.json: state ") && e.contains(&format!("activity {gid} starts itself")), "{e}");
        let mut g3 = g.clone();
        g3.activities.insert("9:1".into(), ChairActivity::GroupedActivityActivity { children: vec![gid.clone()] });
        push(&mut g3, "9:1");
        assert!(new(&s, c.clone(), g3).contains("starts itself"));
        // activities nested deeper than the bound
        let mut g4 = g.clone();
        for k in 0..CHAIR_MAX_NESTING {
            let next = if k + 1 == CHAIR_MAX_NESTING { gid.clone() } else { format!("9:{}", k + 1) };
            g4.activities.insert(format!("9:{k}"), ChairActivity::GroupedActivityActivity { children: vec![next] });
        }
        g4.states[0].activities.push("9:0".into());
        assert!(new(&s, c.clone(), g4).contains("deeper than"));
        // a channel whose bone the skeleton lacks: renamed, or a skeleton of three bones
        let mut s2 = s.clone();
        s2.bones.iter_mut().filter(|b| b.name == "BN_JoyStick_joint").for_each(|b| b.name = "BN_Renamed_joint".into());
        let e = new(&s2, c.clone(), g.clone());
        assert!(e.starts_with("chair/clips.json: clip ") && e.contains("bone BN_JoyStick_joint") && e.contains("is not in the skeleton"), "{e}");
        let mut s3 = s.clone();
        s3.bones.truncate(3);
        assert!(new(&s3, c.clone(), g.clone()).starts_with("chair/clips.json: clip "));
        // a graph made without parsing, its default block missing
        let mut g5 = g.clone();
        g5.default_block = "0:0".into();
        assert!(new(&s, c.clone(), g5).starts_with("chair/graph.json: no default block"));
        // a flag the rules don't implement (bit 5, re-evaluate)
        let mut g6 = g.clone();
        if let Some(ChairActivity::AnimationActivity { flags, .. }) = g6.activities.values_mut().find(|a| matches!(a, ChairActivity::AnimationActivity { .. })) {
            *flags |= 0x20;
        }
        assert!(new(&s, c, g6).contains("not implemented"));
    }

    #[test]
    fn non_finite_time_and_inputs_leave_the_pose_finite() {
        let mut c = seated_chair();
        let (nan, inf) = (f32::NAN, f32::INFINITY);
        let wild = ChairInputs { throttle: nan, roll: nan, pitch: inf, yaw: nan, fire: true, velocity_change: [nan, inf, -inf], ..seated() };
        let big = ChairInputs { roll: inf, yaw: -inf, velocity_change: [1e30, 0.0, 0.0], ..seated() };
        for (dt, i) in [(inf, seated()), (nan, seated()), (-1.0, seated()), (1e30, seated()), (DT, wild), (DT, big), (-inf, wild)] {
            for _ in 0..10 {
                c.step(dt, &i);
                assert!(c.bone_matrices().iter().chain(c.skin_matrices()).all(|m| m.is_finite()), "dt {dt}, {i:?}");
                assert!(c.vars.values().all(|v| v.is_finite()), "dt {dt}, {i:?}: {:?}", c.vars);
            }
        }
        // a NaN stick is a centred one, not a full deflection
        let t = &c.graph.templates;
        let w = c.variable(&t.joystick_weight_variable).unwrap();
        assert!(w <= 1.0);
        for _ in 0..8 {
            c.step(DT, &ChairInputs { roll: nan, pitch: nan, ..seated() });
        }
        assert_eq!(c.variable(&c.graph.templates.joystick_weight_variable), Some(0.0));
    }

    #[test]
    fn curves_as_the_exe_evaluates_them() {
        let k = |x: f32, y: f32, interp: u8, a: f32, b: f32| ChairKey { x, y, interp, a, b };
        // the file's hermite (a = b = 1 over a unit span) is linear; the transition's 0.2 s hermite starts slow
        let lin = [k(0.0, 0.0, 5, 1.0, 1.0), k(1.0, 1.0, 1, 0.0, 0.0)];
        assert_eq!([eval_keys(&lin, -1.0), eval_keys(&lin, 0.25), eval_keys(&lin, 2.0)], [0.0, 0.25, 1.0]);
        let fade = [k(0.0, 0.0, 5, 0.15625, 0.046875), k(0.2, 1.0, 1, 0.0, 0.0)];
        let mid = eval_keys(&fade, 0.1);
        assert!(mid > 0.0 && mid < 1.0 && (eval_keys(&fade, 0.2) - 1.0).abs() < 1e-6);
        assert_eq!(eval_keys(&[k(0.0, 2.0, 0, 0.0, 0.0), k(1.0, 5.0, 1, 0.0, 0.0)], 0.5), 2.0);
        assert_eq!(eval_keys(&[k(0.0, 0.0, 4, 0.0, 0.0), k(1.0, 1.0, 1, 0.0, 0.0)], 0.5), 0.5);
        // a negative fade is 0.2 s, zero a cut
        let block = |fade_time_s: f32| ChairBlock { at: String::new(), fade_time_s, curve: None, wait_flag: 0, wait_slack_s: 0.0, route_by_path_table: 0, path_cost: 100 };
        assert_eq!(fade_keys(&block(-1.0)).last().unwrap().x, 0.2);
        assert_eq!(fade_keys(&block(0.0)).len(), 1);
    }
}
