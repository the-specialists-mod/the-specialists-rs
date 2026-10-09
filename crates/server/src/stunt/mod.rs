//! Server half of TS stunts (CTSStunt) and animation selection (CTSAnimation). Owner: movement
//! work package. Movement physics itself is shared::pm (runs on both sides).
//! Reference: decomp docs/server/player-movement.md §2-3, §5.

use crate::commands::Args;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::sys::clientdata_t;

#[derive(Default)]
pub struct StuntState {}

pub fn game_init() {}
pub fn precache() {}
pub fn on_spawn(_p: &mut Player, _cx: &mut Cx) {}
pub fn pre_think(_p: &mut Player, _cx: &mut Cx) {}
pub fn post_think(_p: &mut Player, _cx: &mut Cx) {}
pub fn client_command(_p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> bool {
    false
}
pub fn fill_client_data(_p: &mut Player, cx: &mut Cx, cd: &mut clientdata_t) {
    let pev = cx.pev();
    cd.iuser4 = pev.iuser4;
    cd.fuser1 = pev.fuser1;
}
