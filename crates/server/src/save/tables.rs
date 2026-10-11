//! TS 3.0's save tables, generated from `ts_i386.so` (the `TYPEDESCRIPTION` arrays in `.data`,
//! with their field types, names, counts and flags) and the decomp (`::Save` parents,
//! `vtables.tsv`, the `LINK_ENTITY_TO_CLASS` functions). Don't edit by hand: regenerate with `save/gen_tables.py`.
//! Each field's comment is its offset in TS's object (`+0x..`), for mapping to the port's fields.

#![allow(non_upper_case_globals)] // the statics are named after TS's classes

use super::Table;
use goldsrc::ext::saverestore::{FieldDesc, FieldType::*};

const fn f(ty: goldsrc::ext::saverestore::FieldType, name: &'static std::ffi::CStr, count: i16, flags: i16) -> FieldDesc {
    FieldDesc { ty, name, count, flags }
}

/// `gEntvarsDescription` (0x1227c0): written as `"ENTVARS"`.
pub static ENTVARS: Table = Table {
    name: c"ENTVARS",
    fields: &[
        f(String, c"classname", 1, 0),         // +0x0
        f(String, c"globalname", 1, 1),        // +0x4
        f(PositionVector, c"origin", 1, 0),    // +0x8
        f(PositionVector, c"oldorigin", 1, 0), // +0x14
        f(Vector, c"velocity", 1, 0),          // +0x20
        f(Vector, c"basevelocity", 1, 0),      // +0x2c
        f(Vector, c"movedir", 1, 0),           // +0x44
        f(Vector, c"angles", 1, 0),            // +0x50
        f(Vector, c"avelocity", 1, 0),         // +0x5c
        f(Vector, c"punchangle", 1, 0),        // +0x68
        f(Vector, c"v_angle", 1, 0),           // +0x74
        f(Float, c"fixangle", 1, 0),           // +0xa0
        f(Float, c"idealpitch", 1, 0),         // +0xa4
        f(Float, c"pitch_speed", 1, 0),        // +0xa8
        f(Float, c"ideal_yaw", 1, 0),          // +0xac
        f(Float, c"yaw_speed", 1, 0),          // +0xb0
        f(Integer, c"modelindex", 1, 0),       // +0xb4
        f(ModelName, c"model", 1, 1),          // +0xb8
        f(ModelName, c"viewmodel", 1, 0),      // +0xbc
        f(ModelName, c"weaponmodel", 1, 0),    // +0xc0
        f(PositionVector, c"absmin", 1, 0),    // +0xc4
        f(PositionVector, c"absmax", 1, 0),    // +0xd0
        f(Vector, c"mins", 1, 1),              // +0xdc
        f(Vector, c"maxs", 1, 1),              // +0xe8
        f(Vector, c"size", 1, 1),              // +0xf4
        f(Time, c"ltime", 1, 0),               // +0x100
        f(Time, c"nextthink", 1, 0),           // +0x104
        f(Integer, c"solid", 1, 0),            // +0x10c
        f(Integer, c"movetype", 1, 0),         // +0x108
        f(Integer, c"skin", 1, 0),             // +0x110
        f(Integer, c"body", 1, 0),             // +0x114
        f(Integer, c"effects", 1, 0),          // +0x118
        f(Float, c"gravity", 1, 0),            // +0x11c
        f(Float, c"friction", 1, 0),           // +0x120
        f(Float, c"light_level", 1, 0),        // +0x124
        f(Float, c"frame", 1, 0),              // +0x130
        f(Float, c"scale", 1, 0),              // +0x144
        f(Integer, c"sequence", 1, 0),         // +0x128
        f(Time, c"animtime", 1, 0),            // +0x134
        f(Float, c"framerate", 1, 0),          // +0x138
        f(Integer, c"controller", 1, 0),       // +0x13c
        f(Integer, c"blending", 1, 0),         // +0x140
        f(Integer, c"rendermode", 1, 0),       // +0x148
        f(Float, c"renderamt", 1, 0),          // +0x14c
        f(Vector, c"rendercolor", 1, 0),       // +0x150
        f(Integer, c"renderfx", 1, 0),         // +0x15c
        f(Float, c"health", 1, 0),             // +0x160
        f(Float, c"frags", 1, 0),              // +0x164
        f(Integer, c"weapons", 1, 0),          // +0x168
        f(Float, c"takedamage", 1, 0),         // +0x16c
        f(Float, c"deadflag", 1, 0),           // +0x170
        f(Vector, c"view_ofs", 1, 0),          // +0x174
        f(Integer, c"button", 1, 0),           // +0x180
        f(Integer, c"impulse", 1, 0),          // +0x184
        f(Edict, c"chain", 1, 0),              // +0x188
        f(Edict, c"dmg_inflictor", 1, 0),      // +0x18c
        f(Edict, c"enemy", 1, 0),              // +0x190
        f(Edict, c"aiment", 1, 0),             // +0x194
        f(Edict, c"owner", 1, 0),              // +0x198
        f(Edict, c"groundentity", 1, 0),       // +0x19c
        f(Integer, c"spawnflags", 1, 0),       // +0x1a0
        f(Float, c"flags", 1, 0),              // +0x1a4
        f(Integer, c"colormap", 1, 0),         // +0x1a8
        f(Integer, c"team", 1, 0),             // +0x1ac
        f(Float, c"max_health", 1, 0),         // +0x1b0
        f(Time, c"teleport_time", 1, 0),       // +0x1b4
        f(Float, c"armortype", 1, 0),          // +0x1b8
        f(Float, c"armorvalue", 1, 0),         // +0x1bc
        f(Integer, c"waterlevel", 1, 0),       // +0x1c0
        f(Integer, c"watertype", 1, 0),        // +0x1c4
        f(String, c"target", 1, 1),            // +0x1c8
        f(String, c"targetname", 1, 1),        // +0x1cc
        f(String, c"netname", 1, 0),           // +0x1d0
        f(String, c"message", 1, 0),           // +0x1d4
        f(Float, c"dmg_take", 1, 0),           // +0x1d8
        f(Float, c"dmg_save", 1, 0),           // +0x1dc
        f(Float, c"dmg", 1, 0),                // +0x1e0
        f(Time, c"dmgtime", 1, 0),             // +0x1e4
        f(SoundName, c"noise", 1, 0),          // +0x1e8
        f(SoundName, c"noise1", 1, 0),         // +0x1ec
        f(SoundName, c"noise2", 1, 0),         // +0x1f0
        f(SoundName, c"noise3", 1, 0),         // +0x1f4
        f(Float, c"speed", 1, 0),              // +0x1f8
        f(Time, c"air_finished", 1, 0),        // +0x1fc
        f(Time, c"pain_finished", 1, 0),       // +0x200
        f(Time, c"radsuit_finished", 1, 0),    // +0x204
    ],
};

/// `CBaseEntity::m_SaveData` (0x1161c0): written as `"BASE"`.
pub static BASE: Table = Table {
    name: c"BASE",
    fields: &[
        f(ClassPtr, c"m_pGoalEnt", 1, 0),   // +0x4
        f(Function, c"m_pfnThink", 1, 0),   // +0xc
        f(Function, c"m_pfnTouch", 1, 0),   // +0x14
        f(Function, c"m_pfnUse", 1, 0),     // +0x1c
        f(Function, c"m_pfnBlocked", 1, 0), // +0x24
    ],
};

/// `CGlobalState::m_SaveData` (0x12355c): written as `"GLOBAL"`.
pub static GLOBAL: Table = Table {
    name: c"GLOBAL",
    fields: &[
        f(Integer, c"m_listCount", 1, 0), // +0x4
    ],
};

/// `gGlobalEntitySaveData` (0x123580): written as `"GENT"`.
pub static GENT: Table = Table {
    name: c"GENT",
    fields: &[
        f(Character, c"name", 64, 0),      // +0x0
        f(Character, c"levelName", 32, 0), // +0x40
        f(Integer, c"state", 1, 0),        // +0x60
    ],
};

/// `CActAnimating::m_SaveData` (0x124140): written as `"CActAnimating"`.
pub static CActAnimating: Table = Table {
    name: c"CActAnimating",
    fields: &[
        f(Integer, c"m_Activity", 1, 0), // +0x80
    ],
};

/// `CAmbientGeneric::m_SaveData` (0x11f800): written as `"CAmbientGeneric"`.
pub static CAmbientGeneric: Table = Table {
    name: c"CAmbientGeneric",
    fields: &[
        f(Float, c"m_flAttenuation", 1, 0), // +0x64
        f(Boolean, c"m_fActive", 1, 0),     // +0xcc
        f(Boolean, c"m_fLooping", 1, 0),    // +0xd0
        f(Character, c"m_dpv", 100, 0),     // +0x68
    ],
};

