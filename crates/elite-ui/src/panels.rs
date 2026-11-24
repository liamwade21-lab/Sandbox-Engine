//! What the four cockpit panels send back (`SendEventToGame`, ARCHITECTURE.md A10, task UI5a): `on_out` turns one event
//! into what it asks for (`PanelAction`), by the events' meaning in `re/elite/notes/ui_messages_panels.md` (each part's
//! events table, grades there). Some ask the ship to do something (the game does it with the same rules as the keys);
//! the others are the panels' own state, which the panels' feeder takes (`feed::Panels::hear`), resolving the pop-ups'
//! buttons to ship actions as Elite's controllers do (the location pop-up's buttons act on the location it was opened for,
//! the module pop-up's switch on its module). An event the mod has nothing to do with gives nothing; `IGNORED` lists them
//! with why.

use crate::feed::Movie;
use crate::Value;

/// A ship function the right panel's Ship tab switches (`ComboOptionSelected`'s option ids, notes part 2 4.2: 1 flight
/// assist, 5 the ship's lights, 7 landing gear, 12 cargo scoop, 15 silent running).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    FlightAssist,
    Lights,
    LandingGear,
    CargoScoop,
    SilentRunning,
}

impl Function {
    /// The option's id in Elite's table.
    pub fn option(self) -> u32 {
        match self {
            Function::FlightAssist => 1,
            Function::Lights => 5,
            Function::LandingGear => 7,
            Function::CargoScoop => 12,
            Function::SilentRunning => 15,
        }
    }

    fn of_option(option: u32) -> Option<Function> {
        [Function::FlightAssist, Function::Lights, Function::LandingGear, Function::CargoScoop, Function::SilentRunning]
            .into_iter()
            .find(|f| f.option() == option)
    }
}

/// What an event asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum PanelAction {
    // The ship's actions: the game does these.
    /// Lock the destination: a `Location::id` (the location pop-up's Lock destination, id 0).
    LockDestination(u32),
    /// Unlock the destination (the pop-up's Unlock destination, id 1).
    Unlock,
    /// Engage supercruise (the pop-up's id 5): with `lock`, lock that place first ("Lock and supercruise").
    EngageSupercruise { lock: Option<u32> },
    /// Engage the hyperdrive (the pop-up's id 6), likewise.
    EngageHyperdrive { lock: Option<u32> },
    /// Switch a module (`Module::id`) on or off (the module pop-up's power switch).
    SetModule { id: u32, on: bool },
    /// Flip a ship function (`ComboOptionSelected`).
    Toggle(Function),
    /// Leave the seat (`OnDisembarkPressed`: the game checks its own rule again, as Elite's handler does). The role movie
    /// sends it TWICE for one press (A, seen in the proof 0.15 s apart: the button's pre-select fires the select, and its
    /// select animation's end selects it again, `_shared/core/skins/AbstractButtonSkin.as` `buttonPreSelectHandler`,
    /// `onSelectAnimationComplete`), so the game must treat it as idempotent: out of the seat, a Disembark does nothing.
    /// Nothing here drops the second one.
    Disembark,
    /// Cancel the drive's charge (`CancelJump`, the Navigation page's Cancel).
    CancelJump,
    /// Show or hide a cockpit backing plate (`SetBackgroundVisible`, the comms panel's: `Gui_Comms_Extended_Panel_Background`,
    /// `Gui_Comms_Full_Panel_Background`).
    Background { name: String, visible: bool },

    // The panels' own state: the feeder takes these.
    /// A movie finished showing or hiding (`ShowComplete`, `HideComplete`, the role panel's `ShowingPanel`, `HidingPanel`).
    ShowComplete(Movie),
    HideComplete(Movie),
    /// The page now shown (`ExternalOnPanel`, `InternalOnPanel`, `OnPanel`: 0 = none).
    Page { movie: Movie, page: i32 },
    /// A pop-up shown or hidden (`ExternalOnPopup`, `InternalOnPopup`, `InternalOnModulePopup`: 0 = none).
    Popup { movie: Movie, state: i32 },
    /// A Navigation row selected (`LocationSelected`): the feeder answers with the location pop-up.
    LocationSelected(u32),
    /// A location pop-up button (`OnLocationPopupButtonSelect`, Elite's button ids) and its Back.
    LocationButton(u32),
    LocationBack,
    /// A Modules row selected (`ToggleModuleState`: Elite does not switch here, it opens the module pop-up).
    ModuleSelected(u32),
    /// The module pop-up's radio (`InternalModuleOptionPopupRadioSelected`: 0 the power switch, 1 repair) and a row button
    /// (`InternalPopupRowSelected`: 2 its Back).
    ModuleOption(u32),
    ModuleRow(u32),
    /// The comms panel's tabs exist (`ChatPanelTabsReady`, each `Show`).
    TabsReady,
    /// The role panel opened a sub-screen or came back to its tabs (`OnRolePanelStateEnter`, `OnRolePanelStateBack`).
    RoleScreen { open: bool },
}

