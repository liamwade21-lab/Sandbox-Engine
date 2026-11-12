//! The sandbox's sounds: Half-Life 2's own, by the names its code plays them under (`Weapon_Pistol.Single`,
//! `Metal_Barrel.ImpactHard`, `Grass.StepLeft`), looked up in its own sound scripts (`soundscript.rs`), so the wave, the
//! volume, the pitch and how far each carries are the game's. Anything that happens pushes a `Cue` on `Sandbox::cues`; only
//! `play_cues` spawns audio.
//!
//! A sound that happens somewhere (a prop landing, a bullet striking) is heard from there: fainter with distance by
//! Source's own rule (`soundscript::distance_gain`) and to the side it is on (Bevy's spatial voices: one listener at the
//! eyes, turned with the view). What he does himself (his weapon, his steps, the menu) plays in the middle.

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, SpatialAudioSink, SpatialListener, Volume};
use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::sound::SoundSettings;
use crate::system::SystemBody;

use super::assets::GmodAssets;
use super::movement::{self, UNIT_M};
use super::soundscript::{CHAN_AUTO, CHAN_STATIC, distance_gain};
use super::world::{Met, SandboxWorld};
use super::{Event, Sandbox};

/// The game's sounds are mixed for a volume slider near full; the port's is 0.3 by default.
const GAIN: f32 = 2.6;

/// The most sounds started in a frame.
const MAX_CUES: usize = 8;

/// The sound level of a wave played by its path from somewhere in the world (`SNDLVL_NORM`).
const LEVEL_NORM: f32 = 75.0;

/// The listener's ears are this far apart, and every placed sound is put this far from between them, in its true
/// direction (the distance's own fading is `distance_gain`'s: the voice's is kept out of it by staying within a unit).
const EAR_GAP: f32 = 0.2;
const VOICE_DIST: f32 = 0.5;
/// What a voice straight ahead loses to the spatial mix (both ears get three quarters), given back.
const SPATIAL_MAKE_UP: f32 = 1.0 / 0.75;

/// Every script name the sandbox's own code plays (the surfaces' are looked up through `surfaceproperties.txt`): a test
/// checks each is in the game's scripts with waves that exist.
#[cfg_attr(not(test), allow(dead_code))]
pub const NAMED: &[&str] = &[
    "Weapon_Pistol.Single",
    "Weapon_Pistol.Reload",
    "Weapon_Pistol.Empty",
    "Weapon_SMG1.Single",
    "Weapon_SMG1.Reload",
    "Weapon_SMG1.Empty",
    "Weapon_Shotgun.Single",
    "Weapon_Shotgun.Double",
    "Weapon_Shotgun.Reload",
    "Weapon_Shotgun.Special1",
    "Weapon_Shotgun.Empty",
    "Weapon_Crowbar.Single",
    "Weapon_Crowbar.Melee_HitWorld",
    "Weapon_PhysCannon.Launch",
    "Weapon_PhysCannon.DryFire",
    "Weapon_PhysCannon.Pickup",
    "Weapon_PhysCannon.Drop",
    "Weapon_PhysCannon.TooHeavy",
    "Weapon_PhysCannon.OpenClaws",
    "Weapon_PhysCannon.CloseClaws",
    "Weapon_PhysCannon.HoldSound",
    "Weapon_MegaPhysCannon.HoldSound",
    "Weapon_Physgun.Off",
    "Player.FallDamage",
    "HL2Player.UseDeny",
    "Player.WeaponSelected",
    "Player.WeaponSelectionMoveSlot",
    "Player.WeaponSelectionClose",
    "Player.DenyWeaponSelection",
];

/// What a cue plays: a wave by its path under `sound/`, or one of the game's named sounds.
#[derive(Clone, Debug, PartialEq)]
pub enum Sound {
    Wave(String),
    Named(String),
}

/// One sound to play. For a wave `volume` and `pitch` are its own; for a named sound they scale the script's.
#[derive(Clone, Debug)]
pub struct Cue {
    pub sound: Sound,
    pub volume: f32,
    pub pitch: f32,
    /// Where it happens (the sandbox's frame, metres); `None`: in his own hands or head.
    pub at: Option<Vec3>,
}

impl Cue {
    pub fn new(path: impl Into<String>, volume: f32, pitch: f32) -> Self {
        Cue { sound: Sound::Wave(path.into()), volume, pitch, at: None }
    }

    /// An interface sound.
    pub fn ui(path: &str) -> Self {
        Cue::new(path, 0.6, 1.0)
    }

    /// One of the game's sounds as its script has it.
    pub fn named(name: impl Into<String>) -> Self {
        Cue { sound: Sound::Named(name.into()), volume: 1.0, pitch: 1.0, at: None }
    }

