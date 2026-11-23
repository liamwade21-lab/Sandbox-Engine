//! `ui/layout.json` (UI2; ARCHITECTURE.md A10; `se_core::elite_ui_layout::Layout`): how the movies in A10's scope sit in
//! the cockpit, from the decoded UI export's `rendering.json` (its `sidewinder_panel_components` is
//! `panels_sidewinder_cockpit.json`, checked equal), `cockpitui_state_machine.json` (the head-look targets) and `hud3d/`
//! (the panel models and their materials, made by `ui_models.rs`). `re/elite/notes/ui_rendering.md` sections 2 to 4 give
//! every value's proof.
//!
//! Elite's numbers are kept, but positions, axes, bounds and quad corners are turned into the mod's axes by
//! `assets::ShipSpace` with no lift (Elite's z mirrored), in the frame they hang in (the cockpit model, the chair rig, a
//! bone, a model): the game places them with the node they hang on, as it places the cockpit's parts.
//!
//! `plan` works the files out from the export alone; `convert` writes them, `verify` compares the folder with a fresh plan,
//! `check` reads the written layout as the game will.

use std::collections::{BTreeMap, BTreeSet};

use bevy::math::{DMat4, DVec3, DVec4};
use serde_json::Value;

use se_core::elite_asset::parse_edm;
use se_core::elite_ui_layout::{
    parse_layout, write_layout, Background, BoneFrame, BoneFrames, ChairRig, Component, GaugeAtlas, GaugePart, GuiModel, GuiObject, GuiPanel, GuiSlotValue,
    HeadLook, Island, JointBone, Layout, MovieKind, MovieLayout, PanelQuad, Rig, Shading, ShaderConstant, SightZone, StandIn, CHAIR_RIG_NAME, LAYOUT_FORMAT,
};

use super::super::assets::ShipSpace;
use super::models::{floats, read_json};
use super::ui_models::{self, Materials};
use super::ui_movies::MOVIES;
use super::{Ctx, Planned, Shipped};

pub(super) const LAYOUT: &str = "ui/layout.json";

/// The cockpit rig's names for model bones (`ui_rendering.md` 4.3, from the rig's name table, A): the head-look targets
/// name these joints.
const HEAD_LOOK_JOINTS: &[(&str, &str)] = &[
    ("LeftPanel_Focus", "!Left_Panel"),
    ("RightPanel_Focus", "!Right_Panel"),
    ("RadarPanel_Focus", "!Radar"),
    ("CommsPanel_Focus", "!CommsPanel_Focus"),
    ("FuelScoop_Focus", "!FuelScoop_Panel"),
    ("Camera", "!Camera"),
];

const HUD_QUAD: &str = "/shader_formulas/HudQuad_Alpha_UnlitNoScale_GuiBrightnessScale_UVScaleOffset_FarPlaneFix_GUIColourRotate_Forward";
const GUI_LIGHTING: &str = "/shader_formulas/GuiLighting_Alpha_GuiLighting_Gui_GuiBrightnessScale_Saturation_GUIColourRotate_Forward";
const SCREEN: &str = "/shader_formulas/Screen_Alpha_UnlitNoScale_GuiParallax_GUIColourRotate_GUIFadePremultiplied_PostGui";
const EDGE: &str = "/shader_formulas/EdgeDetectionForUI_PostGui";
const CLIP: &str = "/shader_formulas/Alpha_UnlitNoScaleLinearToGammaSpace_AntialiasClipOnColourAlpha_GUIColourRotate_PostGui";
/// The constants the formulas hold (rendering.json pointer, what it is, its values, the formula's words it is read from:
/// `plan` checks the words are there).
const SHADER_CONSTANTS: &[(&str, &str, &[f64], &str)] = &[
    (HUD_QUAD, "the uv scale-offset's factor on pUVScaleOffset.xy", &[16.0], "pUVScaleOffset.xy*16"),
    (HUD_QUAD, "unpack16's divisor: unpack16(n) = 2n/65535 - 1", &[65535.0], "n*2/65535 - 1"),
    (HUD_QUAD, "the alpha under which a pixel is discarded", &[1e-5], "t.a < 1e-5"),
    (HUD_QUAD, "the movie texture's gamma (c = t.rgb*t.rgb)", &[2.0], "c = t.rgb*t.rgb"),
    (GUI_LIGHTING, "the movie colour's power", &[1.6], "pow(col.rgb, 1.6)"),
    (GUI_LIGHTING, "the luma weights of the saturation and the parallax's desaturation", &[0.3086, 0.6094, 0.0820], "(0.3086,0.6094,0.0820)"),
    (GUI_LIGHTING, "the black discard: rgb and alpha all under it", &[5.0 / 255.0], "5/255"),
    (SCREEN, "the screen gamma's numerator: pow(rgb, 2.2/gamma)", &[2.2], "2.2/gamma"),
    (SCREEN, "the alpha under which a pixel is discarded", &[1e-5], "col.a < 1e-5"),
    (EDGE, "pSobelDepthThreshold", &[100.0], "pSobelDepthThreshold(100)"),
    (EDGE, "pScanlineUVScale", &[0.0025, 0.0025], "pScanlineUVScale(0.0025,0.0025)"),
    (EDGE, "pSobelEdgeThickness, pixels", &[1.0], "pSobelEdgeThickness(1.0)"),
    (EDGE, "pSobelDistanceScale", &[10.0], "pSobelDistanceScale(10)"),
    (EDGE, "pSobelMaxDistance", &[1.0], "pSobelMaxDistance(1.0)"),
    (EDGE, "pSobelPower", &[4.0], "pSobelPower(4.0)"),
    (EDGE, "pSobelTintLow", &[1.0, 0.0, 0.0], "pSobelTintLow(1,0,0)"),
    (EDGE, "pSobelTintHigh", &[1.0, 0.5, 0.0], "pSobelTintHigh(1,0.5,0)"),
    (CLIP, "the clip test: discard if dot(out, out) is under it", &[0.001], "dot(out,out) < 0.001"),
    ("/user_settings", "DashboardGUIBrightness, this PC's Elite setting (Custom.4.4.misc); that it is the option factor of gGuiBrightness is not traced (E)", &[0.69], "\"DashboardGUIBrightness\":0.69"),
];
/// The screen-space GUI render target (A), and the export's words saying so.
const SCREEN_TARGET: ([u32; 2], &str) = ([1920, 1080], "1920x1080");

