//! The ship's sounds (T5b-1): `sounds.json` (`se_core::elite_sound`) and `sfx/<source id>.ogg`, from the audio export
//! (`sidewinder_audio`): `events.json` (every Wwise event the Sidewinder uses with everything the banks say of it, made by
//! `re/elite/tools/sidewinder_events.py`), `media/` (each recording the events reach, the game's own packets rewrapped,
//! or its exact decoded samples as 16-bit PCM where the stream cannot be rewrapped (63: ADPCM, and Vorbis streams ww2ogg
//! cannot rebuild), made by `re/elite/tools/sidewinder_media.py`, with `index.json` and the source plugins' `plugins.json`).
//!
//! Which events ship: `SHIP_EVENTS` (each with the `triggers.json` entry or state machine cue that proves the game posts
//! it) and `OPEN_EVENTS` (the sender is not found, but the event is for something the mod has: shipped when the banks at
//! rest hold all its media, so the trigger is all a later task adds). Then everything they reach, trimmed from
//! `events.json` without changing a value: the events they post, the actions, the object trees with their actor-mixer
//! parents, the buses up to the master, the attenuations, the game syncs they name (from the Init bank), the media, the
//! source plugins and the names. Left out as derived or not used by the mixer: each object's `resolved` summary, `bank`,
//! `banks`, a Sound's `media` summary (the `media` table replaces it), the State objects (their deltas are inline in each
//! node's state block), the modulators, and the effects other than the Parametric EQ, Guitar Distortion, Peak Limiter and
//! the master's plug-in 0x00BA0003 (not built: `se_core::elite_sound`'s docs). Those in the kept nodes' and buses' slots come from `effects.json` (beside
//! `events.json`, made by `re/elite/tools/sidewinder_effects.py`: the buses' effects are in the Init bank, which
//! `events.json` does not hold), their parameter blocks decoded (`effect`). Every recording shipped
//! passed `sidewinder_media.py`'s proof (`decode_identical` in `media/index.json`: the file decodes to the `.wem`'s audio,
//! sample for sample); `build` refuses any other.
//!
//! The ship UI's events (`ui_sound.rs` picks them) come from a second export of the same shape, made by the same tools
//! (`re/elite/frea_export/sidewinder_ui/audio`); the two are merged (`Export::merge`) before anything is reached, so the
//! one `sounds.json` holds both.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use se_core::elite_sound::{data::FORMAT, event_id, parse_sounds, write_sounds, Mixer, SoundBank, SoundFile, COMPRESSOR, GUITAR_DISTORTION, MASTER_SUITE, PARAMETRIC_EQ, PEAK_LIMITER};

use super::ui_sound::{self, UiPick};
use super::{Ctx, Shipped};

