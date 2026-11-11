//! The sandbox's sounds: Half-Life 2's own files from the Garry's Mod install (`assets.rs`), picked and timed as its sound
//! scripts do (`game_sounds_weapons.txt`, `game_sounds_physics.txt`: the waves, volumes and pitch ranges below are theirs).
//! Anything that happens pushes a `Cue` on `Sandbox::cues`; only `play_cues` spawns audio.

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;

use crate::sound::SoundSettings;

use super::assets::GmodAssets;
use super::movement::{self, UNIT_M};
use super::world::Surface;
use super::{Event, Sandbox};

/// The game's sounds are mixed for a volume slider near full; the port's is 0.3 by default.
const GAIN: f32 = 2.6;

/// The most sounds started in a frame.
const MAX_CUES: usize = 8;

/// One sound to play: a file under `sound/`, its volume (the script's) and pitch (1: as recorded).
#[derive(Clone, Debug)]
pub struct Cue {
    pub path: String,
    pub volume: f32,
    pub pitch: f32,
}

impl Cue {
    pub fn new(path: impl Into<String>, volume: f32, pitch: f32) -> Self {
        Cue { path: path.into(), volume, pitch }
    }

    /// An interface sound.
    pub fn ui(path: &str) -> Self {
        Cue::new(path, 0.6, 1.0)
    }
}

/// A small generator for which wave of a set plays and at what pitch (nothing here needs more).
#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0 >> 8
    }

    /// 0..1.
    pub fn unit(&mut self) -> f32 {
        self.next() as f32 / 16_777_216.0
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    pub fn pick(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }

    /// `PREFIXn.wav` with n one of `numbers`, pitched in `lo..hi` percent.
    pub fn wave(&mut self, prefix: &str, numbers: &[u32], volume: f32, lo: f32, hi: f32) -> Cue {
        let n = numbers[self.pick(numbers.len())];
        Cue::new(format!("{prefix}{n}.wav"), volume, self.range(lo, hi) / 100.0)
    }
}

/// A surface's impact sounds: the file's stem, the numbers that exist, for a hard hit and a soft one.
fn impact_waves(s: Surface) -> ((&'static str, &'static [u32]), (&'static str, &'static [u32])) {
    match s {
        Surface::MetalBarrel => (("physics/metal/metal_barrel_impact_hard", &[1, 2, 3, 5, 6, 7]), ("physics/metal/metal_barrel_impact_soft", &[1, 2, 3, 4])),
        Surface::Metal => (("physics/metal/metal_solid_impact_hard", &[1, 4, 5]), ("physics/metal/metal_solid_impact_soft", &[1, 2, 3])),
        Surface::WoodCrate => (("physics/wood/wood_crate_impact_hard", &[1, 2, 3, 4, 5]), ("physics/wood/wood_crate_impact_soft", &[1, 2, 3])),
        Surface::Wood => (("physics/wood/wood_plank_impact_hard", &[1, 2, 3, 4, 5]), ("physics/wood/wood_plank_impact_soft", &[1, 2, 3])),
        Surface::PlasticBarrel => (("physics/plastic/plastic_barrel_impact_hard", &[1, 2, 3, 4]), ("physics/plastic/plastic_barrel_impact_soft", &[1, 2, 3, 4, 5, 6])),
        Surface::Plastic => (("physics/plastic/plastic_box_impact_hard", &[1, 2, 3, 4]), ("physics/plastic/plastic_box_impact_soft", &[1, 2, 3, 4])),
        Surface::Concrete => (("physics/concrete/concrete_block_impact_hard", &[1, 2, 3]), ("physics/concrete/concrete_impact_soft", &[1, 2, 3])),
        Surface::Cardboard => (("physics/cardboard/cardboard_box_impact_hard", &[1, 2, 3, 4, 5, 6, 7]), ("physics/cardboard/cardboard_box_impact_soft", &[1, 2, 3, 4, 5, 6, 7])),
        Surface::Rubber => (("physics/rubber/rubber_tire_impact_hard", &[1, 2, 3]), ("physics/rubber/rubber_tire_impact_soft", &[1, 2, 3])),
        Surface::Flesh => (("physics/flesh/flesh_impact_hard", &[1, 2, 3, 4, 5, 6]), ("physics/flesh/flesh_squishy_impact_hard", &[1, 2, 3, 4])),
        Surface::Glass => (("physics/glass/glass_impact_hard", &[1, 2, 3]), ("physics/glass/glass_impact_soft", &[1, 2, 3])),
    }
}