    pub fn at(mut self, point: Vec3) -> Self {
        self.at = Some(point);
        self
    }

    pub fn louder(mut self, k: f32) -> Self {
        self.volume *= k;
        self
    }
}

/// A small generator for which wave of a set plays and at what pitch (nothing here needs more).
#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> u32 {
        // (PCG's scramble over the state: the state's own low bits repeat every few draws, and which of two steps
        // plays is one bit: a walk was the same two waves over and over.)
        self.0 = self.0.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let x = ((self.0 >> ((self.0 >> 28) + 4)) ^ self.0).wrapping_mul(277_803_737);
        (x ^ (x >> 22)) >> 8
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
}

/// What the ground itself is, as a surface property: grass where the world grows plants, bare dirt elsewhere.
pub fn ground_name(plants: bool, town: bool) -> &'static str {
    // (Among buildings the ground is streets, sidewalks and yards.)
    if town {
        "concrete"
    } else if plants {
        "grass"
    } else {
        "dirt"
    }
}

/// The surface property of what a trace met: a prop's own (its model's), a wall's or a trunk's, the ground's.
pub fn met_surface(world: Option<&SandboxWorld>, ground: &str, met: Met) -> String {
    match met {
        Met::Prop(id) => world.and_then(|w| w.prop(id)).map_or("default".to_string(), |p| p.material.clone().unwrap_or_else(|| p.surface.prop_name().to_string())),
        Met::Solid(s) => s.prop_name().to_string(),
        Met::Ground => ground.to_string(),
    }
}

pub fn surface_name(sb: &Sandbox, met: Met) -> String {
    met_surface(sb.world.as_ref(), sb.ground_material, met)
}

/// A bullet's or the crowbar's strike on a surface, heard from where it struck.
pub fn bullet_impact(assets: &GmodAssets, surface: &str, at: Vec3) -> Cue {
    Cue::named(assets.scripts.surface(surface).bullet_impact).at(at)
}

/// What a tick's events sound like.
fn event_cues(sb: &mut Sandbox, assets: &GmodAssets) {
    // What is under his feet: a prop, a roof, the ground (looked for only when a step is to sound).
    let on = if sb.events.is_empty() {
        None
    } else {
        Some(match sb.player.ground {
            Some(movement::Hit::Prop(id, _)) => surface_name(sb, Met::Prop(id)),
            Some(movement::Hit::World) => {
                let feet = sb.player.origin * UNIT_M;
                let met = sb.world.as_ref().and_then(|w| w.ray(feet + Vec3::Z * 0.2, Vec3::NEG_Z, 0.5, None)).map_or(Met::Ground, |h| h.met);
                surface_name(sb, met)
            }
            None => sb.last_ground.clone().unwrap_or_else(|| sb.ground_material.to_string()),
        })
    };
    for i in 0..sb.events.len() {
        let surface = assets.scripts.surface(on.as_deref().unwrap_or("default"));
        // `PlayStepSound`: the surface's own waves, at the volume the code gives (the script's is not used).
        let step = |left: bool, volume: f32| Cue { volume, ..Cue::named(if left { surface.step_left.clone() } else { surface.step_right.clone() }) };
        let cue = match sb.events[i] {
            Event::Step { left, speed, ducked } => {
                // `UpdateStepSound`: walking or running by 220 units/s (80 ducked); dirt and grass 0.25 and 0.55, a vent
                // 0.4 and 0.7, the rest 0.2 and 0.5; 65 % of it ducked.
                let running = speed >= if ducked { 80.0 } else { 220.0 };
                let v = match (surface.material, running) {
                    ('D', false) => 0.25,
                    ('D', true) => 0.55,
                    ('V', false) => 0.4,
                    ('V', true) => 0.7,
                    (_, false) => 0.2,
                    (_, true) => 0.5,
                };
                Some(step(left, v * if ducked { 0.65 } else { 1.0 }))
            }
            // `CheckJumpButton`: a step at full volume.
            Event::Jump => Some(step(true, 1.0)),
            Event::Land(speed) => {
                if speed > movement::SAFE_FALL_SPEED {
                    sb.cues.push(Cue::named("Player.FallDamage"));
                }
                // `CheckFalling`: a landing under `PLAYER_FALL_PUNCH_THRESHOLD` (a jump's on level ground) makes no sound;
                // from there a step at 0.85, at full volume when it hurts.
                (speed >= movement::FALL_PUNCH_SPEED).then(|| step(false, if speed > movement::SAFE_FALL_SPEED { 1.0 } else { 0.85 }))
            }
            Event::PropImpact(hit) => {
                // `PhysicsImpactSound`: the surface's hard sound from 150 units/s of change, its soft one under; the
                // volume by the speed squared over 320 units/s squared.
                let s = assets.scripts.surface(&surface_name(sb, Met::Prop(hit.id)));
                let units = hit.speed / UNIT_M;
                let volume = (units * units / (320.0 * 320.0)).clamp(0.0, 1.0);
                (volume > 0.02).then(|| Cue { volume, ..Cue::named(if units > 150.0 { s.impact_hard } else { s.impact_soft }).at(hit.at) })
            }
        };
        sb.cues.extend(cue);
    }
    if on.is_some() && sb.player.ground.is_some() {
        sb.last_ground = on;
    }
}