/// The one conversion from Elite's axes: `ShipSpace` with no lift (the frames here are a model's or a rig's own).
const SPACE: ShipSpace = ShipSpace { lift: 0.0 };

fn point(p: [f32; 3]) -> [f32; 3] {
    SPACE.point(DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)).to_array()
}

/// A box's corners in the mod's axes (the mirror swaps which z is the least).
fn bounds(lo: [f32; 3], hi: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (a, b) = (point(lo), point(hi));
    (std::array::from_fn(|k| a[k].min(b[k])), std::array::from_fn(|k| a[k].max(b[k])))
}

/// A bone's rest frame (its position and its axes' rows, Elite's axes: the rows are its x, y, z axes) in the mod's axes:
/// the matrix whose columns are the axes and the position, through `ShipSpace::skin` (the mirror on both sides).
fn bone_frame(position: [f64; 3], rows: [f64; 12]) -> BoneFrame {
    let row = |k: usize| DVec4::new(rows[4 * k], rows[4 * k + 1], rows[4 * k + 2], 0.0);
    let m = DMat4::from_cols(row(0), row(1), row(2), DVec4::new(position[0], position[1], position[2], 1.0));
    let t = SPACE.skin(m.as_mat4());
    BoneFrame { position: t.w_axis.truncate().to_array(), axes: [t.x_axis.truncate().to_array(), t.y_axis.truncate().to_array(), t.z_axis.truncate().to_array()] }
}

fn ptr<'a>(v: &'a Value, at: &str, file: &str) -> Result<&'a Value, String> {
    v.pointer(at).ok_or_else(|| format!("{file}: no {at}"))
}

fn str_at(v: &Value, key: &str, file: &str) -> Result<String, String> {
    v.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("{file}: no text {key}"))
}

fn opt_str(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn u32_at(v: &Value, key: &str, file: &str) -> Result<u32, String> {
    v.get(key).and_then(Value::as_u64).map(|n| n as u32).ok_or_else(|| format!("{file}: {key} is not a count"))
}

fn f32_at(v: &Value, key: &str, file: &str) -> Result<f32, String> {
    v.get(key).and_then(Value::as_f64).map(|n| n as f32).ok_or_else(|| format!("{file}: {key} is not a number"))
}

fn rig(name: &str, file: &str) -> Result<Rig, String> {
    match name {
        "Root" => Ok(Rig::Cockpit),
        CHAIR_RIG_NAME => Ok(Rig::Chair),
        r => Err(format!("{file}: unknown rig {r}")),
    }
}

fn names(v: &Value, file: &str) -> Result<Vec<String>, String> {
    v.as_array().ok_or_else(|| format!("{file}: not a list"))?.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| format!("{file}: a name is not text"))).collect()
}

