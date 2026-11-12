//! Half-Life 2's sound scripts, read from the Garry's Mod install: which wave files a named sound plays, how loud, at what
//! pitch, and how far it carries (`scripts/game_sounds_*.txt` through `game_sounds_manifest.txt`), and what each surface
//! sounds like under a foot, a bullet or a falling prop (`scripts/surfaceproperties*.txt`). The sandbox names its sounds
//! as the game's code does (`Weapon_Pistol.Single`, `Metal_Barrel.ImpactHard`, `Grass.StepLeft`), so what plays is what
//! Half-Life 2 plays, not a guess. Without the game `fallback` has the handful the sandbox uses, with plain synthesised
//! waves behind them (`assets.rs` `placeholder_sound`).

use std::collections::HashMap;

use se_core::gmod::{GmodInstall, KeyValues, kv_text, parse_keyvalues};

/// A named sound.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundEntry {
    /// Paths under `sound/`.
    pub waves: Vec<String>,
    /// The least and the most it plays at (0..1), and its pitch (1: as recorded).
    pub volume: (f32, f32),
    pub pitch: (f32, f32),
    /// Its sound level in dB (`SNDLVL_*`): how far it carries. 0: heard everywhere at full volume.
    pub level: f32,
    /// Its channel (`CHAN_*`): on any but 0 (auto) and 6 (static) a new sound from the same thing cuts the last one
    /// off. That is the SMG's rattle: each shot ends the one before.
    pub channel: u8,
}

pub const CHAN_AUTO: u8 = 0;
pub const CHAN_STATIC: u8 = 6;

/// What a surface sounds like and how it slides and bounces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceEntry {
    pub step_left: String,
    pub step_right: String,
    pub impact_hard: String,
    pub impact_soft: String,
    pub bullet_impact: String,
    /// `gamematerial`: the letter the step's loudness goes by (D dirt, V vent, the rest alike).
    pub material: char,
}

#[derive(Default)]
pub struct SoundScripts {
    entries: HashMap<String, SoundEntry>,
    surfaces: HashMap<String, SurfaceEntry>,
    /// Per named sound, the waves played since all were last free (a bit each): `pick`.
    played: HashMap<String, u64>,
}

/// `CHAN_*` by name (`soundflags.h`).
fn channel_of(v: &str) -> u8 {
    match v.trim().to_ascii_uppercase().as_str() {
        "CHAN_WEAPON" => 1,
        "CHAN_VOICE" => 2,
        "CHAN_ITEM" => 3,
        "CHAN_BODY" => 4,
        "CHAN_STREAM" => 5,
        "CHAN_STATIC" => CHAN_STATIC,
        "CHAN_VOICE2" => 7,
        _ => CHAN_AUTO,
    }
}

/// `SNDLVL_*` by name (`soundflags.h`).
fn level_of(v: &str) -> f32 {
    let v = v.trim();
    let named = match v.to_ascii_uppercase().as_str() {
        "SNDLVL_NONE" => Some(0.0),
        "SNDLVL_IDLE" => Some(60.0),
        "SNDLVL_STATIC" => Some(66.0),
        "SNDLVL_NORM" => Some(75.0),
        "SNDLVL_TALKING" => Some(80.0),
        "SNDLVL_GUNFIRE" => Some(140.0),
        _ => None,
    };
    if let Some(l) = named {
        return l;
    }
    // `SNDLVL_85dB`, or a bare number.
    let digits: String = v.trim_start_matches(|c: char| !c.is_ascii_digit()).chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    digits.parse().unwrap_or(75.0)
}

/// `"0.7"`, `"0.5,0.8"`, `"VOL_NORM"`, `"PITCH_NORM"`, `"95,105"`: the least and the most.
fn range_of(v: &str, names: &[(&str, f32)], default: f32) -> (f32, f32) {
    let one = |s: &str| -> f32 {
        let s = s.trim();
        names.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, x)| *x).or_else(|| s.parse().ok()).unwrap_or(default)
    };
    match v.split_once(',') {
        Some((a, b)) => {
            let (a, b) = (one(a), one(b));
            (a.min(b), a.max(b))
        }
        None => (one(v), one(v)),
    }
}