/// Every event the game posts, with what proves it (`triggers.json` entry ids, `ship.json` state machine cues).
pub(super) const SHIP_EVENTS: &[(&str, &str)] = &[
    // The internal reporter's 8 drive loops: start in audio mode 1 (engine_loops_start), stopped 3 s after engines off
    // and started again on engines on (engines_on_off).
    ("ShipOrigin_b_sidewinder_internal", "engine_loops_start"),
    ("MainDriveL_b_sidewinder_internal", "engine_loops_start"),
    ("MainDriveR_b_sidewinder_internal", "engine_loops_start"),
    ("MainDriveC_b_sidewinder_internal", "engine_loops_start"),
    ("RetroDriveL_b_sidewinder_internal", "engine_loops_start"),
    ("RetroDriveR_b_sidewinder_internal", "engine_loops_start"),
    ("RetroDriveC_b_sidewinder_internal", "engine_loops_start"),
    ("Front_Centre_b_sidewinder_internal", "engine_loops_start, mode_loops (the cockpit loop of the same name)"),
    ("ShipOrigin_e_sidewinder_internal", "engines_on_off"),
    ("MainDriveL_e_sidewinder_internal", "engines_on_off"),
    ("MainDriveR_e_sidewinder_internal", "engines_on_off"),
    ("MainDriveC_e_sidewinder_internal", "engines_on_off"),
    ("RetroDriveL_e_sidewinder_internal", "engines_on_off"),
    ("RetroDriveR_e_sidewinder_internal", "engines_on_off"),
    ("RetroDriveC_e_sidewinder_internal", "engines_on_off"),
    ("Front_Centre_e_sidewinder_internal", "engines_on_off, mode_loops"),
    // The cockpit reporter's loop sets by audio mode (mode_loops): 9 cockpit loops and 9 shared beds in mode 1, the shared
    // beds and 9 supercruise beds in mode 2, 9 hyperspace beds in mode 3; each _b on entering, _e on leaving.
    ("Back_Centre_b_sidewinder_internal", "mode_loops"),
    ("Back_Centre_e_sidewinder_internal", "mode_loops"),
    ("Back_Left_b_sidewinder_internal", "mode_loops"),
    ("Back_Left_e_sidewinder_internal", "mode_loops"),
    ("Back_Right_b_sidewinder_internal", "mode_loops"),
    ("Back_Right_e_sidewinder_internal", "mode_loops"),
    ("Front_Left_b_sidewinder_internal", "mode_loops"),
    ("Front_Left_e_sidewinder_internal", "mode_loops"),
    ("Front_Right_b_sidewinder_internal", "mode_loops"),
    ("Front_Right_e_sidewinder_internal", "mode_loops"),
    ("Mid_Centre_b_sidewinder_internal", "mode_loops"),
    ("Mid_Centre_e_sidewinder_internal", "mode_loops"),
    ("Mid_Left_b_sidewinder_internal", "mode_loops"),
    ("Mid_Left_e_sidewinder_internal", "mode_loops"),
    ("Mid_Right_b_sidewinder_internal", "mode_loops"),
    ("Mid_Right_e_sidewinder_internal", "mode_loops"),
    ("Back_Centre_SharedCockpit_b", "mode_loops"),
    ("Back_Centre_SharedCockpit_e", "mode_loops"),
    ("Back_Left_SharedCockpit_b", "mode_loops"),
    ("Back_Left_SharedCockpit_e", "mode_loops"),
    ("Back_Right_SharedCockpit_b", "mode_loops"),
    ("Back_Right_SharedCockpit_e", "mode_loops"),
    ("Front_Centre_SharedCockpit_b", "mode_loops"),
    ("Front_Centre_SharedCockpit_e", "mode_loops"),
    ("Front_Left_SharedCockpit_b", "mode_loops"),
    ("Front_Left_SharedCockpit_e", "mode_loops"),
    ("Front_Right_SharedCockpit_b", "mode_loops"),
    ("Front_Right_SharedCockpit_e", "mode_loops"),
    ("Mid_Centre_SharedCockpit_b", "mode_loops"),
    ("Mid_Centre_SharedCockpit_e", "mode_loops"),
    ("Mid_Left_SharedCockpit_b", "mode_loops"),
    ("Mid_Left_SharedCockpit_e", "mode_loops"),
    ("Mid_Right_SharedCockpit_b", "mode_loops"),
    ("Mid_Right_SharedCockpit_e", "mode_loops"),
    ("Back_Centre_Supercruise_Cockpit_b", "mode_loops"),
    ("Back_Centre_Supercruise_Cockpit_e", "mode_loops"),
    ("Back_Left_Supercruise_Cockpit_b", "mode_loops"),
    ("Back_Left_Supercruise_Cockpit_e", "mode_loops"),
    ("Back_Right_Supercruise_Cockpit_b", "mode_loops"),
    ("Back_Right_Supercruise_Cockpit_e", "mode_loops"),
    ("Front_Centre_Supercruise_Cockpit_b", "mode_loops"),
    ("Front_Centre_Supercruise_Cockpit_e", "mode_loops"),
    ("Front_Left_Supercruise_Cockpit_b", "mode_loops"),
    ("Front_Left_Supercruise_Cockpit_e", "mode_loops"),
    ("Front_Right_Supercruise_Cockpit_b", "mode_loops"),
    ("Front_Right_Supercruise_Cockpit_e", "mode_loops"),
    ("Mid_Centre_Supercruise_Cockpit_b", "mode_loops"),
    ("Mid_Centre_Supercruise_Cockpit_e", "mode_loops"),
    ("Mid_Left_Supercruise_Cockpit_b", "mode_loops"),
    ("Mid_Left_Supercruise_Cockpit_e", "mode_loops"),
    ("Mid_Right_Supercruise_Cockpit_b", "mode_loops"),
    ("Mid_Right_Supercruise_Cockpit_e", "mode_loops"),
    ("Back_Centre_Hyperspace_Cockpit_b", "mode_loops"),
    ("Back_Centre_Hyperspace_Cockpit_e", "mode_loops"),
    ("Back_Left_Hyperspace_Cockpit_b", "mode_loops"),
    ("Back_Left_Hyperspace_Cockpit_e", "mode_loops"),
    ("Back_Right_Hyperspace_Cockpit_b", "mode_loops"),
    ("Back_Right_Hyperspace_Cockpit_e", "mode_loops"),
    ("Front_Centre_Hyperspace_Cockpit_b", "mode_loops"),
    ("Front_Centre_Hyperspace_Cockpit_e", "mode_loops"),
    ("Front_Left_Hyperspace_Cockpit_b", "mode_loops"),
    ("Front_Left_Hyperspace_Cockpit_e", "mode_loops"),
    ("Front_Right_Hyperspace_Cockpit_b", "mode_loops"),
    ("Front_Right_Hyperspace_Cockpit_e", "mode_loops"),
    ("Mid_Centre_Hyperspace_Cockpit_b", "mode_loops"),
    ("Mid_Centre_Hyperspace_Cockpit_e", "mode_loops"),
    ("Mid_Left_Hyperspace_Cockpit_b", "mode_loops"),
    ("Mid_Left_Hyperspace_Cockpit_e", "mode_loops"),
    ("Mid_Right_Hyperspace_Cockpit_b", "mode_loops"),
    ("Mid_Right_Hyperspace_Cockpit_e", "mode_loops"),
    // Boost states 0-3 and the refused boost (boost).
    ("boost_idle_b_sidewinder_internal", "boost"),
    ("boost_idle_e_sidewinder_internal", "boost"),
    ("boost_warmup_b_sidewinder_internal", "boost"),
    ("boost_warmup_e_sidewinder_internal", "boost"),
    ("boost_firing_b_sidewinder_internal", "boost"),
    ("boost_firing_e_sidewinder_internal", "boost"),
    ("boost_cooldown_b_sidewinder_internal", "boost"),
    ("boost_cooldown_e_sidewinder_internal", "boost"),
    ("boost_failed_b_sidewinder_internal", "boost"),
    ("boost_failed_e_sidewinder_internal", "boost"),
    // Flight assist and the cockpit beep (carried_00, carried_01 measured).
    ("FlightAssist_b", "carried_00"),
    ("FlightAssist_e", "carried_00"),
    ("Cockpit_FlightAssist_b", "carried_01 (measured)"),
    ("Cockpit_FlightAssist_e", "carried_01 (measured)"),
    // The frame shift drive: charge, countdown and voice, cancel, point of no return (fsd_charge, fsd_countdown,
    // fsd_cancel_and_point_of_no_return; the _e of the point of no return and of the jump at teardown).
    ("hyperdrive_charge_internal_b", "fsd_charge"),
    ("hyperdrive_charge_internal_e", "fsd_charge"),
    ("supercruise_charge_internal_b", "fsd_charge"),
    ("supercruise_charge_internal_e", "fsd_charge"),
    ("VoxV01_FrameshiftDriveCharging", "fsd_charge, voice_fsd_lines"),
    ("hyperdrive_count_internal", "fsd_countdown"),
    ("supercruise_count_internal", "fsd_countdown"),
    ("VoxV01_Number5", "fsd_countdown"),
    ("VoxV01_Number4", "fsd_countdown"),
    ("VoxV01_Number3", "fsd_countdown"),
    ("VoxV01_Number2", "fsd_countdown"),
    ("VoxV01_Number1", "fsd_countdown, voice_fsd_lines"),
    ("VoxV01_Engage", "fsd_countdown, voice_fsd_lines"),
    ("hyperdrive_cancel_internal", "fsd_cancel_and_point_of_no_return"),
    ("hyperdrive_PointOfNoReturn_internal_b", "fsd_cancel_and_point_of_no_return"),
    ("supercruise_PointOfNoReturn_internal_b", "fsd_cancel_and_point_of_no_return"),
    ("hyperdrive_PointOfNoReturn_internal_e", "fsd_cancel_and_point_of_no_return (posted at teardown)"),
    ("supercruise_PointOfNoReturn_internal_e", "fsd_cancel_and_point_of_no_return (posted at teardown)"),
    ("VoxV01_FrameshiftReadyToEngage", "fsd_cancel_and_point_of_no_return, voice_fsd_lines"),
    // Audio mode transitions (audio_mode_transitions) and the global enter events with FrameShiftState (frame_shift_state).
    ("normalflight_supercruise_transition_b", "audio_mode_transitions"),
    ("normalflight_hyperspace_transition_b", "audio_mode_transitions"),
    ("supercruise_normalflight_transition_b", "audio_mode_transitions"),
    ("supercruise_hyperspace_transition_b", "audio_mode_transitions"),
    ("hyperspace_supercruise_transition_b", "audio_mode_transitions"),
    // (Each transition's _e when the next transition begins: FUN_14427b320 releases the last handle, whose destructor
    // FUN_1440ac360 posts it.)
    ("normalflight_supercruise_transition_e", "audio_mode_transitions"),
    ("normalflight_hyperspace_transition_e", "audio_mode_transitions"),
    ("supercruise_normalflight_transition_e", "audio_mode_transitions"),
    ("supercruise_hyperspace_transition_e", "audio_mode_transitions"),
    ("hyperspace_supercruise_transition_e", "audio_mode_transitions"),
    ("normalflight_enter", "frame_shift_state"),
    ("normalflight_enter_from_supercruise", "frame_shift_state"),
    ("normalflight_enter_from_hyperspace", "frame_shift_state"),
    ("supercruise_enter", "frame_shift_state"),
    ("supercruise_enter_from_normalflight", "frame_shift_state"),
    ("supercruise_enter_from_hyperspace", "frame_shift_state"),
    ("hyperspace_enter", "frame_shift_state"),
    ("hyperspace_enter_from_normalflight", "frame_shift_state"),
    ("hyperspace_enter_from_supercruise", "frame_shift_state"),
    ("supercruise_emergencystop", "frame_shift_state"),
    // Mass lock, the drive's cockpit cues and cooldown (fsd_cooldown_masslock_cues).
    ("Cockpit_FrameShiftInitiated", "fsd_cooldown_masslock_cues"),
    ("Cockpit_FrameShiftUnsuccessful", "fsd_cooldown_masslock_cues"),
    ("Cockpit_FrameShiftCancel", "fsd_cooldown_masslock_cues"),
    ("Cockpit_MassLockEngage", "fsd_cooldown_masslock_cues"),
    ("Cockpit_MassLockDisengage", "fsd_cooldown_masslock_cues"),
    ("Message_b_SlowDown", "fsd_cooldown_masslock_cues"),
    ("Message_b_SafetoDisengage", "fsd_cooldown_masslock_cues"),
    ("Cockpit_FsdCooldown_b", "fsd_cooldown_masslock_cues"),
    ("Cockpit_FsdCooldown_e", "fsd_cooldown_masslock_cues"),
    // Lights, fuel scoop, power plant, reboot (cockpit_lights_fuelscoop_reboot).
    ("Cockpit_LightsOn", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_LightsOff", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_FuelScoop_b", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_FuelScoop_e", "cockpit_lights_fuelscoop_reboot"),
    ("PowerPlantCapacityExceeded_b", "cockpit_lights_fuelscoop_reboot"),
    ("PowerPlantCapacityExceeded_e", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_RebootStartSequence_b", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_RebootStartSequence_e", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_RebootSequence_b", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_RebootSequence_e", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_RebootSequenceSuccessful", "cockpit_lights_fuelscoop_reboot"),
    ("Cockpit_RebootSequenceFail", "cockpit_lights_fuelscoop_reboot"),
    // Cargo scoop (cargo_scoop).
    ("CargoScoop_Deployed", "cargo_scoop"),
    ("CargoScoop_Retracted", "cargo_scoop"),
    ("CargoScoop_b", "cargo_scoop"),
    ("CargoScoop_e", "cargo_scoop"),
    ("CargoScoopAquired", "cargo_scoop"),
    ("CargoScoopWarning_b", "cargo_scoop"),
    ("CargoScoopWarning_e", "cargo_scoop"),
    ("CargoScoopRefineryWarning_b", "cargo_scoop"),
    ("CargoScoopRefineryWarning_e", "cargo_scoop"),
    // Landing gear: the cockpit cues (landing_gear) and every sound cue of ship.json's machines (each begin when its cue
    // window opens, end when it closes).
    ("LandingGear_Deployed", "landing_gear"),
    ("LandingGear_Retracted", "landing_gear"),
    ("LandingGearLower_b_sidewinder", "ship.json LandingGearExtension cue LandingGearLower"),
    ("LandingGearLower_e_sidewinder", "ship.json LandingGearExtension cue LandingGearLower"),
    ("LandingGearRaise_b_sidewinder", "ship.json LandingGearExtension cue LandingGearRaise"),
    ("LandingGearRaise_e_sidewinder", "ship.json LandingGearExtension cue LandingGearRaise"),
    ("SideWinder_Rear_Deploy_Door_Hinge_b_landinggear", "ship.json LandingGearCSM cue (Side)Winder_Rear_Deploy_Door_Hinge"),
    ("SideWinder_Rear_Deploy_Door_Hinge_e_landinggear", "ship.json LandingGearCSM cue (Side)Winder_Rear_Deploy_Door_Hinge"),
    ("SideWinder_Rear_Deploy_Motor01_b_landinggear", "ship.json LandingGearCSM cue (Side)Winder_Rear_Deploy_Motor01"),
    ("SideWinder_Rear_Deploy_Motor01_e_landinggear", "ship.json LandingGearCSM cue (Side)Winder_Rear_Deploy_Motor01"),
    ("SideWinder_Rear_Deploy_Motor02_b_landinggear", "ship.json LandingGearCSM cue (Side)Winder_Rear_Deploy_Motor02"),
    ("SideWinder_Rear_Deploy_Motor02_e_landinggear", "ship.json LandingGearCSM cue (Side)Winder_Rear_Deploy_Motor02"),
    ("Sidewinder_Deploy_Motor01_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Deploy_Motor01, carried_07"),
    ("Sidewinder_Deploy_Motor01_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Deploy_Motor01, carried_07"),
    ("Sidewinder_Deploy_Motor02_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Deploy_Motor02"),
    ("Sidewinder_Deploy_Motor02_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Deploy_Motor02"),
    ("Sidewinder_Deploy_Motor03_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Deploy_Motor03"),
    ("Sidewinder_Deploy_Motor03_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Deploy_Motor03"),
    ("Sidewinder_Rear_Stow_Door_Hinge_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Rear_Stow_Door_Hinge"),
    ("Sidewinder_Rear_Stow_Door_Hinge_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Rear_Stow_Door_Hinge"),
    ("Sidewinder_Rear_Stow_Motor01_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Rear_Stow_Motor01"),
    ("Sidewinder_Rear_Stow_Motor01_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Rear_Stow_Motor01"),
    ("Sidewinder_Rear_Stow_Motor02_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Rear_Stow_Motor02"),
    ("Sidewinder_Rear_Stow_Motor02_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Rear_Stow_Motor02"),
    ("Sidewinder_Stow_Motor01_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Stow_Motor01, carried_07"),
    ("Sidewinder_Stow_Motor01_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Stow_Motor01, carried_07"),
    ("Sidewinder_Stow_Motor02_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Stow_Motor02"),
    ("Sidewinder_Stow_Motor02_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Stow_Motor02"),
    ("Sidewinder_Stow_Motor03_b_landinggear", "ship.json LandingGearCSM cue Sidewinder_Stow_Motor03"),
    ("Sidewinder_Stow_Motor03_e_landinggear", "ship.json LandingGearCSM cue Sidewinder_Stow_Motor03"),
    ("DoorOpen_Rear_b_sidewinder_external", "ship.json LandingGearDoorsLeft / Right cue DoorOpen_Rear"),
    ("DoorOpen_Rear_e_sidewinder_external", "ship.json LandingGearDoorsLeft / Right cue DoorOpen_Rear"),
    ("DoorClose_Rear_b_sidewinder_external", "ship.json LandingGearDoorsLeft / Right cue DoorClose_Rear"),
    ("DoorClose_Rear_e_sidewinder_external", "ship.json LandingGearDoorsLeft / Right cue DoorClose_Rear"),
    // Hardpoint doors (hardpoints_doors: ship.json LeftHardpointDoor / RightHardpointDoor cues).
    ("DoorLeft_Open_b_sidewinder_external", "hardpoints_doors"),
    ("DoorLeft_Open_e_sidewinder_external", "hardpoints_doors"),
    ("DoorLeft_Close_b_sidewinder_external", "hardpoints_doors"),
    ("DoorLeft_Close_e_sidewinder_external", "hardpoints_doors"),
    ("DoorRight_Open_b_sidewinder_external", "hardpoints_doors"),
    ("DoorRight_Open_e_sidewinder_external", "hardpoints_doors"),
    ("DoorRight_Close_b_sidewinder_external", "hardpoints_doors"),
    ("DoorRight_Close_e_sidewinder_external", "hardpoints_doors"),
    // Silent running (silent_running), the canopy breach (canopy_crack).
    ("SilentRunning_b", "silent_running"),
    ("SilentRunning_e", "silent_running"),
    ("CockpitBreach_None", "canopy_crack (ship.json CockpitState: AudioPostGlobalEvent at Canopy Breached)"),
    // Heat, malfunction, hull and shield voice lines and sounds (carried_02 .. carried_06).
    ("VoxV01_WarningTemperatureCritical", "carried_02"),
    ("VoxV01_WarningTakingHeatDamage", "carried_02"),
    ("VoxV01_HeatSinkDeployed", "carried_03"),
    ("WarningMalfunction_b", "carried_04"),
    ("VoxV01_HullIntegrityCompromised", "carried_05"),
    ("VoxV01_TakingDamage", "carried_05"),
    ("VoxV01_HullIntegrityCritical", "carried_05"),
    ("VoxV01_UnderAttack", "carried_05 (UnderAttack from 0x144227ad2)"),
    ("VoxV01_ShieldsOffline", "carried_06"),
    ("VoxV01_ShieldsOnline", "carried_06"),
    ("Break_ShieldGenerator", "carried_06"),
    ("TurnOff_ShieldGenerator", "carried_06"),
    ("Reform_ShieldGenerator", "carried_06"),
];

/// Events for things the mod has whose sender is not found: what is missing. `bank:NAME` stands for every unnamed event of
/// that bank. Shipped when the banks at rest hold all their media. Left out as cues about other ships, stations and
/// contacts, which the mod does not have: `TargetSelectShip*` (6), `TargetSelectStation`, `TargetHitIndicator`,
/// `ContactsIncrease`, `HostileContactsIncrease`, `HardpointsDeployedContactsIncrease`, the lock-on cues.
pub(super) const OPEN_EVENTS: &[(&str, &str)] = &[
    ("bank:hpt_pulselaser_small_lod0", "laser_fire: which of the unnamed events is the shot (and its LOD rule) is in the hardpoint's data, not at rest"),
    ("bank:hpt_pulselaser_small_lod1", "laser_fire: as lod0"),
    ("hyperdrive_jump_internal_b", "fsd_jump: its handle is made but no begin post was found"),
    ("hyperdrive_jump_internal_e", "fsd_jump: posted only at the audio object's teardown; the jump's own sender is not found"),
    ("supercruise_jump_internal_b", "fsd_jump: no poster found"),
    ("supercruise_jump_internal_e", "fsd_jump: no poster found"),
    ("hyperdrive_jump_internal", "fsd_jump: no poster found"),
    ("supercruise_jump_internal", "fsd_jump: no poster found"),
    ("supercruise_enter_Internal", "frame_shift_state: the *_Internal events have no poster found"),
    ("supercruise_emergencystop_Internal", "frame_shift_state: no poster found"),
    ("hyperspace_enter_Internal", "frame_shift_state: no poster found"),
    ("hyperspace_exit_Internal", "frame_shift_state: no poster found"),
    ("supercruise_exit_Internal", "frame_shift_state: no poster found"),
    ("supercruise_exit", "frame_shift_state: no poster found"),
    ("hyperspace_exit", "frame_shift_state: no poster found"),
    ("normalflight_exit", "frame_shift_state: no poster found"),
    ("supercruise_manual_exit", "frame_shift_state: no poster found"),
    ("supercruise_manual_exit_Internal", "frame_shift_state: no poster found"),
    ("TargetSelectNavigational", "cockpit_ui_targets_radar: the cockpit handles +0xf20..+0x10a0 are not mapped"),
    ("TargetSelectObject", "cockpit_ui_targets_radar"),
    ("TargetDeselect", "cockpit_ui_targets_radar"),
    ("RadarPulse", "cockpit_ui_targets_radar"),
    ("Cockpit_CycleWeaponGroup", "cockpit_ui_targets_radar"),
    ("AnalysisHUD_Activate", "cockpit_ui_targets_radar"),
    ("AnalysisHUD_Deactivate", "cockpit_ui_targets_radar"),
    ("NightVision_Activate", "cockpit_ui_targets_radar"),
    ("NightVision_Deactivate", "cockpit_ui_targets_radar"),
    ("HptDoor_Open_b", "hardpoints_doors: no code or data names it for the Sidewinder"),
    ("HptDoor_Open_e", "hardpoints_doors: as HptDoor_Open_b"),
    ("HptDoor_Close_b", "hardpoints_doors: as HptDoor_Open_b"),
    ("HptDoor_Close_e", "hardpoints_doors: as HptDoor_Open_b"),
    ("VoxV01_MainFuelTankLow", "low_fuel: no call site found"),
    ("VoxV01_MainFuelTankDrained", "low_fuel: no call site found"),
    ("VoxV01_FlightAssistOn", "voice_other_lines: no call site found"),
    ("VoxV01_flightAssistOff", "voice_other_lines: no call site found"),
    ("VoxV01_RotationalCorrectionEnabled", "voice_other_lines: no call site found"),
    ("VoxV01_RotationalCorrectionDisabled", "voice_other_lines: no call site found"),
    ("VoxV01_LandingGearDeployed", "voice_other_lines: no call site found"),
    ("VoxV01_LandingGearRetracted", "voice_other_lines: no call site found"),
    ("VoxV01_cargoScoopDeployed", "voice_other_lines: no call site found"),
    ("VoxV01_CargoScoopRetracted", "voice_other_lines: no call site found"),
    ("VoxV01_SilentRunningOn", "silent_running: no call site found"),
    ("VoxV01_SilentRunningOff", "silent_running: no call site found"),
    ("VoxV01_WarningOxygenLow", "canopy_crack: oxygen lines have no call site found"),
    ("VoxV01_WarningOxygenCritical", "canopy_crack: oxygen lines have no call site found"),
    ("VoxV01_WarningOxygenDepleted", "canopy_crack: oxygen lines have no call site found"),
    ("VoxV01_CanopyCompromised", "canopy_crack: which lines ShipVoiceReporter kinds 0 / 1 say is not closed"),
    ("VoxV01_CanopyCritical", "canopy_crack: as VoxV01_CanopyCompromised"),
    ("VoxV01_FuelScooping", "voice_other_lines: no call site found"),
    ("VoxV01_FuelScoopingComplete", "voice_other_lines: no call site found"),
    ("VoxV01_FuelScoopDisengaged", "voice_other_lines: no call site found"),
    ("Ship_b_sidewinder_external", "the external reporter's set-up is not traced (the own ship heard from outside)"),
    ("Ship_e_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("MainDrives_b_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("MainDrives_e_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("RetroDrives_b_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("RetroDrives_e_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("boost_firing_b_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("boost_firing_e_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("boost_warmup_b_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("boost_warmup_e_sidewinder_external", "as Ship_b_sidewinder_external"),
    ("ship_b_sidewinder_internal", "the internal reporter's own begin / end: no poster found"),
    ("ship_e_sidewinder_internal", "as ship_b_sidewinder_internal"),
    ("LocalPlayer_Centre_Supercruise_b", "a supercruise loop the reporters build: its poster is not traced"),
    ("LocalPlayer_Centre_Supercruise_e", "as LocalPlayer_Centre_Supercruise_b"),
    ("BC_PlanetaryCockpitSmall_b", "planetary cockpit beds: their poster is not traced"),
    ("BC_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
    ("BL_PlanetaryCockpitSmall_b", "planetary cockpit beds"),
    ("BL_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
    ("BR_PlanetaryCockpitSmall_b", "planetary cockpit beds"),
    ("BR_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
    ("FC_PlanetaryCockpitSmall_b", "planetary cockpit beds"),
    ("FC_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
    ("FL_PlanetaryCockpitSmall_b", "planetary cockpit beds"),
    ("FL_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
    ("FR_PlanetaryCockpitSmall_b", "planetary cockpit beds"),
    ("FR_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
    ("MC_PlanetaryCockpitSmall_b", "planetary cockpit beds"),
    ("MC_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
    ("ML_PlanetaryCockpitSmall_b", "planetary cockpit beds"),
    ("ML_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
    ("MR_PlanetaryCockpitSmall_b", "planetary cockpit beds"),
    ("MR_PlanetaryCockpitSmall_e", "planetary cockpit beds"),
];

/// Object types the mixer reads; the others (State objects, modulators, effects) are left out.
const KEPT_TYPES: &[&str] = &["Sound", "RanSeqCntr", "SwitchCntr", "LayerCntr", "ActorMixer", "Action", "Bus", "AuxBus", "Attenuation"];
const NODE_TYPES: &[&str] = &["Sound", "RanSeqCntr", "SwitchCntr", "LayerCntr", "ActorMixer"];

fn read_json(path: &std::path::Path) -> Result<Value, String> {
    let b = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&b).map_err(|e| format!("{}: {e}", path.display()))
}

fn id_of(v: &Value) -> Option<&str> {
    v.get("id").and_then(Value::as_str)
}

/// An audio export read once: events by id (with those only met nested as posted events), objects, game syncs, media.
pub(super) struct Export {
    events: BTreeMap<String, (Option<String>, Vec<String>)>,
    by_name: BTreeMap<String, String>,
    by_bank: BTreeMap<String, Vec<String>>,
    /// The wanted names no bank of the export holds (`events.json`'s `absent`), lower-cased.
    absent: BTreeSet<String>,
    objects: Map<String, Value>,
    syncs: Value,
    units: Value,
    /// The Wwise Meter effects in the buses' slots (`events.json`'s `meters`, decoded by `sidewinder_events.py`).
    meters: Vec<Value>,
    /// Every effect in a slot of a node or bus of `events.json` (`effects.json`, by `sidewinder_effects.py`).
    effects: Map<String, Value>,
    /// Every recording of `media/index.json` (made by `sidewinder_media.py`) with the export it is in.
    media: BTreeMap<String, (Value, std::path::PathBuf)>,
    /// The source plugins (`media/plugins.json`).
    plugins: Map<String, Value>,
}

impl Export {
    pub(super) fn read(audio: &std::path::Path) -> Result<Export, String> {
        let v = read_json(&audio.join("events.json"))?;
        let mut events = BTreeMap::new();
        let mut by_name = BTreeMap::new();
        let mut by_bank: BTreeMap<String, Vec<String>> = BTreeMap::new();
        fn nested(actions: &[Value], events: &mut BTreeMap<String, (Option<String>, Vec<String>)>) {
            for a in actions {
                if let (Some(t), Some(pe)) = (a.get("target").and_then(Value::as_str), a.get("posted_event").and_then(Value::as_array)) {
                    let name = a.get("target_name").and_then(Value::as_str).map(str::to_string);
                    events.entry(t.to_string()).or_insert_with(|| (name, pe.iter().filter_map(|x| id_of(x).map(str::to_string)).collect()));
                    nested(pe, events);
                }
            }
        }
        let evs = v.get("events").and_then(Value::as_object).ok_or("events.json: no events")?;
        for (key, e) in evs {
            let id = id_of(e).ok_or_else(|| format!("events.json: event {key} without an id"))?.to_string();
            let actions: Vec<Value> = e.get("actions").and_then(Value::as_array).cloned().unwrap_or_default();
            let name = e.get("name").and_then(Value::as_str).map(str::to_string);
            by_name.insert(key.to_lowercase(), id.clone());
            if name.is_none() {
                for b in e.get("banks").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                    by_bank.entry(b.to_string()).or_default().push(key.clone());
                }
            }
            events.insert(id, (name, actions.iter().filter_map(|a| id_of(a).map(str::to_string)).collect()));
            nested(&actions, &mut events);
        }
        let objects = v.get("objects").and_then(Value::as_object).cloned().ok_or("events.json: no objects")?;
        let syncs = v.get("game_syncs").cloned().ok_or("events.json: no game_syncs")?;
        let units = v.get("units").cloned().ok_or("events.json: no units")?;
        let meters = v.get("meters").and_then(Value::as_array).cloned().ok_or("events.json: no meters (re-make it with sidewinder_events.py)")?;
        let fx = read_json(&audio.join("effects.json")).map_err(|e| format!("{e} (make it with re/elite/tools/sidewinder_effects.py)"))?;
        let effects = fx.get("effects").and_then(Value::as_object).cloned().ok_or("effects.json: no effects")?;
        // (The game parameters the effects' curves read, defined by the Init bank: events.json defines only its objects'.)
        let mut syncs = syncs;
        let defs = fx.get("game_parameters").and_then(Value::as_object).ok_or("effects.json: no game_parameters (re-make it with sidewinder_effects.py)")?;
        if let Some(table) = syncs.get_mut("game_parameters").and_then(Value::as_object_mut) {
            for (id, d) in defs {
                table.entry(id.clone()).or_insert_with(|| d.clone());
            }
        }
        let absent = v.get("absent").and_then(Value::as_array).into_iter().flatten().filter_map(|a| a.get("name").and_then(Value::as_str)).map(str::to_lowercase).collect();
        let index = read_json(&audio.join("media").join("index.json"))?;
        let index = index.get("media").and_then(Value::as_object).cloned().ok_or("media/index.json: no media")?;
        let media = index.into_iter().map(|(s, m)| (s, (m, audio.join("media")))).collect();
        let plugins = read_json(&audio.join("media").join("plugins.json"))?;
        let plugins = plugins.get("plugins").and_then(Value::as_object).cloned().ok_or("media/plugins.json: no plugins")?;
        Ok(Export { events, by_name, by_bank, absent, objects, syncs, units, meters, effects, media, plugins })
    }

    /// The event id of a name of the tables (events.json's own key, case aside).
    fn event(&self, name: &str) -> Result<String, String> {
        self.by_name.get(&name.to_lowercase()).cloned().ok_or_else(|| format!("events.json has no event {name}"))
    }

    /// Whether the export holds an event of this name (case aside).
    pub(super) fn has(&self, name: &str) -> bool {
        self.by_name.contains_key(&name.to_lowercase())
    }

    /// Whether the export lists the name as wanted and held by no bank (case aside).
    pub(super) fn absent(&self, name: &str) -> bool {
        self.absent.contains(&name.to_lowercase())
    }

    /// The named events the export holds (their own spelling).
    pub(super) fn names(&self) -> impl Iterator<Item = &str> {
        self.events.values().filter_map(|(n, _)| n.as_deref())
    }

    /// This export with another's (the UI's) added. What both hold must be the same, but for what `sidewinder_events.py`'s
    /// own rule for copies in several banks unites (`merge_copy`: a container's children, each bank listing those it holds,
    /// united and sorted; seen on 2 actor-mixers, the UI export's 9 more GUI banks holding a child more) and the summaries
    /// the trim drops (`resolved`, `bank`, `banks`, a Sound's `media`). A recording both hold is taken from this export.
    fn merge(mut self, other: Export) -> Result<Export, String> {
        for (id, e) in other.events {
            match self.events.get(&id) {
                Some(mine) if *mine != e => return Err(format!("event {id} differs between the exports")),
                Some(_) => {}
                None => {
                    self.events.insert(id, e);
                }
            }
        }
        for (name, id) in other.by_name {
            if self.by_name.get(&name).is_some_and(|mine| *mine != id) {
                return Err(format!("{name} names two events"));
            }
            self.by_name.insert(name, id);
        }
        for (bank, names) in other.by_bank {
            let mine = self.by_bank.entry(bank).or_default();
            for n in names {
                if !mine.contains(&n) {
                    mine.push(n);
                }
            }
        }
        for (id, o) in other.objects {
            let merged = match self.objects.get(&id) {
                Some(mine) => united(&id, mine, &o)?,
                None => o,
            };
            self.objects.insert(id, merged);
        }
        if self.units != other.units {
            return Err("the exports' units differ".into());
        }
        for key in ["volume_threshold_db", "max_voices"] {
            if self.syncs.get(key) != other.syncs.get(key) {
                return Err(format!("the exports' game syncs differ in {key}"));
            }
        }
        for table in ["game_parameters", "state_groups", "switch_groups"] {
            let theirs = other.syncs.get(table).and_then(Value::as_object).cloned().unwrap_or_default();
            let mine = self.syncs.get_mut(table).and_then(Value::as_object_mut).ok_or_else(|| format!("events.json: no game_syncs {table}"))?;
            same_union(mine, theirs, table, &[])?;
        }
        for m in other.meters {
            if !self.meters.contains(&m) {
                self.meters.push(m);
            }
        }
        // (An effect's `holders` are the export's nodes that hold it, its `bank` / `banks` where the export met it; a
        // plugin's `from_` the bank its block was read from: none ships.)
        same_union(&mut self.effects, other.effects, "effect", &["holders", "bank", "banks"])?;
        same_union(&mut self.plugins, other.plugins, "source plugin", &["from_"])?;
        for (s, m) in other.media {
            self.media.entry(s).or_insert(m);
        }
        Ok(self)
    }
}

/// `theirs` added to `mine`; an id both hold must be the same but for the fields `ignore` names (the export's own notes,
/// which nothing ships: `mine`'s are kept).
fn same_union(mine: &mut Map<String, Value>, theirs: Map<String, Value>, what: &str, ignore: &[&str]) -> Result<(), String> {
    let strip = |v: &Value| v.as_object().map(|o| o.iter().filter(|(k, _)| !ignore.contains(&k.as_str())).map(|(k, x)| (k.clone(), x.clone())).collect::<Map<String, Value>>());
    for (id, v) in theirs {
        match mine.get(&id) {
            Some(m) if strip(m) != strip(&v) => return Err(format!("{what} {id} differs between the exports")),
            Some(_) => {}
            None => {
                mine.insert(id, v);
            }
        }
    }
    Ok(())
}

/// One object both exports hold, as `Export::merge` takes it.
fn united(id: &str, mine: &Value, theirs: &Value) -> Result<Value, String> {
    let strip = |v: &Value| {
        let mut o = v.as_object().cloned().unwrap_or_default();
        for k in ["resolved", "bank", "banks", "media", "children"] {
            o.remove(k);
        }
        o
    };
    if strip(mine) != strip(theirs) {
        return Err(format!("object {id} differs between the exports"));
    }
    let mut out = mine.clone();
    let kids = |v: &Value| v.get("children").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<BTreeSet<String>>());
    match (kids(mine), kids(theirs)) {
        (Some(a), Some(b)) if a != b => {
            let t = mine.get("type").and_then(Value::as_str).unwrap_or("");
            if !["ActorMixer", "SwitchCntr", "RanSeqCntr", "LayerCntr"].contains(&t) {
                return Err(format!("object {id} ({t}): its children differ between the exports"));
            }
            out["children"] = Value::Array(a.union(&b).cloned().map(Value::String).collect());
        }
        (a, b) if a.is_some() != b.is_some() => return Err(format!("object {id}: children in one export only")),
        _ => {}
    }
    Ok(out)
}

/// What a set of events reaches.
#[derive(Default)]
struct Reach {
    events: BTreeSet<String>,
    objects: BTreeSet<String>,
    /// Nodes whose children have been walked (a node met as a parent first is in `objects` without them).
    expanded: BTreeSet<String>,
    gps: BTreeSet<String>,
    state_groups: BTreeSet<String>,
    switch_groups: BTreeSet<String>,
    sources: BTreeSet<String>,
    plugins: BTreeSet<String>,
}

impl Reach {
    fn event(&mut self, x: &Export, id: &str) -> Result<(), String> {
        if !self.events.insert(id.to_string()) {
            return Ok(());
        }
        let (_, actions) = x.events.get(id).ok_or_else(|| format!("posted event {id} is not in events.json"))?;
        for a in actions {
            let act = x.objects.get(a).ok_or_else(|| format!("action {a} is not in events.json"))?;
            self.objects.insert(a.clone());
            let target = act.get("target").and_then(Value::as_str).unwrap_or("00000000");
            let p = act.get("params").cloned().unwrap_or(Value::Null);
            match act.get("kind").and_then(Value::as_str).unwrap_or("") {
                "PlayEvent" => self.event(x, target)?,
                "SetState" => {
                    self.state_groups.extend(p.pointer("/state_group/id").and_then(Value::as_str).map(str::to_string));
                }
                "SetSwitch" => {
                    self.switch_groups.extend(p.pointer("/switch_group/id").and_then(Value::as_str).map(str::to_string));
                }
                "SetGameParameter" | "ResetGameParameter" => {
                    self.gps.insert(target.to_string());
                }
                "Play" => self.node(x, target)?,
                // A Stop / Seek / Set target is an id the mixer matches against the chains of what plays; it needs no
                // object of its own in the file.
                _ => {}
            }
        }
        Ok(())
    }

    /// A node with its children, its parents and what they refer to.
    fn node(&mut self, x: &Export, id: &str) -> Result<(), String> {
        let o = x.objects.get(id).ok_or_else(|| format!("object {id} is not in events.json"))?;
        if !self.expanded.insert(id.to_string()) {
            return Ok(());
        }
        self.objects.insert(id.to_string());
        self.refs(x, o)?;
        let mut p = o.pointer("/node/parent/id").and_then(Value::as_str).map(str::to_string);
        while let Some(pid) = p {
            let po = x.objects.get(&pid).ok_or_else(|| format!("parent {pid} is not in events.json"))?;
            if self.objects.insert(pid.clone()) {
                self.refs(x, po)?;
            }
            p = po.pointer("/node/parent/id").and_then(Value::as_str).map(str::to_string);
        }
        for c in o.get("children").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            self.node(x, c)?;
        }
        Ok(())
    }

    fn rtpcs(&mut self, list: Option<&Value>) {
        for r in list.and_then(Value::as_array).into_iter().flatten() {
            if r.get("rtpc_type").and_then(Value::as_str) == Some("GameParameter") {
                self.gps.extend(r.pointer("/game_parameter/id").and_then(Value::as_str).map(str::to_string));
            }
        }
    }

    fn states(&mut self, block: Option<&Value>) {
        for g in block.and_then(|b| b.get("groups")).and_then(Value::as_array).into_iter().flatten() {
            self.state_groups.extend(g.pointer("/group/id").and_then(Value::as_str).map(str::to_string));
        }
    }

    fn refs(&mut self, x: &Export, o: &Value) -> Result<(), String> {
        let t = o.get("type").and_then(Value::as_str).unwrap_or("");
        let base = if NODE_TYPES.contains(&t) { o.get("node") } else { Some(o) };
        let Some(base) = base else { return Ok(()) };
        self.rtpcs(base.get("rtpc"));
        self.states(base.get("states"));
        if let Some(a) = base.pointer("/props/AttenuationID/id").and_then(Value::as_str) {
            let att = x.objects.get(a).ok_or_else(|| format!("attenuation {a} is not in events.json"))?;
            self.objects.insert(a.to_string());
            self.rtpcs(att.get("rtpc"));
        }
        if let Some(b) = base.pointer("/override_bus/id").and_then(Value::as_str) {
            self.bus(x, b)?;
        }
        match t {
            "Sound" => {
                let src = o.get("source_id").and_then(Value::as_str).ok_or("a Sound without a source id")?.to_string();
                if o.get("plugin_type").and_then(Value::as_u64) == Some(2) {
                    self.plugins.insert(src);
                } else {
                    self.sources.insert(src);
                }
            }
            "SwitchCntr" => {
                let g = o.pointer("/group/id").and_then(Value::as_str).unwrap_or_default().to_string();
                if o.get("group_type").and_then(Value::as_str) == Some("State") {
                    self.state_groups.insert(g);
                } else {
                    self.switch_groups.insert(g);
                }
            }
            "LayerCntr" => {
                for l in o.get("layers").and_then(Value::as_array).into_iter().flatten() {
                    self.rtpcs(l.get("rtpc"));
                    self.gps.extend(l.pointer("/crossfade_game_parameter/id").and_then(Value::as_str).map(str::to_string));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn bus(&mut self, x: &Export, id: &str) -> Result<(), String> {
        let mut b = Some(id.to_string());
        while let Some(bid) = b {
            if !self.objects.insert(bid.clone()) {
                return Ok(());
            }
            let o = x.objects.get(&bid).ok_or_else(|| format!("bus {bid} is not in events.json"))?;
            self.refs(x, o)?;
            // A ducked bus that events.json lacks carries none of the Sidewinder's sounds: nothing to duck.
            for d in o.get("ducks").and_then(Value::as_array).into_iter().flatten() {
                if let Some(t) = d.pointer("/bus/id").and_then(Value::as_str).filter(|t| x.objects.contains_key(*t)) {
                    self.bus(x, t)?;
                }
            }
            b = o.pointer("/parent_bus/id").and_then(Value::as_str).map(str::to_string);
        }
        Ok(())
    }

    fn finish(&mut self, x: &Export) {
        for m in &x.meters {
            if let (Some(b), Some(gp)) = (m.get("bus").and_then(Value::as_str), m.get("game_parameter").and_then(Value::as_str)) {
                if self.objects.contains(b) {
                    self.gps.insert(gp.to_string());
                }
            }
        }
        for g in self.switch_groups.clone() {
            if let Some(gp) = x.syncs.pointer(&format!("/switch_groups/{g}/driven_by_game_parameter/id")).and_then(Value::as_str) {
                self.gps.insert(gp.to_string());
            }
        }
    }
}

/// `events.json` reads random / sequence containers as Wwise means them (`AkContainerMode` 0 Random, 1 Sequence, as wwiser's
/// `wdefs.py` gives it; `wwise_hirc.py` had them the wrong way round until 2026-11-16): TargetSelectObject's shuffled blips
/// (073c0fe4) are Random, lockOn_b's timed sequence (2aab7fb6) is Sequence, and random containers are the many.
fn check_container_modes(x: &Export) -> Result<(), String> {
    let mode = |id: &str| x.objects.get(id).and_then(|o| o.get("mode")).and_then(Value::as_str);
    let (mut random, mut sequence) = (0, 0);
    for o in x.objects.values().filter(|o| o.get("type").and_then(Value::as_str) == Some("RanSeqCntr")) {
        match o.get("mode").and_then(Value::as_str) {
            Some("Random") => random += 1,
            Some("Sequence") => sequence += 1,
            other => return Err(format!("events.json: a random / sequence container's mode is {other:?}")),
        }
    }
    if mode("073c0fe4") != Some("Random") || mode("2aab7fb6") != Some("Sequence") || random <= sequence {
        return Err(format!(
            "events.json reads random / sequence containers the wrong way round ({random} Random, {sequence} Sequence): \
             re-make it with re/elite/tools/sidewinder_events.py after fixing wwise_hirc.py's RANSEQ_MODES"
        ));
    }
    Ok(())
}

/// A parameter block from `effects.json`'s hex, with its length checked.
fn block(id: &str, e: &Value, what: &str, len: usize) -> Result<Vec<u8>, String> {
    let at = |s: String| format!("effects.json: {what} {id}: {s}");
    let hex = e.get("params_hex").and_then(Value::as_str).ok_or_else(|| at("no parameter block".into()))?;
    let b: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)).collect::<Result<_, _>>().map_err(|_| at("its block is not hex".into()))?;
    if b.len() != len {
        return Err(at(format!("a block of {} bytes, its layout is {len}", b.len())));
    }
    Ok(b)
}

/// The rules every shipped effect keeps: the base an additive curve adds to is the property value the bank keeps for its
/// parameter, which must be the block's value (`values`, by parameter index); no state may act on its parameters.
fn check_effect_props(id: &str, e: &Value, what: &str, values: &[f64]) -> Result<(), String> {
    let at = |s: String| format!("effects.json: {what} {id}: {s}");
    for pv in e.get("prop_values").and_then(Value::as_array).into_iter().flatten() {
        let (p, v) = (pv.get("id").and_then(Value::as_u64).unwrap_or(999) as usize, pv.get("value").and_then(Value::as_f64).unwrap_or(f64::NAN));
        if values.get(p).is_none_or(|x| (x - v).abs() > 1e-6) {
            return Err(at(format!("property {p} = {v} differs from its block value")));
        }
    }
    if e.pointer("/states/groups").and_then(Value::as_array).is_some_and(|g| !g.is_empty()) {
        return Err(at("states on its parameters are not built".into()));
    }
    Ok(())
}

fn f32_at(b: &[u8], o: usize) -> f64 {
    f64::from(f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]))
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// An effect of `effects.json` the mixer runs, in `sounds.json`'s shape (`se_core::elite_sound::data::Effect`), or None for
/// a plug-in it does not run.
fn effect(id: &str, e: &Value) -> Result<Option<Value>, String> {
    let parsed = match e.get("plugin").and_then(Value::as_str) {
        Some(PARAMETRIC_EQ) => ("eq", eq_block(id, e)?),
        Some(GUITAR_DISTORTION) => ("distortion", distortion_block(id, e)?),
        Some(PEAK_LIMITER) => ("limiter", limiter_block(id, e)?),
        Some(MASTER_SUITE) => ("master", master_block(id, e)?),
        Some(COMPRESSOR) => ("compressor", compressor_block(id, e)?),
        _ => return Ok(None),
    };
    let mut v = serde_json::json!({
        "type": e.get("type"),
        "id": id,
        "name": e.get("name"),
        "plugin": e.get("plugin"),
        "params_hex": e.get("params_hex"),
        "rtpc": e.get("rtpc"),
    });
    v[parsed.0] = parsed.1;
    Ok(Some(v))
}

/// A Parametric EQ's parameter block (bank v135, wwiser's `CAkParameterEQFXParams__SetParamsBlock` for versions up to
/// 168, the layout Elite's own `SetParamsBlock` at 0x144a107a0 copies) decoded: three bands of a u32 filter type, f32 gain
/// (dB), frequency (Hz), Q and a u8 on / off, then the f32 output level (dB) and a u8 LFE flag; every value as stored.
fn eq_block(id: &str, e: &Value) -> Result<Value, String> {
    const TYPES: [&str; 7] = ["LOWPASS", "HIPASS", "BANDPASS", "NOTCH", "LOWSHELF", "HISHELF", "PEAKINGEQ"];
    let b = block(id, e, "EQ", 56)?;
    let mut values = [0.0f64; 17];
    let mut bands = Vec::new();
    for k in 0..3 {
        let o = 17 * k;
        let t = u32_at(&b, o) as usize;
        let filter = TYPES.get(t).ok_or_else(|| format!("effects.json: EQ {id}: band {k}'s filter type {t}"))?;
        let (gain, freq, q, on) = (f32_at(&b, o + 4), f32_at(&b, o + 8), f32_at(&b, o + 12), b[o + 16] != 0);
        values[5 * k..5 * k + 5].copy_from_slice(&[t as f64, gain, freq, q, if on { 1.0 } else { 0.0 }]);
        bands.push(serde_json::json!({ "filter": filter, "gain_db": gain, "frequency": freq, "q": q, "on": on }));
    }
    let (out, lfe) = (f32_at(&b, 51), b[55] != 0);
    values[15] = out;
    values[16] = if lfe { 1.0 } else { 0.0 };
    check_effect_props(id, e, "EQ", &values)?;
    Ok(serde_json::json!({ "bands": bands, "output_level_db": out, "process_lfe": lfe }))
}

/// A Guitar Distortion's parameter block, as Elite's `SetParamsBlock` (0x144a04f50) copies it: six bands (three pre-EQ,
/// three post-EQ) of a u32 type, f32 gain (dB), frequency (Hz), Q and a u8 on / off, then the u32 distortion type and the
/// f32 drive, tone, rectification, output level (dB) and wet share. A band's type is `FUN_144a05110`'s design type
/// (`FUN_144a06470` passes it straight through): its filter by `EqFilter::from_design`. Its parameter indices are its
/// `SetParam`'s (0x144a04b60): band k at (k / 3) x 30 + (k % 3) x 10, then 60-65.
fn distortion_block(id: &str, e: &Value) -> Result<Value, String> {
    const DESIGNS: [&str; 7] = ["LOWSHELF", "PEAKINGEQ", "HISHELF", "LOWPASS", "HIPASS", "BANDPASS", "NOTCH"];
    let b = block(id, e, "Guitar Distortion", 126)?;
    let mut values = [0.0f64; 66];
    let mut bands = Vec::new();
    for k in 0..6 {
        let o = 17 * k;
        let t = u32_at(&b, o) as usize;
        let filter = DESIGNS.get(t).ok_or_else(|| format!("effects.json: Guitar Distortion {id}: band {k}'s design type {t} is not built"))?;
        let (gain, freq, q, on) = (f32_at(&b, o + 4), f32_at(&b, o + 8), f32_at(&b, o + 12), b[o + 16] != 0);
        let p = (k / 3) * 30 + (k % 3) * 10;
        values[p..p + 5].copy_from_slice(&[t as f64, gain, freq, q, if on { 1.0 } else { 0.0 }]);
        bands.push(serde_json::json!({ "filter": filter, "gain_db": gain, "frequency": freq, "q": q, "on": on }));
    }
    let kind = u32_at(&b, 102);
    let [drive, tone, rect, level, wet] = [106, 110, 114, 118, 122].map(|o| f32_at(&b, o));
    values[60..66].copy_from_slice(&[f64::from(kind), drive, tone, rect, level, wet]);
    check_effect_props(id, e, "Guitar Distortion", &values)?;
    let post = bands.split_off(3);
    Ok(serde_json::json!({
        "pre_eq": bands, "post_eq": post, "distortion": kind, "drive": drive, "tone": tone, "rectification": rect,
        "output_level_db": level, "wet_dry": wet,
    }))
}

/// A Peak Limiter's parameter block (`AkPeakLimiterFXParams`, in the order of Elite's `SetParam` 0x144a12330): f32
/// threshold (dB), ratio, lookahead (s), release (s), output level (dB), then u8 LFE and channel link.
fn limiter_block(id: &str, e: &Value) -> Result<Value, String> {
    let b = block(id, e, "Peak Limiter", 22)?;
    let [threshold, ratio, lookahead, release, level] = [0, 4, 8, 12, 16].map(|o| f32_at(&b, o));
    let (lfe, link) = (b[20] != 0, b[21] != 0);
    check_effect_props(id, e, "Peak Limiter", &[threshold, ratio, lookahead, release, level, f64::from(u8::from(lfe)), f64::from(u8::from(link))])?;
    Ok(serde_json::json!({
        "threshold_db": threshold, "ratio": ratio, "lookahead_s": lookahead, "release_s": release, "output_level_db": level,
        "process_lfe": lfe, "channel_link": link,
    }))
}

/// A Wwise Compressor's parameter block (`AkCompressorFXParams`, as Elite's `SetParamsBlock` 0x1449f5300 copies it, in
/// the order of its `SetParam` 0x1449f5180): f32 threshold (dB), ratio, attack (s), release (s), output level (dB), then u8
/// LFE and channel link.
fn compressor_block(id: &str, e: &Value) -> Result<Value, String> {
    let b = block(id, e, "Compressor", 22)?;
    let [threshold, ratio, attack, release, level] = [0, 4, 8, 12, 16].map(|o| f32_at(&b, o));
    let (lfe, link) = (b[20] != 0, b[21] != 0);
    check_effect_props(id, e, "Compressor", &[threshold, ratio, attack, release, level, f64::from(u8::from(lfe)), f64::from(u8::from(link))])?;
    Ok(serde_json::json!({
        "threshold_db": threshold, "ratio": ratio, "attack_s": attack, "release_s": release, "output_level_db": level,
        "process_lfe": lfe, "channel_link": link,
    }))
}

/// The master bus's last plug-in 0x00BA0003's 304-byte block as Elite's own `SetParamsBlock` (0x144af9670) reads it: bytes
/// 0-3 the four modules on; the equaliser's band count (u32 at 4), its bands' on bytes (8..14) and bands (0x0e + 16 n: u32
/// type, f32 frequency, gain, Q); the compressor's band count (u32 0x6e), link mode (u32 0x72), link amount (f32 0x76),
/// link flag (0x7a), bands' on bytes (0x7b..0x7f), crossovers (f32 0x7f, 0x83, 0x87), bands (0x8b + 24 n: threshold, ratio,
/// attack, release, make-up, knee); the channel gains (12 f32 at 0xeb); the limiter's mode (u32 0x11b), threshold, attack,
/// release, make-up (f32 0x11f..0x12b) and link (0x12f). Every value as stored; no curve or state may drive it.
fn master_block(id: &str, e: &Value) -> Result<Value, String> {
    let b = block(id, e, "master plug-in", 0x130)?;
    check_effect_props(id, e, "master plug-in", &[])?;
    let eq: Vec<Value> = (0..6)
        .map(|n| {
            let o = 0x0e + 16 * n;
            serde_json::json!({ "kind": u32_at(&b, o), "frequency": f32_at(&b, o + 4), "gain_db": f32_at(&b, o + 8), "q": f32_at(&b, o + 12), "on": b[8 + n] != 0 })
        })
        .collect();
    let comp: Vec<Value> = (0..4)
        .map(|n| {
            let [t, r, a, rl, m, k] = [0, 4, 8, 12, 16, 20].map(|i| f32_at(&b, 0x8b + 24 * n + i));
            serde_json::json!({ "threshold_db": t, "ratio": r, "attack_s": a, "release_s": rl, "makeup_db": m, "knee_db": k })
        })
        .collect();
    Ok(serde_json::json!({
        "modules_on": [b[0] != 0, b[1] != 0, b[2] != 0, b[3] != 0],
        "eq_bands_n": u32_at(&b, 4),
        "eq": eq,
        "comp_bands_n": u32_at(&b, 0x6e),
        "comp_link": u32_at(&b, 0x72),
        "comp_link_amount": f32_at(&b, 0x76),
        "comp_link_flag": b[0x7a] != 0,
        "comp_bands_on": [b[0x7b] != 0, b[0x7c] != 0, b[0x7d] != 0, b[0x7e] != 0],
        "crossovers": [f32_at(&b, 0x7f), f32_at(&b, 0x83), f32_at(&b, 0x87)],
        "comp": comp,
        "channel_gains_db": (0..12).map(|n| f32_at(&b, 0xeb + 4 * n)).collect::<Vec<f64>>(),
        "limiter_mode": u32_at(&b, 0x11b),
        "limiter_threshold_db": f32_at(&b, 0x11f),
        "limiter_attack_s": f32_at(&b, 0x123),
        "limiter_release_s": f32_at(&b, 0x127),
        "limiter_makeup_db": f32_at(&b, 0x12b),
        "limiter_link": b[0x12f] != 0,
    }))
}

/// `sounds.json` built from the exports: the file, the media files to copy (source id, the file in its export's `media/`)
/// and the UI's events.
struct Built {
    file: SoundFile,
    media: Vec<(String, std::path::PathBuf)>,
    left_out: Vec<String>,
    ui: UiPick,
}

fn build(ctx: &Ctx) -> Result<Built, String> {
    let ship = Export::read(&ctx.audio)?;
    let ui_export = Export::read(&ui_sound::audio(ctx))?;
    let ui = ui_sound::pick(&ui_sound::read_tables(ctx)?, &ui_export)?;
    let x = ship.merge(ui_export)?;
    let proven = |s: &String| x.media.get(s).is_some_and(|(m, _)| m.get("decode_identical") == Some(&Value::Bool(true)));
    let media_ok = |r: &Reach| r.sources.iter().all(proven) && r.plugins.iter().all(|p| x.plugins.contains_key(p));

    let mut all = Reach::default();
    let mut ship_names = Vec::new();
    for (name, _) in SHIP_EVENTS {
        let id = x.event(name)?;
        let mut r = Reach::default();
        r.event(&x, &id)?;
        if !media_ok(&r) {
            return Err(format!("{name}: some of its media is not in media/ or did not pass sidewinder_media.py's decode proof"));
        }
        all.event(&x, &id)?;
        ship_names.push(name.to_string());
    }
    let mut open_names = Vec::new();
    let mut left_out = Vec::new();
    for (entry, why) in OPEN_EVENTS {
        let names: Vec<String> = match entry.strip_prefix("bank:") {
            Some(bank) => x.by_bank.get(bank).cloned().ok_or_else(|| format!("events.json has no unnamed events of bank {bank}"))?,
            None => vec![entry.to_string()],
        };
        for name in names {
            let id = x.event(&name)?;
            let mut r = Reach::default();
            r.event(&x, &id)?;
            if media_ok(&r) {
                all.event(&x, &id)?;
                open_names.push(name);
            } else {
                left_out.push(format!("{name} ({why}): its media is not at rest"));
            }
        }
    }
    // The UI's events (`ui_sound.rs`): every one ships, its media proven as the ship's.
    for name in &ui.events {
        let id = x.event(name)?;
        let mut r = Reach::default();
        r.event(&x, &id)?;
        if !media_ok(&r) {
            return Err(format!("UI event {name}: some of its media is not in the UI export's media/ or did not pass sidewinder_media.py's decode proof"));
        }
        all.event(&x, &id)?;
    }
    all.finish(&x);

    check_container_modes(&x)?;
    // The objects, trimmed.
    let mut objects = Map::new();
    for id in &all.objects {
        let o = &x.objects[id];
        let t = o.get("type").and_then(Value::as_str).unwrap_or("");
        if !KEPT_TYPES.contains(&t) {
            continue;
        }
        let mut o = o.as_object().cloned().unwrap_or_default();
        for k in ["resolved", "bank", "banks"] {
            o.remove(k);
        }
        if t == "Sound" {
            o.remove("media");
        }

        objects.insert(id.clone(), Value::Object(o));
    }
    // The effects the mixer runs: the Parametric EQs, Guitar Distortions, Peak Limiters and Compressors in the slots of the
    // nodes and buses kept (a bus's compressor is run only on the sub-mix bus), and the master's plug-in 0x00BA0003 (run on
    // the output's sum).
    let mut effects = Map::new();
    for o in objects.values() {
        let chain = o.get("node").and_then(|n| n.get("fx")).or_else(|| o.get("fx"));
        for slot in chain.and_then(|c| c.get("fx")).and_then(Value::as_array).into_iter().flatten() {
            let Some(fid) = slot.pointer("/fx_id/id").and_then(Value::as_str) else { continue };
            let Some(e) = x.effects.get(fid) else { continue };
            if !effects.contains_key(fid) {
                if let Some(v) = effect(fid, e)? {
                    effects.insert(fid.to_string(), v);
                }
            }
        }
    }
    // (The game parameters their curves read need the Init bank's definitions: their defaults set the effects' values
    // where nothing sets them.)
    for e in effects.values() {
        all.rtpcs(e.get("rtpc"));
    }
    let mut events = Map::new();
    for id in &all.events {
        let (name, actions) = &x.events[id];
        events.insert(id.clone(), serde_json::json!({ "name": name, "actions": actions }));
    }
    let pick = |table: &str, ids: &BTreeSet<String>| -> Map<String, Value> {
        let t = x.syncs.get(table).and_then(Value::as_object);
        ids.iter().filter_map(|i| t.and_then(|t| t.get(i)).map(|v| (i.clone(), v.clone()))).collect()
    };
    let syncs = serde_json::json!({
        "source": x.syncs.get("source"),
        "volume_threshold_db": x.syncs.get("volume_threshold_db"),
        "max_voices": x.syncs.get("max_voices"),
        "game_parameters": pick("game_parameters", &all.gps),
        "state_groups": pick("state_groups", &all.state_groups),
        "switch_groups": pick("switch_groups", &all.switch_groups),
    });
    let mut media = Map::new();
    let mut files = Vec::new();
    for s in &all.sources {
        let (m, dir) = &x.media[s];
        let file = m.get("file").and_then(Value::as_str).ok_or_else(|| format!("media/index.json: {s} has no file"))?;
        let ext = file.rsplit('.').next().unwrap_or("ogg");
        media.insert(
            s.clone(),
            serde_json::json!({
                "file": format!("sfx/{s}.{ext}"),
                "codec": m.get("codec"),
                "sample_rate": m.get("sample_rate"),
                "channels": m.get("channels"),
                "samples": m.get("samples"),
                "loop_region": m.get("loop_region"),
                "loudness_gain": m.pointer("/akd/loudness_gain"),
                "akd_gain_2": m.pointer("/akd/gain_2"),
                "rms": m.get("rms"),
                "loop_rms": m.get("loop_rms"),
            }),
        );
        files.push((s.clone(), dir.join(file)));
    }
    let mut source_plugins = Map::new();
    for p in &all.plugins {
        let mut rec = x.plugins[p].as_object().cloned().unwrap_or_default();
        rec.remove("from_");
        source_plugins.insert(p.clone(), Value::Object(rec));
    }
    // Names: every reference with a known name in what is kept, and the events'.
    let mut names = BTreeMap::new();
    fn walk(v: &Value, names: &mut BTreeMap<String, Value>) {
        match v {
            Value::Object(m) => {
                if let (Some(Value::String(i)), Some(Value::String(n))) = (m.get("id"), m.get("name")) {
                    if i.len() == 8 {
                        names.insert(i.clone(), Value::String(n.clone()));
                    }
                }
                m.values().for_each(|x| walk(x, names));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, names)),
            _ => {}
        }
    }
    let objects_v = Value::Object(objects);
    let syncs_v = syncs;
    walk(&objects_v, &mut names);
    walk(&syncs_v, &mut names);
    for table in ["game_parameters", "state_groups", "switch_groups"] {
        for (id, d) in syncs_v.get(table).and_then(Value::as_object).into_iter().flatten() {
            if let Some(n) = d.get("name").and_then(Value::as_str) {
                names.insert(id.clone(), Value::String(n.into()));
            }
        }
    }
    for (id, e) in &events {
        if let Some(n) = e.get("name").and_then(Value::as_str) {
            names.insert(id.clone(), Value::String(n.into()));
        }
    }
    // The meters on the buses kept (what goes through them is shipped); their game parameters are in `all.gps` (`finish`).
    let meters: Vec<Value> = x.meters.iter().filter(|m| m.get("bus").and_then(Value::as_str).is_some_and(|b| objects_v.get(b).is_some())).cloned().collect();
    let file = serde_json::json!({
        "format": FORMAT,
        "about": "The Faulcon DeLacy Sidewinder's sound events and its ship UI's (the cockpit interfaces' PlaySound tables and the \
                  HUD events the game posts: ui/sound_tables.json) as Elite Dangerous's Wwise banks define them, and everything \
                  they reach, trimmed from the audio exports' events.json without changing a value (written by copy_sidewinder, \
                  sidewinder/convert/sound.rs and ui_sound.rs; played by se_core::elite_sound::Mixer). Ids are Wwise ids as 8 \
                  hex digits.",
        "units": x.units,
        "ship_events": ship_names,
        "open_events": open_names,
        "events": events,
        "objects": objects_v,
        "game_syncs": syncs_v,
        "media": media,
        "source_plugins": source_plugins,
        "names": names,
        "meters": meters,
        "effects": effects,
    });
    let file: SoundFile = serde_json::from_value(file).map_err(|e| format!("sounds.json's shape: {e}"))?;
    SoundBank::new(file.clone()).map_err(|e| format!("sounds.json: {e}"))?;
    Ok(Built { file, media: files, left_out, ui })
}

pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    let built = build(ctx)?;
    let bytes = write_sounds(&built.file).map_err(|e| e.to_string())?;
    ctx.write("sounds.json", &bytes)?;
    shipped.add(
        "sounds.json",
        &[
            "re/elite/frea_export/sidewinder_audio/events.json (the 62 Wwise bank payloads at rest, read by sidewinder_events.py)".to_string(),
            "re/elite/frea_export/sidewinder_audio/effects.json (the effects in the nodes' and buses' slots, from the same payloads, by sidewinder_effects.py)".to_string(),
            "re/elite/frea_export/sidewinder_ui/audio/events.json and effects.json (the ship UI's events, by the same tools from the 62 payloads and 9 more GUI banks: re/elite/work/ui/sounds/ui_export.py)".to_string(),
        ],
        "A",
        &format!(
            "The ship's Wwise events ({} the game posts, {} whose sender is not found), the ship UI's ({}: ui_sound.rs) and all they \
             reach: {} events with the ones they post, {} objects, {} media, {} source plugins, {} Parametric EQs, {} Guitar \
             Distortions, {} Peak Limiter(s), {} master plug-in(s) 0x00BA0003, {} Compressors, the Init bank's game syncs. Values \
             are the banks'.",
            built.file.ship_events.len(),
            built.file.open_events.len(),
            built.ui.events.len(),
            built.file.events.len(),
            built.file.objects.len(),
            built.file.media.len(),
            built.file.source_plugins.len(),
            built.file.effects.values().filter(|e| e.eq.is_some()).count(),
            built.file.effects.values().filter(|e| e.distortion.is_some()).count(),
            built.file.effects.values().filter(|e| e.limiter.is_some()).count(),
            built.file.effects.values().filter(|e| e.master.is_some()).count(),
            built.file.effects.values().filter(|e| e.compressor.is_some()).count()
        ),
    );
    // sfx/ is this kind's alone: emptied first, so a recording no longer shipped (or now shipped as another file) goes.
    let sfx = ctx.out.join("sfx");
    if sfx.is_dir() {
        std::fs::remove_dir_all(&sfx).map_err(|e| format!("{}: {e}", sfx.display()))?;
    }
    for (src, path) in &built.media {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("ogg");
        let rel = format!("sfx/{src}.{ext}");
        ctx.write(&rel, &bytes)?;
        let (grade, note) = if ext == "ogg" {
            ("A", "The game's own Wwise Vorbis packets, rewrapped into Ogg without re-encoding (ww2ogg, aoTuV 6.03 codebooks; granule positions worked out from the block sizes); decodes to the .wem's audio sample for sample.")
        } else {
            ("A", "Elite's exact decoded samples, not its packets: vgmstream's 16-bit PCM decode of the game's stream (Platinum ADPCM, or a Wwise Vorbis stream ww2ogg cannot rebuild), sample for sample what the .wem decodes to; such a stream cannot be rewrapped in Ogg without re-encoding.")
        };
        // (The export's path from `re/` on, as the other sources are named.)
        let from = path.to_string_lossy().replace('\\', "/");
        let from = from.find("re/elite/").map_or(from.as_str(), |i| &from[i..]);
        shipped.add(&rel, &[format!("Wwise source {src} ({from})")], grade, note);
    }
    shipped.note(&format!(
        "Sounds: the open events left out because the banks at rest lack their media: {}.",
        if built.left_out.is_empty() { "none".to_string() } else { built.left_out.join("; ") }
    ));
    shipped.note(&format!(
        "Ship UI sounds: values of the cockpit interfaces' tables and HUD events of the game with no event in any bank (Elite posts \
         their ids and nothing answers: silence), not shipped: {}.",
        built.ui.silent.join(", ")
    ));
    read_back(ctx, &built)
}

/// Every media `sounds.json` names passed the decode proof (`media/index.json`), is in `sfx/` and is what it says (Ogg
/// Vorbis, or RIFF WAVE for the PCM ones), and the file reads back as the game reads it, with every UI event picked in it.
fn read_back(ctx: &Ctx, built: &Built) -> Result<(), String> {
    let bytes = ctx.read("sounds.json")?;
    check(&bytes)?;
    let bank = parse_sounds(&bytes).map_err(|e| e.to_string())?;
    if let Some(e) = built.ui.events.iter().find(|e| bank.event(e).is_none()) {
        return Err(format!("UI event {e} is missing"));
    }
    let index: BTreeMap<String, Value> = [&ctx.audio, &ui_sound::audio(ctx)]
        .iter()
        .map(|a| read_json(&a.join("media").join("index.json")))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .rev()
        .flat_map(|v| v.get("media").and_then(Value::as_object).cloned().unwrap_or_default())
        .collect();
    for (src, m) in &built.file.media {
        let entry = index.get(&src.to_string()).ok_or_else(|| format!("{}: in neither export's media/index.json", m.file))?;
        if entry.get("decode_identical") != Some(&Value::Bool(true)) {
            return Err(format!("{}: media/index.json does not say it decodes to the .wem's audio", m.file));
        }
        let b = ctx.read(&m.file)?;
        let ok = if m.file.ends_with(".ogg") {
            b.starts_with(b"OggS") && b.get(28..35) == Some(b"\x01vorbis".as_slice())
        } else {
            b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WAVE".as_slice())
        };
        if !ok {
            return Err(format!("{}: not the {} it should be (source {src})", m.file, m.codec));
        }
    }
    Ok(())
}

/// `sounds.json` as the game reads it: parsed, every `SHIP_EVENTS` event in it, the UI's exe events (`ui_sound::check_bank`),
/// and a mixer made from it that hands out the master's plug-in 0x00BA0003 (without it the ship would sound 14 dB too quiet).
pub(super) fn check(bytes: &[u8]) -> Result<(), String> {
    let bank = parse_sounds(bytes).map_err(|e| e.to_string())?;
    for (name, _) in SHIP_EVENTS {
        if bank.event(name).is_none() || !bank.file.ship_events.iter().any(|n| event_id(n) == event_id(name)) {
            return Err(format!("event {name} is missing"));
        }
    }
    ui_sound::check_bank(&bank)?;
    if Mixer::new(std::sync::Arc::new(bank), 0).master_suite().is_none() {
        return Err("the master bus's plug-in 0x00BA0003 is missing".into());
    }
    Ok(())
}

/// Builds `sounds.json` from the audio export into `re/elite/work/sound/t5b/sounds.json` (scratch, out of git) and prints
/// its counts, writing nothing in the mod's folder: `cargo test -p se_engine --profile agent sounds_from_the_export --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn sounds_from_the_export() {
    let ctx = Ctx::from_env().unwrap();
    let built = build(&ctx).unwrap_or_else(|e| panic!("{e}"));
    let bytes = write_sounds(&built.file).unwrap();
    let out = ctx.audio.join("..").join("..").join("work").join("sound").join("t5b").join("sounds.json");
    std::fs::write(&out, &bytes).unwrap();
    check(&bytes).unwrap_or_else(|e| panic!("{e}"));
    // Every shipped event played alone for three seconds: what it starts.
    let bank = std::sync::Arc::new(parse_sounds(&bytes).unwrap());
    let t0 = std::time::Instant::now();
    let (mut silent, mut voices) = (Vec::new(), 0usize);
    for name in built.file.ship_events.iter().chain(&built.file.open_events) {
        let mut m = Mixer::new(bank.clone(), 1);
        m.post(name, 1).unwrap();
        let mut started = 0;
        for k in 0..180 {
            started += m.step(if k == 0 { 0.0 } else { 1.0 / 60.0 }).iter().filter(|u| u.start.is_some()).count();
        }
        voices += started;
        if started == 0 {
            silent.push(name.clone());
        }
    }
    let mut m = Mixer::new(bank.clone(), 2);
    for name in built.file.ship_events.iter().chain(&built.file.open_events) {
        m.post(name, 1).unwrap();
    }
    m.step(0.0);
    let t1 = std::time::Instant::now();
    m.step(1e30);
    println!("every event posted at once, then step(1e30): {:.3} s", t1.elapsed().as_secs_f64());
    println!("every event 3 s alone: {voices} voices started in {:.1} s; {} start nothing (stops, states, sets): {silent:?}", t0.elapsed().as_secs_f64(), silent.len());
    let f = &built.file;
    println!(
        "sounds.json: {} bytes; {} ship + {} open events, {} events in all, {} objects, {} media, {} plugins; left out: {:?}",
        bytes.len(),
        f.ship_events.len(),
        f.open_events.len(),
        f.events.len(),
        f.objects.len(),
        f.media.len(),
        f.source_plugins.len(),
        built.left_out
    );
}
