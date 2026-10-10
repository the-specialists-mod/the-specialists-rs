//! Engine helpers for voice chat: the server's listening masks (`CVoiceGameMgr`) and the client's
//! voice status (`CVoiceStatus`). Keep additions here to avoid merge conflicts.

#[cfg(feature = "server")]
mod server {
    use crate::eng;
    use crate::server::Engine;

    impl Engine {
        /// `pfnVoice_SetClientListening`: whether client `receiver` hears `sender` (1-based).
        /// Steam HLDS starts every client deaf to everyone, so the game DLL must call this.
        pub fn voice_set_client_listening(&self, receiver: i32, sender: i32, listen: bool) -> bool {
            eng!(pfnVoice_SetClientListening(receiver, sender, listen as _)) != 0
        }
    }
}

#[cfg(feature = "client")]
mod client {
    use crate::cl_eng;
    use crate::client::{ClEngine, EngineCell};
    use crate::sys::*;
    use crate::util::to_cstring;

    impl ClEngine {
        /// `GetPlayerUniqueID`: the 16-byte id the voice ban list keys on (1-based player).
        pub fn player_unique_id(&self, player: i32) -> Option<[u8; 16]> {
            let mut id = [0u8; 16];
            let ok = cl_eng!(GetPlayerUniqueID(player, id.as_mut_ptr().cast()));
            (ok != 0).then_some(id)
        }

        /// `pfnServerCmdUnreliable` (the voice manager sends `vban` this way).
        pub fn server_cmd_unreliable(&self, cmd: &str) {
            let s = to_cstring(cmd);
            cl_eng!(pfnServerCmdUnreliable(s.as_ptr().cast_mut()));
        }
    }

    /// One sprite entity for the engine's visible list (the voice icon over a talking player).
    #[derive(Clone, Copy, Debug)]
    pub struct SpriteEntity {
        pub sprite: HSPRITE,
        pub origin: [f32; 3],
        pub scale: f32,
        pub rendermode: i32,
        pub renderfx: i32,
        pub renderamt: i32,
    }

    /// `CVoiceStatus::m_VoiceHeadModels`: the engine keeps the pointers handed to
    /// `CL_CreateVisibleEntity` until the frame is rendered, so the entities live in static
    /// storage, one per player slot.
    static HEAD_MODELS: EngineCell<[cl_entity_t; 32]> = EngineCell::new(zeroed_models());

    const fn zeroed_models() -> [cl_entity_t; 32] {
        // SAFETY: `cl_entity_t` is a C struct of numbers, arrays and raw pointers, for which
        // all-zero bytes are a valid value (the SDK `memset`s it the same way).
        unsafe { std::mem::zeroed() }
    }

    fn zeroed_entity() -> cl_entity_t {
        // SAFETY: as in `zeroed_models`, all-zero bytes are a valid `cl_entity_t`.
        unsafe { std::mem::zeroed() }
    }

    /// Fills entity `slot` (0..32) from `s` and adds it to this frame's visible list
    /// (`CL_CreateVisibleEntity(ET_NORMAL, ...)`). Call from `HUD_CreateEntities`.
    pub fn add_sprite_entity(slot: usize, s: &SpriteEntity) {
        if slot >= 32 || s.sprite == 0 {
            return;
        }
        let model = crate::ext::render::sprite_model(s.sprite);
        if model.is_null() {
            return;
        }
        let ent = &mut HEAD_MODELS.get()[slot];
        *ent = zeroed_entity();
        ent.curstate.rendermode = s.rendermode;
        ent.curstate.renderamt = s.renderamt;
        ent.baseline.renderamt = s.renderamt;
        ent.curstate.renderfx = s.renderfx;
        ent.curstate.framerate = 1.0;
        ent.curstate.scale = s.scale;
        ent.model = model.cast_mut();
        ent.origin = s.origin;
        let p: *mut cl_entity_t = ent;
        // ET_NORMAL is 0 (entity_types.h).
        cl_eng!(CL_CreateVisibleEntity(0, p));
    }
}

#[cfg(feature = "client")]
pub use client::{SpriteEntity, add_sprite_entity};
