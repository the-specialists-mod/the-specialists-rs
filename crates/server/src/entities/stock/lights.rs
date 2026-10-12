//! `light`, `light_spot`, `light_environment` (SDK lights.cpp; same in TS).
//! Lights without a targetname are removed (their light is baked into the BSP). Switchable
//! lights (style >= 32) drive their light style; `light_environment` also sets the sky colour
//! and direction cvars used for studio model lighting.

use super::base::*;
use crate::entities::remove;
use crate::world::{Cx, Entity, UseType};
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::CStr;

pub const SF_LIGHT_START_OFF: i32 = 1;

#[derive(Default)]
pub struct Light<const ENV: bool> {
    /// `m_iStyle` (+0x64).
    style: i32,
    /// `m_iszPattern` (+0x68): the `pattern` key, `ALLOC_STRING`ed.
    pattern: string_t,
}

fn set_style(style: i32, value: &'static CStr) {
    engine().light_style(style, value);
}

/// `sscanf(value, "%d %d %d %d")`: up to four integers, each after optional white space,
/// stopping at the first that doesn't parse (`"100.5 50"` reads one).
fn scan_ints(value: &str) -> Vec<i32> {
    let mut out = Vec::new();
    let mut rest = value.as_bytes();
    while out.len() < 4 {
        let t = rest.trim_ascii_start();
        let (neg, digits) = match t.first() {
            Some(b'-') => (true, &t[1..]),
            Some(b'+') => (false, &t[1..]),
            _ => (false, t),
        };
        let n = digits.iter().take_while(|c| c.is_ascii_digit()).count();
        if n == 0 {
            break;
        }
        let v = digits[..n].iter().fold(0i64, |v, &c| (v * 10 + (c - b'0') as i64).min(1 << 32));
        out.push((if neg { -v } else { v }) as i32);
        rest = &digits[n..];
    }
    out
}

/// Rounds `m * 2^e` to a 64-bit mantissa (round to nearest even), as the x87 stores a product in
/// extended precision.
fn round_ext(m: u128, e: i32) -> (u128, i32) {
    let len = 128 - m.leading_zeros() as i32;
    if len <= 64 {
        return (m, e);
    }
    let shift = len - 64;
    let rem = m & ((1 << shift) - 1);
    let half = 1u128 << (shift - 1);
    let mut r = m >> shift;
    if rem > half || (rem == half && r & 1 == 1) {
        r += 1;
    }
    (r, e + shift)
}

/// `(int)(k * a * b)` as TS's x87 code evaluates it: the double `k` times each integer in 80-bit
/// extended precision, then truncated; out of `int` range gives 0x80000000.
fn x87_mul_trunc(k: f64, a: i32, b: i32) -> i32 {
    if !k.is_finite() {
        return i32::MIN;
    }
    if k == 0.0 || a == 0 || b == 0 {
        return 0;
    }
    let bits = k.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = (bits & ((1 << 52) - 1)) as u128;
    let (m, e) = if raw_exp == 0 { (frac, -1074) } else { (frac | (1 << 52), raw_exp - 1075) };
    let (m, e) = round_ext(m * a.unsigned_abs() as u128, e);
    let (m, e) = round_ext(m * b.unsigned_abs() as u128, e);
    // |value| = m * 2^e, m < 2^65: truncate toward zero.
    let mag = if e >= 0 {
        if e > 40 { u128::MAX } else { m << e }
    } else if e <= -128 {
        0
    } else {
        m >> -e
    };
    let neg = (k < 0.0) ^ (a < 0) ^ (b < 0);
    match i32::try_from(mag) {
        Ok(v) if !neg => v,
        _ if neg && mag <= 1 << 31 => (mag as i64).wrapping_neg() as i32,
        _ => i32::MIN,
    }
}

/// `_light` -> `sv_skycolor_*` (`CEnvLight::KeyValue` 0xcbdb0). One value is grey; four scale
/// the colour by `brightness * (1.0 / 255)` in extended precision (that double is just below
/// 1/255, so full brightness takes 255 to 254); then each channel is
/// `(int)(pow(c * (1.0 / 114), 0.6) * 264)`. Guard: with two or three values (or none) TS uses
/// uninitialised stack values for the missing ones; the port uses 0.
pub fn sky_color(value: &str) -> (i32, i32, i32) {
    const INV_255: f64 = 0.00392156862745098;
    const INV_114: f64 = 0.008771929824561403;
    let nums = scan_ints(value);
    let (mut r, mut g, mut b) = (nums.first().copied().unwrap_or(0), nums.get(1).copied().unwrap_or(0), nums.get(2).copied().unwrap_or(0));
    if nums.len() == 1 {
        g = r;
        b = r;
    } else if nums.len() == 4 {
        let a = nums[3];
        r = x87_mul_trunc(INV_255, a, r);
        g = x87_mul_trunc(INV_255, a, g);
        b = x87_mul_trunc(INV_255, a, b);
    }
    // The pow argument is stored as a double (`fstpl`); its product with 264 stays in 80 bits.
    let f = |c: i32| x87_mul_trunc((c as f64 * INV_114).powf(0.6), 264, 1);
    (f(r), f(g), f(b))
}

