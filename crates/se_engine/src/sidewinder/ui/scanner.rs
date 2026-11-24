//! Elite's scanner (the radar disc between the pilot's knees) drawn natively (ARCHITECTURE.md A10 "The 3D parts (UI6)";
//! `re/elite/notes/ui_hud3d.md` section 1): Elite builds its geometry in code and draws it with its own shaders, so does this.
//!
//! - **What is drawn** (the brief's scope): the disc (`RadarPlane`: the rings and the V wedge of its texture through the colour
//!   matrix, the pulse sweeping over it), the own-ship marker (`RadarPlayer`: three nested triangles shrinking with the range)
//!   and the range indicator (`RadarRangeIndicator`: a tick on a 0.705 rad arc). No contacts: the mod has no other ships, and
//!   in supercruise the Navigation rows (`panels::NavRow`) give each body's distance only, not its place, so Elite's
//!   supercruise icons are left out (no position mapping is built for them).
//! - **Data** (`ui/hud3d/radar.json`, `RadarFile`): the meshes as the exe's builders make them, the materials' values, the range
//!   table. The shader (`sidewinder_ui_radar.wesl`) is the disassembly's formulas; its header says what is Elite's and what is
//!   a stand-in. The stand-ins are the constants of `Stand-ins` below, each with its grade.
//! - **Range**: the Sidewinder's table (500 .. 8000 m, starting at index 0: the data's default, A) stepped by Elite's
//!   `RadarIncreaseRange` / `RadarDecreaseRange` (clamped at both ends, silent there), the value moving linearly over 0.5 s
//!   (`RANGE_RATE`, A; measured 0.47 s, C). The keys come through `RadarKeys`, which the seat's key reader sets (the hook is
//!   the seat's: `RadarKeys` is only read and cleared here). The zoom sounds are posted as the exe's radar update posts them
//!   (`re/elite/notes/ui_sounds.md` section 4: a step in `Indicator_b` and `RadiusIncrease` / `RadiusDecrease`, the end of the
//!   slide `Indicator_e`, B for the begin / end pairing). `RadarPulse` is not posted: what raises it is not traced.
//! - **When and where**: made once the UI is made, placed and shown only while seated with the ship powered (unpowered Elite
//!   removes the whole 3D scanner, notes 7.2, B), on the frames the UI's pass draws (`UiFrame::order`), by the scanner's own
//!   pass in the UI's place in the frame (`pass.rs`'s seam): after the panels' pass and before the holograms', at the view's
//!   full size, display-referred, its last step the panels' `screen_fit`. The view's group it shares with the holograms'
//!   pass is `PartsView`. With `SE_SW_UI=0` nothing of this runs.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::math::Vec4;
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_depth_2d, texture_depth_2d_multisampled, uniform_buffer, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::tasks::{futures, AsyncComputeTaskPool, Task};
use bytemuck::{Pod, Zeroable};

use crate::color_correction::ColorCorrection;

use super::super::draw::ShipDraw;
use super::super::parts::Parts;
use super::super::pilot::Pilot;
use super::super::sound::ui::UiSounds;
use super::super::SidewinderAssets;
use super::load::{read, texture};
use super::pass::{UiFrame, UiGlobalsGpu, UiPassSet, HDR_FORMAT};
use super::place::UiDraw;

pub const RADAR_SHADER: &str = "shaders/sidewinder_ui_radar.wesl";
const RADAR_FILE: &str = "ui/hud3d/radar.json";
const DISC_TEXTURE: &str = "ui/hud3d/tex/radar_radarplane__pdiffusealphatexture.dds";

/// The range animation's rate: t grows by 2 x dt, so a change takes 0.5 s (`FUN_143854bb0`, 0x144dec664, A).
pub const RANGE_RATE: f32 = 2.0;

// ---- Stand-ins (the view settings `FUN_1436800b0` copies from fields whose writers were not found, notes 7.2) ----