/// One movie's layout from its component (a panel movie) or its own record in `rendering.json` `movies`.
fn movie(r: &Value, name: &str, materials: &mut Materials, hud3d: &std::path::Path, textures: &Value) -> Result<MovieLayout, String> {
    let f = format!("rendering.json movies/{name}");
    let rec = ptr(r, &format!("/movies/{name}"), "rendering.json")?;
    let stage = floats::<2>(rec, "stage", &f)?.map(|v| v as u32);
    let mut out = MovieLayout { name: name.to_string(), stage, kind: MovieKind::Panels, component: None, panels: vec![], backgrounds: vec![], quad: None, flat_quads: vec![], screen_materials: vec![] };
    let components = ptr(r, "/sidewinder_panel_components", "rendering.json")?.as_array().ok_or("rendering.json: sidewinder_panel_components is not a list")?;
    if let Some(c) = components.iter().find(|c| c.get("movie").and_then(Value::as_str).is_some_and(|m| m.eq_ignore_ascii_case(name))) {
        let cf = format!("{f} component");
        out.component = Some(Component {
            component: str_at(c, "component", &cf)?,
            key: str_at(c, "key", &cf)?,
            movie: str_at(c, "movie", &cf)?,
            group: str_at(c, "group", &cf)?,
            focus_bone: opt_str(c, "focus_bone"),
            focus_rig: opt_str(c, "focus_rig").map(|x| rig(&x, &cf)).transpose()?,
        });
        for g in c.get("gui_panels").and_then(Value::as_array).ok_or_else(|| format!("{cf}: no gui_panels"))? {
            let n = str_at(g, "name", &cf)?;
            out.panels.push(GuiPanel {
                model: n.to_lowercase(),
                data_key: str_at(g, "data_key", &cf)?,
                flag: u32_at(g, "flag", &cf)?,
                bone: str_at(g, "bone", &cf)?,
                bone_fallback: opt_str(g, "bone_fallback"),
                rig: rig(&str_at(g, "rig", &cf)?, &cf)?,
                name: n,
            });
        }
        for b in c.get("backgrounds").and_then(Value::as_array).ok_or_else(|| format!("{cf}: no backgrounds"))? {
            let n = str_at(b, "name", &cf)?;
            out.backgrounds.push(Background {
                model: n.to_lowercase(),
                parent: str_at(b, "parent", &cf)?,
                flag: u32_at(b, "flag", &cf)?,
                alpha: f32_at(b, "alpha", &cf)?,
                flag2: u32_at(b, "flag2", &cf)?,
                bone: str_at(b, "bone", &cf)?,
                rig: rig(&str_at(b, "rig", &cf)?, &cf)?,
                name: n,
            });
        }
        if let Some(g) = c.get("geometry").filter(|g| !g.is_null()) {
            let verts = g.get("vertices").and_then(Value::as_array).filter(|v| v.len() == 4).ok_or_else(|| format!("{cf}: its quad has not 4 vertices"))?;
            let mut vertices = [[0f32; 5]; 4];
            for (k, v) in verts.iter().enumerate() {
                let n: Vec<f32> = v.as_array().into_iter().flatten().filter_map(Value::as_f64).map(|x| x as f32).collect();
                let [x, y, z, u, w] = n[..] else { return Err(format!("{cf}: quad vertex {k} is not x y z u v")) };
                let p = point([x, y, z]);
                vertices[k] = [p[0], p[1], p[2], u, w];
            }
            let bytes: Vec<u8> = g.get("index_bytes").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).map(|b| b as u8).collect();
            let elite: [u8; 6] = bytes.get(..6).and_then(|b| b.try_into().ok()).ok_or_else(|| format!("{cf}: its quad has not 6 indices"))?;
            if elite.iter().any(|&i| i >= 4) {
                return Err(format!("{cf}: a quad index is outside its 4 vertices"));
            }
            // The triangles turned as the ship's are on load: the mirror would otherwise wind them backwards.
            let mut turned = elite.map(u16::from);
            se_core::elite::to_mod_space(&mut [], &mut turned);
            // The record holds the quad in the panel block beside the GUI panels, without a bone of its own: it is read as
            // in the frame of the component's one GUI panel's bone (the stand-in `stand_ins` names).
            let [panel] = out.panels.as_slice() else { return Err(format!("{cf}: a quad with {} GUI panels: whose bone is it in?", out.panels.len())) };
            out.quad = Some(PanelQuad { vertices, indices: turned.map(|i| i as u8), bone: panel.bone.clone(), rig: panel.rig });
        }
    } else if let Some(c) = rec.get("cockpit_component").filter(|c| !c.is_null()) {
        if let Some(q) = c.get("quads") {
            out.kind = MovieKind::Sights;
            out.flat_quads = names(q, &f)?.iter().map(|n| n.to_lowercase()).collect();
        } else {
            out.kind = MovieKind::Orbital;
            out.flat_quads = vec![str_at(c, "quad", &f)?.to_lowercase()];
        }
    } else if let Some(own) = rec.get("own_material").and_then(Value::as_object) {
        out.kind = MovieKind::Screen;
        for (m, mrec) in own {
            materials.add(hud3d, textures, m, mrec, &format!("rendering.json movies/{name}/own_material"))?;
            out.screen_materials.push(m.to_lowercase());
        }
    } else {
        return Err(format!("{f}: neither a panel component, a cockpit component nor its own material"));
    }
    Ok(out)
}

/// A model's entry from the export's facts (`rendering.json` `panel_models`) and its shipped draws, which must agree.
fn gui_model(r: &Value, stem: &str, edm: &se_core::elite_asset::Edm) -> Result<GuiModel, String> {
    let f = format!("rendering.json panel_models/models/{stem}_");
    let pm = ptr(r, &format!("/panel_models/models/{stem}_"), "rendering.json")?;
    let infos = pm.get("models").and_then(Value::as_array).filter(|m| m.len() == 1).ok_or_else(|| format!("{f}: not one model"))?;
    let info = &infos[0];
    let (lo, hi) = (floats::<3>(info, "bounds_min", &f)?, floats::<3>(info, "bounds_max", &f)?);
    let (bounds_min, bounds_max) = bounds(lo, hi);
    let objects = info.get("objects").and_then(Value::as_array).ok_or_else(|| format!("{f}: no objects"))?;
    if objects.len() != edm.draws.len() {
        return Err(format!("{f}: {} objects, the model has {} draws", objects.len(), edm.draws.len()));
    }
    let mut out = Vec::new();
    for (k, (o, d)) in objects.iter().zip(&edm.draws).enumerate() {
        let of = format!("{f} object {k}");
        let decoded: Vec<_> = d.vertices.iter().map(|v| v.decode()).collect();
        let span = |get: &dyn Fn(&se_core::elite::DecodedVertex, usize) -> f32, n: usize| -> (Vec<f32>, Vec<f32>) {
            ((0..n).map(|a| decoded.iter().map(|v| get(v, a)).fold(f32::MAX, f32::min)).collect(), (0..n).map(|a| decoded.iter().map(|v| get(v, a)).fold(f32::MIN, f32::max)).collect())
        };
        let (uv_lo, uv_hi) = span(&|v, a| v.uv[0][a], 2);
        let (p_lo, p_hi) = span(&|v, a| v.position[a], 3);
        // The export prints them to 5 decimals.
        let near = |got: &[f32], key: &str| -> Result<(), String> {
            let want: Vec<f32> = o.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_f64).map(|x| x as f32).collect();
            if want.len() != got.len() || want.iter().zip(got).any(|(w, g)| (w - g).abs() > 1e-5) {
                return Err(format!("{of}: {key} is {want:?}, the shipped draw's {got:?}"));
            }
            Ok(())
        };
        near(&uv_lo, "uv0_min")?;
        near(&uv_hi, "uv0_max")?;
        near(&p_lo, "pos_min")?;
        near(&p_hi, "pos_max")?;
        if u32_at(o, "verts", &of)? as usize != d.vertices.len() || u32_at(o, "tris", &of)? as usize != d.indices.len() / 3 {
            return Err(format!("{of}: its counts differ from the shipped draw's"));
        }
        let (pos_min, pos_max) = bounds([p_lo[0], p_lo[1], p_lo[2]], [p_hi[0], p_hi[1], p_hi[2]]);
        out.push(GuiObject {
            material: edm.materials[d.material as usize].name.to_lowercase(),
            vertices: d.vertices.len() as u32,
            triangles: (d.indices.len() / 3) as u32,
            uv0_min: [uv_lo[0], uv_lo[1]],
            uv0_max: [uv_hi[0], uv_hi[1]],
            pos_min,
            pos_max,
        });
    }
    let mut islands = Vec::new();
    for (k, i) in pm.get("stage_islands").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let isf = format!("{f} island {k}");
        let (pos_min, pos_max) = bounds(floats::<3>(i, "pos_min", &isf)?, floats::<3>(i, "pos_max", &isf)?);
        islands.push(Island {
            stage_rect: floats::<4>(i, "stage_rect", &isf)?,
            vertices: u32_at(i, "verts", &isf)?,
            pos_min,
            pos_max,
            parts: names(i.get("parts_inside").unwrap_or(&Value::Null), &isf)?,
        });
    }
    Ok(GuiModel { file: format!("ui/models/{stem}.edm"), bounds_min, bounds_max, size_m: [hi[0] - lo[0], hi[1] - lo[1]], objects: out, islands })
}