impl<const ENV: bool> Light<ENV> {
    /// The "on" style: the pattern, or `"m"` without one.
    fn pattern(&self) -> &'static CStr {
        if self.pattern == 0 { c"m" } else { s(self.pattern) }
    }
}

impl<const ENV: bool> Entity for Light<ENV> {
    /// `CLight::Save` (0xcba8c) / `Restore`: `CBaseEntity`'s part, then `"CLight"` (also
    /// `CEnvLight`'s). `Use` is a virtual override and no member pointer is ever set; `Precache` is
    /// `CBaseEntity`'s (empty), so nothing runs after a restore.
    fn save_fields(&mut self, cx: &mut Cx, f: &mut crate::save::Fields) {
        f.base_funcs(cx, crate::save::BaseFuncs::default());
        f.table(&crate::save::tables::CLight);
        f.int(&mut self.style);
        f.string(&mut self.pattern);
    }

    fn key_value(&mut self, cx: &mut Cx, key: &CStr, value: &CStr) -> bool {
        match key.to_bytes() {
            b"_light" if ENV => {
                let eng = engine();
                let (r, g, b) = sky_color(kv_str(value));
                for (name, c) in [(c"sv_skycolor_r", r), (c"sv_skycolor_g", g), (c"sv_skycolor_b", b)] {
                    eng.set_cvar_cstr(name, &goldsrc::util::to_cstring(&c.to_string()));
                }
            }
            b"style" => self.style = kv_i(value),
            b"pitch" => cx.pev().angles[0] = kv_f(value),
            b"pattern" => self.pattern = alloc(value),
            _ => return false,
        }
        true
    }

    fn spawn(&mut self, cx: &mut Cx) {
        if ENV {
            let eng = engine();
            let mut a = cx.pev().angles;
            a[0] = -a[0]; // UTIL_MakeAimVectors
            let (f, _, _) = eng.make_vectors(a);
            for (name, c) in [(c"sv_skyvec_x", f[0]), (c"sv_skyvec_y", f[1]), (c"sv_skyvec_z", f[2])] {
                eng.set_cvar_cstr(name, &goldsrc::util::to_cstring(&format!("{c:.6}")));
            }
        }
        if cx.pev().targetname == 0 {
            remove(cx.edict);
            return;
        }
        if self.style >= 32 {
            if cx.pev().spawnflags & SF_LIGHT_START_OFF != 0 {
                set_style(self.style, c"a");
            } else {
                set_style(self.style, self.pattern());
            }
        }
    }

    fn use_(&mut self, cx: &mut Cx, _activator: *mut edict_t, _caller: *mut edict_t, kind: UseType, _value: f32) {
        if self.style < 32 {
            return;
        }
        let v = cx.pev();
        let off = v.spawnflags & SF_LIGHT_START_OFF != 0;
        if !should_toggle(kind, !off) {
            return;
        }
        if off {
            set_style(self.style, self.pattern());
            v.spawnflags &= !SF_LIGHT_START_OFF;
        } else {
            set_style(self.style, c"a");
            v.spawnflags |= SF_LIGHT_START_OFF;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sky_color() {
        assert_eq!(super::sky_color("114"), (264, 264, 264));
        // Full brightness takes 114 to 113 in 80 bits.
        assert_eq!(super::sky_color("114 0 114 255"), (262, 0, 262));
        assert_eq!(super::sky_color("228 228 228 0"), (0, 0, 0));
        // sscanf stops at the decimal point: one value, so grey.
        assert_eq!(super::sky_color("114.5 0 0 255"), (264, 264, 264));
    }

    #[test]
    fn x87_products() {
        use super::x87_mul_trunc;
        const INV_255: f64 = 0.00392156862745098;
        assert_eq!(x87_mul_trunc(INV_255, 255, 255), 254);
        // A double product rounds the same one up to 255.
        assert_eq!(((INV_255 * 255.0) * 255.0) as i32, 255);
        assert_eq!(x87_mul_trunc(INV_255, 200, 255), 199);
        assert_eq!(x87_mul_trunc(INV_255, 200, 100), 78);
        assert_eq!(x87_mul_trunc(INV_255, 37, 201), 29);
        assert_eq!(x87_mul_trunc(INV_255, 0, 255), 0);
        assert_eq!(x87_mul_trunc(-0.5, 3, 1), -1);
        assert_eq!(x87_mul_trunc(2.5, 264, 1), 660);
        assert_eq!(x87_mul_trunc(1.0e12, 264, 1), i32::MIN);
        assert_eq!(x87_mul_trunc(-1.0, i32::MIN, -1), i32::MIN);
    }

    #[test]
    fn scan_ints_like_sscanf() {
        assert_eq!(super::scan_ints("255 128 64 200"), vec![255, 128, 64, 200]);
        assert_eq!(super::scan_ints("  -3\t4"), vec![-3, 4]);
        assert_eq!(super::scan_ints("100.5 50 50 200"), vec![100]);
        assert_eq!(super::scan_ints("1 2 3 4 5"), vec![1, 2, 3, 4]);
        assert!(super::scan_ints("abc").is_empty());
    }
}
