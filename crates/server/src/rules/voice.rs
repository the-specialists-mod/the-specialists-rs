//! `CVoiceGameMgr` (voice_gamemgr.cpp, 0xf73c8..0xf79bc) with TS's `CMultiplayGameMgrHelper`
//! (0xd0ed0): who may hear whom, the clients' `vban` lists, and the `VoiceMask`/`ReqState`
//! messages that drive the client's voice UI.
//!
//! Steam HLDS starts every client unable to hear anyone; only `Voice_SetClientListening` calls
//! from here open the streams (FWGS defaults to everyone hearing everyone).

use super::state::{self, Mode};
use super::{alive, cvars, other, teams};
use crate::commands::Args;
use goldsrc::Global;
use goldsrc::edict::EdictPtr;
use goldsrc::server::{MsgWriter, engine};
use goldsrc::sys::*;
use std::cell::RefCell;

/// `VOICE_MAX_PLAYERS`: one 32-bit mask per client.
pub const MAX_PLAYERS: usize = 32;
/// `CVoiceGameMgr::Update`: masks are rebuilt every 0.3 s.
pub const UPDATE_INTERVAL: f64 = 0.3;

#[derive(Default)]
struct VoiceMgr {
    msg_mask: i32,
    msg_request_state: i32,
    update_interval: f64,
    /// `g_PlayerModEnable`: the client answered `VModEnable 1`.
    player_mod_enable: u32,
    /// `g_bWantModEnable`: keep sending `ReqState` until the client answers.
    want_mod_enable: u32,
    /// `g_BanMasks`: players each client banned (`vban`).
    ban_masks: [u32; MAX_PLAYERS],
    /// `g_SentGameRulesMasks` / `g_SentBanMasks`: what the client was last told.
    sent_rules_masks: [u32; MAX_PLAYERS],
    sent_ban_masks: [u32; MAX_PLAYERS],
}

static MGR: Global<RefCell<Option<VoiceMgr>>> = Global(RefCell::new(None));

fn with<R>(f: impl FnOnce(&mut VoiceMgr) -> R) -> R {
    let mut m = MGR.0.borrow_mut();
    f(m.get_or_insert_with(VoiceMgr::default))
}

fn max_players() -> usize {
    (engine().max_clients().max(0) as usize).min(MAX_PLAYERS)
}

/// `CVoiceGameMgr::Init` (0xf7438): `VoiceMask` (8 bytes: two longs) and `ReqState` (0). TS calls
/// it from the `CHalfLifeMultiplay` constructor; the port registers once in `GameInit` (the
/// engines keep user messages for the process). The cvars are in `cvars::CVARS`.
pub fn register_messages() {
    let e = engine();
    let mask = e.reg_user_msg(c"VoiceMask", 8);
    let req = e.reg_user_msg(c"ReqState", 0);
    with(|m| {
        m.msg_mask = mask;
        m.msg_request_state = req;
    });
}

/// The precache half of `CVoiceGameMgr::Init`.
pub fn precache() {
    engine().precache_model(c"sprites/voiceicon.spr");
}

/// `CVoiceGameMgr::Update` (0xf750c), from `CHalfLifeMultiplay::Think`.
pub fn update(frametime: f64) {
    let due = with(|m| {
        m.update_interval += frametime;
        m.update_interval >= UPDATE_INTERVAL
    });
    if due {
        update_masks();
    }
}

/// `CVoiceGameMgr::ClientConnected` (0xf7548): ask for the client's `voice_modenable` and resend
/// both masks.
pub fn client_connected(e: *mut edict_t) {
    let i = engine().index_of(e) - 1;
    if !(0..MAX_PLAYERS as i32).contains(&i) {
        return;
    }
    with(|m| {
        m.want_mod_enable |= 1 << i;
        m.sent_rules_masks[i as usize] = 0;
        m.sent_ban_masks[i as usize] = 0;
    });
}

/// `CVoiceGameMgr::ClientCommand` (0xf770c): `vban <hex mask>` and `VModEnable <0|1>`. Returns
/// false for anything else so the other handlers see it.
pub fn client_command(e: *mut edict_t, cmd: &str, args: &Args) -> bool {
    let client = engine().index_of(e) - 1;
    let debug = cvars::on(c"voice_serverdebug");
    if client < 0 || client >= max_players() as i32 {
        if debug {
            engine().print(&format!("CVoiceGameMgr::ClientCommand: cmd {cmd} from invalid client ({client})\n"));
        }
        return true;
    }
    let c = client as usize;
    if cmd.eq_ignore_ascii_case("vban") && args.argc() > 1 {
        for i in 1..args.argc() {
            let mask = parse_hex(args.str(i));
            if i == 1 {
                if debug {
                    engine().print(&format!("CVoiceGameMgr::ClientCommand: vban (0x{mask:x}) from {client}\n"));
                }
                with(|m| m.ban_masks[c] = mask);
            } else if debug {
                engine().print(&format!("CVoiceGameMgr::ClientCommand: invalid index ({i})\n"));
            }
        }
    } else if cmd.eq_ignore_ascii_case("vmodenable") && args.argc() > 1 {
        let on = args.int(1) != 0;
        if debug {
            engine().print(&format!("CVoiceGameMgr::ClientCommand: VModEnable ({})\n", on as i32));
        }
        with(|m| {
            if on {
                m.player_mod_enable |= 1 << c;
            } else {
                m.player_mod_enable &= !(1 << c);
            }
            m.want_mod_enable &= !(1 << c);
        });
    } else {
        return false;
    }
    update_masks();
    true
}