/// The bones the layout names, each in the frames `rendering.json` `bones_elite_axes` gives it.
fn bones(r: &Value, movies: &[MovieLayout], head_look: &[HeadLook], chair_bone: &str) -> Result<BTreeMap<String, BoneFrames>, String> {
    let table = ptr(r, "/bones_elite_axes", "rendering.json")?;
    let mut wanted: BTreeSet<String> = BTreeSet::from([chair_bone.to_string()]);
    let mut optional = BTreeSet::new();
    for m in movies {
        for p in &m.panels {
            match &p.bone_fallback {
                Some(fb) => {
                    optional.insert(p.bone.clone());
                    wanted.insert(fb.clone());
                }
                None => {
                    wanted.insert(p.bone.clone());
                }
            }
        }
        wanted.extend(m.backgrounds.iter().map(|b| b.bone.clone()));
        wanted.extend(m.component.iter().filter_map(|c| c.focus_bone.clone()));
    }
    wanted.extend(HEAD_LOOK_JOINTS.iter().map(|(_, b)| b.to_string()));
    for h in head_look {
        if let Some(bone) = h.joint.as_deref().and_then(|j| j.strip_prefix(CHAIR_RIG_NAME)) {
            wanted.insert(bone.to_string());
        }
    }
    let mut out = BTreeMap::new();
    for name in wanted.iter().chain(&optional) {
        let Some(b) = table.get(name) else {
            if optional.contains(name) {
                continue;
            }
            return Err(format!("rendering.json bones_elite_axes: no bone {name}"));
        };
        let frame = |which: &str| -> Result<Option<BoneFrame>, String> {
            let Some(f) = b.get(which) else { return Ok(None) };
            let bf = format!("rendering.json bones_elite_axes/{name}/{which}");
            let p = floats::<3>(f, "position", &bf)?;
            let rows = floats::<12>(f, "axes_rows", &bf)?;
            Ok(Some(bone_frame(p.map(f64::from), rows.map(f64::from))))
        };
        out.insert(name.clone(), BoneFrames { cockpit: frame("cockpit")?, chair_rig: frame("chair_rig")? });
    }
    Ok(out)
}

/// Every `HeadlookTargetBoneActivity` of the `cockpitui` state machine, by state.
fn head_look(csm: &Value) -> Result<Vec<HeadLook>, String> {
    let f = "cockpitui_state_machine.json";
    let mut out = Vec::new();
    for (state, s) in csm.get("states").and_then(Value::as_object).ok_or_else(|| format!("{f}: no states"))? {
        for a in s.get("activities").and_then(Value::as_array).into_iter().flatten() {
            if a.get("type").and_then(Value::as_str) != Some("HeadlookTargetBoneActivity") {
                continue;
            }
            let af = format!("{f} {state}");
            out.push(HeadLook {
                state: state.clone(),
                panel_component: opt_str(a, "panel_component"),
                joint: opt_str(a, "bone"),
                displacement_m: f32_at(a, "disp_m", &af)?,
                fov_channel_deg: f32_at(a, "fov_channel_deg", &af)?,
                displacement_m_alt: f32_at(a, "disp_m_alt", &af)?,
                fov_channel_deg_alt: f32_at(a, "fov_channel_deg_alt", &af)?,
            });
        }
    }
    for h in &out {
        if let Some(j) = &h.joint {
            if !j.starts_with(CHAIR_RIG_NAME) && !HEAD_LOOK_JOINTS.iter().any(|(n, _)| n == j) {
                return Err(format!("{f} {}: joint {j} is not among HEAD_LOOK_JOINTS", h.state));
            }
        }
    }
    Ok(out)
}

/// The gauge sheet's parts: `gaugepanel_atlas_layout`'s placements with `panel_models` `gauge_parts_stage_bounds`.
fn gauge_atlas(r: &Value) -> Result<GaugeAtlas, String> {
    let f = "rendering.json gaugepanel_atlas_layout";
    let a = ptr(r, "/gaugepanel_atlas_layout", f)?;
    let children = a.get("children").and_then(Value::as_array).ok_or_else(|| format!("{f}: no children"))?;
    let bounds = ptr(r, "/panel_models/gauge_parts_stage_bounds", "rendering.json")?.as_array().ok_or("rendering.json: gauge_parts_stage_bounds is not a list")?;
    if children.len() != bounds.len() {
        return Err(format!("{f}: {} parts, gauge_parts_stage_bounds {}", children.len(), bounds.len()));
    }
    let mut parts = Vec::new();
    for (c, b) in children.iter().zip(bounds) {
        let instance = opt_str(c, "instance");
        if opt_str(b, "instance") != instance {
            return Err(format!("{f}: part {instance:?}'s bounds are not beside it"));
        }
        parts.push(GaugePart {
            class: opt_str(c, "symbol_class"),
            char_id: u32_at(c, "char_id", f)?,
            x: f32_at(c, "x", f)?,
            y: f32_at(c, "y", f)?,
            scale: [f32_at(c, "scale_x", f)?, f32_at(c, "scale_y", f)?],
            depth: u32_at(c, "depth", f)?,
            bounds: [f32_at(b, "x0", f)?, f32_at(b, "y0", f)?, f32_at(b, "x1", f)?, f32_at(b, "y1", f)?],
            instance,
        });
    }
    Ok(GaugeAtlas { stage: floats::<2>(a, "stage", f)?.map(|v| v as u32), parts })
}