/// `CAutoTrigger::m_SaveData` (0x1206a0): written as `"CAutoTrigger"`.
pub static CAutoTrigger: Table = Table {
    name: c"CAutoTrigger",
    fields: &[
        f(String, c"m_globalstate", 1, 0), // +0x6c
        f(Integer, c"triggerType", 1, 0),  // +0x70
    ],
};

/// `CBaseAnimating::m_SaveData` (0x114ac0): written as `"CBaseAnimating"`.
pub static CBaseAnimating: Table = Table {
    name: c"CBaseAnimating",
    fields: &[
        f(Float, c"m_flFrameRate", 1, 0),         // +0x6c
        f(Float, c"m_flGroundSpeed", 1, 0),       // +0x70
        f(Time, c"m_flLastEventCheck", 1, 0),     // +0x74
        f(Boolean, c"m_fSequenceFinished", 1, 0), // +0x78
        f(Boolean, c"m_fSequenceLoops", 1, 0),    // +0x7c
    ],
};

/// `CBaseButton::m_SaveData` (0x115800): written as `"CBaseButton"`.
pub static CBaseButton: Table = Table {
    name: c"CBaseButton",
    fields: &[
        f(Boolean, c"m_fStayPushed", 1, 0),         // +0x104
        f(Boolean, c"m_fRotating", 1, 0),           // +0x108
        f(Integer, c"m_sounds", 1, 0),              // +0x138
        f(Character, c"m_bLockedSound", 1, 0),      // +0x134
        f(Character, c"m_bLockedSentence", 1, 0),   // +0x135
        f(Character, c"m_bUnlockedSound", 1, 0),    // +0x136
        f(Character, c"m_bUnlockedSentence", 1, 0), // +0x137
        f(String, c"m_strChangeTarget", 1, 0),      // +0x10c
    ],
};

/// `CBaseDelay::m_SaveData` (0x11fe40): written as `"CBaseDelay"`.
pub static CBaseDelay: Table = Table {
    name: c"CBaseDelay",
    fields: &[
        f(Float, c"m_flDelay", 1, 0),        // +0x64
        f(String, c"m_iszKillTarget", 1, 0), // +0x68
    ],
};

/// `CBaseDoor::m_SaveData` (0x116740): written as `"CBaseDoor"`.
pub static CBaseDoor: Table = Table {
    name: c"CBaseDoor",
    fields: &[
        f(Character, c"m_bHealthValue", 1, 0),      // +0x13d
        f(Character, c"m_bMoveSnd", 1, 0),          // +0x13e
        f(Character, c"m_bStopSnd", 1, 0),          // +0x13f
        f(Character, c"m_bLockedSound", 1, 0),      // +0x164
        f(Character, c"m_bLockedSentence", 1, 0),   // +0x165
        f(Character, c"m_bUnlockedSound", 1, 0),    // +0x166
        f(Character, c"m_bUnlockedSentence", 1, 0), // +0x167
    ],
};

/// `CBasePlatTrain::m_SaveData` (0x11d4a0): written as `"CBasePlatTrain"`.
pub static CBasePlatTrain: Table = Table {
    name: c"CBasePlatTrain",
    fields: &[
        f(Character, c"m_bMoveSnd", 1, 0), // +0x104
        f(Character, c"m_bStopSnd", 1, 0), // +0x105
        f(Float, c"m_volume", 1, 0),       // +0x108
    ],
};

/// `CBasePlayer::m_playerSaveData` (0x11e2e0): written as `"PLAYER"`.
pub static CBasePlayer: Table = Table {
    name: c"PLAYER",
    fields: &[
        f(Integer, c"m_afButtonLast", 1, 0),       // +0x810
        f(Integer, c"m_afButtonPressed", 1, 0),    // +0x814
        f(Integer, c"m_afButtonReleased", 1, 0),   // +0x818
        f(Integer, c"m_rgItems", 5, 0),            // +0x82c
        f(Integer, c"m_afPhysicsFlags", 1, 0),     // +0x848
        f(Time, c"m_flTimeStepSound", 1, 0),       // +0x850
        f(Time, c"m_flTimeWeaponIdle", 1, 0),      // +0x854
        f(Time, c"m_flSwimTime", 1, 0),            // +0x858
        f(Time, c"m_flDuckTime", 1, 0),            // +0x85c
        f(Time, c"m_flWallJumpTime", 1, 0),        // +0x860
        f(Time, c"m_flSuitUpdate", 1, 0),          // +0x864
        f(Integer, c"m_rgSuitPlayList", 4, 0),     // +0x868
        f(Integer, c"m_iSuitPlayNext", 1, 0),      // +0x878
        f(Integer, c"m_rgiSuitNoRepeat", 32, 0),   // +0x87c
        f(Time, c"m_rgflSuitNoRepeatTime", 32, 0), // +0x8fc
        f(Integer, c"m_lastDamageAmount", 1, 0),   // +0x97c
        f(ClassPtr, c"m_rgpPlayerItems", 6, 0),    // +0x9f8
        f(ClassPtr, c"m_pActiveItem", 1, 0),       // +0xa10
        f(ClassPtr, c"m_pLastItem", 1, 0),         // +0xa18
        f(Integer, c"m_rgAmmo", 32, 0),            // +0xa20
        f(Integer, c"m_idrowndmg", 1, 0),          // +0x9a8
        f(Integer, c"m_idrownrestored", 1, 0),     // +0x9ac
        f(Time, c"m_tSneaking", 1, 0),             // +0x9d8
        f(Integer, c"m_iTrain", 1, 0),             // +0x9bc
        f(Integer, c"m_bitsHUDDamage", 1, 0),      // +0x9b0
        f(Float, c"m_flFallVelocity", 1, 0),       // +0x828
        f(Integer, c"m_iTargetVolume", 1, 0),      // +0x7f4
        f(Integer, c"m_iWeaponVolume", 1, 0),      // +0x7f8
        f(Integer, c"m_iExtraSoundTypes", 1, 0),   // +0x7fc
        f(Integer, c"m_iWeaponFlash", 1, 0),       // +0x800
        f(Boolean, c"m_fLongJump", 1, 0),          // +0x9d4
        f(Boolean, c"m_fInitHUD", 1, 0),           // +0x9b4
        f(Time, c"m_tbdPrev", 1, 0),               // +0x980
        f(EHandle, c"m_pTank", 1, 0),              // +0x9c4
        f(Integer, c"m_iHideHUD", 1, 0),           // +0x9e8
        f(Integer, c"m_iFOV", 1, 0),               // +0x9f0
    ],
};

/// `CBasePlayerItem::m_SaveData` (0x122da0): written as `"CBasePlayerItem"`.
pub static CBasePlayerItem: Table = Table {
    name: c"CBasePlayerItem",
    fields: &[
        f(ClassPtr, c"m_pPlayer", 1, 0), // +0x80
        f(ClassPtr, c"m_pNext", 1, 0),   // +0x84
        f(Integer, c"m_iId", 1, 0),      // +0x88
    ],
};

/// `CBasePlayerWeapon::m_SaveData` (0x122de0): written as `"CBasePlayerWeapon"`.
pub static CBasePlayerWeapon: Table = Table {
    name: c"CBasePlayerWeapon",
    fields: &[
        f(Float, c"m_flNextPrimaryAttack", 1, 0),   // +0x9c
        f(Float, c"m_flNextSecondaryAttack", 1, 0), // +0xa0
        f(Float, c"m_flTimeWeaponIdle", 1, 0),      // +0xa8
        f(Integer, c"m_iPrimaryAmmoType", 1, 0),    // +0xac
        f(Integer, c"m_iSecondaryAmmoType", 1, 0),  // +0xb0
        f(Integer, c"m_iClip", 1, 0),               // +0xb4
        f(Integer, c"m_iDefaultAmmo", 1, 0),        // +0xc4
    ],
};

/// `CBaseToggle::m_SaveData` (0x11fe60): written as `"CBaseToggle"`.
pub static CBaseToggle: Table = Table {
    name: c"CBaseToggle",
    fields: &[
        f(Integer, c"m_toggle_state", 1, 0),         // +0x80
        f(Time, c"m_flActivateFinished", 1, 0),      // +0x84
        f(Float, c"m_flMoveDistance", 1, 0),         // +0x88
        f(Float, c"m_flWait", 1, 0),                 // +0x8c
        f(Float, c"m_flLip", 1, 0),                  // +0x90
        f(Float, c"m_flTWidth", 1, 0),               // +0x94
        f(Float, c"m_flTLength", 1, 0),              // +0x98
        f(PositionVector, c"m_vecPosition1", 1, 0),  // +0x9c
        f(PositionVector, c"m_vecPosition2", 1, 0),  // +0xa8
        f(Vector, c"m_vecAngle1", 1, 0),             // +0xb4
        f(Vector, c"m_vecAngle2", 1, 0),             // +0xc0
        f(Integer, c"m_cTriggersLeft", 1, 0),        // +0xcc
        f(Float, c"m_flHeight", 1, 0),               // +0xd0
        f(EHandle, c"m_hActivator", 1, 0),           // +0xd4
        f(Function, c"m_pfnCallWhenMoveDone", 1, 0), // +0xdc
        f(PositionVector, c"m_vecFinalDest", 1, 0),  // +0xe4
        f(Vector, c"m_vecFinalAngle", 1, 0),         // +0xf0
        f(String, c"m_sMaster", 1, 0),               // +0x100
        f(Integer, c"m_bitsDamageInflict", 1, 0),    // +0xfc
    ],
};

