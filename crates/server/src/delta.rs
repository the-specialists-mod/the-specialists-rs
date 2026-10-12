//! The `delta.lst` conditional encoders TS registers in `RegisterEncoders` (`0xb004c`):
//! `Entity_Encode` (`0xafaa4`), `Player_Encode` (`0xafcbc`) and `Custom_Encode` (`0xafec4`),
//! the SDK's `delta.cpp` unchanged. They drop fields the client can work out itself (the local
//! player's origin, a followed entity's origin, a beam's unused fields) from the entity deltas.
//!
//! Each looks its field indices up once (`*_FieldInit`), on its first call, into statics that
//! live for the process. `Player_Encode` keeps the SDK bug of unsetting by the *entity*
//! description's origin indices (`entity_field_alias`), which are 0 until `Entity_Encode` has run
//! once; its own `player_field_alias` lookup is never used.

use goldsrc::Global;
use goldsrc::server::{Delta, engine};
use goldsrc::sys::{MOVETYPE_FOLLOW, entity_state_t};
use std::cell::Cell;
use std::ffi::CStr;

/// `entity_field_alias` (`0x116360`): origin[0..2], angles[0..2].
const ENTITY_FIELDS: [&CStr; 6] = [c"origin[0]", c"origin[1]", c"origin[2]", c"angles[0]", c"angles[1]", c"angles[2]"];
/// `player_field_alias` (`0x116440`): origin[0..2].
const PLAYER_FIELDS: [&CStr; 3] = [c"origin[0]", c"origin[1]", c"origin[2]"];
/// `custom_entity_field_alias` (`0x1164c0`).
const CUSTOM_FIELDS: [&CStr; 9] =
    [c"origin[0]", c"origin[1]", c"origin[2]", c"angles[0]", c"angles[1]", c"angles[2]", c"skin", c"sequence", c"animtime"];

/// Field indices (the aliases' `field` members, 0 until looked up) and the `initialized` flags.
static ENTITY: Global<Cell<Option<[i32; 6]>>> = Global(Cell::new(None));
static PLAYER: Global<Cell<Option<[i32; 3]>>> = Global(Cell::new(None));
static CUSTOM: Global<Cell<Option<[i32; 9]>>> = Global(Cell::new(None));

/// One `DeltaSetFieldByIndex` (true) or `DeltaUnsetFieldByIndex` (false).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldOp(pub bool, pub i32);

fn lookup<const N: usize>(d: Delta, names: [&CStr; N]) -> [i32; N] {
    names.map(|n| d.find_field(n))
}

/// Guard: a field `delta.lst` lacks is index -1, which the engines would index out of bounds
/// with (TS doesn't check; the shipped `delta.lst` has every field), so it is skipped.
fn apply(d: Delta, ops: &[FieldOp]) {
    for &FieldOp(set, i) in ops.iter().filter(|op| op.1 >= 0) {
        if set { d.set_field(i) } else { d.unset_field(i) }
    }
}

/// The followed-entity rule both `Entity_Encode` and `Player_Encode` end with: a `MOVETYPE_FOLLOW`
/// entity with an aiment doesn't send its origin; otherwise a changed aiment forces it.
fn follow_ops(origin: &[i32], from: &entity_state_t, to: &entity_state_t, ops: &mut Vec<FieldOp>) {
    if to.movetype == MOVETYPE_FOLLOW as i32 && to.aiment != 0 {
        ops.extend(origin.iter().map(|&i| FieldOp(false, i)));
    } else if to.aiment != from.aiment {
        ops.extend(origin.iter().map(|&i| FieldOp(true, i)));
    }
}

/// `Entity_Encode`'s operations with the entity field indices `f`, for the client in slot
/// `current`: the local player's origin isn't sent; nor are the origin and angles of an entity
/// with both `impacttime` and `starttime` set; then [`follow_ops`].
pub fn entity_ops(f: [i32; 6], current: i32, from: &entity_state_t, to: &entity_state_t) -> Vec<FieldOp> {
    let mut ops = Vec::new();
    if to.number - 1 == current {
        ops.extend(f[..3].iter().map(|&i| FieldOp(false, i)));
    }
    if to.impacttime != 0.0 && to.starttime != 0.0 {
        ops.extend(f.iter().map(|&i| FieldOp(false, i)));
    }
    follow_ops(&f[..3], from, to, &mut ops);
    ops
}

/// `Player_Encode`'s operations: as [`entity_ops`] without the impact test, on `f`, which is the
/// *entity* description's indices (the SDK bug, see the module docs).
pub fn player_ops(f: [i32; 6], current: i32, from: &entity_state_t, to: &entity_state_t) -> Vec<FieldOp> {
    let mut ops = Vec::new();
    if to.number - 1 == current {
        ops.extend(f[..3].iter().map(|&i| FieldOp(false, i)));
    }
    follow_ops(&f[..3], from, to, &mut ops);
    ops
}