/// `gRadarSize` and the metres of one of the radar's units (the view's local matrix, view+0x130..): neither found. Measured on
/// session ui1's `a00_start.png` against our own render at 3440 x 1440 after fitting the camera from the canopy lamps (C): the
/// disc's half side (gRadarSize x the metres) so that the disc's outermost ring (the texture's at 0.343 of its side from the
/// middle) is Elite's 497-500 px wide; gRadarSize from the range tick's width, which is the arc's angular spread alone (0.0525 x
/// gRadarSize / 2 each side): at 0.59 our tick measured 9.1 px against Elite's 8.0 in the render (the perspective at the
/// disc's near edge, where the tick sits, included), so 0.59 x 8.0 / 9.1 = 0.52 (+-0.06 for +-1 px), the metres raised by the
/// same factor so the disc keeps its size. The arc itself is the decode's: in the render it spans 229 px from 500 to 8000 m
/// against Elite's 232. Drawn at the screen's own size since the move into the UI's pass (the render's upscale had widened it),
/// the tick at its first place measured 7.0 px at 0.52 and 9.0 px at 0.59 (whole pixels, +-1): 8.0 between them at 0.555, the
/// metres 0.988 x 0.52 / 0.555 = 0.9257.
pub const RADAR_SIZE: f32 = 0.555;
pub const METRES_PER_UNIT: f32 = 0.9257;
/// `gRadarSweepRadius`: 1, so that a contact scaled to 1 lands on the disc's edge as the notes' "the disc edge is reached at
/// x of about 0.9" reads it (B).
pub const SWEEP_RADIUS: f32 = 1.0;
/// `gRadarLuminanceScale` (the view's luminance x its brightness, view+0xec x view+0x40): not found, so fitted (C) to the
/// disc's rings down the column through its middle on session ui1's `a00_start.png` (Elite's red 97-182 levels, green over
/// red 0.12-0.41), on the real render, in two steps: each ring's level brought back through the screen fit
/// (`ui::material::SCREEN_FIT`) to the value it needs, the ratio taken ring by ring and the median kept (at 1 the rings were
/// 41-87, 0.00-0.11: median 2.45; at 2.45 67-129, 0.07-0.19: median 3.81, the plate under the disc taking part of the gain).
/// At 3.81 the rings are 77-182, green over red 0.11-0.31. Good to about +-30 %: the ratios spread from ring to ring because
/// the pulse stands elsewhere on each picture. Since the move into the UI's pass (no plate drawn over the disc, no upscale or
/// bloom) the rings at 3.81 were 109-210, green over red 0.17-0.43: the same step's median 0.698 gives 2.66.
pub const LUMINANCE_SCALE: f32 = 2.66;
/// `gRadarPulseCycle` (s) and `gRadarPulseSpeed` (km/s): not found (E); a ring that crosses the disc's log scale (its edge is
/// 95 km of the pulse's distance) in the cycle.
pub const PULSE_CYCLE_S: f32 = 4.0;
pub const PULSE_SPEED: f32 = 25.0;
/// `gRadarScalePlayer`, `gRadarScaleRangeIndicator` (view+0x84.., +0x90..; not found), measured (C). The marker: 26 px wide
/// and 11 px deep on the picture (the disc's tilt: its ring 500 x 248 px), at the range's lowest step (the tick at its first
/// place: s = sat(500 / 500 + 0.25) = 1), against the disc's half side of 364 px: 2 x 2.1215 x scale.x = 26 / 364 and 3.6745 x
/// scale.z = 11 / 0.496 / 364, 0.0168 and 0.0166 (+-0.0013), taken as one (the marker sits at the disc's middle, at the disc's
/// own scale); y is unread. The tick: in the render at 0.063 it stood 31.4 px tall against Elite's 25-26 (it sits on the disc's
/// near edge, about 1.14 times the middle's scale), so 0.063 x 25.5 / 31.4 = 0.051 (+-0.002); its width is the arc's spread
/// alone (x 0); z is unread (the quad is flat in z).
pub const PLAYER_SCALE: [f32; 3] = [0.0167, 1.0, 0.0167];
pub const RANGE_SCALE: [f32; 3] = [0.0, 0.051, 1.0];
/// The own-ship marker's and the range tick's instance colours (Elite feeds them per instance; not found), brought back from
/// the screen colours on `a00_start.png` through the screen fit (`ui::material::SCREEN_FIT`) and the shader's own factors (C):
/// the marker's mean (210, 87, 0) over its pixels is the value (2.51, 0.382, 0), over pLuminance 24, its resting pulse alpha
/// 0.15 and the three nested triangles' mean cover 14/9; the tick's (230, 209, 0) the value (9.94, 2.43, 0) over pLuminance
/// 8 (its alpha 1). The tick's red stands at the fit's ceiling (0.79): any red from 1.2 up shows within 2 levels, so it is
/// the smallest that does (a lower bound). Since the move into the UI's pass both are x 3.81 / 2.66 (`LUMINANCE_SCALE`'s
/// change, which they take too), and the tick's green, which the move raised (228 levels against 208), brought back through
/// the screen fit: x 0.305. The marker's is not re-checked: its pixels' mean follows the edges' softness (Elite's picture's
/// glow, ours now sharp, added over the disc) more than its colour.
pub const PLAYER_COLOUR: [f32; 3] = [0.642, 0.0974, 0.0];
pub const RANGE_COLOUR: [f32; 3] = [1.776, 0.133, 0.0];

// ---- The file ----

/// One Radar_* material's record.
#[derive(Debug, Clone)]
pub(in crate::sidewinder) struct RadarMaterial {
    pub shader: String,
    pub attrs: BTreeMap<String, Vec<f64>>,
}

impl RadarMaterial {
    fn value(&self, name: &str) -> Result<f32, String> {
        self.attrs.get(name).and_then(|v| v.first()).map(|v| *v as f32).ok_or_else(|| format!("{}: no {name}", self.shader))
    }
}

/// `ui/hud3d/radar.json` (written by the converter, `convert/ui_hud3d.rs`).
#[derive(Debug, Clone)]
pub(in crate::sidewinder) struct RadarFile {
    pub format: u32,
    /// `RadarViewCommonPlaneVertices`: (u, v, x, z).
    pub plane: Vec<[f32; 4]>,
    /// `RadarViewCommonPlayerVertices`: x, y, z, then TEXCOORD1 (u, a, s, w): three nested triangles, w their scale.
    pub player: Vec<[f32; 7]>,
    /// `RadarViewCommonRangeIndicatorVertices`: x, y, z.
    pub range_indicator: Vec<[f32; 3]>,
    pub materials: BTreeMap<String, RadarMaterial>,
    pub ranges_m: Vec<f32>,
}

