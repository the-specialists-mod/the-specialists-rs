//! `HUD_TempEntUpdate` (`0x1001bd00`): the SDK `entity.cpp` loop with TS's two changes:
//! `frametime *= g_flTimeScale` (temp entities slow down in slow motion) and the TS shell sound
//! (`TempEnt_PlayShellSound` `0x100569a0`) on the first bounce.

use goldsrc::ext::render::{self as r, vec3 as v};
use goldsrc::sys::*;
use goldsrc::{efx, ev};

pub type AddVisible = unsafe extern "C" fn(*mut cl_entity_s) -> i32;
pub type PlaySound = unsafe extern "C" fn(*mut TEMPENTITY, f32);

static FRAME: goldsrc::Global<std::cell::Cell<i32>> = goldsrc::Global(std::cell::Cell::new(0));

/// TS shell sound (type in `curstate.iuser1`: 1 `shell.wav`, 2 shotgun shell), played once.
fn shell_sound(t: &mut TEMPENTITY) {
    let kind = t.entity.curstate.iuser1;
    let sample = match kind {
        1 => "weapons/shell.wav",
        // TS names `weapons/sshell.wav`, which TS 3.0 doesn't ship; it has sshell1/2.
        2 => {
            if r::random_long(0, 1) == 0 {
                "weapons/sshell1.wav"
            } else {
                "weapons/sshell2.wav"
            }
        }
        _ => return,
    };
    t.entity.curstate.iuser1 = 0;
    let pitch = (crate::events::slow_pitch() as f32 * r::random_float(0.9, 1.3)) as i32;
    r::play_sound(-1, t.entity.origin, CHAN_AUTO as i32, sample, r::random_float(0.1, 0.3), 1.9, 0, pitch);
}