/// The events of the four movies the mod has nothing to do with, by name, and why (each gives no action). With the events
/// `on_out` maps, this is every event the movies' `api.json` lists, and the ones they build by name (the home grid, the
/// music player, the codex buttons, the text input's): `every_event_is_mapped_or_ignored` checks it.
pub const IGNORED: &[(&[&str], &str)] = &[
    (&["SelectFilters", "ResetFilters", "ExternalPopupRadioSelected", "ExternalPopupRowSelected"], "the Navigation filters: the mod lists every place, it has no filters"),
    (&["ShowGalaxyMap", "ShowSystemMap", "ShowPlanetaryMap"], "the maps are cut (the user's scope)"),
    (
        &["TargetSelected", "RequestFullTargetDetails", "TargetInteracted", "OnFocusOnContactsList", "OnInteractionButtonSelect", "OnInteractionButtonCountdownStart"],
        "contacts: the mod has no other ships",
    ),
    (&["TargetModuleHighlighted", "TargetModuleSelected", "TargetCargoSelected"], "sub-targets and cargo of a target: no targets"),
    (
        &[
            "OnFocusOnCategoryList", "OnFocusOnMissionList", "MissionSelected", "MissionCategorySelected", "OnMissionSelected",
            "OnAssignmentSelected", "OnAssignmentDetailsBackSelected", "OnAssignmentDetailsGalaxyMapSelected", "OnConflictAbandonSelected",
            "OnConflictInfoSelected", "OnMissionAbandonSelected", "OnMissionAcceptSelected", "OnMissionBackSelected",
            "OnMissionDeclineSelected", "OnMissionInfoPopupActionSelected", "OnMissionInfoSelected", "OnMissionMapSelected",
            "OnMissionShareSelected", "OnMissionStepperChange", "OnMissionStepperConfirm", "OnPassengerSelected",
        ],
        "transactions, missions, passengers: none in the mod",
    ),
    (&["OnApexShortRangeSelected", "OnApexLongRangeSelected"], "Apex shuttles: none"),
    (&["OnHandbookSelected"], "the pilot's handbook: the codex is not in scope"),
    (&["ModulePriority"], "power priorities: the mod has none"),
    (&["ModulePowerUpComplete"], "module boot-up: the mod switches modules at once"),
    (&["PerksModuleSelected"], "engineering perks: none"),
    (&["ModuleGroupSelected"], "editing fire groups: the mod's fire groups are fixed (the stock assignment)"),
    (&["SliderOptionSelected", "NullOptionSelected"], "the options the mod lists are all combos"),
    (
        &["InternalPopupRangeSelected", "InternalPopupRadioSelected", "ExternalPopupRangeSelected"],
        "the generic pop-ups (beacon, reboot, self destruct): not listed",
    ),
    (
        &[
            "CargoSelected", "OnEjectPopupAction", "OnEjectPopupBack", "OnOpenGalaxyMap", "OnOpenCodexTradingPage", "MaterialSelected",
            "PerkSelected", "PerkApplySelected", "CancelSythesisSelected", "BinItemSelected", "HopperItemSelected", "SetFilterButton",
        ],
        "cargo items, materials, synthesis, the refinery: the hold is always empty",
    ),
    (
        &[
            "OnTransferItemSelect", "OnTransferAllSelect", "OnTransferSwitchSelected", "OnTransferConfirmSelected",
            "OnTransferCancelSelected", "OnTransferTabToggled", "OnTransferFilterButtonSelected", "OnTransferItemDiscard",
            "OnTransferSwitchConfirmSelect", "OnTransferSwitchCancelSelect", "ShowTransferPopup",
        ],
        "cargo transfers: none",
    ),
    (&["SystemFactionSelected", "SystemFactionDeselected"], "factions: none"),
    (&["OnSubpanelFocused"], "a sub-panel's focus: every Elite controller leaves it a stub"),
    (
        &[
            "ShowSquadrons", "ShowGalacticPowers", "ShowOperations", "ShowSquadronCarrier", "ShowCarrierManagement",
            "ShowCommanderCreator", "ShowNewsFeed", "ShowCommanderLog", "ShowCodex", "ShowSuitCosmetics", "ShowTrainingMissionSelector",
        ],
        "the home grid: stubs in every Elite controller (notes part 2 13.6)",
    ),
    (
        &[
            "AudioPlayerOnPlayPauseToggle", "AudioPlayerOnPrevious", "AudioPlayerOnNext", "AudioPlayerOnRemovePlaylistItem",
            "AudioPlayerOnSkipToPlaylistItem", "AudioPlayerOnClearPlaylist", "AudioPlayerOnToggleAutoRemoveOption",
        ],
        "the music player: stubs in every Elite controller",
    ),
    (
        &["TextInputBeginWithFlags", "SpacePressed", "TextInput", "TextInputComplete", "TextInputEndWithCurrent", "OnTextInputEnd"],
        "chat input: the mod has no chat",
    ),
    (
        &["OnInboxMessageDisplayed", "OnCommsChannelSelect", "OnMessageSelect", "CommsPopupButtonSelected", "TransitionToFullComms"],
        "the comms log's channels and players: the mod has one local log and no players",
    ),
    (
        &["InboxEntrySelected", "DeleteInboxEntrySelected", "DeleteInboxThreadSelected", "InboxShowFilterSelected", "OnInboxDialogPopup", "InboxDialogPanelButtonSelected"],
        "the inbox: no mail",
    ),
    (
        &["OptionsEntry", "onPopupButtonSelected", "onPopupButtonToggled", "PopupRowSelected", "SecondaryPopupRowSelected", "PopupRangeSelected"],
        "the comms options and its pop-ups: none",
    ),
    (
        &["GroupEntrySelected", "GroupOptionsSelected", "InviteSelected", "OnMatchMakingContactSelected", "OnGroupPanelBackSelect", "OnNavItemSelected", "OnTreeMenuItemSelected", "HistoryEntrySelected"],
        "contacts, friends, groups, multicrew, history: none",
    ),
    (
        &[
            "BulletinTextInputBegin", "BulletinTextInputEnd", "NewPostPanelClosed", "OnBulletinNewPostButtonSelected",
            "OnCommitSquadronBulletinStatement", "OnCommitSquadronBulletinTitle", "OnCreateNewBulletinPostSelected",
            "OnSquadronBulletinBackButtonSelected", "OnSquadronBulletinEditButtonSelected", "OnSquadronBulletinEntryDeleteButtonSelected",
            "OnSquadronBulletinEntrySelected", "OnSquadronBulletinNewPostCancelled", "OnSquadronBulletinPinnedButtonToggled",
            "OnSquadronSelectedBulletinPostCancelled", "OnUpdateBulletinPostSelected", "OpenNewBulletinPostPanel",
            "OpenedEditBulletinPostPanel", "OpenedViewBulletinPanel", "SquadronEntrySelected", "UpdatedPostPanelClosed",
            "ViewBulletinPanelClosed",
        ],
        "squadrons and their bulletins: none",
    ),
    (&["OnConflictZoneJoinFactionSelected", "OnScenarioFactionSelected", "OnScenarioResponseSelected"], "conflict zones and scenarios: none"),
    (&["ModuleSelect", "SpecialEffectSelected"], "the engineers' pages embedded in the comms movie: none"),
    (
        &[
            "OnDeployBuggyPressed", "OnBoardPressed", "OnRecallPressed", "OnCancelRecallPressed", "OnDismissPressed", "OnCodexPressed",
            "OnLoadoutPressed", "OnDockPressed", "OnSwitchPressed", "OnSwitchToHelmPressed", "OnSwitchToFighterPressed",
            "OnDeployRigPressed", "OnRecallFighterPressed", "OnCancelRecallFighterPressed", "OnDeployFighterPressed",
            "OnHelmOrderChange", "OnFighterOrderChange", "OnEnterGunnerPressed", "OnLeaveGunnerPressed", "OnViewCrewGamerCard",
            "OnTurretRepairSelected", "OnUtilitiesRepairSelected", "OnSRVRigEquipPress", "OnInfoCodexOnePressed",
            "OnInfoCodexTwoPressed", "OnInfoCodexThreePressed",
        ],
        "SRVs, fighters, crew, docking, the codex: none",
    ),
    (
        &["OnLoadoutInformationSelected", "OnExitLoadoutsPanel", "OnSwitchLoadout", "OnEditLoadoutSelected", "OnCreateLoadoutSelected", "OpenLoadoutBrowser"],
        "suit loadouts: none",
    ),
];