/// The target sights' zones from `CockpitGunsightComponent`'s words: zone k is its offset at byte 96 + 24k, cell size at
/// 104 + 24k and counts at 112 + 24k. Zone 0's offset (byte 96) is not among the export's words: the stand-in (0, 0).
fn sight_zones(r: &Value) -> Result<Vec<SightZone>, String> {
    let f = "rendering.json targetsights_atlas";
    let cells = ptr(r, "/targetsights_atlas/cells", f)?.as_array().ok_or_else(|| format!("{f}: cells is not a list"))?;
    let word = |at: u64, key: &str| -> Option<Vec<f64>> {
        cells.iter().find(|c| c.get("at").and_then(Value::as_u64) == Some(at))?.get(key)?.as_array().map(|v| v.iter().filter_map(Value::as_f64).collect())
    };
    let pair = |at: u64, key: &str| -> Result<[f64; 2], String> { word(at, key).and_then(|v| v.try_into().ok()).ok_or_else(|| format!("{f}: no {key} pair at byte {at}")) };
    (0..3u64)
        .map(|k| {
            let offset = if k == 0 { [0.0, 0.0] } else { pair(96 + 24 * k, "f32")? };
            let counts = pair(112 + 24 * k, "u32")?;
            let size = pair(104 + 24 * k, "f32")?;
            Ok(SightZone { cell_uv: size.map(|v| v as f32), counts: counts.map(|v| v as u32), offset_uv: offset.map(|v| v as f32) })
        })
        .collect()
}

fn shading(r: &Value) -> Result<Shading, String> {
    let rows = ptr(r, "/gui_colour/matrix_default_rows", "rendering.json")?.as_array().filter(|x| x.len() == 4).ok_or("rendering.json: gui_colour's matrix is not 4 rows")?;
    let mut matrix = [[0f32; 3]; 4];
    for (k, row) in rows.iter().enumerate() {
        let v: Vec<f32> = row.as_array().into_iter().flatten().filter_map(Value::as_f64).map(|x| x as f32).collect();
        matrix[k] = v.try_into().map_err(|_| format!("rendering.json: gui_colour row {k} is not 3 numbers"))?;
    }
    let rt = ptr(r, "/shader_assignment_corrected/screen_space_gui_rt", "rendering.json")?.as_str().unwrap_or("");
    if !rt.starts_with(SCREEN_TARGET.1) {
        return Err(format!("rendering.json: the screen-space GUI target is {rt:?}, not {}", SCREEN_TARGET.1));
    }
    let mut constants = Vec::new();
    for (at, name, values, words) in SHADER_CONSTANTS {
        let src = serde_json::to_string(ptr(r, at, "rendering.json")?).map_err(|e| e.to_string())?;
        if !src.contains(words) {
            return Err(format!("rendering.json {at} does not say {words:?}"));
        }
        constants.push(ShaderConstant { formula: at.to_string(), name: name.to_string(), values: values.to_vec(), text: words.to_string() });
    }
    Ok(Shading { gui_colour_matrix: matrix, screen_target: SCREEN_TARGET.0, constants })
}

/// How far a panel quad's fourth corner lies off the plane of its first three, metres (Elite's axes or the mod's alike).
fn off_plane(q: &PanelQuad) -> f32 {
    let p = |k: usize| bevy::math::Vec3::from_slice(&q.vertices[k][..3]);
    let n = (p(1) - p(0)).cross(p(2) - p(0)).normalize();
    (p(3) - p(0)).dot(n).abs()
}

fn stand_ins(r: &Value, movies: &[MovieLayout]) -> Result<Vec<StandIn>, String> {
    let open = ptr(r, "/brightness_path/open", "rendering.json")?.as_str().unwrap_or("").to_string();
    let quads: Vec<String> = movies.iter().filter_map(|m| m.quad.as_ref().map(|q| format!("{} on {}", m.name, q.bone))).collect();
    let comms = movies.iter().find(|m| m.name == "cockpitchatpanel").and_then(|m| m.quad.as_ref()).ok_or("cockpitchatpanel has no quad")?;
    Ok(vec![
        StandIn {
            what: "the bone each panel quad's corners are in".into(),
            value: format!("its component's one GUI panel's bone: {}", quads.join(", ")),
            grade: "B".into(),
            why: "the record holds the quad in the panel block beside its GUI panels, without a bone of its own (ui_rendering.md 2.1); the left quad                   projected through the chair rig's !Left_Panel fits the user's picture to 4.65 px rms (2.2, C)"
                .into(),
        },
        StandIn {
            what: "the panel quads: what they are".into(),
            value: "the panel's pointer surface, not drawn (the game draws the gui_* panel model); its triangles the data's 0 3 2 1 0 3 turned as \
                    the ship's models are (0 2 3 1 3 0): the first faces the pilot, the second away"
                .into(),
            grade: "B".into(),
            why: "Elite's index list winds its two triangles opposite ways (0,3,2 clockwise and 1,0,3 counter-clockwise seen from the front, in \
                  all four quads), so it cannot be a culled draw; how Elite uses the quad is not decoded"
                .into(),
        },
        StandIn {
            what: "the comms panel quad's fourth corner (bottom-right)".into(),
            value: format!("as Elite's data gives it, {:.3} m off the plane of the other three", off_plane(comms)),
            grade: "A".into(),
            why: "its z is the side panels' corner constant -0.139237: a slip in Elite's own data (ui_rendering.md 2.2); the comms panel is drawn on                   the gui_comms_panel model, whose aspect is its 512 x 767 stage's"
                .into(),
        },
        StandIn {
            what: "each panel movie's render-target size".into(),
            value: "its stage size".into(),
            grade: "B".into(),
            why: "no texture-create call was found for a cockpit movie; the panel quads' aspects match their movies' stage aspects, but for the role                   switch's quad (aspect 1.7454, 1152:660, against its 1056 x 660 stage) and the fuel scoop's, which break it unexplained                   (ui_rendering.md 2.5)"
                .into(),
        },
        StandIn {
            what: "the device factor of gGuiBrightness = DashboardGUIBrightness x 1 x the device's block+0x36bc".into(),
            value: "1.0".into(),
            grade: "E".into(),
            why: open,
        },
        StandIn {
            what: "a GUI panel whose bone the model lacks hangs on its fallback bone".into(),
            value: "Gui_Gauges_Panel on !Radar (!Gauges_Panel is not in the Sidewinder's cockpit)".into(),
            grade: "B".into(),
            why: "inferred from the record's fallback field (ui_rendering.md 2.1)".into(),
        },
        StandIn {
            what: "the target sights' zone 0 offset".into(),
            value: "(0, 0)".into(),
            grade: "B".into(),
            why: "its word (byte 96 of CockpitGunsightComponent) is not among the export's; the notes give (0, 0), and the zones' cells fill y 0..1152 of the 2016 px sheet (ui_rendering.md 2.4)".into(),
        },
    ])
}

