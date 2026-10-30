//! Engine sounds: every `EngineEffect` of a module config has a looping `Sound` and a `Volume`
//! (`SoundVolume`), read by FUN_1402a8ed0. A cfg without `Sound` gets the executable's default for
//! its group (FUN_1405693a0, [`EngineEffect::sound_file`]); without `Volume` the volume is 1, or
//! 0.02 for thrusters.
//!
//! SE plays the sounds through SDL_mixer channels with fades (`seSound::FadeIn` / `FadeOut`,
//! FUN_140567f30 / FUN_140568060 on `Mix_FadeInChannelTimed` / `Mix_FadeOutChannel`) and channel
//! volumes (`Mix_Volume`, FUN_140567eb0) at the `SoundVolume` setting. It imports no positional
//! call (`Mix_SetPosition`, `Mix_SetDistance`, `Mix_SetPanning`), so the sounds are not attenuated
//! with distance or panned. The port plays one looping voice per sound file of the flown ship:
//! its volume is the sum over that file's effects of `Volume` x the effect's level (the level
//! that also scales its plume: spooled engine, RCS command, warp), as one SDL_mixer channel per
//! effect would mix, clipped to 1 and times `SoundVolume`, faded over [`FADE_S`]. Assumed: the
//! fade time (SE's fade calls for ship sounds were not traced); voices are silent outside flight
//! mode.
//!
//! [`EngineEffect::sound_file`]: se_core::ship::EngineEffect::sound_file

use std::collections::HashMap;
use std::sync::Arc;

use bevy::audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;

use super::{Flight, Ship, ShipLoading, ShipVisuals, plume_level};
use crate::data::SeData;
use crate::sound::SoundSettings;

/// Fade time (s) of a voice from silence to full volume.
pub const FADE_S: f32 = 0.3;

/// Loaded engine sounds by VFS path (`None`: not found).
#[derive(Resource, Default)]
pub struct EngineSoundCache(HashMap<String, Option<Handle<AudioSource>>>);

/// One looping sound of a ship and the plumes (effects) that drive it.
struct Voice {
    entity: Entity,
    /// (plume index, `Volume`).
    effects: Vec<(usize, f32)>,
    volume: f32,
}

#[derive(Component, Default)]
pub struct ShipSounds {
    voices: Vec<Voice>,
}

/// Starts the voices of ships whose model has finished loading.
pub fn start_engine_sounds(
    mut commands: Commands,
    data: Res<SeData>,
    mut cache: ResMut<EngineSoundCache>,
    mut assets: ResMut<Assets<AudioSource>>,
    ships: Query<(Entity, &ShipVisuals), (Without<ShipSounds>, Without<ShipLoading>)>,
) {
    for (ship, vis) in &ships {
        let mut by_file: Vec<(Handle<AudioSource>, Vec<(usize, f32)>)> = Vec::new();
        for (i, p) in vis.plumes.iter().enumerate() {
            let Some(name) = p.spec.sound.as_deref() else { continue };
            let handle = cache
                .0
                .entry(name.to_owned())
                .or_insert_with(|| {
                    let found = se_core::ship::sound_candidates(name).into_iter().find_map(|path| data.vfs.read(&path).ok());
                    match found {
                        Some(b) => Some(assets.add(AudioSource { bytes: Arc::from(b) })),
                        None => {
                            warn!("ship sound {name} not found");
                            None
                        }
                    }
                })
                .clone();
            let Some(handle) = handle else { continue };
            match by_file.iter_mut().find(|(h, _)| *h == handle) {
                Some((_, v)) => v.push((i, p.spec.volume)),
                None => by_file.push((handle, vec![(i, p.spec.volume)])),
            }
        }
        let voices = by_file
            .into_iter()
            .map(|(handle, effects)| {
                let entity = commands
                    .spawn((ChildOf(ship), AudioPlayer(handle), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)).paused()))
                    .id();
                Voice { entity, effects, volume: 0.0 }
            })
            .collect::<Vec<_>>();
        if !voices.is_empty() {
            info!("ship sounds: {} voices", voices.len());
        }
        commands.entity(ship).insert(ShipSounds { voices });
    }
}

/// Volume (0..1, before `SoundVolume`) of a voice from its effects' levels.
pub fn voice_level(effects: &[(usize, f32)], levels: &[f32]) -> f32 {
    effects.iter().map(|&(i, v)| v * levels.get(i).copied().unwrap_or(0.0)).sum::<f32>().clamp(0.0, 1.0)
}

/// Fades every voice towards its level.
pub fn update_engine_sounds(
    time: Res<Time>,
    flight: Res<Flight>,
    settings: Res<SoundSettings>,
    mut ships: Query<(Entity, &Ship, &ShipVisuals, &mut ShipSounds)>,
    mut sinks: Query<&mut AudioSink>,
) {
    let dt = time.delta_secs();
    for (entity, ship, vis, mut sounds) in &mut ships {
        let flying = flight.active && flight.ship == Some(entity);
        let levels: Vec<f32> = vis.plumes.iter().map(|p| plume_level(&p.spec, ship, &flight.input)).collect();
        for voice in &mut sounds.voices {
            let want = if flying { voice_level(&voice.effects, &levels) * settings.volume } else { 0.0 };
            let step = dt / FADE_S;
            voice.volume += (want - voice.volume).clamp(-step, step);
            let Ok(mut sink) = sinks.get_mut(voice.entity) else { continue };
            if voice.volume <= 1e-4 {
                if !sink.is_paused() {
                    sink.pause();
                }
                continue;
            }
            sink.set_volume(Volume::Linear(voice.volume));
            if sink.is_paused() {
                sink.play();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_sum_their_effects_and_clip() {
        // Two thrusters at SE's default 0.02, one firing fully.
        assert!((voice_level(&[(0, 0.02), (1, 0.02)], &[1.0, 0.0]) - 0.02).abs() < 1e-6);
        // Two main engines at volume 1, half thrust: clipped to full.
        assert_eq!(voice_level(&[(0, 1.0), (1, 1.0)], &[0.5, 0.75]), 1.0);
        assert_eq!(voice_level(&[(3, 1.0)], &[0.0]), 0.0);
    }
}
