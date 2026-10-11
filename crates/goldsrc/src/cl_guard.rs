//! Guards for client engine calls that stop or crash the client engine (port rule: the game must
//! never crash, hang or stop, even where TS does). The server half is [`crate::guard`].
//!
//! Steam GoldSrc (checked against the HL25 `hw.so`, which ships with symbols) and FWGS stop the
//! client (`Sys_Error`/`Host_Error`) or read out of bounds on a few inputs the client could pass.
//! The wrappers in `client.rs` and `ext/` check those first and skip the call (or return what the
//! engine answers for a bad argument), logging each kind of refusal to the console once with
//! "(port guard)". Valid input reaches the engine unchanged, so TS behaviour is kept. This module
//! holds the pure checks and the bookkeeping, unit-tested.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Steam `SPR_Init`: the HUD sprite table has 256 slots, and `SPR_Load` `Sys_Error`s ("cannot
/// allocate more than 256 HUD sprites") on a new name when they are all taken.
pub const MAX_HUD_SPRITES: usize = 256;
/// Steam `S_FindName`: a sound name longer than 63 bytes is a `Sys_Error` ("Sound name too
/// long"); FWGS's `sfx_t::name[64]` refuses the same names quietly.
pub const MAX_SOUND_NAME: usize = 63;
/// The Steam client's `cl.event_precache[MAX_EVENTS]`: `CL_FireEvents` indexes it with a queued
/// event's index unchecked (FWGS refuses an index outside 1..MAX_EVENTS itself).
pub const MAX_EVENTS: u16 = 256;
/// FWGS `MAX_USER_MESSAGES`: `pfnHookUserMsg` `Host_Error`s on a new name past it (the Steam
/// client keeps a list).
pub const XASH_MAX_USER_MESSAGES: usize = 197;

/// True the first time `key` is seen this process: refusals are logged once, not every frame.
pub fn once(key: &str) -> bool {
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashSet::new).insert(key.to_owned())
}

/// The Steam client's HUD sprite slots as `SPR_Load` fills them: handle `i + 1` is slot `i`, a
/// new name takes the first free slot (one whose load failed is free again), and the table lives
/// until the client disconnects. The port can't see a disconnect's reset, so names whose handle
/// a later load reuses are dropped; the table only fills up past what the game ever loads.
#[derive(Default)]
pub struct HudSprites {
    /// Lower-cased name → handle (`SPR_Load` compares with `Q_stricmp`).
    loaded: HashMap<Vec<u8>, i32>,
}

impl HudSprites {
    /// Whether `SPR_Load(name)` is safe: a name already in the table, or a free slot left.
    pub fn may_load(&self, name: &[u8]) -> bool {
        self.loaded.contains_key(&name.to_ascii_lowercase()) || self.loaded.len() < MAX_HUD_SPRITES
    }

    /// Records the handle `SPR_Load(name)` returned (0: not loaded, the slot stays free).
    pub fn record(&mut self, name: &[u8], handle: i32) {
        let key = name.to_ascii_lowercase();
        self.loaded.retain(|k, h| *h != handle && *k != key);
        if handle > 0 {
            self.loaded.insert(key, handle);
        }
    }
}

/// Steam `R_LoadMapSprite` (`LoadMapSprite`) `Sys_Error`s ("Unsupported map image file format")
/// unless the lower-cased name ends in `bmp` or `tga`.
pub fn map_sprite_name_ok(name: &[u8]) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(b"bmp") || lower.ends_with(b"tga")
}

/// `sizeof(mstudiobodyparts_t)`: `name[64]`, `nummodels`, `base`, `modelindex`.
const BODYPART_SIZE: usize = 76;
/// `sizeof(mstudiomodel_t)`.
const STUDIOMODEL_SIZE: usize = 112;