/// `CBreakable::m_SaveData` (0x118240): written as `"CBreakable"`.
pub static CBreakable: Table = Table {
    name: c"CBreakable",
    fields: &[
        f(Integer, c"m_Material", 1, 0),      // +0x78
        f(Integer, c"m_Explosion", 1, 0),     // +0x7c
        f(Float, c"m_angle", 1, 0),           // +0x84
        f(String, c"m_iszGibModel", 1, 0),    // +0x88
        f(String, c"m_iszSpawnObject", 1, 0), // +0x8c
    ],
};

/// `CBubbling::m_SaveData` (0x116b20): written as `"CBubbling"`.
pub static CBubbling: Table = Table {
    name: c"CBubbling",
    fields: &[
        f(Integer, c"m_density", 1, 0),   // +0x64
        f(Integer, c"m_frequency", 1, 0), // +0x68
        f(Integer, c"m_state", 1, 0),     // +0x70
    ],
};

/// `CChangeLevel::m_SaveData` (0x120740): written as `"CChangeLevel"`.
pub static CChangeLevel: Table = Table {
    name: c"CChangeLevel",
    fields: &[
        f(Character, c"m_szMapName", 32, 0),      // +0x104
        f(Character, c"m_szLandmarkName", 32, 0), // +0x124
        f(String, c"m_changeTarget", 1, 0),       // +0x144
        f(Float, c"m_changeTargetDelay", 1, 0),   // +0x148
    ],
};

/// `CCycler::m_SaveData` (0x119300): written as `"CCycler"`.
pub static CCycler: Table = Table {
    name: c"CCycler",
    fields: &[
        f(Integer, c"m_animate", 1, 0), // +0x164
    ],
};

/// `CCyclerSprite::m_SaveData` (0x119320): written as `"CCyclerSprite"`.
pub static CCyclerSprite: Table = Table {
    name: c"CCyclerSprite",
    fields: &[
        f(Integer, c"m_animate", 1, 0), // +0x64
        f(Time, c"m_lastTime", 1, 0),   // +0x68
        f(Float, c"m_maxFrame", 1, 0),  // +0x6c
    ],
};

/// `CEnvExplosion::m_SaveData` (0x117f40): written as `"CEnvExplosion"`.
pub static CEnvExplosion: Table = Table {
    name: c"CEnvExplosion",
    fields: &[
        f(Integer, c"m_iMagnitude", 1, 0),  // +0x164
        f(Integer, c"m_spriteScale", 1, 0), // +0x168
    ],
};

/// `CEnvGlobal::m_SaveData` (0x115780): written as `"CEnvGlobal"`.
pub static CEnvGlobal: Table = Table {
    name: c"CEnvGlobal",
    fields: &[
        f(String, c"m_globalstate", 1, 0),   // +0x64
        f(Integer, c"m_triggermode", 1, 0),  // +0x68
        f(Integer, c"m_initialstate", 1, 0), // +0x6c
    ],
};

/// `CEnvSound::m_SaveData` (0x11f840): written as `"CEnvSound"`.
pub static CEnvSound: Table = Table {
    name: c"CEnvSound",
    fields: &[
        f(Float, c"m_flRadius", 1, 0),   // +0x64
        f(Float, c"m_flRoomtype", 1, 0), // +0x68
    ],
};

/// `CEnvSpark::m_SaveData` (0x1158e0): written as `"CEnvSpark"`.
pub static CEnvSpark: Table = Table {
    name: c"CEnvSpark",
    fields: &[
        f(Float, c"m_flDelay", 1, 0), // +0x64
    ],
};

/// `CFrictionModifier::m_SaveData` (0x120680): written as `"CFrictionModifier"`.
pub static CFrictionModifier: Table = Table {
    name: c"CFrictionModifier",
    fields: &[
        f(Float, c"m_frictionFraction", 1, 0), // +0x64
    ],
};

/// `CFuncPlat::m_SaveData` (0x11d4e0): written as `"CFuncPlat"`.
pub static CFuncPlat: Table = Table {
    name: c"CFuncPlat",
    fields: &[
        f(String, c"targettop", 1, 0), // +0x10c
        f(String, c"targetbot", 1, 0), // +0x110
    ],
};

/// `CFuncPlatRot::m_SaveData` (0x11d500): written as `"CFuncPlatRot"`.
pub static CFuncPlatRot: Table = Table {
    name: c"CFuncPlatRot",
    fields: &[
        f(Vector, c"m_end", 1, 0),   // +0x114
        f(Vector, c"m_start", 1, 0), // +0x120
    ],
};

/// `CFuncRotating::m_SaveData` (0x114ec0): written as `"CFuncRotating"`.
pub static CFuncRotating: Table = Table {
    name: c"CFuncRotating",
    fields: &[
        f(Float, c"m_flFanFriction", 1, 0), // +0x64
        f(Float, c"m_flAttenuation", 1, 0), // +0x68
        f(Float, c"m_flVolume", 1, 0),      // +0x6c
        f(Float, c"m_pitch", 1, 0),         // +0x70
        f(Integer, c"m_sounds", 1, 0),      // +0x74
    ],
};

/// `CFuncTank::m_SaveData` (0x118440): written as `"CFuncTank"`.
pub static CFuncTank: Table = Table {
    name: c"CFuncTank",
    fields: &[
        f(Float, c"m_yawCenter", 1, 0),            // +0x78
        f(Float, c"m_yawRate", 1, 0),              // +0x7c
        f(Float, c"m_yawRange", 1, 0),             // +0x80
        f(Float, c"m_yawTolerance", 1, 0),         // +0x84
        f(Float, c"m_pitchCenter", 1, 0),          // +0x88
        f(Float, c"m_pitchRate", 1, 0),            // +0x8c
        f(Float, c"m_pitchRange", 1, 0),           // +0x90
        f(Float, c"m_pitchTolerance", 1, 0),       // +0x94
        f(Time, c"m_fireLast", 1, 0),              // +0x98
        f(Float, c"m_fireRate", 1, 0),             // +0x9c
        f(Time, c"m_lastSightTime", 1, 0),         // +0xa0
        f(Float, c"m_persist", 1, 0),              // +0xa4
        f(Float, c"m_minRange", 1, 0),             // +0xa8
        f(Float, c"m_maxRange", 1, 0),             // +0xac
        f(Vector, c"m_barrelPos", 1, 0),           // +0xb0
        f(Float, c"m_spriteScale", 1, 0),          // +0xbc
        f(String, c"m_iszSpriteSmoke", 1, 0),      // +0xc0
        f(String, c"m_iszSpriteFlash", 1, 0),      // +0xc4
        f(Integer, c"m_bulletType", 1, 0),         // +0xc8
        f(Vector, c"m_sightOrigin", 1, 0),         // +0xd0
        f(Integer, c"m_spread", 1, 0),             // +0xdc
        f(ClassPtr, c"m_pController", 1, 0),       // +0x64
        f(Vector, c"m_vecControllerUsePos", 1, 0), // +0x6c
        f(Time, c"m_flNextAttack", 1, 0),          // +0x68
        f(Integer, c"m_iBulletDamage", 1, 0),      // +0xcc
        f(String, c"m_iszMaster", 1, 0),           // +0xe0
    ],
};

/// `CFuncTankControls::m_SaveData` (0x118600): written as `"CFuncTankControls"`.
pub static CFuncTankControls: Table = Table {
    name: c"CFuncTankControls",
    fields: &[
        f(ClassPtr, c"m_pTank", 1, 0), // +0x64
    ],
};

/// `CFuncTankLaser::m_SaveData` (0x1185e0): written as `"CFuncTankLaser"`.
pub static CFuncTankLaser: Table = Table {
    name: c"CFuncTankLaser",
    fields: &[
        f(ClassPtr, c"m_pLaser", 1, 0), // +0xe4
        f(Time, c"m_laserTime", 1, 0),  // +0xe8
    ],
};