impl RadarFile {
    pub fn parse(bytes: &[u8]) -> Result<RadarFile, String> {
        let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| format!("{RADAR_FILE}: {e}"))?;
        let ff = RADAR_FILE;
        let materials = json_field(&v, "materials", ff)?
            .as_object()
            .ok_or_else(|| format!("{ff}: materials is not an object"))?
            .iter()
            .map(|(k, m)| Ok((k.clone(), RadarMaterial { shader: json_str(m, "shader", k)?, attrs: json_attrs(m, k)? })))
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        let f = RadarFile {
            format: json_field(&v, "format", ff)?.as_u64().unwrap_or(0) as u32,
            plane: json_rows::<4>(&v, "plane", ff)?,
            player: json_rows::<7>(&v, "player", ff)?,
            range_indicator: json_rows::<3>(&v, "range_indicator", ff)?,
            materials,
            ranges_m: json_field(&v, "ranges_m", ff)?.as_array().ok_or_else(|| format!("{ff}: ranges_m"))?.iter().map(|x| json_f32(x, ff)).collect::<Result<_, _>>()?,
        };
        if f.format != 1 {
            return Err(format!("{RADAR_FILE}: format {}", f.format));
        }
        if f.plane.len() != 6 || f.player.len() != 9 || f.range_indicator.len() != 6 {
            return Err(format!("{RADAR_FILE}: {} plane, {} player, {} range indicator vertices", f.plane.len(), f.player.len(), f.range_indicator.len()));
        }
        if f.ranges_m.len() < 2 || f.ranges_m.windows(2).any(|w| w[1] <= w[0]) {
            return Err(format!("{RADAR_FILE}: the range table {:?} does not rise", f.ranges_m));
        }
        for (m, shader) in [("radar_radarplane", "RadarPlane"), ("radar_radarplayer", "RadarPlayer"), ("radar_radarrangeindicator", "RadarRangeIndicator")] {
            let r = f.materials.get(m).ok_or_else(|| format!("{RADAR_FILE}: no material {m}"))?;
            if r.shader != shader {
                return Err(format!("{RADAR_FILE}: {m} is {}", r.shader));
            }
            r.value("pLuminance")?;
        }
        Ok(f)
    }
}

// ---- JSON (the converter's files, read field by field: the crate has serde_json, not serde's derive) ----

pub(super) fn json_field<'a>(v: &'a serde_json::Value, k: &str, f: &str) -> Result<&'a serde_json::Value, String> {
    v.get(k).ok_or_else(|| format!("{f}: no {k}"))
}

pub(super) fn json_f32(v: &serde_json::Value, f: &str) -> Result<f32, String> {
    v.as_f64().map(|x| x as f32).ok_or_else(|| format!("{f}: {v} is not a number"))
}

pub(super) fn json_str(v: &serde_json::Value, k: &str, f: &str) -> Result<String, String> {
    json_field(v, k, f)?.as_str().map(str::to_string).ok_or_else(|| format!("{f}: {k} is not text"))
}

/// A list of `N` numbers.
pub(super) fn json_array<const N: usize>(v: &serde_json::Value, f: &str) -> Result<[f32; N], String> {
    let a = v.as_array().filter(|a| a.len() == N).ok_or_else(|| format!("{f}: not {N} numbers"))?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = json_f32(x, f)?;
    }
    Ok(out)
}

/// A list of rows of `N` numbers.
pub(super) fn json_rows<const N: usize>(v: &serde_json::Value, k: &str, f: &str) -> Result<Vec<[f32; N]>, String> {
    json_field(v, k, f)?.as_array().ok_or_else(|| format!("{f}: {k} is not a list"))?.iter().map(|r| json_array::<N>(r, &format!("{f} {k}"))).collect()
}

/// A material's attributes: name -> values.
pub(super) fn json_attrs(v: &serde_json::Value, f: &str) -> Result<BTreeMap<String, Vec<f64>>, String> {
    let o = json_field(v, "attrs", f)?.as_object().ok_or_else(|| format!("{f}: attrs is not an object"))?;
    o.iter().map(|(k, x)| Ok((k.clone(), x.as_array().ok_or_else(|| format!("{f}: {k}"))?.iter().map(|n| n.as_f64().ok_or_else(|| format!("{f}: {k}"))).collect::<Result<Vec<f64>, String>>()?))).collect()
}

// ---- The range ----

/// The range: the table's index and the value sliding between two entries (`FUN_143858200`, `FUN_143854bb0`).
#[derive(Clone, Debug, PartialEq)]
pub struct RadarRange {
    pub index: usize,
    from: f32,
    to: f32,
    /// The slide's time, 0..1 (1: still).
    t: f32,
}

impl RadarRange {
    /// At the table's default index (`+0x2c` = 0, A).
    pub fn new(table: &[f32]) -> RadarRange {
        RadarRange { index: 0, from: table[0], to: table[0], t: 1.0 }
    }

    /// A key: the index one up or down, clamped (`FUN_143858070`); the slide starts from the value shown now. Whether it moved.
    pub fn step(&mut self, table: &[f32], up: bool) -> bool {
        let next = if up { (self.index + 1).min(table.len() - 1) } else { self.index.saturating_sub(1) };
        if next == self.index {
            return false;
        }
        self.from = self.value();
        self.to = table[next];
        self.index = next;
        self.t = 0.0;
        true
    }

