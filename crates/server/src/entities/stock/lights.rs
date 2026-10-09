//! `light`, `light_spot`, `light_environment` (SDK lights.cpp; same in TS).
//! Lights without a targetname are removed (their light is baked into the BSP). Switchable
//! lights (style >= 32) drive their light style; `light_environment` also sets the sky colour
//! and direction cvars used for studio model lighting.

use super::base::*;
use crate::entities::remove;
use crate::world::{Cx, Entity, UseType};
use goldsrc::server::{engine, intern};
use goldsrc::sys::*;
use std::ffi::CStr;

pub const SF_LIGHT_START_OFF: i32 = 1;

#[derive(Default)]
pub struct Light<const ENV: bool> {
    style: i32,
    pattern: Option<&'static CStr>,
}

fn set_style(style: i32, value: &'static CStr) {
    engine().light_style(style, value);
}

/// `_light` -> `sv_skycolor_*` (CEnvLight::KeyValue).
pub fn sky_color(value: &str) -> (i32, i32, i32) {
    let nums: Vec<i32> = value.split_whitespace().map_while(|t| t.parse::<f32>().ok().map(|f| f as i32)).collect();
    let (mut r, mut g, mut b) = (nums.first().copied().unwrap_or(0), nums.get(1).copied().unwrap_or(0), nums.get(2).copied().unwrap_or(0));
    if nums.len() == 1 {
        g = r;
        b = r;
    } else if nums.len() >= 4 {
        let v = nums[3] as f64 / 255.0;
        r = (r as f64 * v) as i32;
        g = (g as f64 * v) as i32;
        b = (b as f64 * v) as i32;
    }
    let f = |c: i32| ((c as f64 / 114.0).powf(0.6) * 264.0) as i32;
    (f(r), f(g), f(b))
}

impl<const ENV: bool> Entity for Light<ENV> {
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
            b"pattern" => self.pattern = Some(intern(kv_str(value))),
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
                set_style(self.style, self.pattern.unwrap_or(c"m"));
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
            set_style(self.style, self.pattern.unwrap_or(c"m"));
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
        assert_eq!(super::sky_color("114 0 114 255"), (264, 0, 264));
        assert_eq!(super::sky_color("228 228 228 0"), (0, 0, 0));
    }
}