/// `Custom_Encode`'s operations (beams), by beam type `rendermode & 0xf`: types other than
/// points/entpoint (0, 1) don't send the origin, any type but points not the angles, types other
/// than entpoint/ents (1, 2) not skin and sequence; an unchanged whole-second `animtime`
/// (truncated) isn't sent.
pub fn custom_ops(f: [i32; 9], from: &entity_state_t, to: &entity_state_t) -> Vec<FieldOp> {
    let mut ops = Vec::new();
    let beam = (to.rendermode & 0xf) as u32;
    if beam > 1 {
        ops.extend(f[..3].iter().map(|&i| FieldOp(false, i)));
    }
    if beam != 0 {
        ops.extend(f[3..6].iter().map(|&i| FieldOp(false, i)));
    }
    if beam.wrapping_sub(1) > 1 {
        ops.extend(f[6..8].iter().map(|&i| FieldOp(false, i)));
    }
    if from.animtime as i32 == to.animtime as i32 {
        ops.push(FieldOp(false, f[8]));
    }
    ops
}

/// `Entity_Encode` (`0xafaa4`).
pub fn entity_encode(d: Delta, from: &entity_state_t, to: &entity_state_t) {
    let f = ENTITY.0.get().unwrap_or_else(|| {
        let f = lookup(d, ENTITY_FIELDS);
        ENTITY.0.set(Some(f));
        f
    });
    apply(d, &entity_ops(f, engine().current_player(), from, to));
}

/// `Player_Encode` (`0xafcbc`).
pub fn player_encode(d: Delta, from: &entity_state_t, to: &entity_state_t) {
    if PLAYER.0.get().is_none() {
        PLAYER.0.set(Some(lookup(d, PLAYER_FIELDS)));
    }
    let f = ENTITY.0.get().unwrap_or([0; 6]);
    apply(d, &player_ops(f, engine().current_player(), from, to));
}

/// `Custom_Encode` (`0xafec4`).
pub fn custom_encode(d: Delta, from: &entity_state_t, to: &entity_state_t) {
    let f = CUSTOM.0.get().unwrap_or_else(|| {
        let f = lookup(d, CUSTOM_FIELDS);
        CUSTOM.0.set(Some(f));
        f
    });
    apply(d, &custom_ops(f, from, to));
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: [i32; 6] = [10, 11, 12, 13, 14, 15];
    const ORIGIN_UNSET: [FieldOp; 3] = [FieldOp(false, 10), FieldOp(false, 11), FieldOp(false, 12)];

    fn st(f: impl FnOnce(&mut entity_state_t)) -> entity_state_t {
        let mut s = entity_state_t::default();
        f(&mut s);
        s
    }

    #[test]
    fn entity_encode_rules() {
        let z = entity_state_t::default();
        // The receiving client's own entity (number = slot + 1).
        assert_eq!(entity_ops(E, 2, &z, &st(|s| s.number = 3)), ORIGIN_UNSET);
        assert_eq!(entity_ops(E, 2, &z, &st(|s| s.number = 4)), []);
        // Both impact times set: origin and angles.
        let t = st(|s| (s.impacttime, s.starttime, s.number) = (1.0, 2.0, 9));
        assert_eq!(entity_ops(E, 0, &t, &t), E.map(|i| FieldOp(false, i)));
        // Following an entity: no origin; a new aiment otherwise forces it.
        let follow = st(|s| (s.movetype, s.aiment, s.number) = (MOVETYPE_FOLLOW as i32, 5, 9));
        assert_eq!(entity_ops(E, 0, &z, &follow), ORIGIN_UNSET);
        let moved = st(|s| (s.aiment, s.number) = (5, 9));
        assert_eq!(entity_ops(E, 0, &z, &moved), [FieldOp(true, 10), FieldOp(true, 11), FieldOp(true, 12)]);
        // The player encoder has no impact test.
        assert_eq!(player_ops(E, 0, &t, &t), []);
        assert_eq!(player_ops(E, 2, &z, &st(|s| s.number = 3)), ORIGIN_UNSET);
    }

    #[test]
    fn custom_encode_rules() {
        let f = [0, 1, 2, 3, 4, 5, 6, 7, 8];
        let beam = |mode: i32, a0: f32, a1: f32| custom_ops(f, &st(|s| s.animtime = a0), &st(|s| (s.rendermode, s.animtime) = (mode, a1)));
        let unset = |v: &[i32]| v.iter().map(|&i| FieldOp(false, i)).collect::<Vec<_>>();
        // Points (0): only skin/sequence, and animtime when its whole second is unchanged.
        assert_eq!(beam(0, 1.2, 1.9), unset(&[6, 7, 8]));
        assert_eq!(beam(0, 1.2, 2.1), unset(&[6, 7]));
        // Entpoint (1): angles only; ents (2): origin and angles; others everything.
        assert_eq!(beam(1, 0.0, 5.0), unset(&[3, 4, 5]));
        assert_eq!(beam(2, 0.0, 5.0), unset(&[0, 1, 2, 3, 4, 5]));
        assert_eq!(beam(0x13, 0.0, 5.0), unset(&[0, 1, 2, 3, 4, 5, 6, 7]));
    }
}
