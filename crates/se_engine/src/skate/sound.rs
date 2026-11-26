//! Skate 3's own sounds for the skater (the user's 2026-11-12: "add the sounds as well that are originally not in the rust
//! port but in the skate 3 files"): the wheels rolling, the pop, the landings, the pushes, the bails. They are taken from the
//! user's copy of Skate 3's audio banks once and kept as Ogg files in `sfx/` (`convert.rs`), each a `Need` of `mod.cfg`, and
//! checked on first enable as every mod's sound is (`mods::check_sound`).
//!
//! A file's name says what it is for: `ROLE_N.ogg` (`pop_1.ogg`, `pop_2.ogg`, ...). The roles are `ROLES`; each sound of a
//! role is picked in turn without repeats.

use std::collections::HashMap;

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackMode, PlaybackSettings, Volume};
use bevy::prelude::*;

use super::SkateEvent;
use crate::pitched_audio::{PitchedSound, Speed, Wave};
use crate::sandbox::Sandbox;

/// What the sounds are for: a one-shot by event, or the rolling loop.
#[cfg_attr(not(test), allow(dead_code))]
pub const ROLES: [&str; 9] = ["roll", "pop", "land", "land_hard", "push", "bail", "slide", "brake", "flip"];

/// The game's sounds are mixed for a volume slider near full; the port's is 0.3 by default (as the sandbox's `GAIN`).
const GAIN: f32 = 2.6;

/// The sounds by role, decoded once (they play as `PitchedSound`s: `pitched_audio.rs`, rodio's own speed can hang the sound).
#[derive(Default)]
pub struct SoundBank {
    pub by_role: HashMap<String, Vec<Wave>>,
}

impl SoundBank {
    /// Every `sfx/ROLE_N.ogg` the mod needs, checked and decoded once (a refusal names the file).
    pub fn load(read: &dyn Fn(&str) -> Result<Vec<u8>, String>, needs: &[String]) -> Result<SoundBank, (String, String)> {
        let mut bank = SoundBank::default();
        for rel in needs.iter().filter(|r| r.starts_with("sfx/") && r.ends_with(".ogg")) {
            let bytes = read(rel).map_err(|e| (rel.clone(), e))?;
            let (source, _) = crate::mods::check_sound(bytes).map_err(|e| (rel.clone(), e))?;
            let wave = Wave::decode(&source).ok_or_else(|| (rel.clone(), "can't be decoded".to_string()))?;
            let stem = rel.trim_start_matches("sfx/").trim_end_matches(".ogg");
            let role = stem.rsplit_once('_').filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit())).map(|(r, _)| r).unwrap_or(stem);
            bank.by_role.entry(role.to_string()).or_default().push(wave);
        }
        Ok(bank)
    }

    pub fn count(&self) -> usize {
        self.by_role.values().map(Vec::len).sum()
    }
}

/// A looping voice: its role, which of the role's sounds, and its speed (read by its source).
#[derive(Component)]
pub struct LoopVoice {
    role: &'static str,
    index: usize,
    speed: Speed,
}

/// The turn of each role.
#[derive(Default)]
pub struct Played {
    turn: HashMap<String, usize>,
}

/// The next of a role's sounds, in turn.
fn next<'a>(played: &mut Played, bank: &'a SoundBank, role: &str) -> Option<&'a Wave> {
    let list = bank.by_role.get(role).filter(|l| !l.is_empty())?;
    let turn = played.turn.entry(role.to_string()).or_insert(0);
    *turn = (*turn + 1) % list.len();
    list.get(*turn)
}

