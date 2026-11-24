//! The UI's materials' values (ARCHITECTURE.md A10 UI3): each GUI material's values from the layout
//! (`GuiMaterial::attrs`), its pipeline's blend and culling and its depth test from its own flags (`GuiFlags`), for the
//! UI's pass (`pass.rs`: the foreground panels with Elite's `GuiLighting`, tag `fg`, `PanelParams`; the dark plates with
//! `GuiBackground`, tag `bg`, `PlateParams`; both in `sidewinder_ui_pass.wesl`).

use bevy::math::Vec4;
use bevy::prelude::*;
use bevy::render::render_resource::{BlendComponent, BlendFactor, BlendOperation, BlendState, ColorTargetState, Face, PrimitiveState, ShaderType};
use se_core::elite_ship::Attr;

/// The user's `DashboardGUIBrightness` (`Custom.4.4.misc`, this PC's Elite setting, the layout's shading constant
/// `/user_settings`) times the device factor, whose value is not found (the layout's stand-in: 1.0, grade E):
/// `gGuiBrightness = options+0xa0 x block[+0x36bc]` (`ui_rendering.md` 3.3).
pub const GUI_BRIGHTNESS: f32 = 0.69 * 1.0;

/// The fit from Elite's GUI value (the shader's result with `gExposureCompensation` and `gHDRScalingWrite` at 1) to the
/// linear colour Elite's screen shows (the last step, ours, grade C): screen = A v^p / (v^p + B). Fitted on 14 channel
/// samples of session ui1's `a00_start.png` against the same movie's texels (the runtime's proof picture of the gauge sheet,
/// `.optimize/scratch/run/ui4a/parked_cockpitgaugepanel.png`): the text cores, the fuel bar, the pips' rows, the heat flame,
/// the cyan shield figures and throttle ticks; rms 2.1, at most 3.7 levels of 255 (the report gives every sample).
pub const SCREEN_FIT: [f32; 3] = [0.790, 1.84, 1.2336];

/// A material's blend, depth and culling, as its flags give them (Direct3D's states counted from 0, B: the notes read the
/// GUI materials' `pSrcBlend` 1, `pDestBlend` 5 as ONE, INV_SRC_ALPHA, premultiplied; so `pCullMode` 2 is BACK).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GuiFlags {
    blend: Option<(u8, u8)>,
    cull: u8,
    depth_test: bool,
    depth_write: bool,
}

fn attr<'a>(attrs: &'a [Attr], name: &str) -> Result<&'a Attr, String> {
    attrs.iter().find(|a| a.name == name).ok_or_else(|| format!("no attribute {name}"))
}

/// An attribute's first `n` values as a Vec4 (the rest 0).
fn attr4(attrs: &[Attr], name: &str, n: usize) -> Result<Vec4, String> {
    let a = attr(attrs, name)?;
    if a.values.len() < n {
        return Err(format!("{name} has {} values, not {n}", a.values.len()));
    }
    Ok(Vec4::from_array(std::array::from_fn(|i| a.values.get(i).filter(|_| i < n).copied().unwrap_or(0.0) as f32)))
}

fn attr1(attrs: &[Attr], name: &str) -> Result<f32, String> {
    Ok(attr4(attrs, name, 1)?.x)
}

impl GuiFlags {
    pub fn of(attrs: &[Attr]) -> Result<GuiFlags, String> {
        let state = |name: &str| -> Result<u8, String> {
            let v = attr1(attrs, name)?;
            (v >= 0.0 && v <= 255.0 && v.fract() == 0.0).then_some(v as u8).ok_or_else(|| format!("{name} is {v}"))
        };
        let blend = if state("pBlendEnable")? != 0 { Some((state("pSrcBlend")?, state("pDestBlend")?)) } else { None };
        let flags = GuiFlags { blend, cull: state("pCullMode")?, depth_test: state("pDepthEnable")? != 0, depth_write: state("pDepthWriteMask")? != 0 };
        if let Some((s, d)) = blend {
            factor(s).and(factor(d)).ok_or_else(|| format!("blend states {s}, {d} are not Direct3D's"))?;
        }
        if flags.cull > 2 {
            return Err(format!("cull mode {}", flags.cull));
        }
        Ok(flags)
    }