/// `CFuncTrackChange::m_SaveData` (0x11d620): written as `"CFuncTrackChange"`.
pub static CFuncTrackChange: Table = Table {
    name: c"CFuncTrackChange",
    fields: &[
        f(ClassPtr, c"m_trackTop", 1, 1),      // +0x12c
        f(ClassPtr, c"m_trackBottom", 1, 1),   // +0x130
        f(ClassPtr, c"m_train", 1, 1),         // +0x134
        f(String, c"m_trackTopName", 1, 1),    // +0x138
        f(String, c"m_trackBottomName", 1, 1), // +0x13c
        f(String, c"m_trainName", 1, 1),       // +0x140
        f(Integer, c"m_code", 1, 0),           // +0x144
        f(Integer, c"m_targetState", 1, 0),    // +0x148
        f(Integer, c"m_use", 1, 0),            // +0x14c
    ],
};

/// `CFuncTrackTrain::m_SaveData` (0x11d560): written as `"CFuncTrackTrain"`.
pub static CFuncTrackTrain: Table = Table {
    name: c"CFuncTrackTrain",
    fields: &[
        f(ClassPtr, c"m_ppath", 1, 0),     // +0x64
        f(Float, c"m_length", 1, 0),       // +0x68
        f(Float, c"m_height", 1, 0),       // +0x6c
        f(Float, c"m_speed", 1, 0),        // +0x70
        f(Float, c"m_dir", 1, 0),          // +0x74
        f(Float, c"m_startSpeed", 1, 0),   // +0x78
        f(Vector, c"m_controlMins", 1, 0), // +0x7c
        f(Vector, c"m_controlMaxs", 1, 0), // +0x88
        f(Integer, c"m_sounds", 1, 0),     // +0x98
        f(Float, c"m_flVolume", 1, 0),     // +0x9c
        f(Float, c"m_flBank", 1, 0),       // +0xa0
        f(Float, c"m_oldSpeed", 1, 0),     // +0xa4
    ],
};

/// `CFuncTrain::m_SaveData` (0x11d520): written as `"CFuncTrain"`.
pub static CFuncTrain: Table = Table {
    name: c"CFuncTrain",
    fields: &[
        f(Integer, c"m_sounds", 1, 0),         // +0x110
        f(EVars, c"m_pevCurrentTarget", 1, 0), // +0x10c
        f(Boolean, c"m_activated", 1, 0),      // +0x114
    ],
};

/// `CGamePlayerZone::m_SaveData` (0x11bb60): written as `"CGamePlayerZone"`.
pub static CGamePlayerZone: Table = Table {
    name: c"CGamePlayerZone",
    fields: &[
        f(String, c"m_iszInTarget", 1, 0),  // +0x68
        f(String, c"m_iszOutTarget", 1, 0), // +0x6c
        f(String, c"m_iszInCount", 1, 0),   // +0x70
        f(String, c"m_iszOutCount", 1, 0),  // +0x74
    ],
};

/// `CGameText::m_SaveData` (0x11bb50): written as `"CGameText"`.
pub static CGameText: Table = Table {
    name: c"CGameText",
    fields: &[
        f(Character, c"m_textParms", 40, 0), // +0x68
    ],
};

/// `CGibShooter::m_SaveData` (0x116cc0): written as `"CGibShooter"`.
pub static CGibShooter: Table = Table {
    name: c"CGibShooter",
    fields: &[
        f(Integer, c"m_iGibs", 1, 0),          // +0x6c
        f(Integer, c"m_iGibCapacity", 1, 0),   // +0x70
        f(Integer, c"m_iGibMaterial", 1, 0),   // +0x74
        f(Integer, c"m_iGibModelIndex", 1, 0), // +0x78
        f(Float, c"m_flGibVelocity", 1, 0),    // +0x7c
        f(Float, c"m_flVariance", 1, 0),       // +0x80
        f(Float, c"m_flGibLife", 1, 0),        // +0x84
    ],
};

/// `CGlow::m_SaveData` (0x116c80): written as `"CGlow"`.
pub static CGlow: Table = Table {
    name: c"CGlow",
    fields: &[
        f(Time, c"m_lastTime", 1, 0),  // +0x64
        f(Float, c"m_maxFrame", 1, 0), // +0x68
    ],
};

/// `CGunTarget::m_SaveData` (0x11d6b0): written as `"CGunTarget"`.
pub static CGunTarget: Table = Table {
    name: c"CGunTarget",
    fields: &[
        f(Boolean, c"m_on", 1, 0), // +0x164
    ],
};

/// `CLaser::m_SaveData` (0x116c40): written as `"CLaser"`.
pub static CLaser: Table = Table {
    name: c"CLaser",
    fields: &[
        f(ClassPtr, c"m_pSprite", 1, 0),            // +0x64
        f(String, c"m_iszSpriteName", 1, 0),        // +0x68
        f(PositionVector, c"m_firePosition", 1, 0), // +0x6c
    ],
};

/// `CLight::m_SaveData` (0x11b8e0): written as `"CLight"`.
pub static CLight: Table = Table {
    name: c"CLight",
    fields: &[
        f(Integer, c"m_iStyle", 1, 0),    // +0x64
        f(String, c"m_iszPattern", 1, 0), // +0x68
    ],
};

/// `CLightning::m_SaveData` (0x116b60): written as `"CLightning"`.
pub static CLightning: Table = Table {
    name: c"CLightning",
    fields: &[
        f(Integer, c"m_active", 1, 0),         // +0x64
        f(String, c"m_iszStartEntity", 1, 0),  // +0x68
        f(String, c"m_iszEndEntity", 1, 0),    // +0x6c
        f(Float, c"m_life", 1, 0),             // +0x70
        f(Integer, c"m_boltWidth", 1, 0),      // +0x74
        f(Integer, c"m_noiseAmplitude", 1, 0), // +0x78
        f(Integer, c"m_brightness", 1, 0),     // +0x7c
        f(Integer, c"m_speed", 1, 0),          // +0x80
        f(Float, c"m_restrike", 1, 0),         // +0x84
        f(Integer, c"m_spriteTexture", 1, 0),  // +0x88
        f(String, c"m_iszSpriteName", 1, 0),   // +0x8c
        f(Integer, c"m_frameStart", 1, 0),     // +0x90
        f(Float, c"m_radius", 1, 0),           // +0x94
    ],
};

/// `CMomentaryDoor::m_SaveData` (0x1167b0): written as `"CMomentaryDoor"`.
pub static CMomentaryDoor: Table = Table {
    name: c"CMomentaryDoor",
    fields: &[
        f(Character, c"m_bMoveSnd", 1, 0), // +0x104
    ],
};

/// `CMomentaryRotButton::m_SaveData` (0x115880): written as `"CMomentaryRotButton"`.
pub static CMomentaryRotButton: Table = Table {
    name: c"CMomentaryRotButton",
    fields: &[
        f(Integer, c"m_lastUsed", 1, 0),  // +0x104
        f(Integer, c"m_direction", 1, 0), // +0x108
        f(Float, c"m_returnSpeed", 1, 0), // +0x10c
        f(Vector, c"m_start", 1, 0),      // +0x110
        f(Vector, c"m_end", 1, 0),        // +0x11c
        f(Integer, c"m_sounds", 1, 0),    // +0x128
    ],
};

/// `CMultiManager::m_SaveData` (0x1206e0): written as `"CMultiManager"`.
pub static CMultiManager: Table = Table {
    name: c"CMultiManager",
    fields: &[
        f(Integer, c"m_cTargets", 1, 0),     // +0x104
        f(Integer, c"m_index", 1, 0),        // +0x108
        f(Time, c"m_startTime", 1, 0),       // +0x10c
        f(String, c"m_iTargetName", 16, 0),  // +0x110
        f(Float, c"m_flTargetDelay", 16, 0), // +0x150
    ],
};

/// `CMultiSource::m_SaveData` (0x1157c0): written as `"CMultiSource"`.
pub static CMultiSource: Table = Table {
    name: c"CMultiSource",
    fields: &[
        f(EHandle, c"m_rgEntities", 32, 0),  // +0x64
        f(Integer, c"m_rgTriggered", 32, 0), // +0x164
        f(Integer, c"m_iTotal", 1, 0),       // +0x1e4
        f(String, c"m_globalstate", 1, 0),   // +0x1e8
    ],
};

/// `CPathCorner::m_SaveData` (0x11d1e0): written as `"CPathCorner"`.
pub static CPathCorner: Table = Table {
    name: c"CPathCorner",
    fields: &[
        f(Float, c"m_flWait", 1, 0), // +0x64
    ],
};

/// `CPathTrack::m_SaveData` (0x11d200): written as `"CPathTrack"`.
pub static CPathTrack: Table = Table {
    name: c"CPathTrack",
    fields: &[
        f(Float, c"m_length", 1, 0),       // +0x64
        f(ClassPtr, c"m_pnext", 1, 0),     // +0x6c
        f(ClassPtr, c"m_paltpath", 1, 0),  // +0x74
        f(ClassPtr, c"m_pprevious", 1, 0), // +0x70
        f(String, c"m_altName", 1, 0),     // +0x68
    ],
};

