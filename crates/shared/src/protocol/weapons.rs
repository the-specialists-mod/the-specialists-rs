//! Message payloads for the weapons area (layouts: decomp server/architecture.md §3.2,
//! client/architecture-hud.md §3.3, weapons.md §6.2) and the TS packing of the gun into
//! `clientdata_t` / `weapon_data_t` (server/architecture.md §7.2-7.3).
//!
//! Every payload has `write` (server side, through a [`MsgSink`]) and `read` (client side, from
//! the raw message bytes). Byte layout = the `WRITE_*` calls of TS 3.0.

use crate::weapons::{Gun, MAX_WEAPONS, info, kind, kind_of};
use goldsrc_sys::{clientdata_t, weapon_data_t};
use std::ffi::CStr;

/// Destination for `WRITE_*` calls. `Vec<u8>` implements it with the engine's little-endian
/// encoding (for tests and demos); the server implements it over `MsgWriter`.
pub trait MsgSink {
    fn byte(&mut self, v: i32);
    fn char(&mut self, v: i32);
    fn short(&mut self, v: i32);
    fn string(&mut self, s: &CStr);
}

impl MsgSink for Vec<u8> {
    fn byte(&mut self, v: i32) {
        self.push(v as u8);
    }
    fn char(&mut self, v: i32) {
        self.push(v as i8 as u8);
    }
    fn short(&mut self, v: i32) {
        self.extend_from_slice(&(v as i16).to_le_bytes());
    }
    fn string(&mut self, s: &CStr) {
        self.extend_from_slice(s.to_bytes_with_nul());
    }
}

/// Minimal little-endian reader over a message body.
pub struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    pub fn byte(&mut self) -> Option<u8> {
        let v = *self.b.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }
    pub fn char(&mut self) -> Option<i8> {
        self.byte().map(|v| v as i8)
    }
    pub fn short(&mut self) -> Option<i16> {
        let s = self.b.get(self.pos..self.pos + 2)?;
        self.pos += 2;
        Some(i16::from_le_bytes([s[0], s[1]]))
    }
    pub fn string(&mut self) -> Option<&'a str> {
        let rest = self.b.get(self.pos..)?;
        let n = rest.iter().position(|&c| c == 0)?;
        self.pos += n + 1;
        std::str::from_utf8(&rest[..n]).ok()
    }
}

/// `WeaponInfo` (6): the active weapon. With kung fu: id 0, the kung-fu bar, zeros.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponInfo {
    pub id: u8,
    pub clip: u8,
    pub reserve: i16,
    pub mode: u8,
    pub items: u8,
}

impl WeaponInfo {
    pub fn write(&self, w: &mut impl MsgSink) {
        w.byte(self.id as i32);
        w.byte(self.clip as i32);
        w.short(self.reserve as i32);
        w.byte(self.mode as i32);
        w.byte(self.items as i32);
    }
    pub fn read(b: &[u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        Some(WeaponInfo { id: r.byte()?, clip: r.byte()?, reserve: r.short()?, mode: r.byte()?, items: r.byte()? })
    }
}

/// `WStatus` (3): one inventory record (sent when its owned flag changes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WStatus {
    pub id: u8,
    pub owned: u8,
    pub clip: u8,
}

impl WStatus {
    pub fn write(&self, w: &mut impl MsgSink) {
        w.byte(self.id as i32);
        w.byte(self.owned as i32);
        w.byte(self.clip as i32);
    }
    pub fn read(b: &[u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        Some(WStatus { id: r.byte()?, owned: r.byte()?, clip: r.byte()? })
    }
}

/// `ActItems` (2, ALL): attachments switched on (laser 2, flashlight 4) | 0x40 akimbo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActItems {
    pub entindex: u8,
    pub items: u8,
}

impl ActItems {
    pub fn write(&self, w: &mut impl MsgSink) {
        w.byte(self.entindex as i32);
        w.byte(self.items as i32);
    }
    pub fn read(b: &[u8]) -> Option<Self> {
        let mut r = Reader::new(b);
        Some(ActItems { entindex: r.byte()?, items: r.byte()? })
    }
}