/// `sscanf("%x")`: leading hex digits (optional `0x`), 0 if none.
pub fn parse_hex(s: &str) -> u32 {
    let s = s.trim_start();
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    let end = s.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(s.len());
    u32::from_str_radix(&s[..end.min(8)], 16).unwrap_or(0)
}

/// `CMultiplayGameMgrHelper::CanPlayerHearPlayer` (0xd0ed0). Teamplay: teammates only. Round
/// modes and LMS: a talker who is dead for the rest of the round (respawn time -1) is heard only
/// by listeners in the same state.
fn can_player_hear_player(listener: *mut edict_t, talker: *mut edict_t) -> bool {
    let (mode, teamplay, lms, _) = state::mode();
    if teamplay && !teams::same_team(listener, talker) {
        return false;
    }
    if matches!(mode, Mode::RoundTdm | Mode::Objective) || lms {
        let out_of_round = |e: *mut edict_t| !alive(e.v()) && other(e, |p| p.respawn_at < 0.0).unwrap_or(false);
        if out_of_round(talker) {
            return out_of_round(listener);
        }
    }
    true
}

/// One client's audible mask from the rules (`gameRulesMask` in `UpdateMasks`).
pub fn rules_mask(listener: usize, players: &[bool], all_talk: bool, can_hear: &dyn Fn(usize, usize) -> bool) -> u32 {
    let mut mask = 0;
    for (talker, &present) in players.iter().enumerate().take(MAX_PLAYERS) {
        if present && (all_talk || can_hear(listener, talker)) {
            mask |= 1 << talker;
        }
    }
    mask
}

/// `CVoiceGameMgr::UpdateMasks` (0xf79bc).
fn update_masks() {
    with(|m| m.update_interval = 0.0);
    let e = engine();
    let n = max_players();
    let all_talk = cvars::on(c"sv_alltalk");
    let edicts: Vec<*mut edict_t> = (1..=n as i32).map(|i| e.player(i)).collect();
    let present: Vec<bool> = edicts.iter().map(|&p| super::is_player(p)).collect();
    let (msg_mask, msg_req) = with(|m| (m.msg_mask, m.msg_request_state));
    for c in 0..n {
        if !present[c] {
            continue;
        }
        let ed = edicts[c];
        let (want, mod_enable, ban) = with(|m| (m.want_mod_enable & (1 << c) != 0, m.player_mod_enable & (1 << c) != 0, m.ban_masks[c]));
        if want {
            MsgWriter::begin(MSG_ONE as i32, msg_req, None, ed).end();
        }
        let mask = if mod_enable { rules_mask(c, &present, all_talk, &|l, t| can_player_hear_player(edicts[l], edicts[t])) } else { 0 };
        let changed = with(|m| {
            let changed = m.sent_rules_masks[c] != mask || m.sent_ban_masks[c] != ban;
            m.sent_rules_masks[c] = mask;
            m.sent_ban_masks[c] = ban;
            changed
        });
        if changed {
            MsgWriter::begin(MSG_ONE as i32, msg_mask, None, ed).long(mask as i32).long(ban as i32).end();
        }
        for o in 0..n {
            let hear = mask & (1 << o) != 0 && ban & (1 << o) == 0;
            e.voice_set_client_listening(c as i32 + 1, o as i32 + 1, hear);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vban_masks_parse_like_sscanf_hex() {
        assert_eq!(parse_hex("0"), 0);
        assert_eq!(parse_hex("1a"), 0x1a);
        assert_eq!(parse_hex("ffffffff"), u32::MAX);
        assert_eq!(parse_hex("0x10"), 0x10);
        assert_eq!(parse_hex("zz"), 0);
        assert_eq!(parse_hex("12g"), 0x12);
    }

    #[test]
    fn rules_mask_respects_presence_alltalk_and_rules() {
        let present = [true, true, false, true];
        // Player 0 hears only even talkers by the rules.
        let even = |_: usize, t: usize| t.is_multiple_of(2);
        assert_eq!(rules_mask(0, &present, false, &even), 0b0001);
        assert_eq!(rules_mask(0, &present, true, &even), 0b1011, "sv_alltalk ignores the rules, not presence");
    }
}
