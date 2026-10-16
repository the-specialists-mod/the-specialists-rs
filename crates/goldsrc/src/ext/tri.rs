//! Safe wrappers over the engine's triangle API (`pTriAPI`, triangleapi.h) for the client's
//! TriAPI effects (lasers, particles, scope overlay, client bullets). TS's fog is raw OpenGL
//! ([`crate::ext::gl`]). Keep additions here to avoid merge conflicts.
//!
//! Each wrapper is one `tri!` call; like the macro, it panics with the slot name if the engine
//! left the slot null.

use crate::sys::*;
use crate::tri;

/// `RenderMode`: a `kRender*` mode for the following primitives.
pub fn render_mode(mode: i32) {
    tri!(RenderMode(mode));
}

/// `CullFace`.
pub fn cull_face(style: TRICULLSTYLE) {
    tri!(CullFace(style));
}

/// `Begin`: starts a primitive (`TRI_QUADS`, ...).
pub fn begin(primitive: i32) {
    tri!(Begin(primitive));
}

/// `End`: ends the primitive started by [`begin`].
pub fn end() {
    tri!(End());
}

/// `Color4f`.
pub fn color4f(r: f32, g: f32, b: f32, a: f32) {
    tri!(Color4f(r, g, b, a));
}

/// `TexCoord2f`.
pub fn tex_coord2f(u: f32, v: f32) {
    tri!(TexCoord2f(u, v));
}

/// `Vertex3fv`.
pub fn vertex3fv(p: &[f32; 3]) {
    tri!(Vertex3fv(p.as_ptr()));
}

/// `SpriteTexture`: binds `frame` of a sprite model (a handle from
/// [`sprite_model`](crate::ext::render::sprite_model)). False when the engine can't bind it.
/// Guarded: Steam GoldSrc's `R_TriangleSpriteTexture` dereferences the model unchecked, so a
/// null one (a sprite that didn't load) binds nothing, FWGS's answer.
///
/// A frame outside the sprite binds GoldSrc's frame on both engines ([`sprite_frame`]).
pub fn sprite_texture(model: *const model_s, frame: i32) -> bool {
    if model.is_null() {
        return false;
    }
    let Some(frame) = sprite_frame(frame, crate::ext::spectator::model_frames(model)) else { return false };
    tri!(SpriteTexture(model.cast_mut(), frame)) != 0
}

/// The frame GoldSrc's `R_GetSpriteFrame` (Steam `hw.so` `0x143420`) binds for `frame` of a
/// sprite with `count` frames: a frame below 0 or past the last is frame 0 (with a developer
/// "Sprite: no such frame" line); no frames binds nothing. FWGS clamps past-the-end frames to
/// the last one instead and would read before its table with none, so the port asks it for
/// GoldSrc's frame.
pub fn sprite_frame(frame: i32, count: i32) -> Option<i32> {
    if count <= 0 {
        return None;
    }
    Some(if frame < 0 || frame >= count { 0 } else { frame })
}

#[cfg(test)]
mod tests {
    use super::sprite_frame;

    #[test]
    fn out_of_range_frames_are_goldsrcs_frame_0() {
        assert_eq!(sprite_frame(3, 4), Some(3));
        assert_eq!(sprite_frame(4, 4), Some(0));
        assert_eq!(sprite_frame(9, 4), Some(0));
        assert_eq!(sprite_frame(-1, 4), Some(0));
        assert_eq!(sprite_frame(0, 0), None);
    }
}