/// The layout and the files it names (models and textures), from the export alone; the textures no export holds.
fn plan(ctx: &Ctx) -> Result<(Vec<Planned>, Layout, BTreeSet<(String, String, String)>), String> {
    let dec = &ctx.ui_decoded;
    let r = read_json(&dec.join("rendering.json"))?;
    let panels = read_json(&dec.join("panels_sidewinder_cockpit.json"))?;
    if Some(&panels) != r.get("sidewinder_panel_components") {
        return Err("panels_sidewinder_cockpit.json is not rendering.json's sidewinder_panel_components".into());
    }
    let hud3d = dec.join("hud3d");
    let textures = read_json(&hud3d.join("textures.json"))?;
    let hud_materials = read_json(&hud3d.join("materials.json"))?;
    let mut mats = Materials::default();
    let movies = MOVIES.iter().map(|n| movie(&r, n, &mut mats, &hud3d, &textures)).collect::<Result<Vec<_>, _>>()?;
    let stems: BTreeSet<&String> = movies.iter().flat_map(|m| m.panels.iter().map(|p| &p.model).chain(m.backgrounds.iter().map(|b| &b.model)).chain(&m.flat_quads)).collect();
    let mut files = Vec::new();
    let mut models = BTreeMap::new();
    for stem in stems {
        let (edm, planned) = ui_models::model(&hud3d, stem)?;
        for m in &edm.materials {
            let rec = hud_materials.get(m.name.to_lowercase()).ok_or_else(|| format!("hud3d/materials.json: no material {}", m.name))?;
            mats.add(&hud3d, &textures, &m.name, rec, "hud3d/materials.json")?;
        }
        models.insert(stem.clone(), gui_model(&r, stem, &edm)?);
        files.push(planned);
    }
    for m in &movies {
        if m.kind == MovieKind::Orbital {
            let want = ptr(&r, &format!("/movies/{}/cockpit_component/material", m.name), "rendering.json")?.as_str().unwrap_or("").to_lowercase();
            if m.flat_quads.iter().any(|q| models[q].objects.iter().any(|o| o.material != want)) {
                return Err(format!("{}: its quad's material is not the component's {want}", m.name));
            }
        }
    }
    let csm = read_json(&dec.join("cockpitui_state_machine.json"))?;
    let head_look = head_look(&csm)?;
    let chair = ptr(&r, "/chair_rig_attachment", "rendering.json")?;
    let chair_rig = ChairRig { bone: str_at(chair, "chair_bone", "rendering.json chair_rig_attachment")?, offset: point(floats::<3>(chair, "offset_in_cockpit_elite_axes", "rendering.json chair_rig_attachment")?) };
    let layout = Layout {
        format: LAYOUT_FORMAT,
        bones: bones(&r, &movies, &head_look, &chair_rig.bone)?,
        head_look_joints: HEAD_LOOK_JOINTS.iter().map(|(j, b)| JointBone { joint: j.to_string(), bone: b.to_string() }).collect(),
        head_look,
        chair_rig,
        gauge_atlas: gauge_atlas(&r)?,
        sight_zones: sight_zones(&r)?,
        shading: shading(&r)?,
        stand_ins: stand_ins(&r, &movies)?,
        movies,
        models,
        materials: mats.table,
    };
    let bytes = write_layout(&layout);
    if parse_layout(&bytes).map_err(|e| e.to_string())? != layout {
        return Err("layout.json reads back different".into());
    }
    files.push(Planned {
        path: LAYOUT.into(),
        note: format!(
            "How the {} movies sit in the cockpit: their stages, components, GUI panels and plates with their bones and models, the panel quads, \
             {} panel models with their uv0 rectangles and islands, {} materials with every attribute, {} bones, the chair rig, the head-look \
             targets, the gauge sheet, the target sights' zones and the shaders' constants. Elite's numbers, positions in the mod's axes \
             (z mirrored), the quads' triangles turned to match. Stand-ins (the notes' grades): {}.",
            layout.movies.len(),
            layout.models.len(),
            layout.materials.len(),
            layout.bones.len(),
            layout.stand_ins.iter().map(|s| format!("{}: {} ({})", s.what, s.value, s.grade)).collect::<Vec<_>>().join("; ")
        ),
        bytes,
        sources: vec![
            "decoded/rendering.json".into(),
            "decoded/panels_sidewinder_cockpit.json".into(),
            "decoded/cockpitui_state_machine.json".into(),
            "decoded/hud3d/materials.json, textures.json".into(),
            "re/elite/notes/ui_rendering.md sections 2 to 4 (the head-look joints' names: 4.3)".into(),
        ],
        grade: "B",
    });
    files.extend(mats.textures.into_values());
    Ok((files, layout, mats.missing))
}

pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    let (files, layout, missing) = plan(ctx)?;
    let tex: usize = files.iter().filter(|f| f.path.starts_with("ui/tex/")).count();
    let total: usize = files.iter().map(|f| f.bytes.len()).sum();
    Planned::ship(ctx, shipped, files)?;
    let gaps: Vec<String> = missing.iter().map(|(t, m, why)| format!("{t} (material {m}: {why})")).collect();
    shipped.note(&format!(
        "The ship UI: not shipped, as no export holds them whole: the textures {}. Not shipped either: the UIModuleManager's per-panel scales and \
         tuning block (what they multiply was not found, E), the multicrew seats' alternative chairs, and the movies out of A10's scope.",
        gaps.join(", ")
    ));
    println!(
        "ui layout: {} movies, {} models, {} materials, {tex} textures, {} bones, {:.1} MB; textures no export holds: {}",
        layout.movies.len(),
        layout.models.len(),
        layout.materials.len(),
        layout.bones.len(),
        total as f64 / 1048576.0,
        gaps.join(", ")
    );
    Ok(())
}

/// The folder's layout, panel models and UI textures are what the export makes. Writes nothing.
pub(super) fn verify(ctx: &Ctx) -> Result<(), String> {
    let (files, ..) = plan(ctx)?;
    Planned::same(ctx, &files, &["ui/models", "ui/tex"])?;
    println!("ui layout: {} files match the UI export", files.len());
    Ok(())
}