/// `KFuPower` (1): kung-fu power as a signed char, `round(bar)`.
pub fn write_kfu_power(w: &mut impl MsgSink, bar: f32) {
    w.char(bar.round_ties_even() as i32);
}
pub fn read_kfu_power(b: &[u8]) -> Option<i8> {
    Reader::new(b).char()
}

/// `TSSpace` (1): free inventory slots.
pub fn write_ts_space(w: &mut impl MsgSink, free: i32) {
    w.byte(free);
}

/// `CurWeapon` (3): `B state, B id, B clip`. TS only sends the 0,0,0 reset.
pub fn write_cur_weapon_reset(w: &mut impl MsgSink) {
    w.byte(0);
    w.byte(0);
    w.byte(0);
}

/// `PlInfo` (3, ALL): `B entindex, B key, B value`; key 0 = laser colour (`lasclr_<n>`).
pub fn write_pl_info(w: &mut impl MsgSink, entindex: i32, key: i32, value: i32) {
    w.byte(entindex);
    w.byte(key);
    w.byte(value);
}

/// `CustomWP` (-1): `"%i %f %f %f %f %i"` = id, accuracy, view kick, accuracy kick, damage,
/// cost (`SendWeaponParams` 0x9eb98).
pub fn custom_wp_string(id: u8, accuracy: f32, view_kick: f32, acc_kick: f32, damage: f32, cost: i32) -> String {
    format!("{} {:.6} {:.6} {:.6} {:.6} {}", id, accuracy, view_kick, acc_kick, damage, cost)
}

/// Parses a `CustomWP` string into (id, accuracy, view kick, accuracy kick, damage, cost).
pub fn parse_custom_wp(s: &str) -> Option<(u8, f32, f32, f32, f32, i32)> {
    let mut it = s.split_whitespace();
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    ))
}

/// HL weapon id the TS gun reports to the engine (`m_iId`, slot of `weapon_data_t`).
pub const TSGUN_ID: i32 = 16;

/// Ammo count of the paired single/akimbo record (`clientdata.ammo_shells`).
fn twin_clip(gun: &Gun) -> i32 {
    let Some(w) = info(gun.current) else { return 0 };
    if w.akimbo != -1 {
        gun.status[w.akimbo as usize].clip
    } else if w.single != -1 && w.single as u8 != gun.current {
        gun.status[w.single as usize].clip
    } else {
        0
    }
}

/// TS `UpdateClientData` 0xb0294 weapon fields (`sendweapons` set, active item = TS gun).
/// `powerups` is `pl+0x724` (`vuser1[1]`).
pub fn pack_clientdata(gun: &Gun, powerups: u16, cd: &mut clientdata_t) {
    cd.m_flNextAttack = gun.next_attack;
    cd.vuser1[1] = powerups as f32;
    cd.ammo_shells = 0;
    cd.ammo_rockets = 0;
    cd.ammo_cells = 0;
    cd.m_iId = TSGUN_ID;
    cd.vuser3[0] = gun.current as f32;
    let Some(s) = gun.slot() else {
        cd.vuser3[1] = gun.kick_combo as f32;
        cd.vuser3[2] = gun.punch_combo as f32;
        return;
    };
    cd.vuser3[1] = s.mode as f32;
    cd.vuser3[2] = s.items as f32;
    cd.vuser4[0] = s.toggle as i32 as f32;
    cd.vuser4[1] = s.clip as f32;
    cd.ammo_nails = s.left;
    cd.ammo_shells = twin_clip(gun);
    cd.vuser4[2] = s.in_reload as f32;
    cd.vuser2[0] = s.state as f32;
    cd.vuser2[1] = gun.current_reserve() as f32;
    cd.vuser2[2] = s.alt_grip as i32 as f32;
    cd.vuser1[2] = s.active as f32;
}