/// The frame's sounds: each event's one-shot, and the loops (the wheels rolling, by speed band; the powerslide; the foot
/// brake) following what he does.
#[allow(clippy::too_many_arguments)]
pub fn play(
    mut commands: Commands,
    mut sb: ResMut<Sandbox>,
    assets: Option<Res<super::SkateAssets>>,
    settings: Res<crate::sound::SoundSettings>,
    mut sounds: ResMut<Assets<PitchedSound>>,
    mut played: Local<Played>,
    mut loops: Query<(Entity, &LoopVoice, &mut AudioSink)>,
) {
    let master = settings.volume * GAIN;
    let (Some(sk), Some(assets)) = (sb.skate.as_mut(), assets) else {
        for (e, ..) in &loops {
            commands.entity(e).despawn();
        }
        return;
    };
    let events: Vec<SkateEvent> = sk.events.drain(..).collect();
    let bank = &assets.sounds;
    for ev in events {
        let (role, volume) = match ev {
            SkateEvent::Pop => ("pop", 0.9),
            SkateEvent::Flip => ("flip", 0.9),
            SkateEvent::Land(down) if down > 4.5 => ("land_hard", (0.7 + down * 0.04).min(1.0)),
            SkateEvent::Land(down) => ("land", (0.55 + down * 0.1).min(1.0)),
            SkateEvent::Push => ("push", 0.8),
            SkateEvent::Bail => ("bail", 1.0),
        };
        if sk.log {
            info!("skate: sound {role} at {:.2}", volume);
        }
        if let Some(wave) = next(&mut played, bank, role) {
            let h = sounds.add(PitchedSound::once(wave, 1.0));
            commands.spawn((AudioPlayer(h), PlaybackSettings { mode: PlaybackMode::Despawn, volume: Volume::Linear(volume * master), ..default() }));
        }
    }
    // The loops wanted now: (role, which sound, volume, speed).
    let mut want: Vec<(&'static str, usize, f32, f32)> = Vec::new();
    if let Some(v) = sk.rolling {
        // Skate 3's grain is one recording from slow to fast rolling; its bands play by speed (to 8.5 m/s, pushing speed).
        let bands = bank.by_role.get("roll").map_or(0, Vec::len);
        if bands > 0 {
            let band = ((v / 8.5 * bands as f32) as usize).min(bands - 1);
            let quiet = if sk.sliding { 0.3 } else { 1.0 };
            want.push(("roll", band, (v / 1.5).min(1.0) * 0.7 * quiet * master, 0.95 + 0.1 * (v / 8.5).min(1.0)));
        }
    }
    if sk.sliding {
        want.push(("slide", 0, 0.9 * master, 1.0));
    }
    if sk.braking {
        want.push(("brake", 0, 0.8 * master, 1.0));
    }
    for (e, lv, mut sink) in &mut loops {
        match want.iter().position(|w| w.0 == lv.role && w.1 == lv.index) {
            Some(i) => {
                let (_, _, vol, speed) = want.remove(i);
                sink.set_volume(Volume::Linear(vol));
                lv.speed.set(speed);
            }
            None => commands.entity(e).despawn(),
        }
    }
    for (role, index, vol, speed) in want {
        let Some(wave) = bank.by_role.get(role).and_then(|l| l.get(index)) else { continue };
        let speed = Speed::new(speed);
        // (It loops inside its source, so its pitch can follow; it never ends: `Once` keeps it until it's despawned.)
        let h = sounds.add(PitchedSound::looping(wave, &speed));
        commands.spawn((LoopVoice { role, index, speed }, AudioPlayer(h), PlaybackSettings { mode: PlaybackMode::Once, volume: Volume::Linear(vol), ..default() }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every sound the mod needs has a role, and every role has a sound (the bank as `copy_skate3` lists it).
    #[test]
    fn every_role_has_its_sounds() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mods/skate3");
        let cfg = std::fs::read_to_string(dir.join("mod.cfg")).unwrap();
        let needs: Vec<String> = cfg.lines().filter_map(|l| l.strip_prefix("Need \"")?.strip_suffix('"').map(str::to_string)).collect();
        let read = |rel: &str| std::fs::read(dir.join(rel)).map_err(|e| e.to_string());
        let bank = SoundBank::load(&read, &needs).unwrap();
        for role in bank.by_role.keys() {
            assert!(ROLES.contains(&role.as_str()), "a sound for no role: {role}");
        }
        for role in ROLES {
            assert!(bank.by_role.get(role).is_some_and(|l| !l.is_empty()), "no sound for {role}");
        }
    }
}