/// `CPendulum::m_SaveData` (0x114f20): written as `"CPendulum"`.
pub static CPendulum: Table = Table {
    name: c"CPendulum",
    fields: &[
        f(Float, c"m_accel", 1, 0),     // +0x64
        f(Float, c"m_distance", 1, 0),  // +0x68
        f(Time, c"m_time", 1, 0),       // +0x6c
        f(Float, c"m_damp", 1, 0),      // +0x70
        f(Float, c"m_maxSpeed", 1, 0),  // +0x74
        f(Float, c"m_dampSpeed", 1, 0), // +0x78
        f(Vector, c"m_center", 1, 0),   // +0x7c
        f(Vector, c"m_start", 1, 0),    // +0x88
    ],
};

/// `CPushable::m_SaveData` (0x1182e0): written as `"CPushable"`.
pub static CPushable: Table = Table {
    name: c"CPushable",
    fields: &[
        f(Float, c"m_maxSpeed", 1, 0), // +0x11c
        f(Time, c"m_soundTime", 1, 0), // +0x120
    ],
};

/// `CRecharge::m_SaveData` (0x119180): written as `"CRecharge"`.
pub static CRecharge: Table = Table {
    name: c"CRecharge",
    fields: &[
        f(Time, c"m_flNextCharge", 1, 0),   // +0x104
        f(Integer, c"m_iReactivate", 1, 0), // +0x108
        f(Integer, c"m_iJuice", 1, 0),      // +0x10c
        f(Integer, c"m_iOn", 1, 0),         // +0x110
        f(Time, c"m_flSoundTime", 1, 0),    // +0x114
    ],
};

/// `CRevertSaved::m_SaveData` (0x11e540): written as `"CRevertSaved"`.
pub static CRevertSaved: Table = Table {
    name: c"CRevertSaved",
    fields: &[
        f(Float, c"m_messageTime", 1, 0), // +0x64
        f(Float, c"m_loadTime", 1, 0),    // +0x68
    ],
};

/// `CRuleEntity::m_SaveData` (0x11bb40): written as `"CRuleEntity"`.
pub static CRuleEntity: Table = Table {
    name: c"CRuleEntity",
    fields: &[
        f(String, c"m_iszMaster", 1, 0), // +0x64
    ],
};

/// `CSpeaker::m_SaveData` (0x11f870): written as `"CSpeaker"`.
pub static CSpeaker: Table = Table {
    name: c"CSpeaker",
    fields: &[
        f(Integer, c"m_preset", 1, 0), // +0x64
    ],
};

/// `CSprite::m_SaveData` (0x116ca0): written as `"CSprite"`.
pub static CSprite: Table = Table {
    name: c"CSprite",
    fields: &[
        f(Time, c"m_lastTime", 1, 0),  // +0x64
        f(Float, c"m_maxFrame", 1, 0), // +0x68
    ],
};

/// `CTSBomb::m_SaveData` (0x113620): written as `"CTSBomb"`.
pub static CTSBomb: Table = Table {
    name: c"CTSBomb",
    fields: &[
        f(Integer, c"defusesec", 1, 0),  // +0x80
        f(Integer, c"cancutwire", 1, 0), // +0x84
    ],
};

/// `CTSDMHill::m_SaveData` (0x111160): written as `"CTSDMHill"`.
pub static CTSDMHill: Table = Table {
    name: c"CTSDMHill",
    fields: &[
        f(Integer, c"fragseverycheck", 1, 0),     // +0x70
        f(Integer, c"fragseverycheckteam", 1, 0), // +0x74
        f(Float, c"nextcheck", 1, 0),             // +0x78
    ],
};

/// `CTSGroundWeapon::m_SaveData` (0x1145e0): written as `"CTSGroundWeapon"`.
pub static CTSGroundWeapon: Table = Table {
    name: c"CTSGroundWeapon",
    fields: &[
        f(Integer, c"tsweaponid", 1, 0),  // +0xa4
        f(Integer, c"respawntime", 1, 0), // +0xa0
        f(Integer, c"extraclip", 1, 0),   // +0xa8
    ],
};

/// `CTSHack::m_SaveData` (0x113640): written as `"CTSHack"`.
pub static CTSHack: Table = Table {
    name: c"CTSHack",
    fields: &[
        f(Integer, c"hacksec", 1, 0), // +0x7c
    ],
};

/// `CTSMapGlobals::m_SaveData` (0x1111a0): written as `"CTSMapGlobals"`.
pub static CTSMapGlobals: Table = Table {
    name: c"CTSMapGlobals",
    fields: &[
        f(Integer, c"player_respawn_num", 1, 0),   // +0x64
        f(Integer, c"player_at_a_time_num", 1, 0), // +0x68
    ],
};

/// `CTSObjective::m_SaveData` (0x113600): written as `"CTSObjective"`.
pub static CTSObjective: Table = Table {
    name: c"CTSObjective",
    fields: &[
        f(Integer, c"attacker_team", 1, 0), // +0x68
    ],
};

/// `CTSObjectiveManager::m_SaveData` (0x1134e0): written as `"CTSObjectiveManager"`.
pub static CTSObjectiveManager: Table = Table {
    name: c"CTSObjectiveManager",
    fields: &[
        f(String, c"messages[0]", 1, 0),      // +0x64
        f(String, c"messages[1]", 1, 0),      // +0x68
        f(String, c"messages[2]", 1, 0),      // +0x6c
        f(String, c"messages[3]", 1, 0),      // +0x70
        f(String, c"targets[0]", 1, 0),       // +0x74
        f(String, c"targets[1]", 1, 0),       // +0x78
        f(String, c"targets[2]", 1, 0),       // +0x7c
        f(String, c"targets[3]", 1, 0),       // +0x80
        f(String, c"targets[4]", 1, 0),       // +0x84
        f(String, c"targets[5]", 1, 0),       // +0x88
        f(String, c"targets[6]", 1, 0),       // +0x8c
        f(String, c"targets[7]", 1, 0),       // +0x90
        f(String, c"end_round_target", 1, 0), // +0x94
    ],
};

/// `CTSPowerUp::m_SaveData` (0x114340): written as `"CTSPowerUp"`.
pub static CTSPowerUp: Table = Table {
    name: c"CTSPowerUp",
    fields: &[
        f(Integer, c"poweruptype", 1, 0),  // +0x84
        f(Integer, c"pwupduration", 1, 0), // +0x94
    ],
};

/// `CTSSlowMotion::m_SaveData` (0x111140): written as `"CTSSlowMotion"`.
pub static CTSSlowMotion: Table = Table {
    name: c"CTSSlowMotion",
    fields: &[
        f(Float, c"duration", 1, 0),     // +0xf8
        f(Float, c"slow_ammount", 1, 0), // +0xfc
    ],
};

/// `CTSTeamEscape::m_SaveData` (0x113650): written as `"CTSTeamEscape"`.
pub static CTSTeamEscape: Table = Table {
    name: c"CTSTeamEscape",
    fields: &[
        f(Integer, c"escape_percentage", 1, 0), // +0xf4
    ],
};

/// `CTSWinGiver::m_SaveData` (0x1135c0): written as `"CTSWinGiver"`.
pub static CTSWinGiver: Table = Table {
    name: c"CTSWinGiver",
    fields: &[
        f(Integer, c"cash_for_winner", 1, 0), // +0x6c
        f(Integer, c"cash_for_loser", 1, 0),  // +0x70
        f(Integer, c"winnerteam", 1, 0),      // +0x74
        f(Integer, c"numberofcall", 1, 0),    // +0x78
    ],
};

/// `CTriggerCamera::m_SaveData` (0x1207a0): written as `"CTriggerCamera"`.
pub static CTriggerCamera: Table = Table {
    name: c"CTriggerCamera",
    fields: &[
        f(EHandle, c"m_hPlayer", 1, 0),    // +0x6c
        f(EHandle, c"m_hTarget", 1, 0),    // +0x74
        f(ClassPtr, c"m_pentPath", 1, 0),  // +0x7c
        f(String, c"m_sPath", 1, 0),       // +0x80
        f(Float, c"m_flWait", 1, 0),       // +0x84
        f(Time, c"m_flReturnTime", 1, 0),  // +0x88
        f(Time, c"m_flStopTime", 1, 0),    // +0x8c
        f(Float, c"m_moveDistance", 1, 0), // +0x90
        f(Float, c"m_targetSpeed", 1, 0),  // +0x94
        f(Float, c"m_initialSpeed", 1, 0), // +0x98
        f(Float, c"m_acceleration", 1, 0), // +0x9c
        f(Float, c"m_deceleration", 1, 0), // +0xa0
        f(Integer, c"m_state", 1, 0),      // +0xa4
    ],
};

/// `CTriggerChangeTarget::m_SaveData` (0x120780): written as `"CTriggerChangeTarget"`.
pub static CTriggerChangeTarget: Table = Table {
    name: c"CTriggerChangeTarget",
    fields: &[
        f(String, c"m_iszNewTarget", 1, 0), // +0x6c
    ],
};

