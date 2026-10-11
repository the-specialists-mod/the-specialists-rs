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
pub fn sprite_texture(model: *const model_s, frame: i32) -> bool {
    tri!(SpriteTexture(model.cast_mut(), frame)) != 0
}
