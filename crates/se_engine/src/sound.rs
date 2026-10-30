//! SE's interface sounds: `data/sounds/menu/button.ogg` when a button, check box or list entry is
//! clicked and `menu.ogg` when a drop-down menu opens (SE's FUN_140568ab0 loads both), at the
//! `SoundVolume` setting (main-def.cfg 30 of 100; Settings "Sound effects").
//!
//! The clicks come from egui's output events of the last interface pass. Assumed: which widgets
//! sound (SE's own interface is not egui).

use std::sync::Arc;

use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use bevy_egui::EguiOutput;
use bevy_egui::egui::{WidgetType, output::OutputEvent};

use crate::data::SeData;

/// SE's `SoundVolume` 30 (of 100).
pub const DEFAULT_SOUND_VOLUME: f32 = 0.3;

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SoundSettings { volume: DEFAULT_SOUND_VOLUME })
            .init_resource::<UiSounds>()
            .add_systems(Startup, load_sounds)
            .add_systems(Update, play_clicks);
    }
}

#[derive(Resource, Debug, Clone, PartialEq)]
pub struct SoundSettings {
    /// Linear volume of the sound effects, 0.0..=1.0.
    pub volume: f32,
}

#[derive(Resource, Default)]
struct UiSounds {
    button: Option<Handle<AudioSource>>,
    menu: Option<Handle<AudioSource>>,
}

fn load_sounds(data: Res<SeData>, mut assets: ResMut<Assets<AudioSource>>, mut sounds: ResMut<UiSounds>) {
    let mut load = |name: &str| match data.vfs.read(&format!("sounds/menu/{name}")) {
        Ok(b) => Some(assets.add(AudioSource { bytes: Arc::from(b) })),
        Err(e) => {
            warn!("sounds: cannot read {name}: {e}");
            None
        }
    };
    sounds.button = load("button.ogg");
    sounds.menu = load("menu.ogg");
}

fn play_clicks(mut commands: Commands, outputs: Query<Ref<EguiOutput>>, sounds: Res<UiSounds>, settings: Res<SoundSettings>) {
    if settings.volume <= 0.0 {
        return;
    }
    for out in &outputs {
        if !out.is_changed() {
            continue;
        }
        // One sound per pass, the menu's if a drop-down opened.
        let mut play = None;
        for ev in &out.platform_output.events {
            if let OutputEvent::Clicked(info) = ev {
                match info.typ {
                    WidgetType::ComboBox => play = sounds.menu.clone(),
                    WidgetType::Button | WidgetType::Checkbox | WidgetType::RadioButton | WidgetType::SelectableLabel | WidgetType::CollapsingHeader => {
                        play = play.or(sounds.button.clone())
                    }
                    _ => {}
                }
            }
        }
        if let Some(h) = play {
            commands.spawn((AudioPlayer(h), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(settings.volume))));
        }
    }
}