/// A wave's name without the characters Source puts in front of it (`^` far variant, `)` spatial stereo, `*` stream...).
fn wave_path(v: &str) -> String {
    v.trim().trim_start_matches(['*', '#', '@', '>', '<', '^', ')', '(', '}', '$', '!', '?']).replace('\\', "/").to_ascii_lowercase()
}

fn entry_of(block: &[KeyValues]) -> Option<SoundEntry> {
    let mut waves: Vec<String> = block.iter().filter(|k| k.key.eq_ignore_ascii_case("wave")).filter_map(|k| k.text()).map(wave_path).collect();
    for k in block.iter().filter(|k| k.key.eq_ignore_ascii_case("rndwave")) {
        if let Some(list) = k.list() {
            waves.extend(list.iter().filter(|k| k.key.eq_ignore_ascii_case("wave")).filter_map(|k| k.text()).map(wave_path));
        }
    }
    waves.retain(|w| !w.is_empty());
    if waves.is_empty() {
        return None;
    }
    let volume = kv_text(block, "volume").map_or((1.0, 1.0), |v| range_of(v, &[("VOL_NORM", 1.0)], 1.0));
    let pitch = kv_text(block, "pitch").map_or((100.0, 100.0), |v| range_of(v, &[("PITCH_NORM", 100.0), ("PITCH_LOW", 95.0), ("PITCH_HIGH", 120.0)], 100.0));
    let level = kv_text(block, "soundlevel").map_or(75.0, level_of);
    let channel = kv_text(block, "channel").map_or(CHAN_AUTO, channel_of);
    Some(SoundEntry { waves, volume: (volume.0.clamp(0.0, 1.0), volume.1.clamp(0.0, 1.0)), pitch: (pitch.0 / 100.0, pitch.1 / 100.0), level, channel })
}

impl SoundScripts {
    /// Every sound and surface the game's manifests name. A file that is missing or can't be read is skipped.
    pub fn load(install: &GmodInstall) -> Self {
        let text = |path: &str| install.read(path).map(|b| String::from_utf8_lossy(&b).into_owned());
        let listed = |manifest: &str| -> Vec<String> {
            let Some(t) = text(manifest) else { return Vec::new() };
            parse_keyvalues(&t).iter().filter_map(|k| k.list()).flatten().filter_map(|k| k.text()).map(|p| p.replace('\\', "/").to_ascii_lowercase()).collect()
        };
        let mut out = SoundScripts::default();
        // (Later files don't replace earlier ones' sounds, as the engine warns of a duplicate and keeps the first.)
        for file in listed("scripts/game_sounds_manifest.txt") {
            let Some(t) = text(&file) else { continue };
            for k in parse_keyvalues(&t) {
                let Some(e) = k.list().and_then(entry_of) else { continue };
                out.entries.entry(k.key.to_ascii_lowercase()).or_insert(e);
            }
        }
        // Surfaces: a block starts as the one it is based on, named before it, else as "default" (the engine's rule: dirt
        // names no soft impact, and takes the default's).
        let mut raw: Vec<(String, Vec<KeyValues>)> = Vec::new();
        let mut files = listed("scripts/surfaceproperties_manifest.txt");
        if files.is_empty() {
            files.push("scripts/surfaceproperties.txt".to_string());
        }
        for file in files {
            let Some(t) = text(&file) else { continue };
            for k in parse_keyvalues(&t) {
                if let Some(list) = k.list() {
                    raw.push((k.key.to_ascii_lowercase(), list.to_vec()));
                }
            }
        }
        for (name, block) in &raw {
            let base = kv_text(block, "base").map_or("default".to_string(), |b| b.to_ascii_lowercase());
            let mut e = out.surfaces.get(&base).or_else(|| out.surfaces.get("default")).cloned().unwrap_or_default();
            let set = |field: &mut String, key: &str| {
                if let Some(v) = kv_text(block, key) {
                    *field = v.to_string();
                }
            };
            set(&mut e.step_left, "stepleft");
            set(&mut e.step_right, "stepright");
            set(&mut e.impact_hard, "impacthard");
            set(&mut e.impact_soft, "impactsoft");
            set(&mut e.bullet_impact, "bulletimpact");
            if let Some(c) = kv_text(block, "gamematerial").and_then(|v| v.trim().chars().next()) {
                e.material = c.to_ascii_uppercase();
            }
            out.surfaces.entry(name.clone()).or_insert(e);
        }
        out
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.entries.len(), self.surfaces.len())
    }

    /// A sound by its name (any case): the game's, else the sandbox's own for a computer without it.
    pub fn get(&self, name: &str) -> Option<SoundEntry> {
        self.entries.get(&name.to_ascii_lowercase()).cloned().or_else(|| fallback(name))
    }

    /// Which of a named sound's `n` waves plays now: each once before any plays again, as the game's emitter picks
    /// them (its `available` marks), so a step is never the same wave three times running.
    pub fn pick(&mut self, name: &str, n: usize, roll: usize) -> usize {
        let n = n.clamp(1, 64);
        let played = self.played.entry(name.to_ascii_lowercase()).or_insert(0);
        let all = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
        if *played & all == all {
            *played = 0;
        }
        let free: Vec<usize> = (0..n).filter(|i| *played & (1 << i) == 0).collect();
        let i = free[roll % free.len()];
        *played |= 1 << i;
        i
    }

    /// A surface by its `surfaceprop` name.
    pub fn surface(&self, name: &str) -> SurfaceEntry {
        self.surfaces.get(&name.to_ascii_lowercase()).cloned().unwrap_or_else(|| fallback_surface(name))
    }
}