/// `CTriggerRelay::m_SaveData` (0x1206c0): written as `"CTriggerRelay"`.
pub static CTriggerRelay: Table = Table {
    name: c"CTriggerRelay",
    fields: &[
        f(Integer, c"triggerType", 1, 0), // +0x6c
    ],
};

/// `CWallHealth::m_SaveData` (0x119ba0): written as `"CWallHealth"`.
pub static CWallHealth: Table = Table {
    name: c"CWallHealth",
    fields: &[
        f(Time, c"m_flNextCharge", 1, 0),   // +0x104
        f(Integer, c"m_iReactivate", 1, 0), // +0x108
        f(Integer, c"m_iJuice", 1, 0),      // +0x10c
        f(Integer, c"m_iOn", 1, 0),         // +0x110
        f(Time, c"m_flSoundTime", 1, 0),    // +0x114
    ],
};

/// `CWeaponBox::m_SaveData` (0x122e60): written as `"CWeaponBox"`.
pub static CWeaponBox: Table = Table {
    name: c"CWeaponBox",
    fields: &[
        f(Integer, c"m_rgAmmo", 32, 0),         // +0xfc
        f(String, c"m_rgiszAmmo", 32, 0),       // +0x7c
        f(ClassPtr, c"m_rgpPlayerItems", 6, 0), // +0x64
        f(Integer, c"m_cAmmoTypes", 1, 0),      // +0x17c
    ],
};

/// `CWreckage::m_SaveData` (0x119350): written as `"CWreckage"`.
pub static CWreckage: Table = Table {
    name: c"CWreckage",
    fields: &[
        f(Time, c"m_flStartTime", 1, 0), // +0x164
    ],
};

/// `CXenPLight::m_SaveData` (0x124150): written as `"CXenPLight"`.
pub static CXenPLight: Table = Table {
    name: c"CXenPLight",
    fields: &[
        f(ClassPtr, c"m_pGlow", 1, 0), // +0x84
    ],
};

/// `CXenTree::m_SaveData` (0x124160): written as `"CXenTree"`.
pub static CXenTree: Table = Table {
    name: c"CXenTree",
    fields: &[
        f(ClassPtr, c"m_pTrigger", 1, 0), // +0x84
    ],
};

