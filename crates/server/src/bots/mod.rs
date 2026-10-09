//! Server-side bots (fake clients) used to exercise dedicated servers without a human client.
//! Owner: rules work package.

use crate::commands::Args;
use crate::player::Player;
use crate::world::Cx;

pub struct Bot {}

pub fn game_init() {}
/// Called every server frame: drives each bot with `pfnRunPlayerMove`.
pub fn start_frame() {}
pub fn client_command(_p: &mut Player, _cx: &mut Cx, _cmd: &str, _args: &Args) -> bool {
    false
}