/// The event's whole-number argument `at` (Elite's ids are ints; a missing or odd one gives nothing).
fn int(args: &[Value], at: usize) -> Option<i64> {
    args.get(at).and_then(Value::as_f64).filter(|v| v.fract() == 0.0).map(|v| v as i64)
}

fn id(args: &[Value], at: usize) -> Option<u32> {
    int(args, at).and_then(|v| u32::try_from(v).ok())
}

/// What one event from a panel's movie asks for (empty: nothing the mod does; see `IGNORED`).
pub fn on_out(movie: Movie, name: &str, args: &[Value]) -> Vec<PanelAction> {
    use PanelAction as A;
    let page = |at| int(args, at).map(|v| v as i32);
    let action = match name {
        "ShowComplete" | "ShowingPanel" => Some(A::ShowComplete(movie)),
        "HideComplete" | "HidingPanel" => Some(A::HideComplete(movie)),
        "ExternalOnPanel" | "InternalOnPanel" | "OnPanel" => page(0).map(|page| A::Page { movie, page }),
        "ExternalOnPopup" | "InternalOnPopup" | "InternalOnModulePopup" => page(0).map(|state| A::Popup { movie, state }),
        "LocationSelected" => id(args, 0).map(A::LocationSelected),
        "OnLocationPopupButtonSelect" => id(args, 0).map(A::LocationButton),
        "OnLocationPopupBack" => Some(A::LocationBack),
        "CancelJump" => Some(A::CancelJump),
        "ToggleModuleState" => id(args, 0).map(A::ModuleSelected),
        "InternalModuleOptionPopupRadioSelected" => id(args, 0).map(A::ModuleOption),
        "InternalPopupRowSelected" => id(args, 0).map(A::ModuleRow),
        "ComboOptionSelected" => id(args, 0).and_then(Function::of_option).map(A::Toggle),
        "OnDisembarkPressed" => Some(A::Disembark),
        "ChatPanelTabsReady" => Some(A::TabsReady),
        "SetBackgroundVisible" => match (args.first().and_then(Value::as_str), args.get(1).and_then(Value::as_bool)) {
            (Some(name), Some(visible)) => Some(A::Background { name: name.to_string(), visible }),
            _ => None,
        },
        "OnRolePanelStateEnter" => Some(A::RoleScreen { open: true }),
        "OnRolePanelStateBack" => Some(A::RoleScreen { open: false }),
        _ => None,
    };
    action.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(movie: Movie, name: &str, args: &[Value]) -> Option<PanelAction> {
        let mut out = on_out(movie, name, args);
        assert!(out.len() <= 1);
        out.pop()
    }

    #[test]
    fn events_map_to_actions() {
        let left = Movie::CockpitExternalPanel;
        let right = Movie::CockpitInternalPanel2;
        assert_eq!(one(left, "ShowComplete", &[]), Some(PanelAction::ShowComplete(left)));
        assert_eq!(one(Movie::RolePanel3, "HidingPanel", &[]), Some(PanelAction::HideComplete(Movie::RolePanel3)));
        assert_eq!(one(left, "ExternalOnPanel", &[3.into()]), Some(PanelAction::Page { movie: left, page: 3 }));
        assert_eq!(one(right, "InternalOnModulePopup", &[1.into()]), Some(PanelAction::Popup { movie: right, state: 1 }));
        assert_eq!(one(left, "LocationSelected", &[7.into()]), Some(PanelAction::LocationSelected(7)));
        assert_eq!(one(left, "OnLocationPopupButtonSelect", &[5.into()]), Some(PanelAction::LocationButton(5)));
        assert_eq!(one(left, "OnLocationPopupBack", &[]), Some(PanelAction::LocationBack));
        assert_eq!(one(right, "ToggleModuleState", &[12.into()]), Some(PanelAction::ModuleSelected(12)));
        assert_eq!(one(right, "InternalModuleOptionPopupRadioSelected", &[0.into()]), Some(PanelAction::ModuleOption(0)));
        assert_eq!(one(right, "ComboOptionSelected", &[7.into()]), Some(PanelAction::Toggle(Function::LandingGear)));
        assert_eq!(one(right, "ComboOptionSelected", &[15.into()]), Some(PanelAction::Toggle(Function::SilentRunning)));
        assert_eq!(one(right, "ComboOptionSelected", &[16.into()]), None, "the beacon: not listed");
        assert_eq!(one(Movie::RolePanel3, "OnDisembarkPressed", &[]), Some(PanelAction::Disembark));
        assert_eq!(
            one(Movie::CockpitChatPanel, "SetBackgroundVisible", &["Gui_Comms_Full_Panel_Background".into(), true.into()]),
            Some(PanelAction::Background { name: "Gui_Comms_Full_Panel_Background".into(), visible: true })
        );
        assert_eq!(one(Movie::CockpitChatPanel, "ChatPanelTabsReady", &[]), Some(PanelAction::TabsReady));
        for ignored in ["ShowGalaxyMap", "TargetSelected", "ModulePriority", "TextInput", "OnBoardPressed", "AudioPlayerOnNext"] {
            assert_eq!(one(left, ignored, &[1.into(), 2.into()]), None, "{ignored}");
        }
        // Odd arguments give nothing rather than a wrong id.
        assert_eq!(one(left, "LocationSelected", &[]), None);
        assert_eq!(one(left, "LocationSelected", &[(-1).into()]), None);
        assert_eq!(one(left, "LocationSelected", &[1.5.into()]), None);
    }

    /// Whether some argument shape of the movies' gives an action.
    fn mapped(movie: Movie, name: &str) -> bool {
        let probes: [&[Value]; 4] = [&[], &[1.into()], &[1.into(), 1.into()], &["x".into(), true.into()]];
        probes.iter().any(|args| !on_out(movie, name, args).is_empty())
    }

    /// Every event the four movies send (their `api.json`'s `events_to_game`) is mapped by `on_out` or named in `IGNORED`,
    /// once; nothing in `IGNORED` gives an action. `IGNORED` also names events the movies build by name (the home grid's
    /// `Show*`, the music player's, the codex buttons'), which `api.json` cannot list.
    #[test]
    #[ignore = "reads the movies' api.json in the export (out of git)"]
    fn every_event_is_mapped_or_ignored() {
        let dir = std::env::var("ELITE_UI_EXPORT")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/elite/frea_export/sidewinder_ui/decoded").into());
        let ignored: Vec<&str> = IGNORED.iter().flat_map(|(names, _)| names.iter().copied()).collect();
        let mut seen = std::collections::HashSet::new();
        let twice: Vec<&str> = ignored.iter().copied().filter(|n| !seen.insert(*n)).collect();
        assert!(twice.is_empty(), "named twice: {twice:?}");
        let mut problems = Vec::new();
        for movie in [Movie::CockpitExternalPanel, Movie::CockpitInternalPanel2, Movie::CockpitChatPanel, Movie::RolePanel3] {
            let path = format!("{dir}/movies/{}/api.json", movie.name());
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let api: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
            let events = api["events_to_game"].as_object().expect("events_to_game");
            for name in events.keys() {
                match (mapped(movie, name), ignored.contains(&name.as_str())) {
                    (false, false) => problems.push(format!("{}: {name} neither mapped nor ignored", movie.name())),
                    (true, true) => problems.push(format!("{}: {name} mapped and ignored", movie.name())),
                    _ => {}
                }
            }
            for name in &ignored {
                if mapped(movie, name) {
                    problems.push(format!("{}: ignored {name} gives an action", movie.name()));
                }
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }
}
