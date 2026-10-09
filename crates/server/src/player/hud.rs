//! Keeps the client HUD in sync: sends TS HUD messages when values change. Owner: rules work
//! package. Reference: decomp docs/server/architecture.md §3.2, client/architecture-hud.md §3.

use super::Player;
use crate::messages::{self, Msg};
use crate::world::Cx;

#[derive(Default)]
pub struct HudCache {
    pub health: Option<u8>,
}

pub fn reset(p: &mut Player) {
    p.hud = HudCache::default();
}

pub fn update(p: &mut Player, cx: &mut Cx) {
    let hp = cx.pev().health.clamp(0.0, 255.0) as u8;
    if p.hud.health != Some(hp) {
        p.hud.health = Some(hp);
        messages::one(Msg::TsHealth, cx.edict).byte(hp as i32).end();
    }
}