    /// The pipeline's culling and its colour target's blend. (`depth_write` has no use in the UI's pass, which has no depth
    /// attachment: every GUI material of the layout has it off, `the_shipped_materials_read`.)
    pub fn apply(self, primitive: &mut PrimitiveState, target: &mut ColorTargetState) {
        primitive.cull_mode = match self.cull {
            1 => Some(Face::Front),
            2 => Some(Face::Back),
            _ => None,
        };
        target.blend = self.blend.and_then(|(s, d)| {
            let c = BlendComponent { src_factor: factor(s)?, dst_factor: factor(d)?, operation: BlendOperation::Add };
            Some(BlendState { color: c, alpha: c })
        });
    }

    /// The material tests its depth against the scene's.
    pub fn depth_test(self) -> bool {
        self.depth_test
    }
}

/// Direct3D's blend factor `n`, counted from 0 (ZERO, ONE, SRC_COLOR, INV_SRC_COLOR, SRC_ALPHA, INV_SRC_ALPHA, DEST_ALPHA,
/// INV_DEST_ALPHA, DEST_COLOR, INV_DEST_COLOR).
fn factor(n: u8) -> Option<BlendFactor> {
    use BlendFactor as F;
    [F::Zero, F::One, F::Src, F::OneMinusSrc, F::SrcAlpha, F::OneMinusSrcAlpha, F::DstAlpha, F::OneMinusDstAlpha, F::Dst, F::OneMinusDst].get(n as usize).copied()
}

// ---- The panels (GuiLighting) ----

/// `sidewinder_ui_pass.wesl` `PanelParams`: the material's own values.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct PanelParams {
    /// rgb pGuiTint, w gStandardMapScales.x.
    pub tint: Vec4,
    /// x pCloudynessScale, y pSaturation, zw pCloudynessTile.
    pub cloud: Vec4,
    pub scrolls: Vec4,
    pub tint_a: Vec4,
    pub tint_b: Vec4,
    /// x pParallaxDesaturate, y pParallaxLerp, z pParallaxScale.
    pub parallax: Vec4,
    /// pDiffuse_AlphaMetrics as the material stores it.
    pub metrics: Vec4,
}

impl PanelParams {
    pub fn new(attrs: &[Attr]) -> Result<PanelParams, String> {
        let tile = attr4(attrs, "pCloudynessTile", 2)?;
        Ok(PanelParams {
            tint: attr4(attrs, "pGuiTint", 3)?.truncate().extend(attr1(attrs, "gStandardMapScales")?),
            cloud: Vec4::new(attr1(attrs, "pCloudynessScale")?, attr1(attrs, "pSaturation")?, tile.x, tile.y),
            scrolls: attr4(attrs, "pCloudynessScrollsAB", 4)?,
            tint_a: attr4(attrs, "pCloudynessTintA", 4)?,
            tint_b: attr4(attrs, "pCloudynessTintB", 4)?,
            parallax: Vec4::new(attr1(attrs, "pParallaxDesaturate")?, attr1(attrs, "pParallaxLerp")?, attr1(attrs, "pParallaxScale")?, 0.0),
            metrics: attr4(attrs, "pDiffuse_AlphaMetrics", 4)?,
        })
    }
}

// ---- The plates (GuiBackground) ----