    /// The frame's time: whether the slide ended now.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.t >= 1.0 {
            return false;
        }
        self.t = (self.t + RANGE_RATE * dt).min(1.0);
        self.t >= 1.0
    }

    /// The range shown (`gRadarRange`), metres.
    pub fn value(&self) -> f32 {
        self.from + self.t * (self.to - self.from)
    }
}

/// The scanner's range keys this frame (Elite's `RadarIncreaseRange`, `RadarDecreaseRange`): set by the seat's key reader,
/// read and cleared by `update`.
#[derive(Resource, Default)]
pub struct RadarKeys {
    pub up: bool,
    pub down: bool,
}

// ---- The draws ----

/// `sidewinder_ui_radar.wesl` `RadarParams`: one draw's own values.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct RadarParams {
    /// x the draw (0 disc, 1 own-ship marker, 2 range indicator), y pLuminance, z pRadarPulseAlphaModifier, w
    /// pRadarPulseAlphaReductionRate.
    pub kind: Vec4,
    /// rgb the instance colour (marker, tick), w unused; xyz the draw's scale (`gRadarScalePlayer`, `gRadarScaleRangeIndicator`).
    pub colour: Vec4,
    pub scale: Vec4,
}

/// `sidewinder_ui_radar.wesl` `RadarFrame`: the frame's values, written in place.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct RadarFrameGpu {
    /// The disc's frame in the frame's world: the ship's frame x the cockpit node's matrix now x `disc_frame`.
    pub model: [[f32; 4]; 4],
    /// x gRadarTime (s, within the pulse's cycle), y gRadarRange, z gRadarMinimumRange, w gRadarMaximumRange.
    pub range: [f32; 4],
    /// x gRadarSize, y gRadarSweepRadius, z gRadarPulseCycle, w gRadarPulseSpeed.
    pub size: [f32; 4],
    /// x gRadarLuminanceScale, y gRadarLuminanceInRadarModeOnlyVisibility, z ...InBothModesVisibility, w gPhaseOutAlpha (radar
    /// mode: 1, 1, 1; the docking view is out of scope).
    pub look: [f32; 4],
}

/// One vertex of the scanner's meshes: its position in the radar's own units (the disc's quad +-1, times gRadarSize in the
/// shader) and one uv (the disc's texture coordinates; the marker's triangle scale in u).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct RadarVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

/// The three meshes (disc, own-ship marker, range indicator) in one list, and each draw's first vertex and count.
fn vertices(f: &RadarFile) -> (Vec<RadarVertex>, [(u32, u32); 3]) {
    let meshes: [Vec<RadarVertex>; 3] = [
        f.plane.iter().map(|v| RadarVertex { position: [v[2], 0.0, v[3]], uv: [v[0], v[1]] }).collect(),
        f.player.iter().map(|v| RadarVertex { position: [v[0], v[1], v[2]], uv: [v[6], 0.0] }).collect(),
        f.range_indicator.iter().map(|v| RadarVertex { position: *v, uv: [0.0; 2] }).collect(),
    ];
    let mut draws = [(0, 0); 3];
    let mut all = Vec::new();
    for (d, m) in draws.iter_mut().zip(meshes) {
        *d = (all.len() as u32, m.len() as u32);
        all.extend(m);
    }
    (all, draws)
}

/// The scanner's draws, made once: the meshes, the draws' values (`RadarParams` x 3, encoded), the disc's texture.
pub struct RadarParts {
    vertices: Vec<RadarVertex>,
    draws: [(u32, u32); 3],
    params: Vec<u8>,
    disc: Handle<Image>,
}

/// What the scanner's pass draws this frame, made by `update` and extracted: its parts (None: no scanner) and the frame's
/// values (None: not drawn this frame).
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct RadarShow {
    parts: Option<Arc<RadarParts>>,
    frame: Option<RadarFrameGpu>,
}

// ---- The systems ----

/// What the loader read.
struct Read {
    file: RadarFile,
    disc: Image,
    layout: se_core::elite_ui_layout::Layout,
}

#[derive(Resource)]
struct RadarLoading(Task<Result<Read, String>>);

/// The scanner, once made.
#[derive(Resource)]
pub struct Radar {
    table: Vec<f32>,
    pub range: RadarRange,
    /// The cockpit rig's node the scanner hangs on, and the disc's frame in that node's model space.
    node: usize,
    frame: Mat4,
    /// Seconds within the pulse's cycle.
    clock: f64,
}

#[derive(Resource)]
struct RadarOff;

/// The disc's frame on `!Radar`: in the plate's plane (its x-z, the bone's x and z: the plate model is flat in y), centred on
/// the plate model's middle, its x to the ship's right, y up, z (the marker's tip) toward the ship's nose (-z of the cockpit's
/// space, B: the marker points ahead), scaled by the metres of one of the radar's units.
fn disc_frame(layout: &se_core::elite_ui_layout::Layout) -> Result<Mat4, String> {
    use se_core::elite_ui_layout::Rig;
    let b = layout.bone_frame("!Radar", Rig::Cockpit).ok_or("the layout has no !Radar")?;
    let plate = layout.models.get("gui_radar_panel_background").ok_or("the layout has no gui_radar_panel_background")?;
    let bone = super::place::bone_matrix(b);
    let middle = (Vec3::from_array(plate.bounds_min) + Vec3::from_array(plate.bounds_max)) * 0.5;
    // (Elite's radar space is left-handed: x right, y up, z ahead; each axis is turned to the ship's own, so z is mirrored.)
    let sign = |axis: Vec3, want: Vec3| if bone.transform_vector3(axis).dot(want) >= 0.0 { 1.0 } else { -1.0 };
    let s = Vec3::new(sign(Vec3::X, Vec3::X), sign(Vec3::Y, Vec3::Y), sign(Vec3::Z, Vec3::NEG_Z)) * METRES_PER_UNIT;
    Ok(bone * Mat4::from_translation(middle) * Mat4::from_scale(s))
}