fn numbered(stem: &str, numbers: &[u32]) -> Vec<String> {
    numbers.iter().map(|n| format!("{stem}{n}.wav")).collect()
}

/// The sounds the sandbox names, as Half-Life 2's scripts have them, for when the scripts can't be read (the game is not
/// installed: the waves are then synthesised by their names).
fn fallback(name: &str) -> Option<SoundEntry> {
    let lower = name.to_ascii_lowercase();
    // (The channels the scripts give them: a weapon's shot on the weapon's, its other noises on the item's, steps on the body's.)
    let shot = [".single", ".double", ".launch", ".dryfire", ".melee_hit", ".melee_hitworld"].iter().any(|s| lower.ends_with(s));
    let channel = if lower.starts_with("weapon_") {
        if shot { 1 } else { 3 }
    } else if lower.contains(".step") {
        4
    } else {
        CHAN_AUTO
    };
    let e = |waves: Vec<String>, volume: f32, pitch: (f32, f32), level: f32| Some(SoundEntry { waves, volume: (volume, volume), pitch: (pitch.0 / 100.0, pitch.1 / 100.0), level, channel });
    let one = |w: &str| vec![w.to_string()];
    match lower.as_str() {
        "weapon_pistol.single" => e(one("weapons/pistol/pistol_fire2.wav"), 0.55, (98.0, 102.0), 140.0),
        "weapon_pistol.reload" => e(one("weapons/pistol/pistol_reload1.wav"), 0.7, (100.0, 100.0), 75.0),
        "weapon_pistol.empty" => e(one("weapons/pistol/pistol_empty.wav"), 0.7, (100.0, 100.0), 75.0),
        "weapon_smg1.single" => e(one("weapons/smg1/smg1_fire1.wav"), 0.55, (95.0, 105.0), 140.0),
        "weapon_smg1.reload" => e(one("weapons/smg1/smg1_reload.wav"), 0.7, (100.0, 100.0), 75.0),
        "weapon_smg1.empty" => e(one("weapons/smg1/smg_empty.wav"), 0.7, (100.0, 100.0), 75.0),
        "weapon_shotgun.single" => e(one("weapons/shotgun/shotgun_fire7.wav"), 0.86, (98.0, 101.0), 140.0),
        "weapon_shotgun.double" => e(one("weapons/shotgun/shotgun_dbl_fire7.wav"), 1.0, (90.0, 95.0), 140.0),
        "weapon_shotgun.reload" => e(numbered("weapons/shotgun/shotgun_reload", &[1, 2, 3]), 0.7, (100.0, 100.0), 75.0),
        "weapon_shotgun.special1" => e(one("weapons/shotgun/shotgun_cock.wav"), 0.7, (100.0, 100.0), 75.0),
        "weapon_shotgun.empty" => e(one("weapons/shotgun/shotgun_empty.wav"), 0.7, (95.0, 100.0), 75.0),
        "weapon_crowbar.single" => e(one("weapons/iceaxe/iceaxe_swing1.wav"), 0.55, (95.0, 100.0), 75.0),
        "weapon_crowbar.melee_hit" | "weapon_crowbar.melee_hitworld" => e(numbered("weapons/crowbar/crowbar_impact", &[1, 2]), 0.7, (90.0, 120.0), 75.0),
        "weapon_physcannon.launch" => e(numbered("weapons/physcannon/superphys_launch", &[1, 2, 3, 4]), 0.45, (110.0, 120.0), 75.0),
        "weapon_physcannon.dryfire" => e(one("weapons/physcannon/physcannon_dryfire.wav"), 0.4, (100.0, 100.0), 75.0),
        "weapon_physcannon.pickup" => e(one("weapons/physcannon/physcannon_pickup.wav"), 0.4, (100.0, 100.0), 75.0),
        "weapon_physcannon.drop" => e(one("weapons/physcannon/physcannon_drop.wav"), 0.4, (100.0, 100.0), 75.0),
        "weapon_physcannon.tooheavy" => e(one("weapons/physcannon/physcannon_tooheavy.wav"), 0.5, (100.0, 100.0), 75.0),
        "weapon_physcannon.openclaws" => e(one("weapons/physcannon/physcannon_claws_open.wav"), 0.4, (100.0, 100.0), 75.0),
        "weapon_physcannon.closeclaws" => e(one("weapons/physcannon/physcannon_claws_close.wav"), 0.4, (100.0, 100.0), 75.0),
        "weapon_physcannon.holdsound" => e(one("weapons/physcannon/hold_loop.wav"), 0.4, (100.0, 100.0), 75.0),
        "weapon_physgun.off" => e(one("weapons/physgun_off.wav"), 0.5, (100.0, 100.0), 75.0),
        "player.falldamage" => e(numbered("player/pl_fallpain", &[1, 3]), 0.75, (100.0, 100.0), 75.0),
        "hl2player.usedeny" => e(one("common/wpn_denyselect.wav"), 0.4, (100.0, 100.0), 75.0),
        "player.weaponselected" => e(one("common/wpn_select.wav"), 0.32, (100.0, 100.0), 0.0),
        "player.weaponselectionmoveslot" => e(one("common/wpn_moveselect.wav"), 0.32, (100.0, 100.0), 0.0),
        "player.weaponselectionopen" => e(one("common/wpn_hudon.wav"), 0.32, (100.0, 100.0), 0.0),
        "player.weaponselectionclose" => e(one("common/wpn_hudoff.wav"), 0.32, (100.0, 100.0), 0.0),
        "player.denyweaponselection" => e(one("common/wpn_denyselect.wav"), 0.32, (100.0, 100.0), 0.0),
        _ => {
            // A surface's sounds: `Stem.StepLeft`, `Stem.ImpactHard`...
            let (stem, what) = lower.split_once('.')?;
            let s = FALLBACK_SURFACES.iter().find(|s| s.0.eq_ignore_ascii_case(stem))?;
            match what {
                "stepleft" => e(numbered(s.1, &[1, 3]), 1.0, (100.0, 100.0), 75.0),
                "stepright" => e(numbered(s.1, &[2, 4]), 1.0, (100.0, 100.0), 75.0),
                "impacthard" => e(numbered(s.2, &[1, 2, 3]), 0.6, (95.0, 105.0), 75.0),
                "impactsoft" => e(numbered(s.3, &[1, 2, 3]), 0.5, (95.0, 105.0), 75.0),
                "bulletimpact" => e(numbered(s.4, &[1, 2, 3]), 0.7, (95.0, 105.0), 75.0),
                _ => None,
            }
        }
    }
}

