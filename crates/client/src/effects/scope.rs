//! Scope overlay (`DrawScopeOverlay` `0x10054510`, rendering doc §3.3 step 7).
//!
//! TS draws it in the transparent pass, after the particles, as world-space TriAPI quads (no raw
//! OpenGL): `sprites/quarterscope.spr` (one 256×256 frame holding the top-left quarter of the
//! scope) four times around a point 10 units in front of the view, mirrored into each quadrant so
//! the texture's bottom-right corner meets at the centre of the screen.

use super::psys::Camera;
use goldsrc::ext::render::{Vec3, vec3 as v};
use goldsrc::ext::tri;
use goldsrc::sys::*;

pub const SPRITE: &str = "sprites/quarterscope.spr";

/// One quad: (texture coordinate, vertex) in TS emit order.
pub type Quad = [([f32; 2], Vec3); 4];

/// The four quads for a view (`origin`, `forward`, `right`, `up`) and integer FOV.
///
/// Half the size of one quad is `max(fov·0.1, 1)·0.5`, the product and the clamp on the x87 and
/// the result stored as a float (`0x100545fe..0x10054620`), and the quads sit at
/// `origin + forward·10`. Quadrant order and vertex order are TS's: (+right, +up), (−right, +up),
/// (+right, −up), (−right, −up); each quad's outer corner gets uv (0, 0) and the shared centre
/// uv (1, 1). Every sum is a float vector operation (`Vector::operator+/-`, `0x10035b10` and
/// `0x1003d200`, or inlined), and TS adds the up offset before the right one: the quad's centre is
/// `(c ± up) ± right` and each corner `(centre ± up) ± right` (2.1's `0x10094300` is the same; issue #274).
pub fn quads(cam: &Camera, fov: i32) -> [Quad; 4] {
    let s = ((fov as f64 * 0.1).max(1.0) * 0.5) as f32;
    let c = v::ma(cam.origin, 10.0, cam.forward);
    let (r, u) = (v::scale(cam.right, s), v::scale(cam.up, s));
    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)].map(|(sx, sy): (f32, f32)| {
        let (r, u) = (v::scale(r, sx), v::scale(u, sy));
        let a = v::add(v::add(c, u), r);
        let corners = [v::add(v::add(a, u), r), v::sub(v::add(a, u), r), v::sub(v::sub(a, u), r), v::add(v::sub(a, u), r)];
        [0, 1, 2, 3].map(|i| (uv[i], corners[i]))
    })
}

/// `DrawScopeOverlay`: first person, alive, not observing, and the (eased) FOV below 40. The
/// caller checks for the hardware renderer, as TS does.
/// The sprite goes through the particle system's cache (render mode key 2, `0x1005455b`).
pub fn draw(cam: &Camera, third_person: bool) {
    let fov = crate::state::with(|s| s.fov) as i32;
    if third_person || fov >= 40 || crate::hud::dead_or_observer() {
        return;
    }
    let Some(sprite) = super::with_psys(|ps| ps.sprite(SPRITE, kRenderTransTexture as i32).map(|s| ps.sprite_handle(s))) else { return };
    if !super::laser::set_sprite(sprite, 0) {
        return;
    }
    tri::render_mode(kRenderTransTexture as i32);
    tri::cull_face(TRICULLSTYLE_TRI_NONE);
    tri::begin(TRI_QUADS as i32);
    tri::color4f(1.0, 1.0, 1.0, 1.0);
    for q in quads(cam, fov) {
        for (uv, p) in q {
            tri::tex_coord2f(uv[0], uv[1]);
            tri::vertex3fv(&p);
        }
    }
    tri::end();
    tri::render_mode(kRenderNormal as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera { origin: [10.0, 20.0, 30.0], forward: [1.0, 0.0, 0.0], right: [0.0, -1.0, 0.0], up: [0.0, 0.0, 1.0], ..Default::default() }
    }

    #[test]
    fn quads_meet_at_the_centre_with_mirrored_uvs() {
        let q = quads(&cam(), 20);
        let centre = [20.0, 20.0, 30.0];
        for quad in &q {
            // uv (1, 1) is always the shared centre, uv (0, 0) the outer corner 2 units out per axis.
            let (_, c) = quad.iter().find(|(uv, _)| *uv == [1.0, 1.0]).unwrap();
            assert_eq!(*c, centre);
            let (_, o) = quad.iter().find(|(uv, _)| *uv == [0.0, 0.0]).unwrap();
            assert_eq!(((o[1] - centre[1]).abs(), (o[2] - centre[2]).abs(), o[0]), (2.0, 2.0, 20.0));
        }
        // First quad is +right (−y here), +up.
        assert_eq!(q[0][0].1, [20.0, 18.0, 32.0]);
        assert_eq!(q[3][0].1, [20.0, 22.0, 28.0]);
    }

    #[test]
    fn sums_round_as_ts_adds_them() {
        // `0x10054790..0x100547f9`: centre `(c + up) + right`, corner `(centre + up) + right`, in
        // floats. Adding right first rounds the corner's x to 0x447ff1ec.
        let cam = Camera {
            origin: [1019.54, -1994.98, 64.1],
            forward: [0.36, 0.48, 0.8],
            right: [0.8, -0.6, 0.0],
            up: [-0.48, -0.64, 0.6],
            ..Default::default()
        };
        let q = quads(&cam, 20);
        assert_eq!(q[0][0].1.map(f32::to_bits), [0x447f_f1eb, 0xc4f9_151e, 0x4292_9999]);
        assert_eq!(q[0][2].1.map(f32::to_bits), [0x447f_c8f5, 0xc4f8_c5c2, 0x4290_3333]);
        // fov 13: 13 × 0.1 on the x87 is 1.3 → half size 0.65; the float product 1.3000001 gave
        // 0.65000004.
        let cam = Camera { forward: [0.0, 1.0, 0.0], right: [1.0, 0.0, 0.0], up: [0.0, 0.0, 1.0], ..Default::default() };
        assert_eq!(quads(&cam, 13)[0][0].1, [1.3, 10.0, 1.3]);
    }

    #[test]
    fn size_clamps_at_low_fov() {
        let q = quads(&cam(), 5);
        assert_eq!(q[0][0].1, [20.0, 19.0, 31.0], "max(fov*0.1, 1) * 0.5 = 0.5 half size");
    }
}