fn load_due(ui: Option<Res<UiDraw>>, radar: Option<Res<Radar>>, loading: Option<Res<RadarLoading>>, off: Option<Res<RadarOff>>) -> bool {
    ui.is_some() && radar.is_none() && loading.is_none() && off.is_none()
}

fn start_load(mut commands: Commands) {
    commands.insert_resource(RadarLoading(AsyncComputeTaskPool::get().spawn(async move {
        let file = RadarFile::parse(&read(RADAR_FILE)?)?;
        let disc = texture(DISC_TEXTURE, false, false)?;
        let layout = se_core::elite_ui_layout::parse_layout(&read("ui/layout.json")?).map_err(|e| e.to_string())?;
        Ok(Read { file, disc, layout })
    })));
}

fn build(world: &mut World) {
    let Some(mut loading) = world.get_resource_mut::<RadarLoading>() else { return };
    let Some(done) = futures::check_ready(&mut loading.0) else { return };
    world.remove_resource::<RadarLoading>();
    if let Err(e) = done.and_then(|r| make(world, r)) {
        warn!("sidewinder ui: the scanner is off: {e}");
        world.insert_resource(RadarOff);
    }
}

fn make(world: &mut World, r: Read) -> Result<(), String> {
    let node = world.resource::<SidewinderAssets>().rig.cockpit_node().ok_or("the cockpit's root is not in the ship's rig")?;
    let frame = disc_frame(&r.layout)?;
    let table = r.file.ranges_m.clone();
    let disc = world.resource_mut::<Assets<Image>>().add(r.disc);
    let m = &r.file.materials;
    let params = |name: &str, kind: f32, colour: [f32; 3], scale: [f32; 3]| -> Result<Vec<u8>, String> {
        let mat = &m[name];
        let pulse = |n: &str| mat.attrs.get(n).and_then(|v| v.first()).map_or(0.0, |v| *v as f32);
        Ok(crate::stars::gpu_bytes(&RadarParams {
            kind: Vec4::new(kind, mat.value("pLuminance")?, pulse("pRadarPulseAlphaModifier"), pulse("pRadarPulseAlphaReductionRate")),
            colour: Vec3::from_array(colour).extend(0.0),
            scale: Vec3::from_array(scale).extend(0.0),
        }))
    };
    let params = [
        params("radar_radarplane", 0.0, [0.0; 3], [1.0; 3])?,
        params("radar_radarplayer", 1.0, PLAYER_COLOUR, PLAYER_SCALE)?,
        params("radar_radarrangeindicator", 2.0, RANGE_COLOUR, RANGE_SCALE)?,
    ]
    .concat();
    let (vertices, draws) = vertices(&r.file);
    world.insert_resource(RadarShow { parts: Some(Arc::new(RadarParts { vertices, draws, params, disc })), frame: None });
    info!("sidewinder ui: the scanner is made (3 draws), drawn by its pass while seated");
    world.insert_resource(Radar { range: RadarRange::new(&table), table, node, frame, clock: 0.0 });
    Ok(())
}

/// The ship gone: the scanner goes with the UI.
fn teardown(world: &mut World) {
    world.remove_resource::<Radar>();
    world.insert_resource(RadarShow::default());
    world.remove_resource::<RadarLoading>();
    world.remove_resource::<RadarOff>();
}

fn ui_gone(ui: Option<Res<UiDraw>>, radar: Option<Res<Radar>>, off: Option<Res<RadarOff>>) -> bool {
    ui.is_none() && (radar.is_some() || off.is_some())
}