/// Whether `StudioSetupModel(bodypart)` is safe for the studio header `hdr` and the current
/// entity's `body`. Both engines' `R_StudioSetupModel` only replace a body part above
/// `numbodyparts` (so `numbodyparts` itself, or a negative one, is read out of bounds), then
/// compute the submodel as `body / base % nummodels` in C ints: a zero `base` or `nummodels`
/// (a malformed model) is an integer division by zero, and a negative result indexes before the
/// submodel array.
pub fn bodypart_ok(hdr: &[u8], bodypart: i32, body: i32) -> bool {
    let rd = |o: usize| hdr.get(o..o + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (Some(numbodyparts), Some(bodypartindex)) = (rd(204), rd(208)) else { return false };
    if bodypart < 0 || bodypart >= numbodyparts || bodypartindex < 0 {
        return false;
    }
    let bp = bodypartindex as usize + bodypart as usize * BODYPART_SIZE;
    let (Some(nummodels), Some(base), Some(modelindex)) = (rd(bp + 64), rd(bp + 68), rd(bp + 72)) else { return false };
    if nummodels <= 0 || base <= 0 || body < 0 || modelindex < 0 {
        return false;
    }
    (modelindex as usize).checked_add(nummodels as usize * STUDIOMODEL_SIZE).is_some_and(|end| end <= hdr.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_per_key() {
        assert!(once("cl_guard test a"));
        assert!(!once("cl_guard test a"));
        assert!(once("cl_guard test b"));
    }

    #[test]
    fn hud_sprite_slots() {
        let mut t = HudSprites::default();
        for i in 0..MAX_HUD_SPRITES {
            let n = format!("sprites/s{i}.spr");
            assert!(t.may_load(n.as_bytes()));
            t.record(n.as_bytes(), i as i32 + 1);
        }
        // Full: known names (any case) still load, new ones don't.
        assert!(t.may_load(b"SPRITES/S3.SPR"));
        assert!(!t.may_load(b"sprites/new.spr"));
        // A failed load leaves its slot free; a reused handle replaces the old name.
        let mut u = HudSprites::default();
        u.record(b"a.spr", 1);
        u.record(b"missing.spr", 0);
        assert_eq!(u.loaded.len(), 1);
        u.record(b"b.spr", 1);
        assert!(u.loaded.len() == 1 && u.loaded.contains_key(b"b.spr".as_slice()));
    }

    #[test]
    fn map_sprite_names() {
        assert!(map_sprite_name_ok(b"overviews/ts_dojo.bmp"));
        assert!(map_sprite_name_ok(b"overviews/ts_dojo.TGA"));
        assert!(!map_sprite_name_ok(b"overviews/ts_dojo.png"));
        assert!(!map_sprite_name_ok(b""));
    }

    fn header(bodyparts: &[(i32, i32)]) -> Vec<u8> {
        // Header, then the body parts, then each part's submodels.
        let mut h = vec![0u8; 244];
        h[204..208].copy_from_slice(&(bodyparts.len() as i32).to_le_bytes());
        h[208..212].copy_from_slice(&244i32.to_le_bytes());
        let mut models = 244 + bodyparts.len() * BODYPART_SIZE;
        let mut parts = Vec::new();
        for &(nummodels, base) in bodyparts {
            let mut p = vec![0u8; BODYPART_SIZE];
            p[64..68].copy_from_slice(&nummodels.to_le_bytes());
            p[68..72].copy_from_slice(&base.to_le_bytes());
            p[72..76].copy_from_slice(&(models as i32).to_le_bytes());
            models += nummodels.max(0) as usize * STUDIOMODEL_SIZE;
            parts.extend(p);
        }
        h.extend(parts);
        h.resize(models, 0);
        h
    }

    #[test]
    fn studio_body_parts() {
        let h = header(&[(2, 1), (3, 2)]);
        assert!(bodypart_ok(&h, 0, 0));
        assert!(bodypart_ok(&h, 1, 5));
        assert!(!bodypart_ok(&h, 2, 0), "numbodyparts itself is past the array");
        assert!(!bodypart_ok(&h, -1, 0));
        assert!(!bodypart_ok(&h, 0, -1), "a negative body indexes before the submodels");
        assert!(!bodypart_ok(&header(&[(0, 1)]), 0, 0), "nummodels 0 divides by zero");
        assert!(!bodypart_ok(&header(&[(1, 0)]), 0, 0), "base 0 divides by zero");
        assert!(!bodypart_ok(&h[..300], 1, 0), "a truncated model");
    }
}