/// A bullet's or the crowbar's hit on a surface (`None`: the ground).
pub fn bullet_impact(rng: &mut Rng, surface: Option<Surface>) -> Cue {
    let (stem, numbers): (&str, &[u32]) = match surface {
        None => ("physics/surfaces/sand_impact_bullet", &[1, 2, 3, 4]),
        Some(Surface::Metal | Surface::MetalBarrel) => ("physics/metal/metal_solid_impact_bullet", &[1, 2, 3, 4]),
        Some(Surface::Wood | Surface::WoodCrate) => ("physics/wood/wood_solid_impact_bullet", &[1, 2, 3, 4, 5]),
        Some(Surface::Plastic) => ("physics/plastic/plastic_box_impact_bullet", &[1, 2, 3, 4, 5]),
        Some(Surface::PlasticBarrel) => ("physics/plastic/plastic_barrel_impact_bullet", &[1, 2, 3]),
        Some(Surface::Concrete) => ("physics/concrete/concrete_impact_bullet", &[1, 2, 3, 4]),
        Some(Surface::Cardboard) => ("physics/cardboard/cardboard_box_impact_bullet", &[1, 2, 3, 4, 5]),
        Some(Surface::Rubber) => ("physics/rubber/rubber_tire_impact_bullet", &[1, 2, 3]),
        Some(Surface::Flesh) => ("physics/flesh/flesh_impact_bullet", &[1, 2, 3, 4, 5]),
        Some(Surface::Glass) => ("physics/glass/glass_impact_bullet", &[1, 2, 3, 4]),
    };
    rng.wave(stem, numbers, 0.7, 95.0, 105.0)
}

/// A footstep on what he stands on: the ground's dirt, or the prop's own surface (`Dirt.StepLeft` is waves 1 and 3, the right
/// foot's 2 and 4).
fn step_cue(rng: &mut Rng, on: Option<Surface>, left: bool, volume: f32) -> Cue {
    let stem = match on {
        None => "player/footsteps/dirt",
        Some(Surface::Metal | Surface::MetalBarrel) => "player/footsteps/metal",
        Some(Surface::Wood | Surface::WoodCrate | Surface::Cardboard) => "player/footsteps/wood",
        Some(Surface::Concrete | Surface::Glass) => "player/footsteps/concrete",
        Some(_) => "player/footsteps/tile",
    };
    let numbers: &[u32] = if left { &[1, 3] } else { &[2, 4] };
    rng.wave(stem, numbers, volume, 96.0, 104.0)
}

/// What a tick's events sound like.
fn event_cues(sb: &mut Sandbox) {
    let on = match sb.player.ground {
        Some(movement::Hit::Prop(id, _)) => sb.world.as_ref().and_then(|w| w.prop(id)).map(|p| p.surface),
        _ => None,
    };
    for i in 0..sb.events.len() {
        let cue = match sb.events[i] {
            Event::Step { left, speed, ducked } => {
                // `UpdateStepSound`: dirt 0.25 walking, 0.55 running (220 units/s and up; 80 ducked), 0.65 of it ducked.
                let running = speed >= if ducked { 80.0 } else { 220.0 };
                let v = if running { 0.55 } else { 0.25 } * if ducked { 0.65 } else { 1.0 };
                Some(step_cue(&mut sb.rng, on, left, v))
            }
            Event::Jump => Some(step_cue(&mut sb.rng, sb.last_ground, true, 0.6)),
            Event::Land(speed) => {
                if speed > movement::SAFE_FALL_SPEED {
                    // (`Player.FallDamage`.)
                    let n = if sb.rng.pick(2) == 0 { 1 } else { 3 };
                    sb.cues.push(Cue::new(format!("player/pl_fallpain{n}.wav"), 0.75, 1.0));
                }
                let v = if speed > movement::SAFE_FALL_SPEED {
                    1.0
                } else if speed > movement::SAFE_FALL_SPEED / 2.0 {
                    0.85
                } else {
                    0.5
                };
                (speed > 120.0).then(|| step_cue(&mut sb.rng, on, false, v))
            }
            Event::PropImpact(hit) => {
                // `PhysCollisionSound`: the volume by the speed squared over 320 units/s squared; hard from about 2.5 m/s.
                let ((hard, hard_n), (soft, soft_n)) = impact_waves(hit.surface);
                let units = hit.speed / UNIT_M;
                let volume = (units * units / (320.0 * 320.0)).clamp(0.08, 1.0) * 0.6;
                // (Further off, fainter: there is no 3D sound here.)
                let far = 1.0 / (1.0 + ((hit.at - sb.eye).length() / 6.0).powi(2));
                let (stem, numbers) = if hit.speed > 2.5 { (hard, hard_n) } else { (soft, soft_n) };
                Some(Cue { volume: volume * far, ..sb.rng.wave(stem, numbers, 1.0, 95.0, 105.0) })
            }
        };
        sb.cues.extend(cue);
    }
    sb.last_ground = on;
}