/// The written layout as the game reads it: every model's file holds its objects' draws with their materials, and every
/// texture it names is there.
pub(super) fn check(ctx: &Ctx, bytes: &[u8]) -> Result<(), String> {
    let l = parse_layout(bytes).map_err(|e| e.to_string())?;
    for (stem, m) in &l.models {
        let edm = parse_edm(&ctx.read(&m.file)?).map_err(|e| format!("{}: {e}", m.file))?;
        let mats: Vec<String> = edm.draws.iter().map(|d| edm.materials[d.material as usize].name.to_lowercase()).collect();
        if mats != m.objects.iter().map(|o| o.material.clone()).collect::<Vec<_>>() {
            return Err(format!("model {stem}: its objects are not {}'s draws", m.file));
        }
    }
    for t in l.materials.values().flat_map(|m| &m.textures) {
        if let GuiSlotValue::Tex { file, .. } = &t.value {
            ctx.read(file)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `!Left_Panel` in the cockpit model (`rendering.json` `bones_elite_axes`), worked by hand: the mirror negates z of
    /// the position and of the x and y axes, and negates the z axis' x and y (its own direction flips with the mirror).
    #[test]
    fn a_bone_turns_into_the_mods_axes() {
        let rows = [0.347397, 0.003415, 0.937712, 0.0, -0.006561, 0.999977, -0.001211, 0.0, -0.937695, -0.005732, 0.347412, 0.0];
        let b = bone_frame([-1.04593, -0.390667, 0.577033], rows);
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(close(b.position, [-1.04593, -0.390667, -0.577033]), "{:?}", b.position);
        assert!(close(b.axes[0], [0.347397, 0.003415, -0.937712]), "{:?}", b.axes[0]);
        assert!(close(b.axes[1], [-0.006561, 0.999977, 0.001211]), "{:?}", b.axes[1]);
        assert!(close(b.axes[2], [0.937695, 0.005732, 0.347412]), "{:?}", b.axes[2]);
        // The left quad's first corner, placed by Elite's frame then mirrored, lands where the mod's frame puts the mirrored corner.
        let v = [-0.539922f64, 0.259209, -0.053418];
        let elite = DVec3::new(rows[0], rows[1], rows[2]) * v[0] + DVec3::new(rows[4], rows[5], rows[6]) * v[1] + DVec3::new(rows[8], rows[9], rows[10]) * v[2] + DVec3::new(-1.04593, -0.390667, 0.577033);
        let m = point([v[0] as f32, v[1] as f32, v[2] as f32]);
        let ours: [f32; 3] = std::array::from_fn(|k| b.position[k] + b.axes[0][k] * m[0] + b.axes[1][k] * m[1] + b.axes[2][k] * m[2]);
        assert!(close(ours, point(elite.as_vec3().to_array())), "{ours:?}");
    }

    fn shipped_layout() -> Layout {
        parse_layout(&std::fs::read(super::super::out_dir().join(LAYOUT)).unwrap()).unwrap()
    }

    /// Each shipped panel quad, placed in the cockpit model's space by its bone (the chair rig's at rest: plus the rig's
    /// offset), winds as Elite's data does once turned into the mod's right-handed axes: its first triangle
    /// counter-clockwise seen from the pilot's eye (`!Camera`), its second clockwise. A change in the converter shows here.
    #[test]
    fn shipped_quads_wind_as_elites_data() {
        use bevy::math::Vec3;
        let l = shipped_layout();
        let eye = Vec3::from(l.bone_frame("!Camera", Rig::Cockpit).unwrap().position);
        let mut seen = 0;
        for m in &l.movies {
            let Some(q) = &m.quad else { continue };
            let f = l.bone_frame(&q.bone, q.rig).unwrap();
            let origin = Vec3::from(f.position) + if q.rig == Rig::Chair { Vec3::from(l.chair_rig.offset) } else { Vec3::ZERO };
            let corner = |k: usize| origin + Vec3::from(f.axes[0]) * q.vertices[k][0] + Vec3::from(f.axes[1]) * q.vertices[k][1] + Vec3::from(f.axes[2]) * q.vertices[k][2];
            for (k, t) in q.indices.chunks_exact(3).enumerate() {
                let (a, b, c) = (corner(t[0] as usize), corner(t[1] as usize), corner(t[2] as usize));
                let facing = (b - a).cross(c - a).dot(eye - a) > 0.0;
                assert_eq!(facing, k == 0, "{}: triangle {t:?} should {}face the pilot", m.name, if k == 0 { "" } else { "not " });
            }
            seen += 1;
        }
        assert_eq!(seen, 4, "the left, right, role and comms quads");
    }

    /// The shipped head-look targets are `ui_rendering.md` section 4.3's table: state, joint, displacement (m), field-of-view
    /// channel normal and second (degrees).
    #[test]
    fn shipped_head_look_is_the_notes_table() {
        let want: &[(&str, Option<&str>, f32, f32)] = &[
            ("Comms Panel Full", Some("CommsPanel_Focus"), -8.0, -5.0),
            ("External Panel", Some("LeftPanel_Focus"), -17.0, -13.0),
            ("Internal Panel", Some("RightPanel_Focus"), -17.0, -13.0),
            ("Radar Panel", Some("RadarPanel_Focus"), -20.0, -20.0),
            ("Role Switch", Some("CockpitChair_Bone_Rig!RoleSwitch_Panel"), -17.0, -17.0),
            ("Select UI Mode", None, 10.0, 0.0),
        ];
        let l = shipped_layout();
        let got: Vec<(&str, Option<&str>, f32, f32)> = l.head_look.iter().map(|h| (h.state.as_str(), h.joint.as_deref(), h.fov_channel_deg, h.fov_channel_deg_alt)).collect();
        assert_eq!(got, want);
        assert!(l.head_look.iter().all(|h| h.displacement_m == 0.0 && h.displacement_m_alt == 0.0));
    }

    #[test]
    fn bounds_swap_the_mirrored_z() {
        assert_eq!(bounds([-1.0, -2.0, -0.5], [1.0, 2.0, 0.25]), ([-1.0, -2.0, -0.25], [1.0, 2.0, 0.5]));
    }

    /// Each shipped panel model's extent (x, y) is its size in `ui_rendering.md` section 2.5's table (metres, 3 decimals).
    #[test]
    fn panel_models_have_the_notes_sizes() {
        const SIZES: &[(&str, f32, f32)] = &[
            ("gui_alert_panel", 0.464, 0.040),
            ("gui_cockpitschematic_panel", 0.627, 0.318),
            ("gui_cockpitschematic_panel_background", 0.523, 0.000),
            ("gui_comms_default_panel_background", 0.292, 0.065),
            ("gui_comms_extended_panel_background", 0.294, 0.184),
            ("gui_comms_full_panel_background", 0.320, 0.467),
            ("gui_comms_panel", 0.250, 0.374),
            ("gui_gauges_panel", 1.086, 0.264),
            ("gui_info_default_panel_background", 0.314, 0.068),
            ("gui_info_extended_panel_background", 0.320, 0.197),
            ("gui_info_panel", 0.283, 0.157),
            ("gui_left_panel", 0.870, 0.489),
            ("gui_left_panel_background", 1.078, 0.639),
            ("gui_lrail_panel", 0.756, 0.652),
            ("gui_mc_gauges", 1.732, 0.267),
            ("gui_mc_gauges_background", 1.943, 0.280),
            ("gui_minileft_default_panel_background", 0.472, 0.274),
            ("gui_minileft_full_panel_background", 0.472, 0.398),
            ("gui_minileft_panel", 0.401, 0.338),
            ("gui_minileft_target_location_background", 0.472, 0.333),
            ("gui_minileft_target_panel_background", 0.472, 0.255),
            ("gui_minileft_targetsub_panel_background", 0.472, 0.405),
            ("gui_miniright_panel", 0.392, 0.261),
            ("gui_miniright_panel_background", 0.484, 0.483),
            ("gui_radar_panel_background", 1.122, 0.000),
            ("gui_right_panel", 0.870, 0.489),
            ("gui_right_panel_background", 1.075, 0.637),
            ("gui_roleswitch_panel", 0.520, 0.298),
            ("gui_roleswitch_panel_background", 0.628, 0.419),
            ("gui_rrail_panel", 0.754, 0.652),
            ("gui_targetschematic_panel", 0.373, 0.310),
            ("gui_targetschematic_panel_background", 0.523, 0.000),
            ("gui_warning_panel", 0.239, 0.118),
            ("gui_warning_panel_background", 0.349, 0.139),
            ("ontopquad", 1.000, 0.000),
            ("ontopquadstenciled", 1.000, 0.000),
            ("orbitalhudquad", 10.000, 0.000),
            ("quad", 1.000, 0.000),
        ];
        let dir = super::super::out_dir().join("ui").join("models");
        let shipped: BTreeSet<String> = std::fs::read_dir(&dir).unwrap().flatten().filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".edm").map(str::to_string)).collect();
        assert_eq!(shipped, SIZES.iter().map(|s| s.0.to_string()).collect(), "the shipped models are the table's");
        for (stem, x, y) in SIZES {
            let edm = parse_edm(&std::fs::read(dir.join(format!("{stem}.edm"))).unwrap()).unwrap();
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for v in edm.draws.iter().flat_map(|d| &d.vertices) {
                let p = v.decode().position;
                for a in 0..2 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
            let size = [hi[0] - lo[0], hi[1] - lo[1]];
            assert!((size[0] - x).abs() <= 0.0006 && (size[1] - y).abs() <= 0.0006, "{stem}: {size:?}, the notes say {x} x {y}");
        }
    }
}