/// Each frame once made: the keys and the range's slide (seated), the frame's values with the disc placed on the cockpit.
#[allow(clippy::too_many_arguments)]
fn update(
    mut radar: ResMut<Radar>,
    mut show: ResMut<RadarShow>,
    mut keys: ResMut<RadarKeys>,
    mut sounds: Option<ResMut<UiSounds>>,
    pilot: Option<Res<Pilot>>,
    draw: Option<Res<ShipDraw>>,
    parts: Res<Parts>,
    seat: Res<super::super::seat::Seat>,
    control: Res<crate::mods::ModControl>,
    time: Res<Time<Real>>,
) {
    let radar = &mut *radar;
    let seated = super::seated(&seat, &control);
    // (Elite's gate, notes 7.2: the radar's power not 0: the ship powered and its sensors on.)
    let powered = pilot.as_ref().is_some_and(|p| p.powered && p.modules[super::super::panels::M_SENSORS]);
    let ship = draw.as_ref().filter(|d| d.seat_shown()).and_then(|d| d.frame()).filter(|_| seated && powered);
    let dt = time.delta_secs();
    if ship.is_some() {
        let mut post = |e: &str| {
            if let Some(s) = sounds.as_mut() {
                s.post(e);
            }
        };
        for up in [true, false].into_iter().filter(|&up| if up { keys.up } else { keys.down }) {
            if radar.range.step(&radar.table, up) {
                post("Indicator_b");
                post(if up { "RadiusIncrease" } else { "RadiusDecrease" });
            }
        }
        if radar.range.tick(dt) {
            post("Indicator_e");
        }
        radar.clock = (radar.clock + f64::from(dt)) % f64::from(PULSE_CYCLE_S);
    }
    *keys = RadarKeys::default();
    let (min, max) = (radar.table[0], radar.table[radar.table.len() - 1]);
    let now = ship.map(|ship| RadarFrameGpu {
        model: (ship.to_matrix() * parts.matrix(radar.node) * radar.frame).to_cols_array_2d(),
        range: [radar.clock as f32, radar.range.value(), min, max],
        size: [RADAR_SIZE, SWEEP_RADIUS, PULSE_CYCLE_S, PULSE_SPEED],
        look: [LUMINANCE_SCALE, 1.0, 1.0, 1.0],
    });
    if show.frame != now {
        show.frame = now;
    }
}

// ---- The 3D parts' view group (render world): the scanner's and the holograms' passes, through `pass.rs`'s seam ----


/// A view's group's key: the depth texture and the buffers it binds.
type ViewKey = (TextureId, BufferId, BufferId);

/// The view's group the scanner's and the holograms' passes share (group 0 of both shaders): the view's `View` uniform (dynamic
/// offset), the UI pass's values (`UiFrame::globals`, written in place when they change), the scene's depth texture (the
/// holograms' depth test); made per view and kept until its depth texture or a buffer changes, as `pass.rs` keeps its own.
#[derive(Resource)]
pub(super) struct PartsView {
    /// Without and with MSAA (the depth texture's kind).
    layouts: [BindGroupLayoutDescriptor; 2],
    globals: Option<Buffer>,
    written: Option<UiGlobalsGpu>,
    views: HashMap<Entity, (ViewKey, BindGroup)>,
}

impl PartsView {
    pub(super) fn layout(&self, msaa: bool) -> &BindGroupLayoutDescriptor {
        &self.layouts[msaa as usize]
    }

    /// The view's group on a frame the UI's pass draws (`UiFrame::order` non-empty: the seat's cockpit drawn) into an HDR
    /// view's main texture.
    pub(super) fn group(&self, view: Entity, frame: &UiFrame, target: &ViewTarget) -> Option<&BindGroup> {
        if frame.order.is_empty() || target.main_texture_format() != HDR_FORMAT {
            return None;
        }
        self.views.get(&view).map(|(_, g)| g)
    }
}

fn init_parts_view(mut commands: Commands) {
    let layouts = [false, true].map(|msaa| {
        BindGroupLayoutDescriptor::new(
            "sidewinder_ui_parts_view",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (uniform_buffer::<ViewUniform>(true), uniform_buffer_sized(false, None), if msaa { texture_depth_2d_multisampled() } else { texture_depth_2d() }),
            ),
        )
    });
    commands.insert_resource(PartsView { layouts, globals: None, written: None, views: HashMap::new() });
}

/// The pass's values, made with the UI and written in place when they change; each view's group (a view gone, its group goes).
fn prepare_parts_view(
    views: Query<(Entity, &ViewDepthStencilTexture, &Msaa), With<ColorCorrection>>,
    frame: Res<UiFrame>,
    mut parts: ResMut<PartsView>,
    uniforms: Res<ViewUniforms>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
) {
    let pv = &mut *parts;
    if frame.geometry.is_none() {
        pv.globals = None;
        pv.written = None;
        pv.views.clear();
        return;
    }
    match pv.globals.as_ref() {
        None => {
            pv.globals = Some(device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("sidewinder_ui_parts_globals"),
                contents: bytemuck::bytes_of(&frame.globals),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            }));
            pv.written = Some(frame.globals);
        }
        Some(b) if pv.written != Some(frame.globals) => {
            queue.write_buffer(b, 0, bytemuck::bytes_of(&frame.globals));
            pv.written = Some(frame.globals);
        }
        Some(_) => {}
    }
    let (Some(globals), Some(view_buffer), Some(view_binding)) = (pv.globals.as_ref(), uniforms.uniforms.buffer(), uniforms.uniforms.binding()) else {
        pv.views.clear();
        return;
    };
    pv.views.retain(|e, _| views.contains(*e));
    for (entity, depth, msaa) in &views {
        let key = (depth.texture().id(), view_buffer.id(), globals.id());
        if pv.views.get(&entity).is_some_and(|(k, _)| *k == key) {
            continue;
        }
        let depth_view = depth.texture().create_view(&TextureViewDescriptor { aspect: TextureAspect::DepthOnly, ..default() });
        let group = device.create_bind_group(
            "sidewinder_ui_parts_view",
            &cache.get_bind_group_layout(&pv.layouts[(msaa.samples() > 1) as usize]),
            &BindGroupEntries::sequential((view_binding.clone(), globals.as_entire_binding(), &depth_view)),
        );
        pv.views.insert(entity, (key, group));
    }
}

