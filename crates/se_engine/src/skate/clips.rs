//! The Skate 3 clips the mod plays (`copy_skate3` converts exactly these, `anim.rs` plays them by name), and whether each
//! loops. Names are Skate 3's own (`OnBoard.abin`): `R_` riding, `T_` flip tricks (a `_CYC` the board turning, an `_OUT` the
//! catch), `L_` landings, `W_` wipeouts, `IA_` in the air, `MED_CRV_POSE_*` the carve's lean poses.

pub const CLIPS: &[(&str, bool)] = &[
    // Standing still on the board, rolling, tucked at speed.
    ("R_STAND_IDLE2_N_0_CYC", true),
    ("R_IDLE_HCOM_000", true),
    ("R_IDLE_RIDE_N_0_CYC", true),
    ("R_IDLE_SPEEDTUCK_N_0_CYC", true),
    // The carve: lean poses from one side to the other.
    ("MED_CRV_POSE_0", false),
    ("MED_CRV_POSE_1", false),
    ("MED_CRV_POSE_2", false),
    ("MED_CRV_POSE_3", false),
    ("MED_CRV_POSE_4", false),
    ("MED_CRV_POSE_5", false),
    ("MED_CRV_POSE_6", false),
    ("MED_CRV_POSE_7", false),
    ("MED_CRV_POSE_8", false),
    ("MED_CRV_POSE_9", false),
    // Pushing: into it, a stroke, and back on the board.
    ("R_PUSHLSP_HSTR_N_0_INTO", false),
    ("R_PUSHLSP_HSTR_N_0_CYC1", false),
    ("R_PUSHHSP_HSTR_N_0_INTO", false),
    ("R_PUSHHSP_HSTR_N_0_CYC1", false),
    ("R_PUSH_L_N_OUT_FRONT", false),
    ("R_PUSH_H_N_OUT_FRONT", false),
    // The foot brake, the powerslide.
    ("R_BRAKE_N_N_0_INTO", false),
    ("R_BRAKE_N_N_0_CYC", true),
    ("R_BRAKE_N_N_0_OUT", false),
    ("R_SLIDE_BS_HSP_INTO", false),
    ("R_SLIDE_BS_HSP_CYC", true),
    ("R_SLIDE_BS_HSP_OUT_000", false),
    // The crouch before a jump, the ollie's pop and its air.
    ("R_ANTIC_CROUCH_OLLIE_N_0_INTO", false),
    ("R_ANTIC_OLLIE_N_0_CYC", true),
    ("OLLIE_LOW_G", false),
    ("OLLIE_LOW_A", false),
    ("OLLIE_HIGH_G", false),
    ("OLLIE_HIGH_A", false),
    // In the air.
    ("IA_IDLE_N_N_0_CYC", true),
    // Flip tricks: the board turning, then the catch.
    ("T_LOW_KICK_CYC1", true),
    ("T_KICKFLIP_LOW_DCATCH_0_OUT1", false),
    ("T_LOW_HEEL_CYC1", true),
    ("T_HEELFLIP_LOW_DCATCH_OUT1", false),
    ("T_POPSHUVIT_L_CYC", true),
    ("T_POPSHUVIT_LOW_UFLIP_OUT1", false),
    ("T_FSPOPSHUVIT_L_CYC", true),
    ("T_FSPOPSHUVIT_LOW_UFLIP_OUT1", false),
    ("T_360FLIP_L_CYC", true),
    ("T_360FLIP_LOW_DCATCH_OUT1", false),
    ("T_LASERFLIP_L_CYC", true),
    ("T_LASERFLIP_LOW_DCATCH_OUT1", false),
    // Landings: clean, heavy, sketchy.
    ("L_LCOM", false),
    ("L_HCOM_LIMP", false),
    ("L_HCOM_HIMP", false),
    ("L_SKETCH_FS_LCOM", false),
    // Wipeouts and getting up.
    ("W_FLAIL_LEAN000TWIST000_N_0_N", true),
    ("W_BRACE_LEAN000TWIST000_N_0_N", false),
    ("W_RECOVERY_ONFRONT_N_0_N", false),
    ("W_RECOVERY_ONBACKFACE_N_0_N", false),
];