/// The sound that goes on while something is held (the gravity gun's hum).
#[derive(Component)]
pub struct LoopVoice(&'static str);

/// A cue's voice, kept only to say so in the log (`SE_SANDBOX_SOUNDS=1`: a test can't listen).
#[derive(Component)]
pub struct CueVoice(String, f32);

/// A voice on one of his own channels (`CHAN_*`), which the next sound there cuts off.
#[derive(Component)]
pub struct ChannelVoice(u8);

/// The one listener the placed sounds are heard by: at the render frame's origin (his eyes), turned as the view is.
#[derive(Component)]
pub struct Ears;

/// `SE_SANDBOX_SOUNDS=1`: log every sound as it starts to play.
fn log_sounds() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SE_SANDBOX_SOUNDS").is_some())
}

/// Says which cues the audio thread took (a file it can't decode never gets a sink).
pub fn log_voices(started: Query<&CueVoice, Or<(Added<AudioSink>, Added<SpatialAudioSink>)>>) {
    for v in &started {
        info!("sandbox: playing {} at volume {:.2}", v.0, v.1);
    }
}

pub fn spawn_ears(mut commands: Commands) {
    // The ears the other way round: rodio's `Spatial` makes the channel of the ear FURTHER from a sound the louder one
    // (measured 2026-11-11 by recording the speakers: a barrel dropped on his right was 5.7 dB louder on the left).
    let ears = SpatialListener { left_ear_offset: Vec3::X * EAR_GAP / 2.0, right_ear_offset: Vec3::X * -EAR_GAP / 2.0 };
    commands.spawn((Name::new("Sandbox ears"), Ears, ears, Transform::IDENTITY));
}