/// Client side of [`pack_clientdata`]: rebuilds the current record (prediction). Returns the
/// power-up bits.
pub fn unpack_clientdata(cd: &clientdata_t, gun: &mut Gun) -> u16 {
    gun.next_attack = cd.m_flNextAttack;
    let id = (cd.vuser3[0] as i32).clamp(0, MAX_WEAPONS as i32 - 1) as u8;
    gun.current = id;
    if id == 0 {
        gun.kick_combo = cd.vuser3[1] as i32;
        gun.punch_combo = cd.vuser3[2] as i32;
    } else {
        let reserve = cd.vuser2[1] as i32;
        if let Some(w) = info(id) {
            gun.reserve[(w.ammo as usize).min(15)] = reserve;
            if w.akimbo != -1 {
                gun.status[w.akimbo as usize].clip = cd.ammo_shells;
            } else if w.single != -1 && w.single as u8 != id {
                gun.status[w.single as usize].clip = cd.ammo_shells;
            }
        }
        let s = &mut gun.status[id as usize];
        s.owned = true;
        s.mode = cd.vuser3[1] as u8;
        s.items = cd.vuser3[2] as u32;
        s.toggle = cd.vuser4[0] != 0.0;
        s.clip = cd.vuser4[1] as i32;
        s.left = cd.ammo_nails;
        // `clip - left`, 0 when below 1 (client `0x10003ae1..0x10003aed`: `setle`/`dec`/`and`).
        // Only the client unpacks; the server keeps its own records.
        // Wrapping as TS's int subtraction: both come from the server's delta data.
        s.right = s.clip.wrapping_sub(s.left).max(0);
        s.in_reload = cd.vuser4[2] as i32;
        s.state = cd.vuser2[0] as i32;
        s.alt_grip = cd.vuser2[2] != 0.0;
        s.active = cd.vuser1[2] as u32;
        if kind_of(id) == kind::KUNGFU {
            s.owned = false;
        }
    }
    cd.vuser1[1] as u16
}

/// TS `GetWeaponData` 0xb00ac: the single entry (slot 16) for the TS gun.
pub fn pack_weapon_data(gun: &Gun, wd: &mut weapon_data_t) {
    wd.m_iId = TSGUN_ID;
    wd.m_iClip = gun.clip_mirror;
    wd.m_flTimeWeaponIdle = gun.idle.max(-0.001);
    wd.m_flNextPrimaryAttack = gun.next_primary.max(-0.001);
    wd.m_flNextSecondaryAttack = gun.next_secondary.max(-0.001);
    wd.m_fInReload = 0;
    wd.m_fInSpecialReload = 0;
    wd.fuser1 = gun.fuser1.max(-0.001);
    wd.fuser2 = 0.0;
    wd.fuser3 = 0.0;
    wd.iuser1 = 0;
    wd.iuser2 = 0;
    wd.iuser3 = 0;
}