/// Every TS class's `Save` chain (its vtable `Save`, then the parents that one calls first).
pub static CHAINS: &[(&str, &[&Table])] = &[
    ("CActAnimating", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CActAnimating]),
    ("CAmbientGeneric", &[&ENTVARS, &BASE, &CAmbientGeneric]),
    ("CAutoTrigger", &[&ENTVARS, &BASE, &CBaseDelay, &CAutoTrigger]),
    ("CBaseAnimating", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating]),
    ("CBaseButton", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBaseButton]),
    ("CBaseDMStart", &[&ENTVARS, &BASE]),
    ("CBaseDelay", &[&ENTVARS, &BASE, &CBaseDelay]),
    ("CBaseDoor", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBaseDoor]),
    ("CBaseEntity", &[&ENTVARS, &BASE]),
    ("CBaseMonster", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CBasePlatTrain", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBasePlatTrain]),
    ("CBasePlayer", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBasePlayer]),
    ("CBasePlayerAmmo", &[&ENTVARS, &BASE]),
    ("CBasePlayerItem", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBasePlayerItem]),
    ("CBasePlayerWeapon", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBasePlayerItem, &CBasePlayerWeapon]),
    ("CBaseSpectator", &[&ENTVARS, &BASE]),
    ("CBaseToggle", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CBaseTrigger", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CBeam", &[&ENTVARS, &BASE]),
    ("CBlood", &[&ENTVARS, &BASE]),
    ("CBloodSplat", &[&ENTVARS, &BASE]),
    ("CBreakable", &[&ENTVARS, &BASE, &CBreakable]),
    ("CBubbling", &[&ENTVARS, &BASE, &CBubbling]),
    ("CButtonTarget", &[&ENTVARS, &BASE]),
    ("CChangeLevel", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CChangeLevel]),
    ("CCorpse", &[&ENTVARS, &BASE]),
    ("CCycler", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CCycler]),
    ("CCyclerProbe", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CCycler]),
    ("CCyclerSprite", &[&ENTVARS, &BASE, &CCyclerSprite]),
    ("CDeadHEV", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CDecal", &[&ENTVARS, &BASE]),
    ("CEnvBeverage", &[&ENTVARS, &BASE, &CBaseDelay]),
    ("CEnvExplosion", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CEnvExplosion]),
    ("CEnvFunnel", &[&ENTVARS, &BASE, &CBaseDelay]),
    ("CEnvGlobal", &[&ENTVARS, &BASE, &CEnvGlobal]),
    ("CEnvLight", &[&ENTVARS, &BASE, &CLight]),
    ("CEnvShooter", &[&ENTVARS, &BASE, &CBaseDelay, &CGibShooter]),
    ("CEnvSound", &[&ENTVARS, &BASE, &CEnvSound]),
    ("CEnvSpark", &[&ENTVARS, &BASE, &CEnvSpark]),
    ("CFade", &[&ENTVARS, &BASE]),
    ("CFireAndDie", &[&ENTVARS, &BASE, &CBaseDelay]),
    ("CFrictionModifier", &[&ENTVARS, &BASE, &CFrictionModifier]),
    ("CFuncConveyor", &[&ENTVARS, &BASE]),
    ("CFuncIllusionary", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CFuncMonsterClip", &[&ENTVARS, &BASE]),
    ("CFuncPlat", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBasePlatTrain, &CFuncPlat]),
    ("CFuncPlatRot", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBasePlatTrain, &CFuncPlat, &CFuncPlatRot]),
    ("CFuncRotating", &[&ENTVARS, &BASE, &CFuncRotating]),
    ("CFuncTank", &[&ENTVARS, &BASE, &CFuncTank]),
    ("CFuncTankControls", &[&ENTVARS, &BASE, &CFuncTankControls]),
    ("CFuncTankGun", &[&ENTVARS, &BASE, &CFuncTank]),
    ("CFuncTankLaser", &[&ENTVARS, &BASE, &CFuncTank, &CFuncTankLaser]),
    ("CFuncTankMortar", &[&ENTVARS, &BASE, &CFuncTank]),
    ("CFuncTankRocket", &[&ENTVARS, &BASE, &CFuncTank]),
    (
        "CFuncTrackAuto",
        &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBasePlatTrain, &CFuncPlat, &CFuncPlatRot, &CFuncTrackChange],
    ),
    (
        "CFuncTrackChange",
        &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBasePlatTrain, &CFuncPlat, &CFuncPlatRot, &CFuncTrackChange],
    ),
    ("CFuncTrackTrain", &[&ENTVARS, &BASE, &CFuncTrackTrain]),
    ("CFuncTrain", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBasePlatTrain, &CFuncTrain]),
    ("CFuncTrainControls", &[&ENTVARS, &BASE]),
    ("CFuncWall", &[&ENTVARS, &BASE]),
    ("CFuncWallToggle", &[&ENTVARS, &BASE]),
    ("CGameCounter", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGameCounterSet", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGameEnd", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGamePlayerEquip", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGamePlayerHurt", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGamePlayerTeam", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGamePlayerZone", &[&ENTVARS, &BASE, &CRuleEntity, &CGamePlayerZone]),
    ("CGameScore", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGameTeamMaster", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGameTeamSet", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CGameText", &[&ENTVARS, &BASE, &CRuleEntity, &CGameText]),
    ("CGenericCycler", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CCycler]),
    ("CGib", &[&ENTVARS, &BASE]),
    ("CGibShooter", &[&ENTVARS, &BASE, &CBaseDelay, &CGibShooter]),
    ("CGlow", &[&ENTVARS, &BASE, &CGlow]),
    ("CGunTarget", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CGunTarget]),
    ("CHealthKit", &[&ENTVARS, &BASE]),
    ("CInfoBreakable", &[&ENTVARS, &BASE]),
    ("CInfoIntermission", &[&ENTVARS, &BASE]),
    ("CItem", &[&ENTVARS, &BASE]),
    ("CItemAntidote", &[&ENTVARS, &BASE]),
    ("CItemBattery", &[&ENTVARS, &BASE]),
    ("CItemSecurity", &[&ENTVARS, &BASE]),
    ("CItemSoda", &[&ENTVARS, &BASE]),
    ("CItemSuit", &[&ENTVARS, &BASE]),
    ("CLadder", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CLaser", &[&ENTVARS, &BASE, &CLaser]),
    ("CLight", &[&ENTVARS, &BASE, &CLight]),
    ("CLightning", &[&ENTVARS, &BASE, &CLightning]),
    ("CM61Grenade", &[&ENTVARS, &BASE]),
    ("CMessage", &[&ENTVARS, &BASE]),
    ("CMomentaryDoor", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CMomentaryDoor]),
    ("CMomentaryRotButton", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CMomentaryRotButton]),
    ("CMultiManager", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CMultiManager]),
    ("CMultiSource", &[&ENTVARS, &BASE, &CMultiSource]),
    ("CNullEntity", &[&ENTVARS, &BASE]),
    ("CPathCorner", &[&ENTVARS, &BASE, &CPathCorner]),
    ("CPathTrack", &[&ENTVARS, &BASE, &CPathTrack]),
    ("CPendulum", &[&ENTVARS, &BASE, &CPendulum]),
    ("CPlatTrigger", &[&ENTVARS, &BASE]),
    ("CPointEntity", &[&ENTVARS, &BASE]),
    ("CPushable", &[&ENTVARS, &BASE, &CBreakable, &CPushable]),
    ("CRecharge", &[&ENTVARS, &BASE, &CRecharge]),
    ("CRenderFxManager", &[&ENTVARS, &BASE]),
    ("CRevertSaved", &[&ENTVARS, &BASE, &CRevertSaved]),
    ("CRotButton", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBaseButton]),
    ("CRotDoor", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CBaseDoor]),
    ("CRuleBrushEntity", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CRuleEntity", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CRulePointEntity", &[&ENTVARS, &BASE, &CRuleEntity]),
    ("CShake", &[&ENTVARS, &BASE]),
    ("CShower", &[&ENTVARS, &BASE]),
    ("CSoundEnt", &[&ENTVARS, &BASE]),
    ("CSpeaker", &[&ENTVARS, &BASE, &CSpeaker]),
    ("CSprayCan", &[&ENTVARS, &BASE]),
    ("CSprite", &[&ENTVARS, &BASE, &CSprite]),
    ("CStripWeapons", &[&ENTVARS, &BASE]),
    ("CTSAmmoPack", &[&ENTVARS, &BASE]),
    ("CTSBigPistolAmmo", &[&ENTVARS, &BASE]),
    ("CTSBigPistolWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSBomb", &[&ENTVARS, &BASE, &CTSObjective, &CTSBomb]),
    ("CTSBriefcase", &[&ENTVARS, &BASE]),
    ("CTSCustomTarget", &[&ENTVARS, &BASE]),
    ("CTSDMHill", &[&ENTVARS, &BASE, &CTSDMHill]),
    ("CTSDeadCorpse", &[&ENTVARS, &BASE]),
    ("CTSEmitter", &[&ENTVARS, &BASE]),
    ("CTSFlag", &[&ENTVARS, &BASE]),
    ("CTSFog", &[&ENTVARS, &BASE]),
    ("CTSGrenade", &[&ENTVARS, &BASE]),
    ("CTSGrenadeWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSGroundAmmo", &[&ENTVARS, &BASE]),
    ("CTSGroundWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSGun", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBasePlayerItem, &CBasePlayerWeapon]),
    ("CTSHack", &[&ENTVARS, &BASE, &CTSObjective, &CTSHack]),
    ("CTSKatanaWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSKnife", &[&ENTVARS, &BASE]),
    ("CTSKnifeWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSMGWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSMapGlobals", &[&ENTVARS, &BASE, &CTSMapGlobals]),
    ("CTSModel", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTSObjective", &[&ENTVARS, &BASE, &CTSObjective]),
    ("CTSObjectiveManager", &[&ENTVARS, &BASE, &CTSObjectiveManager]),
    ("CTSObjectivePointer", &[&ENTVARS, &BASE]),
    ("CTSParticle", &[&ENTVARS, &BASE]),
    ("CTSPhysicObject", &[&ENTVARS, &BASE]),
    ("CTSPistolAmmo", &[&ENTVARS, &BASE]),
    ("CTSPistolWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSPlayerAmount", &[&ENTVARS, &BASE, &CBaseDelay]),
    ("CTSPowerUp", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CTSPowerUp]),
    ("CTSRifleAmmo", &[&ENTVARS, &BASE]),
    ("CTSRifleWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSSMGAmmo", &[&ENTVARS, &BASE]),
    ("CTSSMGWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSScoreGiver", &[&ENTVARS, &BASE, &CTSWinGiver]),
    ("CTSSellZone", &[&ENTVARS, &BASE]),
    ("CTSShotgunAmmo", &[&ENTVARS, &BASE]),
    ("CTSShotgunWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSShrapnel", &[&ENTVARS, &BASE]),
    ("CTSSlowMotion", &[&ENTVARS, &BASE, &CTSSlowMotion]),
    ("CTSSlowMotionPoint", &[&ENTVARS, &BASE]),
    ("CTSSniperAmmo", &[&ENTVARS, &BASE]),
    ("CTSSniperWeapon", &[&ENTVARS, &BASE, &CTSGroundWeapon]),
    ("CTSSuperJump", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CTSPowerUp]),
    ("CTSTeamEscape", &[&ENTVARS, &BASE, &CTSObjective, &CTSTeamEscape]),
    ("CTSTimer", &[&ENTVARS, &BASE]),
    ("CTSTrigger", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTSWinGiver", &[&ENTVARS, &BASE, &CTSWinGiver]),
    ("CTSWind", &[&ENTVARS, &BASE]),
    ("CTSWorldGun", &[&ENTVARS, &BASE]),
    ("CTargetCDAudio", &[&ENTVARS, &BASE]),
    ("CTestEffect", &[&ENTVARS, &BASE, &CBaseDelay]),
    ("CTriggerCDAudio", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerCamera", &[&ENTVARS, &BASE, &CBaseDelay, &CTriggerCamera]),
    ("CTriggerChangeTarget", &[&ENTVARS, &BASE, &CBaseDelay, &CTriggerChangeTarget]),
    ("CTriggerCounter", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerEndSection", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerGravity", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerHurt", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerMonsterJump", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerMultiple", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerOnce", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerPush", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerRelay", &[&ENTVARS, &BASE, &CBaseDelay, &CTriggerRelay]),
    ("CTriggerSave", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerTeleport", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle]),
    ("CTriggerVolume", &[&ENTVARS, &BASE]),
    ("CVipEscape", &[&ENTVARS, &BASE]),
    ("CWallHealth", &[&ENTVARS, &BASE, &CWallHealth]),
    ("CWeaponBox", &[&ENTVARS, &BASE, &CWeaponBox]),
    ("CWeaponCycler", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBasePlayerItem, &CBasePlayerWeapon]),
    ("CWorld", &[&ENTVARS, &BASE]),
    ("CWorldItem", &[&ENTVARS, &BASE]),
    ("CWreckage", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CBaseToggle, &CWreckage]),
    ("CXenHair", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CActAnimating]),
    ("CXenHull", &[&ENTVARS, &BASE]),
    ("CXenPLight", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CActAnimating, &CXenPLight]),
    ("CXenSpore", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CActAnimating]),
    ("CXenSporeLarge", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CActAnimating]),
    ("CXenSporeMed", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CActAnimating]),
    ("CXenSporeSmall", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CActAnimating]),
    ("CXenTree", &[&ENTVARS, &BASE, &CBaseDelay, &CBaseAnimating, &CActAnimating, &CXenTree]),
    ("CXenTreeTrigger", &[&ENTVARS, &BASE]),
];

/// The chain of a TS class ([`CHAINS`]).
pub fn chain(ts_class: &str) -> Option<&'static [&'static Table]> {
    CHAINS.iter().find(|(c, _)| *c == ts_class).map(|(_, t)| *t)
}

