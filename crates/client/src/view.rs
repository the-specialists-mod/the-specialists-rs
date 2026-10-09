//! View: V_CalcRefdef (bob, punch, recoil via cl_viewangles), third-person camera, slow-motion
//! tint and fades. Owner: rendering/prediction work package.
//! Reference: decomp docs/client/rendering-effects-prediction.md §6-7.

use goldsrc::client::engine;
use goldsrc::sys::ref_params_t;

pub fn init() {}
pub fn frame(_time: f64) {}
pub fn cam_think() {}
pub fn is_third_person() -> bool {
    false
}

/// First-person view from the predicted origin.
pub fn calc_refdef(p: &mut ref_params_t) {
    for i in 0..3 {
        p.vieworg[i] = p.simorg[i] + p.viewheight[i];
        p.viewangles[i] = p.cl_viewangles[i] + p.punchangle[i];
    }
    let (f, r, u) = engine().angle_vectors(p.viewangles);
    (p.forward, p.right, p.up) = (f, r, u);
    p.nextView = 0;
}