/// The sound that goes on while something is held (the gravity gun's hum).
#[derive(Component)]
pub struct LoopVoice(&'static str);

/// A cue's voice, kept only to say so in the log (`SE_SANDBOX_SOUNDS=1`: a test can't listen).
#[derive(Component)]
pub struct CueVoice(String, f32);

/// `SE_SANDBOX_SOUNDS=1`: log every sound as it is cued and as it starts to play.
fn log_sounds() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SE_SANDBOX_SOUNDS").is_some())
}

/// Says which cues the audio thread took (a file it can't decode never gets a sink).
pub fn log_voices(started: Query<&CueVoice, Added<AudioSink>>) {
    for v in &started {
        info!("sandbox: playing {} at volume {:.2}", v.0, v.1);
    }
}

/// Play what was cued this frame, and keep the loop that is wanted going.
pub fn play_cues(
    mut commands: Commands,
    mut sb: ResMut<Sandbox>,
    mut assets: ResMut<GmodAssets>,
    mut sources: ResMut<Assets<AudioSource>>,
    settings: Res<SoundSettings>,
    mut loops: Query<(Entity, &LoopVoice, Option<&mut AudioSink>)>,
) {
    let sb = &mut *sb;
    event_cues(sb);
    let master = settings.volume * GAIN;
    // (A pile of props coming down is hundreds of hits a second: the loudest few are what is heard.)
    if sb.cues.len() > MAX_CUES {
        sb.cues.sort_by(|a, b| b.volume.total_cmp(&a.volume));
        sb.cues.truncate(MAX_CUES);
    }
    for cue in sb.cues.drain(..) {
        let Some(h) = assets.sound(&mut sources, &cue.path) else {
            if log_sounds() {
                info!("sandbox: no sound for {}", cue.path);
            }
            continue;
        };
        let volume = (cue.volume * master).clamp(0.0, 1.5);
        if volume <= 0.01 {
            continue;
        }
        let mut voice = commands.spawn((AudioPlayer(h), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)).with_speed(cue.pitch.clamp(0.5, 2.0))));
        if log_sounds() {
            voice.insert(CueVoice(cue.path.clone(), volume));
        }
    }
    let want = sb.loop_sound.filter(|_| sb.walking());
    let mut playing = false;
    for (e, voice, sink) in &mut loops {
        if Some(voice.0) == want.map(|w| w.0) {
            playing = true;
            if let (Some(mut sink), Some(w)) = (sink, want) {
                sink.set_volume(Volume::Linear((w.1 * master).clamp(0.0, 1.5)));
            }
        } else {
            commands.entity(e).despawn();
        }
    }
    if let (false, Some((path, volume))) = (playing, want) {
        if let Some(h) = assets.sound(&mut sources, path) {
            commands.spawn((LoopVoice(path), AudioPlayer(h), PlaybackSettings::LOOP.with_volume(Volume::Linear((volume * master).clamp(0.0, 1.5)))));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waves_come_from_their_sets() {
        let mut rng = Rng(7);
        for _ in 0..200 {
            let c = rng.wave("physics/metal/metal_barrel_impact_hard", &[1, 2, 3, 5, 6, 7], 0.6, 98.0, 100.0);
            let n: u32 = c.path.trim_end_matches(".wav").rsplit("hard").next().unwrap().parse().unwrap();
            assert!([1, 2, 3, 5, 6, 7].contains(&n));
            assert!((0.98..=1.0).contains(&c.pitch));
        }
        // Left and right feet take different waves.
        let l = step_cue(&mut rng, None, true, 0.25);
        let r = step_cue(&mut rng, None, false, 0.25);
        assert!(l.path.ends_with("1.wav") || l.path.ends_with("3.wav"));
        assert!(r.path.ends_with("2.wav") || r.path.ends_with("4.wav"));
    }

    /// With the game installed: every wave the tables name is there.
    #[test]
    fn every_named_wave_exists() {
        let Some(root) = super::super::assets::find_install() else {
            eprintln!("Garry's Mod is not installed: skipped");
            return;
        };
        let install = se_core::gmod::GmodInstall::open(&root).unwrap();
        let mut missing = Vec::new();
        let mut check = |stem: &str, numbers: &[u32]| {
            for n in numbers {
                let p = format!("sound/{stem}{n}.wav");
                if !install.exists(&p) {
                    missing.push(p);
                }
            }
        };
        for s in [Surface::Metal, Surface::MetalBarrel, Surface::Wood, Surface::WoodCrate, Surface::Plastic, Surface::PlasticBarrel, Surface::Concrete, Surface::Cardboard, Surface::Rubber, Surface::Flesh, Surface::Glass] {
            let ((h, hn), (so, sn)) = impact_waves(s);
            check(h, hn);
            check(so, sn);
        }
        for stem in ["player/footsteps/dirt", "player/footsteps/metal", "player/footsteps/wood", "player/footsteps/concrete", "player/footsteps/tile"] {
            check(stem, &[1, 2, 3, 4]);
        }
        assert!(missing.is_empty(), "{missing:?}");
    }
}