/// Play what was cued this frame, and keep the loop that is wanted going.
#[allow(clippy::too_many_arguments)]
pub fn play_cues(
    mut commands: Commands,
    mut sb: ResMut<Sandbox>,
    mut assets: ResMut<GmodAssets>,
    mut sources: ResMut<Assets<AudioSource>>,
    settings: Res<SoundSettings>,
    cam: Res<FlyCamera>,
    bodies: Query<&SystemBody>,
    mut ears: Query<&mut Transform, With<Ears>>,
    mut loops: Query<(Entity, &LoopVoice, Option<&mut AudioSink>)>,
    channels: Query<(Entity, &ChannelVoice)>,
) {
    let sb = &mut *sb;
    event_cues(sb, &assets);
    let master = settings.volume * GAIN;
    // The frame's turn to the render frame's axes (the camera is its origin), and the ears with the view.
    let to_render = sb.world.as_ref().and_then(|w| bodies.iter().find(|b| b.obj == w.frame.obj).map(|b| (b.orient * w.frame.rotation()).as_quat()));
    if let Ok(mut t) = ears.single_mut() {
        let rot = cam.rot.as_quat();
        if t.rotation != rot {
            t.rotation = rot;
        }
    }
    // Each cue as a wave, its volume where he hears it, its pitch, and its direction if it has a place.
    let mut voices: Vec<(String, f32, f32, Option<Vec3>, u8)> = Vec::with_capacity(sb.cues.len());
    for cue in sb.cues.drain(..) {
        let (path, volume, pitch, level, channel) = match &cue.sound {
            Sound::Wave(p) => (p.clone(), cue.volume, cue.pitch, LEVEL_NORM, CHAN_AUTO),
            Sound::Named(name) => {
                let Some(e) = assets.scripts.get(name) else {
                    if log_sounds() {
                        info!("sandbox: no sound named {name}");
                    }
                    continue;
                };
                let wave = e.waves[assets.scripts.pick(name, e.waves.len(), sb.rng.next() as usize)].clone();
                (wave, sb.rng.range(e.volume.0, e.volume.1) * cue.volume, sb.rng.range(e.pitch.0, e.pitch.1) * cue.pitch, e.level, e.channel)
            }
        };
        let (gain, dir) = match (cue.at, to_render) {
            (Some(at), Some(q)) => {
                let off = at - sb.eye;
                let dist = off.length();
                // (Within arm's reach it is in his head as much as anywhere.)
                (distance_gain(level, dist), (dist > 0.3).then(|| q * (off / dist)))
            }
            _ => (1.0, None),
        };
        // (Only what he makes himself has a channel to cut off: every prop and wall is its own source.)
        voices.push((path, volume * gain, pitch, dir, if cue.at.is_none() { channel } else { CHAN_AUTO }));
    }
    // (A pile of props coming down is hundreds of hits a second: the loudest few are what is heard.)
    if voices.len() > MAX_CUES {
        voices.sort_by(|a, b| b.1.total_cmp(&a.1));
        voices.truncate(MAX_CUES);
    }
    // Per channel, whether the voices on it from earlier frames were cut, and the voice this frame put on it.
    let mut cut = [false; 8];
    let mut fresh: [Option<Entity>; 8] = [None; 8];
    for (path, volume, pitch, dir, channel) in voices {
        let Some(h) = assets.sound(&mut sources, &path) else {
            if log_sounds() {
                info!("sandbox: no sound for {path}");
            }
            continue;
        };
        let volume = (volume * master).clamp(0.0, 1.5);
        if volume <= 0.004 {
            continue;
        }
        let speed = pitch.clamp(0.5, 2.0);
        let mut voice = match dir {
            Some(d) => commands.spawn((
                AudioPlayer(h),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear((volume * SPATIAL_MAKE_UP).min(1.6))).with_speed(speed).with_spatial(true),
                // (Both, so the voice starts where it belongs: the transforms were propagated before this frame's commands.)
                Transform::from_translation(d * VOICE_DIST),
                GlobalTransform::from_translation(d * VOICE_DIST),
            )),
            None => commands.spawn((AudioPlayer(h), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)).with_speed(speed))),
        };
        if log_sounds() {
            voice.insert(CueVoice(format!("{path}{}", if dir.is_some() { " (placed)" } else { "" }), volume));
        }
        // A new sound on a channel ends the last one there (`SND_PickDynamicChannel`): the SMG's shots don't pile up.
        let c = channel as usize;
        if channel != CHAN_AUTO && channel != CHAN_STATIC && c < cut.len() {
            let id = voice.insert(ChannelVoice(channel)).id();
            if !cut[c] {
                cut[c] = true;
                for (e, v) in &channels {
                    if v.0 == channel {
                        commands.entity(e).try_despawn();
                    }
                }
            }
            if let Some(old) = fresh[c].replace(id) {
                commands.entity(old).try_despawn();
            }
        }
    }
    // The loop: one of the game's named sounds, at its script's volume times what was asked.
    let want = sb.loop_sound.filter(|_| sb.walking()).and_then(|(name, k)| {
        let e = assets.scripts.get(name)?;
        Some((name, e.waves.first()?.clone(), e.volume.0 * k))
    });
    let mut playing = false;
    for (e, voice, sink) in &mut loops {
        if Some(voice.0) == want.as_ref().map(|w| w.0) {
            playing = true;
            if let (Some(mut sink), Some(w)) = (sink, want.as_ref()) {
                sink.set_volume(Volume::Linear((w.2 * master).clamp(0.0, 1.5)));
            }
        } else {
            commands.entity(e).despawn();
        }
    }
    if let (false, Some((name, path, volume))) = (playing, want) {
        if let Some(h) = assets.sound(&mut sources, &path) {
            commands.spawn((LoopVoice(name), AudioPlayer(h), PlaybackSettings::LOOP.with_volume(Volume::Linear((volume * master).clamp(0.0, 1.5)))));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generator_stays_in_its_ranges() {
        let mut rng = Rng(7);
        for _ in 0..500 {
            let v = rng.range(0.98, 1.02);
            assert!((0.98..=1.02).contains(&v));
            assert!(rng.pick(4) < 4);
        }
    }

    #[test]
    fn cues_carry_what_they_are_given() {
        let c = Cue::named("Weapon_Pistol.Single").at(Vec3::new(1.0, 2.0, 3.0)).louder(0.5);
        assert_eq!(c.sound, Sound::Named("Weapon_Pistol.Single".to_string()));
        assert_eq!((c.volume, c.at), (0.5, Some(Vec3::new(1.0, 2.0, 3.0))));
        let u = Cue::ui("buttons/button15.wav");
        assert_eq!((u.sound, u.at), (Sound::Wave("buttons/button15.wav".to_string()), None));
    }
}