/// `sidewinder_ui_pass.wesl` `PlateParams`: the material's own values.
///
/// The plate record's float (`Background::alpha`, 0.5 or 0.2) is NOT multiplied in: measured, it is no opacity (C). Over the
/// planet behind the info strip in the user's `1.webp` (the info movie draws nothing between its lines; its log is shown,
/// so the extended plate lies there too), Elite lets 0.003 to 0.069 of the planet's blue through (four places along the
/// strip, linear light). This shader with the materials' own values gives 0.26 for the default plate at its densest (alpha
/// (0.79 - 0.15) x 1.15) and 0.05 for the extended one (alpha (0.94 - 0.15) x 1.198), together 0.01 to 0.26; with the
/// records' 0.5 and 0.2 multiplied in, 0.63 and 0.81, together at least 0.5. So the float is the fade time
/// `ui_messages_hud.md` section 7 reads it as.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct PlateParams {
    /// rgb pGuiTint, w pScanlineScale.
    pub tint: Vec4,
    /// x pAlphaClip, y pBackgroundAlpha, z pBackgroundScale (the scene term's, which is off), w pDestAlphaScale.
    pub alpha: Vec4,
    /// x pCloudynessScale, zw pCloudynessTile.
    pub cloud: Vec4,
    pub scrolls: Vec4,
    /// xy pScanlineTile.
    pub scan: Vec4,
}

impl PlateParams {
    pub fn new(attrs: &[Attr]) -> Result<PlateParams, String> {
        let tile = attr4(attrs, "pCloudynessTile", 2)?;
        Ok(PlateParams {
            tint: attr4(attrs, "pGuiTint", 3)?.truncate().extend(attr1(attrs, "pScanlineScale")?),
            alpha: Vec4::new(attr1(attrs, "pAlphaClip")?, attr1(attrs, "pBackgroundAlpha")?, attr1(attrs, "pBackgroundScale")?, attr1(attrs, "pDestAlphaScale")?),
            cloud: Vec4::new(attr1(attrs, "pCloudynessScale")?, 0.0, tile.x, tile.y),
            scrolls: attr4(attrs, "pCloudynessScrollsAB", 4)?,
            scan: attr4(attrs, "pScanlineTile", 2)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str, v: &[f64]) -> Attr {
        Attr { name: name.into(), kind: 5, values: v.to_vec() }
    }

    /// The GUI materials' flags (blend 1 / 5, cull 2, depth on, no write) are a premultiplied blend, back faces culled.
    #[test]
    fn flags_are_the_materials_own() {
        let attrs = [a("pBlendEnable", &[1.0]), a("pSrcBlend", &[1.0]), a("pDestBlend", &[5.0]), a("pCullMode", &[2.0]), a("pDepthEnable", &[1.0]), a("pDepthWriteMask", &[0.0])];
        let f = GuiFlags::of(&attrs).unwrap();
        assert_eq!(f, GuiFlags { blend: Some((1, 5)), cull: 2, depth_test: true, depth_write: false });
        assert_eq!((factor(1), factor(5)), (Some(BlendFactor::One), Some(BlendFactor::OneMinusSrcAlpha)));
        let mut bad = attrs.clone();
        bad[1] = a("pSrcBlend", &[12.0]);
        assert!(GuiFlags::of(&bad).is_err());
        assert!(GuiFlags::of(&attrs[1..]).is_err(), "a missing flag is refused");
    }

    /// Every fg and bg material the shipped layout holds gives its values and flags.
    #[test]
    fn the_shipped_materials_read() {
        let bytes = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/sidewinder/ui/layout.json")).unwrap();
        let layout = se_core::elite_ui_layout::parse_layout(&bytes).unwrap();
        use se_core::elite_ui_layout::MaterialTag;
        for (name, m) in &layout.materials {
            match m.tag {
                MaterialTag::Foreground => PanelParams::new(&m.attrs).map(|_| ()),
                MaterialTag::Background => PlateParams::new(&m.attrs).map(|_| ()),
                _ => continue,
            }
            .and_then(|_| GuiFlags::of(&m.attrs))
            .map(|f| assert!(!f.depth_write, "{name} writes depth: the UI's pass has no depth attachment"))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}