// ---- The scanner's pass (render world) ----

/// The buffers and group made from one `RadarParts` (a new ship's are new).
struct RadarBuffers {
    /// The parts they were made from (`Arc::as_ptr`).
    key: usize,
    vertices: Buffer,
    frame: Buffer,
    written: Option<RadarFrameGpu>,
    params: Buffer,
    /// Made once the disc's texture exists.
    material: Option<BindGroup>,
}

#[derive(Resource)]
struct RadarGpu {
    layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    /// Without and with MSAA (the view group's kind), queued with the first scanner and kept.
    pipelines: Option<[CachedRenderPipelineId; 2]>,
    made: Option<RadarBuffers>,
    logged: bool,
}

fn init_radar(mut commands: Commands, assets: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "sidewinder_ui_radar",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (uniform_buffer_sized(false, None), uniform_buffer_sized(false, None), texture_2d(TextureSampleType::Float { filterable: true }), sampler(SamplerBindingType::Filtering)),
        ),
    );
    commands.insert_resource(RadarGpu { layout, shader: assets.load(RADAR_SHADER), pipelines: None, made: None, logged: false });
}

/// The scanner's pipeline: `pDepthEnable 0` (no depth test), both faces, and the blend, a stand-in (E: the effect's blend
/// state is not in the material files): the colour added weighted by its alpha (`SrcAlpha`, `One`), the alpha being the
/// shaders' mask and pulse; the weighting is done in the shader before the last step (so the screen fit sees what Elite adds),
/// so the pipeline adds, the target's alpha kept.
fn radar_pipeline(gpu: &RadarGpu, view: &BindGroupLayoutDescriptor) -> RenderPipelineDescriptor {
    let add = BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
    let keep = BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add };
    RenderPipelineDescriptor {
        label: Some("sidewinder_ui_radar".into()),
        layout: vec![view.clone(), gpu.layout.clone()],
        vertex: VertexState {
            shader: gpu.shader.clone(),
            entry_point: Some("vertex".into()),
            buffers: vec![VertexBufferLayout::from_vertex_formats(VertexStepMode::Vertex, [VertexFormat::Float32x3, VertexFormat::Float32x2])],
            ..default()
        },
        fragment: Some(FragmentState {
            shader: gpu.shader.clone(),
            entry_point: Some("fragment".into()),
            targets: vec![Some(ColorTargetState { format: HDR_FORMAT, blend: Some(BlendState { color: add, alpha: keep }), write_mask: ColorWrites::ALL })],
            ..default()
        }),
        primitive: PrimitiveState { topology: PrimitiveTopology::TriangleList, cull_mode: None, ..default() },
        ..default()
    }
}

/// The scanner's buffers made when its parts arrive, the frame's values written in place, the group made once the disc's
/// texture exists; a pipeline's failure logged once (the scanner is then not drawn, the ship works).
#[allow(clippy::too_many_arguments)]
fn prepare_radar(show: Res<RadarShow>, view: Res<PartsView>, mut gpu: ResMut<RadarGpu>, device: Res<RenderDevice>, queue: Res<RenderQueue>, cache: Res<PipelineCache>, images: Res<RenderAssets<GpuImage>>) {
    let gpu = &mut *gpu;
    let Some(parts) = show.parts.as_ref() else {
        gpu.made = None;
        return;
    };
    if gpu.pipelines.is_none() {
        gpu.pipelines = Some([false, true].map(|msaa| cache.queue_render_pipeline(radar_pipeline(gpu, view.layout(msaa)))));
    }
    let key = Arc::as_ptr(parts) as usize;
    if gpu.made.as_ref().is_none_or(|m| m.key != key) {
        let init = |label: &'static str, contents: &[u8], usage: BufferUsages| device.create_buffer_with_data(&BufferInitDescriptor { label: Some(label), contents, usage });
        gpu.made = Some(RadarBuffers {
            key,
            vertices: init("sidewinder_ui_radar_vertices", bytemuck::cast_slice(&parts.vertices), BufferUsages::VERTEX),
            frame: init("sidewinder_ui_radar_frame", bytemuck::bytes_of(&RadarFrameGpu::default()), BufferUsages::UNIFORM | BufferUsages::COPY_DST),
            written: None,
            params: init("sidewinder_ui_radar_params", &parts.params, BufferUsages::UNIFORM),
            material: None,
        });
    }
    let Some(made) = gpu.made.as_mut() else { return };
    if let Some(now) = show.frame
        && made.written != Some(now)
    {
        queue.write_buffer(&made.frame, 0, bytemuck::bytes_of(&now));
        made.written = Some(now);
    }
    if made.material.is_none()
        && let Some(disc) = images.get(&parts.disc)
    {
        let entries = BindGroupEntries::sequential((made.params.as_entire_binding(), made.frame.as_entire_binding(), &disc.texture_view, &disc.sampler));
        made.material = Some(device.create_bind_group("sidewinder_ui_radar", &cache.get_bind_group_layout(&gpu.layout), &entries));
    }
    for id in gpu.pipelines.iter().flatten() {
        if let CachedPipelineState::Err(e) = cache.get_render_pipeline_state(*id)
            && !std::mem::replace(&mut gpu.logged, true)
        {
            error!("sidewinder ui: the scanner's pipeline failed, it is not drawn: {e}");
        }
    }
}