/// # Safety
/// Pointers come from the engine's `HUD_TempEntUpdate` call.
#[allow(clippy::too_many_arguments)]
pub unsafe fn update(frametime: f64, client_time: f64, cl_gravity: f64, free: *mut *mut TEMPENTITY, active: *mut *mut TEMPENTITY, add_visible: AddVisible,
                     play_sound: PlaySound, time_scale: f32) {
    unsafe {
        if active.is_null() || (*active).is_null() {
            return;
        }
        let frametime = frametime * time_scale as f64;
        ev!(EV_SetUpPlayerPrediction(0, 1));
        ev!(EV_PushPMStates());
        ev!(EV_SetSolidPlayers(-1));
        let tf = (FRAME.0.get() + 1) & 31;
        FRAME.0.set(tf);

        let mut t = *active;
        if frametime <= 0.0 {
            while !t.is_null() {
                if (*t).flags & FTENT_NOMODEL as i32 == 0 {
                    add_visible(&mut (*t).entity);
                }
                t = (*t).next;
            }
            ev!(EV_PopPMStates());
            return;
        }

        let ft = frametime as f32;
        let ct = client_time as f32;
        let fast_freq = client_time * 5.5;
        let gravity = -ft * cl_gravity as f32;
        let gravity_slow = gravity * 0.5;
        let mut prev: *mut TEMPENTITY = std::ptr::null_mut();
        while !t.is_null() {
            let p = &mut *t;
            let next = p.next;
            let life = p.die - ct;
            let mut alive = true;
            if life < 0.0 {
                if p.flags & FTENT_FADEOUT as i32 != 0 {
                    if p.entity.curstate.rendermode == kRenderNormal as i32 {
                        p.entity.curstate.rendermode = kRenderTransTexture as i32;
                    }
                    p.entity.curstate.renderamt = (p.entity.baseline.renderamt as f32 * (1.0 + life * p.fadeSpeed)) as i32;
                    if p.entity.curstate.renderamt <= 0 {
                        alive = false;
                    }
                } else {
                    alive = false;
                }
            }
            if !alive {
                p.next = *free;
                *free = t;
                if prev.is_null() {
                    *active = next;
                } else {
                    (*prev).next = next;
                }
                t = next;
                continue;
            }
            prev = t;
            p.entity.prevstate.origin = p.entity.origin;
            let vel = p.entity.baseline.origin;

            if p.flags & FTENT_SPARKSHOWER as i32 != 0 {
                if ct > p.entity.baseline.scale {
                    r::spark_effect(p.entity.origin, 8, -200, 200);
                    p.entity.baseline.framerate -= 0.1;
                    if p.entity.baseline.framerate <= 0.0 {
                        p.die = ct;
                    } else {
                        p.die = ct + 0.5;
                        p.entity.baseline.scale = ct + 0.1;
                    }
                }
            } else if p.flags & FTENT_PLYRATTACHMENT as i32 != 0 {
                if let Some(c) = r::entity(p.clientIndex as i32) {
                    p.entity.origin = v::add(c.origin, p.tentOffset);
                }
            } else if p.flags & FTENT_SINEWAVE as i32 != 0 {
                p.x += vel[0] * ft;
                p.y += vel[1] * ft;
                p.entity.origin[0] = p.x + (vel[2] + ct * p.entity.prevstate.frame).sin() * (10.0 * p.entity.curstate.framerate);
                p.entity.origin[1] = p.y + (vel[2] + fast_freq as f32 + 0.7).sin() * (8.0 * p.entity.curstate.framerate);
                p.entity.origin[2] += vel[2] * ft;
            } else if p.flags & FTENT_SPIRAL as i32 != 0 {
                let salt = (t as usize & 0xffff) as f32;
                p.entity.origin[0] += vel[0] * ft + 8.0 * (ct * 20.0 + salt).sin();
                p.entity.origin[1] += vel[1] * ft + 4.0 * (ct * 30.0 + salt).sin();
                p.entity.origin[2] += vel[2] * ft;
            } else {
                p.entity.origin = v::ma(p.entity.origin, ft, vel);
            }

            if p.flags & FTENT_SPRANIMATE as i32 != 0 {
                p.entity.curstate.frame += ft * p.entity.curstate.framerate;
                if p.entity.curstate.frame >= p.frameMax {
                    p.entity.curstate.frame -= p.entity.curstate.frame.trunc();
                    if p.flags & FTENT_SPRANIMATELOOP as i32 == 0 {
                        p.die = ct;
                        t = next;
                        continue;
                    }
                }
            } else if p.flags & FTENT_SPRCYCLE as i32 != 0 {
                p.entity.curstate.frame += ft * 10.0;
                if p.entity.curstate.frame >= p.frameMax {
                    p.entity.curstate.frame -= p.entity.curstate.frame.trunc();
                }
            }

            if p.flags & FTENT_ROTATE as i32 != 0 {
                p.entity.angles = v::ma(p.entity.angles, ft, p.entity.baseline.angles);
                p.entity.latched.prevangles = p.entity.angles;
            }

            if p.flags & (FTENT_COLLIDEALL | FTENT_COLLIDEWORLD) as i32 != 0 {
                let mut frac = 1.0f32;
                let mut normal = [0.0f32; 3];
                let mut tr: pmtrace_s = std::mem::zeroed();
                let (mut a, mut b) = (p.entity.prevstate.origin, p.entity.origin);
                ev!(EV_SetTraceHull(2));
                if p.flags & FTENT_COLLIDEALL as i32 != 0 {
                    ev!(EV_PlayerTrace(a.as_mut_ptr(), b.as_mut_ptr(), PM_STUDIO_BOX as i32, -1, &mut tr));
                    if tr.fraction != 1.0 {
                        let pe = r::physent(tr.ent);
                        if tr.ent == 0 || pe.is_none_or(|pe| pe.info != p.clientIndex as i32) {
                            frac = tr.fraction;
                            normal = tr.plane.normal;
                            if let Some(cb) = p.hitcallback {
                                cb(t, &mut tr);
                            }
                        }
                    }
                } else {
                    ev!(EV_PlayerTrace(a.as_mut_ptr(), b.as_mut_ptr(), (PM_STUDIO_BOX | PM_WORLD_ONLY) as i32, -1, &mut tr));
                    if tr.fraction != 1.0 {
                        frac = tr.fraction;
                        normal = tr.plane.normal;
                        if p.flags & FTENT_SPARKSHOWER as i32 != 0 {
                            p.entity.baseline.origin = v::scale(p.entity.baseline.origin, 0.6);
                            if v::len(p.entity.baseline.origin) < 10.0 {
                                p.entity.baseline.framerate = 0.0;
                            }
                        }
                        if let Some(cb) = p.hitcallback {
                            cb(t, &mut tr);
                        }
                    }
                }
                if frac != 1.0 {
                    p.entity.origin = v::ma(p.entity.prevstate.origin, frac * ft, p.entity.baseline.origin);
                    let mut damp = p.bounceFactor;
                    if p.flags & (FTENT_GRAVITY | FTENT_SLOWGRAVITY) as i32 != 0 {
                        damp *= 0.5;
                        if normal[2] > 0.9 && p.entity.baseline.origin[2] <= 0.0 && p.entity.baseline.origin[2] >= gravity * 3.0 {
                            damp = 0.0;
                            p.flags &= !((FTENT_ROTATE | FTENT_GRAVITY | FTENT_SLOWGRAVITY | FTENT_COLLIDEWORLD | FTENT_SMOKETRAIL) as i32);
                            p.entity.angles[0] = 0.0;
                            p.entity.angles[2] = 0.0;
                        }
                    }
                    shell_sound(p);
                    if p.hitSound != 0 {
                        play_sound(t, damp);
                    }
                    if p.flags & FTENT_COLLIDEKILL as i32 != 0 {
                        p.flags &= !(FTENT_FADEOUT as i32);
                        p.die = ct;
                    } else {
                        if damp != 0.0 {
                            let proj = v::dot(p.entity.baseline.origin, normal);
                            p.entity.baseline.origin = v::ma(p.entity.baseline.origin, -proj * 2.0, normal);
                            p.entity.angles[1] = -p.entity.angles[1];
                        }
                        if damp != 1.0 {
                            p.entity.baseline.origin = v::scale(p.entity.baseline.origin, damp);
                            p.entity.angles = v::scale(p.entity.angles, 0.9);
                        }
                    }
                }
            }

            if p.flags & FTENT_FLICKER as i32 != 0 && tf == p.entity.curstate.effects {
                if let Some(dl) = r::alloc_dlight(0) {
                    dl.origin = p.entity.origin;
                    dl.radius = 60.0;
                    dl.color.r = 255;
                    dl.color.g = 120;
                    dl.color.b = 0;
                    dl.die = ct + 0.01;
                }
            }
            if p.flags & FTENT_SMOKETRAIL as i32 != 0 {
                let (mut a, mut b) = (p.entity.prevstate.origin, p.entity.origin);
                efx!(R_RocketTrail(a.as_mut_ptr(), b.as_mut_ptr(), 1));
            }
            if p.flags & FTENT_GRAVITY as i32 != 0 {
                p.entity.baseline.origin[2] += gravity;
            } else if p.flags & FTENT_SLOWGRAVITY as i32 != 0 {
                p.entity.baseline.origin[2] += gravity_slow;
            }
            if p.flags & FTENT_CLIENTCUSTOM as i32 != 0 {
                if let Some(cb) = p.callback {
                    cb(t, ft, ct);
                }
            }
            if p.flags & FTENT_NOMODEL as i32 == 0 && add_visible(&mut p.entity) == 0 && p.flags & FTENT_PERSIST as i32 == 0 {
                p.die = ct;
                p.flags &= !(FTENT_FADEOUT as i32);
            }
            t = next;
        }
        ev!(EV_PopPMStates());
    }
}