/// Without the game: a surface's script stem, then its step, hard, soft and bullet waves' stems.
const FALLBACK_SURFACES: [(&str, &str, &str, &str, &str); 8] = [
    ("Dirt", "player/footsteps/dirt", "physics/surfaces/sand_impact_bullet", "physics/surfaces/sand_impact_bullet", "physics/surfaces/sand_impact_bullet"),
    ("Grass", "player/footsteps/grass", "physics/surfaces/sand_impact_bullet", "physics/surfaces/sand_impact_bullet", "physics/surfaces/sand_impact_bullet"),
    ("Concrete", "player/footsteps/concrete", "physics/concrete/concrete_block_impact_hard", "physics/concrete/concrete_impact_soft", "physics/concrete/concrete_impact_bullet"),
    ("SolidMetal", "player/footsteps/metal", "physics/metal/metal_solid_impact_hard", "physics/metal/metal_solid_impact_soft", "physics/metal/metal_solid_impact_bullet"),
    ("Wood", "player/footsteps/wood", "physics/wood/wood_plank_impact_hard", "physics/wood/wood_plank_impact_soft", "physics/wood/wood_solid_impact_bullet"),
    ("Plastic_Box", "player/footsteps/tile", "physics/plastic/plastic_box_impact_hard", "physics/plastic/plastic_box_impact_soft", "physics/plastic/plastic_box_impact_bullet"),
    ("Rubber", "player/footsteps/tile", "physics/rubber/rubber_tire_impact_hard", "physics/rubber/rubber_tire_impact_soft", "physics/rubber/rubber_tire_impact_bullet"),
    ("Flesh", "player/footsteps/tile", "physics/flesh/flesh_impact_hard", "physics/flesh/flesh_impact_hard", "physics/flesh/flesh_impact_bullet"),
];