/// `LINK_ENTITY_TO_CLASS`: the TS class each classname builds.
pub fn class_of(classname: &[u8]) -> Option<&'static str> {
    Some(match classname {
        b"DelayedUse" => "CBaseDelay",
        b"PlatSpawnInsideTrigger" => "CFuncPlat",
        b"TSAmmoPack" => "CTSAmmoPack",
        b"TSWorldGun" => "CTSWorldGun",
        b"ambient_generic" => "CAmbientGeneric",
        b"ammo_9mmAR" => "CTSBigPistolAmmo",
        b"ammo_ARgrenades" => "CTSKnifeWeapon",
        b"ammo_buckshot" => "CTSShotgunAmmo",
        b"ammo_crossbow" => "CTSSniperAmmo",
        b"ammo_gaussclip" => "CTSRifleAmmo",
        b"ammo_rpgclip" => "CTSSMGAmmo",
        b"beam" => "CBeam",
        b"bodyque" => "CCorpse",
        b"button_target" => "CButtonTarget",
        b"cycler" => "CGenericCycler",
        b"cycler_prdroid" => "CCyclerProbe",
        b"cycler_sprite" => "CCyclerSprite",
        b"cycler_weapon" => "CWeaponCycler",
        b"cycler_wreckage" => "CWreckage",
        b"deadcorpse" => "CTSDeadCorpse",
        b"env_beam" => "CLightning",
        b"env_beverage" => "CEnvBeverage",
        b"env_blood" => "CBlood",
        b"env_bubbles" => "CBubbling",
        b"env_debris" => "CEnvSpark",
        b"env_explosion" => "CEnvExplosion",
        b"env_fade" => "CFade",
        b"env_funnel" => "CEnvFunnel",
        b"env_global" => "CEnvGlobal",
        b"env_glow" => "CGlow",
        b"env_laser" => "CLaser",
        b"env_lightning" => "CLightning",
        b"env_message" => "CMessage",
        b"env_render" => "CRenderFxManager",
        b"env_shake" => "CShake",
        b"env_shooter" => "CEnvShooter",
        b"env_sound" => "CEnvSound",
        b"env_spark" => "CEnvSpark",
        b"env_sprite" => "CSprite",
        b"fireanddie" => "CFireAndDie",
        b"func_breakable" => "CBreakable",
        b"func_button" => "CBaseButton",
        b"func_conveyor" => "CFuncConveyor",
        b"func_door" => "CBaseDoor",
        b"func_door_rotating" => "CRotDoor",
        b"func_friction" => "CFrictionModifier",
        b"func_guntarget" => "CGunTarget",
        b"func_healthcharger" => "CWallHealth",
        b"func_illusionary" => "CFuncIllusionary",
        b"func_ladder" => "CLadder",
        b"func_monsterclip" => "CFuncMonsterClip",
        b"func_pendulum" => "CPendulum",
        b"func_plat" => "CFuncPlat",
        b"func_platrot" => "CFuncPlatRot",
        b"func_pushable" => "CPushable",
        b"func_recharge" => "CRecharge",
        b"func_rot_button" => "CRotButton",
        b"func_rotating" => "CFuncRotating",
        b"func_tank" => "CFuncTankGun",
        b"func_tankcontrols" => "CFuncTankControls",
        b"func_tanklaser" => "CFuncTankLaser",
        b"func_tankmortar" => "CFuncTankMortar",
        b"func_tankrocket" => "CFuncTankRocket",
        b"func_trackautochange" => "CFuncTrackAuto",
        b"func_trackchange" => "CFuncTrackChange",
        b"func_tracktrain" => "CFuncTrackTrain",
        b"func_train" => "CFuncTrain",
        b"func_traincontrols" => "CFuncTrainControls",
        b"func_wall" => "CFuncWall",
        b"func_wall_toggle" => "CFuncWallToggle",
        b"func_water" => "CBaseDoor",
        b"game_counter" => "CGameCounter",
        b"game_counter_set" => "CGameCounterSet",
        b"game_end" => "CGameEnd",
        b"game_player_equip" => "CGamePlayerEquip",
        b"game_player_hurt" => "CGamePlayerHurt",
        b"game_player_team" => "CGamePlayerTeam",
        b"game_score" => "CGameScore",
        b"game_team_master" => "CGameTeamMaster",
        b"game_team_set" => "CGameTeamSet",
        b"game_text" => "CGameText",
        b"game_zone_player" => "CGamePlayerZone",
        b"gibshooter" => "CGibShooter",
        b"info_breakable" => "CInfoBreakable",
        b"info_intermission" => "CInfoIntermission",
        b"info_landmark" => "CPointEntity",
        b"info_null" => "CNullEntity",
        b"info_player_deathmatch" => "CBaseDMStart",
        b"info_player_start" => "CBaseDMStart",
        b"info_player_team1" => "CBaseDMStart",
        b"info_player_team2" => "CBaseDMStart",
        b"info_player_team3" => "CBaseDMStart",
        b"info_player_team4" => "CBaseDMStart",
        b"info_target" => "CPointEntity",
        b"info_teleport_destination" => "CPointEntity",
        b"infodecal" => "CDecal",
        b"item_antidote" => "CItemAntidote",
        b"item_battery" => "CItemBattery",
        b"item_healthkit" => "CHealthKit",
        b"item_longjump" => "CTSSuperJump",
        b"item_security" => "CItemSecurity",
        b"item_sodacan" => "CItemSoda",
        b"item_suit" => "CItemSuit",
        b"knife" => "CTSKnife",
        b"light" => "CLight",
        b"light_environment" => "CEnvLight",
        b"light_spot" => "CLight",
        b"m61_grenade" => "CM61Grenade",
        b"momentary_door" => "CMomentaryDoor",
        b"momentary_rot_button" => "CMomentaryRotButton",
        b"monster_hevsuit_dead" => "CDeadHEV",
        b"multi_manager" => "CMultiManager",
        b"multisource" => "CMultiSource",
        b"path_corner" => "CPathCorner",
        b"path_track" => "CPathTrack",
        b"player" => "CBasePlayer",
        b"player_loadsaved" => "CRevertSaved",
        b"player_weaponstrip" => "CStripWeapons",
        b"respawn" => "CBasePlayer",
        b"shrapnel" => "CTSShrapnel",
        b"soundent" => "CSoundEnt",
        b"spark_shower" => "CShower",
        b"speaker" => "CSpeaker",
        b"target_cdaudio" => "CTargetCDAudio",
        b"test_effect" => "CTestEffect",
        b"trigger" => "CBaseTrigger",
        b"trigger_auto" => "CAutoTrigger",
        b"trigger_autosave" => "CTriggerSave",
        b"trigger_camera" => "CTriggerCamera",
        b"trigger_cdaudio" => "CTriggerCDAudio",
        b"trigger_changelevel" => "CChangeLevel",
        b"trigger_changetarget" => "CTriggerChangeTarget",
        b"trigger_counter" => "CTriggerCounter",
        b"trigger_endsection" => "CTriggerEndSection",
        b"trigger_gravity" => "CTriggerGravity",
        b"trigger_hurt" => "CTriggerHurt",
        b"trigger_monsterjump" => "CTriggerMonsterJump",
        b"trigger_multiple" => "CTriggerMultiple",
        b"trigger_once" => "CTriggerOnce",
        b"trigger_push" => "CTriggerPush",
        b"trigger_relay" => "CTriggerRelay",
        b"trigger_teleport" => "CTriggerTeleport",
        b"trigger_transition" => "CTriggerVolume",
        b"ts_bomb" => "CTSBomb",
        b"ts_briefcase" => "CTSBriefcase",
        b"ts_customtarget" => "CTSCustomTarget",
        b"ts_dmhill" => "CTSDMHill",
        b"ts_emitter" => "CTSEmitter",
        b"ts_flag" => "CTSFlag",
        b"ts_fog" => "CTSFog",
        b"ts_groundammo" => "CTSGroundAmmo",
        b"ts_groundweapon" => "CTSGroundWeapon",
        b"ts_hack" => "CTSHack",
        b"ts_mapglobals" => "CTSMapGlobals",
        b"ts_model" => "CTSModel",
        b"ts_objective_manager" => "CTSObjectiveManager",
        b"ts_objective_ptr" => "CTSObjectivePointer",
        b"ts_particle" => "CTSParticle",
        b"ts_playeramount" => "CTSPlayerAmount",
        b"ts_powerup" => "CTSPowerUp",
        b"ts_scoregiver" => "CTSScoreGiver",
        b"ts_sellzone" => "CTSSellZone",
        b"ts_slowmotion" => "CTSSlowMotion",
        b"ts_slowmotionpoint" => "CTSSlowMotionPoint",
        b"ts_teamescape" => "CTSTeamEscape",
        b"ts_timer" => "CTSTimer",
        b"ts_trigger" => "CTSTrigger",
        b"ts_wind" => "CTSWind",
        b"ts_wingiver" => "CTSWinGiver",
        b"weapon_9MMAR" => "CTSBigPistolWeapon",
        b"weapon_crossbow" => "CTSSniperWeapon",
        b"weapon_egon" => "CTSRifleWeapon",
        b"weapon_gauss" => "CTSRifleWeapon",
        b"weapon_handgrenade" => "CTSGrenadeWeapon",
        b"weapon_hornetgun" => "CTSMGWeapon",
        b"weapon_rpg" => "CTSSMGWeapon",
        b"weapon_satchel" => "CTSKnifeWeapon",
        b"weapon_shotgun" => "CTSShotgunWeapon",
        b"weapon_snark" => "CTSKatanaWeapon",
        b"weapon_tsgun" => "CTSGun",
        b"weaponbox" => "CWeaponBox",
        b"world_items" => "CWorldItem",
        b"worldspawn" => "CWorld",
        b"xen_hair" => "CXenHair",
        b"xen_hull" => "CXenHull",
        b"xen_plantlight" => "CXenPLight",
        b"xen_spore_large" => "CXenSporeLarge",
        b"xen_spore_medium" => "CXenSporeMed",
        b"xen_spore_small" => "CXenSporeSmall",
        b"xen_tree" => "CXenTree",
        b"xen_ttrigger" => "CXenTreeTrigger",
        _ => return None,
    })
}