/// The scanner's pass's place in `Core3d`: after the panels' (`UiPassSet`); the holograms' pass runs after it.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ScannerPassSet;

/// The scanner's pass: its three draws into the view's main texture at its full size.
fn scanner_pass(
    view: ViewQuery<(Entity, &ViewTarget, &ViewUniformOffset, &Msaa), With<ColorCorrection>>,
    show: Res<RadarShow>,
    frame: Res<UiFrame>,
    parts_view: Res<PartsView>,
    gpu: Res<RadarGpu>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (entity, target, offset, msaa) = view.into_inner();
    let (Some(parts), Some(_), Some(made), Some(view_group)) = (show.parts.as_ref(), show.frame, gpu.made.as_ref(), parts_view.group(entity, &frame, target)) else { return };
    let pipeline = gpu.pipelines.and_then(|p| cache.get_render_pipeline(p[(msaa.samples() > 1) as usize]));
    let (Some(pipeline), Some(material), true) = (pipeline, made.material.as_ref(), made.key == Arc::as_ptr(parts) as usize) else { return };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    use bevy::render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(ctx.command_encoder(), "sidewinder_ui_scanner");
    let scene = RenderPassColorAttachment { view: target.main_texture_view(), depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } };
    let encoder = ctx.command_encoder();
    let mut p = encoder.begin_render_pass(&RenderPassDescriptor { label: Some("sidewinder_ui_scanner"), color_attachments: &[Some(scene)], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
    p.set_pipeline(pipeline);
    p.set_bind_group(0, view_group, &[offset.offset]);
    p.set_bind_group(1, material, &[]);
    p.set_vertex_buffer(0, *made.vertices.slice(..));
    for (k, &(first, count)) in parts.draws.iter().enumerate() {
        p.draw(first..first + count, k as u32..k as u32 + 1);
    }
    drop(p);
    span.end(ctx.command_encoder());
}

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<RadarShow>::default())
        .init_resource::<RadarKeys>()
        .init_resource::<RadarShow>()
        .add_systems(Update, (start_load.run_if(load_due), build.run_if(resource_exists::<RadarLoading>), teardown.run_if(ui_gone)))
        .add_systems(PostUpdate, update.in_set(super::UiSet::Place).run_if(resource_exists::<Radar>));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .add_systems(RenderStartup, (init_parts_view, init_radar))
        .add_systems(Render, prepare_radar.in_set(RenderSystems::PrepareResources))
        .add_systems(Render, prepare_parts_view.in_set(RenderSystems::PrepareBindGroups))
        .add_systems(Core3d, scanner_pass.in_set(ScannerPassSet).after(UiPassSet).in_set(Core3dSystems::PostProcess));
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: [f32; 5] = [500.0, 1000.0, 2000.0, 4000.0, 8000.0];

    /// A step slides linearly over 0.5 s (A) and ends there; both ends clamp without a slide.
    #[test]
    fn the_range_slides_as_elites() {
        let mut r = RadarRange::new(&TABLE);
        assert_eq!(r.value(), 500.0);
        assert!(!r.step(&TABLE, false), "the bottom clamps");
        assert!(r.step(&TABLE, true));
        assert!(!r.tick(0.125));
        assert!((r.value() - 625.0).abs() < 1e-3, "a quarter of the way at 0.125 s: {}", r.value());
        assert!(!r.tick(0.25));
        assert!((r.value() - 875.0).abs() < 1e-3);
        assert!(r.tick(0.2), "it ends at 0.5 s");
        assert_eq!(r.value(), 1000.0);
        assert!(!r.tick(1.0));
        for _ in 0..4 {
            r.step(&TABLE, true);
        }
        assert_eq!(r.index, 4);
        assert!(!r.step(&TABLE, true), "the top clamps");
        // A step during a slide starts from the value shown.
        r.tick(1.0);
        r.step(&TABLE, false);
        r.tick(0.25);
        assert!(r.step(&TABLE, false));
        assert!((r.from - 6000.0).abs() < 1e-3 && r.to == 2000.0);
    }

    /// The shipped file reads: its meshes, its materials' luminances (8, 24, 8: the notes' 1.7), the table.
    #[test]
    fn the_shipped_radar_reads() {
        let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder").join(RADAR_FILE)).unwrap();
        let f = RadarFile::parse(&bytes).unwrap();
        assert_eq!(f.ranges_m, TABLE.to_vec());
        let lum: Vec<f32> = ["radar_radarplane", "radar_radarplayer", "radar_radarrangeindicator"].iter().map(|m| f.materials[*m].value("pLuminance").unwrap()).collect();
        assert_eq!(lum, [8.0, 24.0, 8.0]);
        // The marker's tip 3.6745 ahead, its base +-2.1215; the triangles' scales a third, two thirds, one.
        assert_eq!(f.player[2][2], 3.6745);
        assert_eq!(f.player.iter().map(|v| (v[6] * 3.0).round()).collect::<Vec<_>>(), [1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0]);
        // The range tick's quad: x and y +-1, z 0.
        assert!(f.range_indicator.iter().all(|v| v[0].abs() == 1.0 && v[1].abs() == 1.0 && v[2] == 0.0));
        let (v, draws) = vertices(&f);
        assert_eq!((v.len(), draws), (21, [(0, 6), (6, 9), (15, 6)]));
    }
}