/// Client side of [`pack_weapon_data`].
pub fn unpack_weapon_data(wd: &weapon_data_t, gun: &mut Gun) {
    gun.clip_mirror = wd.m_iClip;
    gun.idle = wd.m_flTimeWeaponIdle;
    gun.next_primary = wd.m_flNextPrimaryAttack;
    gun.next_secondary = wd.m_flNextSecondaryAttack;
    gun.fuser1 = wd.fuser1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weapons::id;

    #[test]
    fn weapon_info_layout() {
        let m = WeaponInfo { id: 13, clip: 30, reserve: 90, mode: 0, items: 8 };
        let mut b = Vec::new();
        m.write(&mut b);
        assert_eq!(b, vec![13, 30, 90, 0, 0, 8]);
        assert_eq!(b.len(), 6, "registered size");
        assert_eq!(WeaponInfo::read(&b), Some(m));
        let mut b = Vec::new();
        WStatus { id: 2, owned: 1, clip: 15 }.write(&mut b);
        assert_eq!(b.len(), 3);
        let mut b = Vec::new();
        write_kfu_power(&mut b, 66.5);
        assert_eq!(read_kfu_power(&b), Some(66));
    }

    #[test]
    fn custom_wp_round_trip() {
        let s = custom_wp_string(13, 0.0025, 5.5, 0.0001, 39.0, 3000);
        assert_eq!(s, "13 0.002500 5.500000 0.000100 39.000000 3000");
        assert_eq!(parse_custom_wp(&s), Some((13, 0.0025, 5.5, 0.0001, 39.0, 3000)));
    }

    #[test]
    fn clientdata_round_trip() {
        let mut g = Gun::default();
        g.take_weapon(id::BERETTA, None, 0);
        g.take_weapon(id::BERETTA, None, 0);
        g.current = id::BERETTA_AKIMBO;
        g.reserve[1] = 77;
        g.status[id::BERETTA_AKIMBO as usize].toggle = true;
        let mut cd = clientdata_t::default();
        pack_clientdata(&g, 0x10, &mut cd);
        assert_eq!(cd.vuser3[0], 8.0);
        assert_eq!(cd.vuser4[1], 30.0);
        assert_eq!(cd.ammo_nails, 15);
        assert_eq!(cd.ammo_shells, 15, "the single record's clip");
        assert_eq!(cd.vuser2[1], 77.0);
        let mut g2 = Gun::default();
        assert_eq!(unpack_clientdata(&cd, &mut g2), 0x10);
        assert_eq!(g2.current, id::BERETTA_AKIMBO);
        assert_eq!(g2.status[8].clip, 30);
        assert!(g2.status[8].toggle);
        assert_eq!(g2.reserve[1], 77);
        assert_eq!((g2.status[8].left, g2.status[8].right), (15, 15));
        cd.vuser4[1] = 10.0;
        let mut g3 = Gun::default();
        unpack_clientdata(&cd, &mut g3);
        assert_eq!((g3.status[8].left, g3.status[8].right), (15, 0), "right clip clamped at 0");
        let mut wd = weapon_data_t::default();
        g.next_primary = -0.5;
        pack_weapon_data(&g, &mut wd);
        assert_eq!(wd.m_iId, 16);
        assert_eq!(wd.m_flNextPrimaryAttack, -0.001);
    }

    fn xorshift(seed: u64) -> impl FnMut() -> u64 {
        let mut x = seed;
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        }
    }

    /// The message readers take any bytes: short, oversized, unterminated or non-UTF-8 strings.
    #[test]
    fn readers_survive_garbage() {
        let mut next = xorshift(0x1234_5678_9abc_def1);
        let mut bufs: Vec<Vec<u8>> = vec![vec![], vec![0xff], vec![0xff; 32], vec![b'a'; 100]];
        for _ in 0..2000 {
            let len = (next() % 24) as usize;
            bufs.push((0..len).map(|_| next() as u8).collect());
        }
        for b in &bufs {
            let _ = WeaponInfo::read(b);
            let _ = WStatus::read(b);
            let _ = ActItems::read(b);
            let _ = read_kfu_power(b);
            let mut r = Reader::new(b);
            for _ in 0..8 {
                let _ = (r.byte(), r.char(), r.short(), r.string());
            }
            let _ = parse_custom_wp(&String::from_utf8_lossy(b));
        }
        assert_eq!(WeaponInfo::read(&[1, 2, 3, 4, 5]), None, "one byte short");
        assert_eq!(parse_custom_wp("999 nan inf -inf 1e40 99999999999"), None, "id and cost out of range");
    }

    /// `clientdata_t` comes from the server's delta: any id, NaN and huge values unpack without a
    /// panic.
    #[test]
    fn unpack_clientdata_survives_any_values() {
        let vals = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 8.0, 37.0, 38.0, 255.0, 1e10, -1e10];
        let ints = [i32::MIN, -1, 0, 1, i32::MAX];
        for &a in &vals {
            for &n in &ints {
                let cd = clientdata_t {
                    vuser1: [a; 3],
                    vuser2: [a; 3],
                    vuser3: [a; 3],
                    vuser4: [a; 3],
                    ammo_nails: n,
                    ammo_shells: n,
                    m_flNextAttack: a,
                    ..Default::default()
                };
                let mut g = Gun::default();
                unpack_clientdata(&cd, &mut g);
                assert!(g.status.iter().all(|s| s.right >= 0));
            }
        }
        let mut cd = clientdata_t::default();
        cd.vuser3[0] = id::BERETTA as f32;
        cd.vuser4[1] = -3e9; // saturates to i32::MIN
        cd.ammo_nails = 1;
        let mut g = Gun::default();
        unpack_clientdata(&cd, &mut g);
        assert_eq!(g.status[id::BERETTA as usize].right, i32::MAX, "clip - left wraps as TS's int does");
    }
}