fn fallback_surface(name: &str) -> SurfaceEntry {
    let n = name.to_ascii_lowercase();
    let stem = if n.contains("grass") {
        "Grass"
    } else if n.contains("dirt") || n.contains("sand") || n.contains("mud") {
        "Dirt"
    } else if n.contains("wood") || n.contains("cardboard") || n.contains("paper") {
        "Wood"
    } else if n.contains("plastic") || n.contains("glass") {
        "Plastic_Box"
    } else if n.contains("rubber") || n.contains("tire") {
        "Rubber"
    } else if n.contains("flesh") || n.contains("watermelon") {
        "Flesh"
    } else if n.contains("metal") || n.contains("canister") || n.contains("barrel") {
        "SolidMetal"
    } else {
        "Concrete"
    };
    SurfaceEntry {
        step_left: format!("{stem}.StepLeft"),
        step_right: format!("{stem}.StepRight"),
        impact_hard: format!("{stem}.ImpactHard"),
        impact_soft: format!("{stem}.ImpactSoft"),
        bullet_impact: format!("{stem}.BulletImpact"),
        material: if matches!(stem, "Dirt" | "Grass") { 'D' } else { 'C' },
    }
}

/// How loud a sound of this level is `dist` metres off (0..1), as Source's mixer has it (`SND_GetGainFromMult`):
/// `SNDLVL_TO_DIST_MULT` with `snd_refdb` 60 and `snd_refdist` 36 units, the gain one over the distance times it (ten at
/// most), what is over a half pressed smoothly under 1 (the compression eases off for the loudest levels: a gunshot is
/// full across a field), and the faintest fading out to nothing. Level 0: heard everywhere.
pub fn distance_gain(level: f32, dist_m: f32) -> f32 {
    if level <= 0.0 {
        return 1.0;
    }
    let units = dist_m / super::movement::UNIT_M;
    let mult = 10f32.powf(60.0 / 20.0) / 10f32.powf(level / 20.0) / 36.0;
    let relative = units * mult;
    let mut gain = if relative > 0.1 { 1.0 / relative } else { 10.0 };
    const THRESH: f32 = 0.5;
    if gain > THRESH {
        // (`SND_GAIN_COMP_EXP_MAX` 2.5 up to 90 dB, easing to `_MIN` 0.8 at 140.)
        let power = if level > 90.0 { 2.5 + (level.min(140.0) - 90.0) / 50.0 * (0.8 - 2.5) } else { 2.5 };
        let y = -1.0 / (THRESH.powf(power) * (THRESH - 1.0));
        gain = 1.0 - 1.0 / (y * gain.powf(power));
    }
    if gain < 0.01 {
        gain = 0.01 * (2.0 - relative * 0.01);
    }
    gain.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_as_the_scripts_write_them() {
        assert_eq!(level_of("SNDLVL_GUNFIRE"), 140.0);
        assert_eq!(level_of("SNDLVL_75dB"), 75.0);
        assert_eq!(level_of("SNDLVL_NORM"), 75.0);
        assert_eq!(level_of("SNDLVL_NONE"), 0.0);
        assert_eq!(range_of("0.7", &[], 1.0), (0.7, 0.7));
        assert_eq!(range_of("0.5, 0.8", &[], 1.0), (0.5, 0.8));
        assert_eq!(range_of("VOL_NORM", &[("VOL_NORM", 1.0)], 0.3), (1.0, 1.0));
        assert_eq!(range_of("98,102", &[], 100.0), (98.0, 102.0));
        assert_eq!(wave_path("^weapons\\Pistol/pistol_fire2.wav"), "weapons/pistol/pistol_fire2.wav");
        assert_eq!(wave_path(")weapons/physcannon/hold_loop.wav"), "weapons/physcannon/hold_loop.wav");
    }

    #[test]
    fn a_block_gives_its_waves_volume_pitch_and_level() {
        let text = r#"
"Weapon_Test.Single"
{
	"channel"		"CHAN_WEAPON"
	"volume"		"0.55"
	"soundlevel"	"SNDLVL_GUNFIRE"
	"pitch"			"98,102"
	"wave"		"^weapons/pistol/pistol_fire2.wav"
}
"Dirt.StepLeft"
{
	"soundlevel"		"SNDLVL_75dB"
	"pitch"			"PITCH_NORM"
	"volume"		"1.0"
	"rndwave"
	{
		"wave"		"player/footsteps/dirt1.wav"
		"wave"		"player/footsteps/dirt3.wav"
	}
}
"#;
        let kv = parse_keyvalues(text);
        assert_eq!(kv.len(), 2);
        let a = entry_of(kv[0].list().unwrap()).unwrap();
        assert_eq!(a.waves, ["weapons/pistol/pistol_fire2.wav"]);
        assert_eq!((a.volume, a.level), ((0.55, 0.55), 140.0));
        assert!((a.pitch.0 - 0.98).abs() < 1e-6 && (a.pitch.1 - 1.02).abs() < 1e-6);
        let b = entry_of(kv[1].list().unwrap()).unwrap();
        assert_eq!(b.waves, ["player/footsteps/dirt1.wav", "player/footsteps/dirt3.wav"]);
        assert_eq!(b.level, 75.0);
    }

    #[test]
    fn a_sound_fades_with_distance_as_sources_does() {
        // 75 dB: full within about 200 units (3.8 m), half as loud each doubling beyond.
        let near = distance_gain(75.0, 1.0);
        assert!(near > 0.85, "{near}");
        let (a, b) = (distance_gain(75.0, 20.0), distance_gain(75.0, 40.0));
        assert!((a / b - 2.0).abs() < 0.05 && a < 0.25 && a > 0.15, "{a} {b}");
        // A gunshot (140 dB) is as loud across the whole field; level 0 everywhere.
        assert!(distance_gain(140.0, 60.0) > 0.95);
        assert_eq!(distance_gain(0.0, 500.0), 1.0);
        // Never louder nearer than further, never over 1.
        let mut last = 1.0;
        for i in 1..400 {
            let g = distance_gain(80.0, i as f32 * 0.25);
            assert!(g <= last + 1e-6 && g <= 1.0);
            last = g;
        }
    }

    /// With the game installed: the scripts read, and every sound the sandbox names is there with waves that exist.
    #[test]
    fn the_games_scripts_have_every_sound_the_sandbox_names() {
        let Some(root) = super::super::assets::find_install() else {
            eprintln!("Garry's Mod is not installed: skipped");
            return;
        };
        let install = GmodInstall::open(&root).unwrap();
        let s = SoundScripts::load(&install);
        let (sounds, surfaces) = s.counts();
        eprintln!("{sounds} sounds, {surfaces} surfaces");
        assert!(sounds > 1000 && surfaces > 50);
        let mut missing = Vec::new();
        for name in super::super::audio::NAMED {
            match s.entries.get(&name.to_ascii_lowercase()) {
                None => missing.push(format!("{name} (no entry)")),
                Some(e) => missing.extend(e.waves.iter().filter(|w| !install.exists(&format!("sound/{w}"))).map(|w| format!("{name}: {w}"))),
            }
        }
        // Every surface the sandbox gives a prop, the ground, a wall or a trunk has its five sounds.
        for surface in super::super::world::Surface::ALL.iter().map(|s| s.prop_name()).chain(["grass", "dirt", "concrete", "wood", "sand", "rock"]) {
            let e = s.surfaces.get(surface).unwrap_or_else(|| panic!("no surface {surface}"));
            for n in [&e.step_left, &e.step_right, &e.impact_hard, &e.impact_soft, &e.bullet_impact] {
                match s.entries.get(&n.to_ascii_lowercase()) {
                    None => missing.push(format!("{surface}: {n} (no entry)")),
                    Some(x) => missing.extend(x.waves.iter().filter(|w| !install.exists(&format!("sound/{w}"))).map(|w| format!("{n}: {w}"))),
                }
            }
        }
        assert!(missing.is_empty(), "{missing:#?}");
        // And the fallbacks say what the game says, for the ones that are single files.
        for name in ["Weapon_Pistol.Single", "Weapon_SMG1.Single", "Weapon_Shotgun.Single", "Weapon_Shotgun.Double", "Weapon_Pistol.Reload", "Weapon_SMG1.Reload", "Weapon_Shotgun.Special1"] {
            let (game, own) = (s.entries.get(&name.to_ascii_lowercase()).unwrap(), fallback(name).unwrap());
            assert_eq!(game.waves, own.waves, "{name}");
            assert!((game.volume.0 - own.volume.0).abs() < 1e-3 && (game.pitch.0 - own.pitch.0).abs() < 1e-3 && game.level == own.level && game.channel == own.channel, "{name}: {game:?} {own:?}");
        }
        // A shot cuts the last one off; a hit on a wall doesn't.
        assert_eq!(s.entries["weapon_smg1.single"].channel, 1);
        assert!(matches!(s.entries["concrete.bulletimpact"].channel, CHAN_AUTO | CHAN_STATIC));
    }

    #[test]
    fn every_wave_plays_before_any_plays_again() {
        let mut s = SoundScripts::default();
        let mut last = usize::MAX;
        let mut run = 1;
        for roll in [0usize, 0, 0, 0, 5, 5, 5, 5, 1, 0, 1, 0] {
            let i = s.pick("Grass.StepLeft", 2, roll);
            assert!(i < 2);
            run = if i == last { run + 1 } else { 1 };
            assert!(run <= 2, "the same wave three times running");
            last = i;
        }
        // (One wave: always it. Each name its own marks.)
        assert_eq!((s.pick("One", 1, 7), s.pick("One", 1, 3)), (0, 0));
        let firsts: Vec<usize> = (0..4).map(|_| s.pick("Four", 4, 0)).collect();
        assert_eq!(firsts, vec![0, 1, 2, 3]);
    }
}
