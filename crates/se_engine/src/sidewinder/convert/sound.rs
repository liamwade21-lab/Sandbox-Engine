//! The ship's sounds (T5b-1): `sounds.json` (`se_core::elite_sound`) and `sfx/<source id>.ogg`, from the audio export
//! (`sidewinder_audio`): `events.json` (every Wwise event the Sidewinder uses with everything the banks say of it, made by
//! `re/elite/tools/sidewinder_events.py`), `media/` (each recording the events reach, the game's own packets rewrapped,
//! made by `re/elite/tools/sidewinder_media.py`, with `index.json` and the source plugins' `plugins.json`).
//!
//! Which events ship: `SHIP_EVENTS` (each with the `triggers.json` entry or state machine cue that proves the game posts
//! it) and `OPEN_EVENTS` (the sender is not found, but the event is for something the mod has: shipped when the banks at
//! rest hold all its media, so the trigger is all a later task adds). Then everything they reach, trimmed from
//! `events.json` without changing a value: the events they post, the actions, the object trees with their actor-mixer
//! parents, the buses up to the master, the attenuations, the game syncs they name (from the Init bank), the media, the
//! source plugins and the names. Left out as derived or not used by the mixer: each object's `resolved` summary, `bank`,
//! `banks`, a Sound's `media` summary (the `media` table replaces it), the State objects (their deltas are inline in each
//! node's state block), the modulators and effects (not built: `se_core::elite_sound`'s docs). Every recording shipped
//! passed `sidewinder_media.py`'s proof (`decode_identical` in `media/index.json`: the file decodes to the `.wem`'s audio,
//! sample for sample); `build` refuses any other.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use se_core::elite_sound::{data::FORMAT, event_id, parse_sounds, write_sounds, Mixer, SoundBank, SoundFile};

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
    ("normalflight_supercruise_transition_e", "audio_mode_transitions: when the _e is posted is not traced"),
    ("normalflight_hyperspace_transition_e", "audio_mode_transitions: when the _e is posted is not traced"),
    ("supercruise_normalflight_transition_e", "audio_mode_transitions: when the _e is posted is not traced"),
    ("supercruise_hyperspace_transition_e", "audio_mode_transitions: when the _e is posted is not traced"),
    ("hyperspace_supercruise_transition_e", "audio_mode_transitions: when the _e is posted is not traced"),
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

/// The audio export read once: events by id (with those only met nested as posted events), objects, game syncs.
struct Export {
    events: BTreeMap<String, (Option<String>, Vec<String>)>,
    by_name: BTreeMap<String, String>,
    by_bank: BTreeMap<String, Vec<String>>,
    objects: Map<String, Value>,
    syncs: Value,
    units: Value,
    /// The Wwise Meter effects in the buses' slots (`events.json`'s `meters`, decoded by `sidewinder_events.py`).
    meters: Vec<Value>,
}

impl Export {
    fn read(audio: &std::path::Path) -> Result<Export, String> {
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
        Ok(Export { events, by_name, by_bank, objects, syncs, units, meters })
    }

    /// The event id of a name of the tables (events.json's own key, case aside).
    fn event(&self, name: &str) -> Result<String, String> {
        self.by_name.get(&name.to_lowercase()).cloned().ok_or_else(|| format!("events.json has no event {name}"))
    }
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

/// `sounds.json` built from the export: the file and the media files to copy (source id, file in `media/`).
struct Built {
    file: SoundFile,
    media: Vec<(String, String)>,
    left_out: Vec<String>,
}

fn build(audio: &std::path::Path) -> Result<Built, String> {
    let x = Export::read(audio)?;
    let index = read_json(&audio.join("media").join("index.json"))?;
    let index = index.get("media").and_then(Value::as_object).cloned().ok_or("media/index.json: no media")?;
    let plugins = read_json(&audio.join("media").join("plugins.json"))?;
    let plugins = plugins.get("plugins").and_then(Value::as_object).cloned().ok_or("media/plugins.json: no plugins")?;
    let proven = |s: &String| index.get(s).is_some_and(|m| m.get("decode_identical") == Some(&Value::Bool(true)));
    let media_ok = |r: &Reach| r.sources.iter().all(proven) && r.plugins.iter().all(|p| plugins.contains_key(p));

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
        let m = &index[s];
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
        files.push((s.clone(), file.to_string()));
    }
    let mut source_plugins = Map::new();
    for p in &all.plugins {
        let mut rec = plugins[p].as_object().cloned().unwrap_or_default();
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
        "about": "The Faulcon DeLacy Sidewinder's sound events as Elite Dangerous's Wwise banks define them, and everything they reach, \
                  trimmed from the audio export's events.json without changing a value (written by copy_sidewinder, \
                  sidewinder/convert/sound.rs; played by se_core::elite_sound::Mixer). Ids are Wwise ids as 8 hex digits.",
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
    });
    let file: SoundFile = serde_json::from_value(file).map_err(|e| format!("sounds.json's shape: {e}"))?;
    SoundBank::new(file.clone()).map_err(|e| format!("sounds.json: {e}"))?;
    Ok(Built { file, media: files, left_out })
}

pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    let built = build(&ctx.audio)?;
    let bytes = write_sounds(&built.file).map_err(|e| e.to_string())?;
    ctx.write("sounds.json", &bytes)?;
    shipped.add(
        "sounds.json",
        &["re/elite/frea_export/sidewinder_audio/events.json (the 62 Wwise bank payloads at rest, read by sidewinder_events.py)".to_string()],
        "A",
        &format!(
            "The ship's Wwise events ({} the game posts, {} whose sender is not found) and all they reach: {} events with the ones \
             they post, {} objects, {} media, {} source plugins, the Init bank's game syncs. Values are the banks'.",
            built.file.ship_events.len(),
            built.file.open_events.len(),
            built.file.events.len(),
            built.file.objects.len(),
            built.file.media.len(),
            built.file.source_plugins.len()
        ),
    );
    // sfx/ is this kind's alone: emptied first, so a recording no longer shipped (or now shipped as another file) goes.
    let sfx = ctx.out.join("sfx");
    if sfx.is_dir() {
        std::fs::remove_dir_all(&sfx).map_err(|e| format!("{}: {e}", sfx.display()))?;
    }
    let media_dir = ctx.audio.join("media");
    for (src, file) in &built.media {
        let bytes = std::fs::read(media_dir.join(file)).map_err(|e| format!("media/{file}: {e}"))?;
        let ext = file.rsplit('.').next().unwrap_or("ogg");
        let rel = format!("sfx/{src}.{ext}");
        ctx.write(&rel, &bytes)?;
        let (grade, note) = if ext == "ogg" {
            ("A", "The game's own Wwise Vorbis packets, rewrapped into Ogg without re-encoding (ww2ogg, aoTuV 6.03 codebooks; granule positions worked out from the block sizes); decodes to the .wem's audio sample for sample.")
        } else {
            ("A", "The game's audio decoded to 16-bit PCM by vgmstream (ADPCM, or a Vorbis stream ww2ogg cannot rebuild): an Ogg would mean re-encoding.")
        };
        shipped.add(&rel, &[format!("Wwise source {src} (re/elite/frea_export/sidewinder_audio/media/{file})")], grade, note);
    }
    shipped.note(&format!(
        "Sounds: the open events left out because the banks at rest lack their media: {}.",
        if built.left_out.is_empty() { "none".to_string() } else { built.left_out.join("; ") }
    ));
    read_back(ctx, &built)
}

/// Every media `sounds.json` names passed the decode proof (`media/index.json`), is in `sfx/` and is what it says (Ogg
/// Vorbis, or RIFF WAVE for the PCM ones), and the file reads back as the game reads it.
fn read_back(ctx: &Ctx, built: &Built) -> Result<(), String> {
    let bytes = ctx.read("sounds.json")?;
    check(&bytes)?;
    let index = read_json(&ctx.audio.join("media").join("index.json"))?;
    for (src, m) in &built.file.media {
        let entry = index.pointer(&format!("/media/{src}")).ok_or_else(|| format!("{}: not in media/index.json", m.file))?;
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

/// `sounds.json` as the game reads it: parsed, every `SHIP_EVENTS` event in it, and a mixer made from it.
pub(super) fn check(bytes: &[u8]) -> Result<(), String> {
    let bank = parse_sounds(bytes).map_err(|e| e.to_string())?;
    for (name, _) in SHIP_EVENTS {
        if bank.event(name).is_none() || !bank.file.ship_events.iter().any(|n| event_id(n) == event_id(name)) {
            return Err(format!("event {name} is missing"));
        }
    }
    let _ = Mixer::new(std::sync::Arc::new(bank), 0);
    Ok(())
}

/// Builds `sounds.json` from the audio export into `re/elite/work/sound/t5b/sounds.json` (scratch, out of git) and prints
/// its counts, writing nothing in the mod's folder: `cargo test -p se_engine --profile agent sounds_from_the_export --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn sounds_from_the_export() {
    let ctx = Ctx::from_env().unwrap();
    let built = build(&ctx.audio).unwrap_or_else(|e| panic!("{e}"));
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
