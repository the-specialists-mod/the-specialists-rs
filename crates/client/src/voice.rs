//! Voice status and voice banning: TS's `CVoiceStatus` (SDK 2.x game_shared/voice_status.cpp,
//! `GetClientVoiceMgr` 10017570) and `CVoiceBanMgr` (`voice_ban.dt`). Reference: decomp
//! docs/client/architecture-hud.md §1-3.
//!
//! * The engine reports who is talking through `HUD_VoiceStatus` ([`update_speaker_status`]).
//! * Talking players get a `sprites/voiceicon.spr` over their head ([`create_entities`]) and a
//!   name label on the right of the screen; the local player gets a talk icon ([`redraw`]).
//! * The scoreboard shows each audible player's voice state, and clicking it enters squelch mode,
//!   where a click on a player row mutes or unmutes them ([`toggle_block`]). Bans persist in
//!   `<gamedir>/voice_ban.dt` and go to the server as `vban <mask>`, which stops it relaying
//!   that player's voice (`CVoiceGameMgr` in the server's `rules::voice`).
//!
//! TS drew the labels and icons with VGUI1 panels and TGA images; the port draws them in immediate
//! mode like its other VGUI replacements (docs/implementation/client-hud.md §7).

use crate::hud::{self, Dc};
use goldsrc::Global;
use goldsrc::client::{Cvar, engine};
use goldsrc::ext::voice::{SpriteEntity, add_sprite_entity};
use goldsrc::sys::*;
use std::cell::RefCell;

/// `MAX_VOICE_SPEAKERS`: speaker labels on screen at once.
pub const MAX_SPEAKERS: usize = 7;
/// `VOICE_MAX_PLAYERS`.
pub const MAX_PLAYERS: usize = 32;
/// `SCOREBOARD_BLINK_FREQUENCY`: talking icons alternate every 0.3 s.
pub const BLINK_FREQUENCY: f64 = 0.3;
/// `CreateEntities` puts the head icon this far above the player's origin.
pub const HEAD_ICON_HEIGHT: f32 = 45.0;
/// `BANMGR_FILEVERSION`.
pub const BAN_FILE_VERSION: i32 = 1;
const BAN_FILE: &str = "voice_ban.dt";
/// `HUD_PRINTTALK`.
const PRINT_TALK: u8 = 3;
const EF_NODRAW_BIT: i32 = EF_NODRAW as i32;

// ---------------------------------------------------------------------------------------------
// Ban list (`CVoiceBanMgr`)

/// `HashPlayerID`: the byte sum the SDK buckets ids by (the file is written bucket by bucket).
pub fn hash_player_id(id: &[u8; 16]) -> u8 {
    id.iter().fold(0u8, |h, &b| h.wrapping_add(b))
}

/// The banned unique ids, kept in the order the SDK's hash table would list them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BanList {
    ids: Vec<[u8; 16]>,
}

impl BanList {
    /// `CVoiceBanMgr::Init`: a little-endian version int, then 16-byte ids. Wrong versions are
    /// ignored; a trailing partial id is dropped.
    pub fn parse(bytes: &[u8]) -> Self {
        let mut list = BanList::default();
        let Some(v) = bytes.get(..4) else { return list };
        if i32::from_le_bytes([v[0], v[1], v[2], v[3]]) != BAN_FILE_VERSION {
            return list;
        }
        for id in bytes[4..].as_chunks::<16>().0 {
            list.set(id, true);
        }
        list
    }

    /// `CVoiceBanMgr::SaveState`.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = BAN_FILE_VERSION.to_le_bytes().to_vec();
        for id in self.iter() {
            out.extend_from_slice(id);
        }
        out
    }

    /// Ids in hash-bucket order, then insertion order (`ForEachBannedPlayer`).
    pub fn iter(&self) -> impl Iterator<Item = &[u8; 16]> {
        (0..=255u8).flat_map(move |b| self.ids.iter().filter(move |id| hash_player_id(id) == b))
    }

    pub fn contains(&self, id: &[u8; 16]) -> bool {
        self.ids.contains(id)
    }

    /// `SetPlayerBan`.
    pub fn set(&mut self, id: &[u8; 16], banned: bool) {
        match (banned, self.contains(id)) {
            (true, false) => self.ids.push(*id),
            (false, true) => self.ids.retain(|x| x != id),
            _ => {}
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// `ForEachBannedPlayer`'s line. The SDK prints each id byte with `%2x` from a signed `char`, so
/// bytes from 0x80 come out sign-extended (`ffffff80`); on Windows the line is upper-cased.
pub fn ban_line(n: usize, id: &[u8; 16], upper: bool) -> String {
    let hex: String = id.iter().map(|&b| format!("{:2x}", b as i8 as i32 as u32)).collect();
    let s = format!("Ban {n}: {hex}\n");
    if upper { s.to_uppercase() } else { s }
}

// ---------------------------------------------------------------------------------------------
// State

/// Scoreboard voice icon for one player (`UpdateBanButton` 100185f0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeakerIcon {
    /// Not audible under the server's rules: no icon.
    None,
    Banned,
    /// Talking; `bright` alternates every [`BLINK_FREQUENCY`] (`640_speaker4`/`640_speaker3`).
    Talking {
        bright: bool,
    },
    NeverSpoken,
    NotTalking,
}

#[derive(Default)]
struct Voice {
    /// `m_VoicePlayers`: talking now (bit = client index - 1).
    talking: u32,
    /// `m_VoiceEnabledPlayers`: has talked since the client started.
    spoken: u32,
    /// `m_AudiblePlayers` and `m_ServerBannedPlayers` from `VoiceMask`.
    audible: u32,
    server_banned: u32,
    /// `m_bServerModEnable`: what the server was last told (-1 = unknown).
    server_mod_enable: i32,
    last_update_server_state: f32,
    blink_timer: f64,
    /// `m_Labels[i].m_clientindex` (0-based client, `None` = free).
    labels: [Option<usize>; MAX_SPEAKERS],
    /// `m_bTalking`, `m_bServerAcked`.
    local_talking: bool,
    server_acked: bool,
    squelch: bool,
    bans: BanList,
    ban_path: String,
    head_sprite: HSPRITE,
    modenable: Option<Cvar>,
    clientdebug: Option<Cvar>,
}

static VOICE: Global<RefCell<Option<Voice>>> = Global(RefCell::new(None));

/// The closure must not call back into this module.
fn with<R>(f: impl FnOnce(&mut Voice) -> R) -> R {
    let mut v = VOICE.0.borrow_mut();
    f(v.get_or_insert_with(|| Voice { server_mod_enable: -1, ..Voice::default() }))
}

fn debug() -> bool {
    with(|v| v.clientdebug.is_some_and(|c| c.value() != 0.0))
}

fn bit(i: usize) -> u32 {
    if i < MAX_PLAYERS { 1 << i } else { 0 }
}

// ---------------------------------------------------------------------------------------------
// Init / frame

/// `CVoiceStatus::Init` (100178c0): cvars, `voice_showbanned`, the ban file, `VoiceMask` and
/// `ReqState` hooks.
pub fn init() {
    let e = engine();
    let modenable = Cvar::register(c"voice_modenable", c"1", 1);
    let clientdebug = Cvar::register(c"voice_clientdebug", c"0", 0);
    e.add_command(c"voice_showbanned", show_banned);
    let path = format!("{}/{BAN_FILE}", e.game_directory());
    let bans = std::fs::read(&path).map(|b| BanList::parse(&b)).unwrap_or_default();
    with(|v| {
        *v = Voice {
            server_mod_enable: -1,
            modenable: Some(modenable),
            clientdebug: Some(clientdebug),
            bans,
            ban_path: path,
            ..Voice::default()
        };
    });
    e.hook_user_msg(c"VoiceMask", msg_voice_mask);
    e.hook_user_msg(c"ReqState", msg_req_state);
}

/// `CVoiceStatus::VidInit` (10017af0): the head icon sprite. TS also reads
/// `scripts/voicemodel.txt` into `m_VoiceHeadModelHeight` (`+0x177e0`), which nothing reads:
/// `CreateEntities` (`0x10018053`) places the icon at a constant 45 units. The read has no
/// effect, so the port skips it.
pub fn vid_init() {
    let h = engine().spr_load("sprites/voiceicon.spr");
    with(|v| v.head_sprite = h);
}

/// `CVoiceStatus::Frame` (10017ea0), from `HUD_Frame`: tell the server about ban changes once a
/// second, and run the scoreboard blink timer.
pub fn frame(frametime: f64) {
    let now = engine().client_time();
    if now - with(|v| v.last_update_server_state) > 1.0 {
        update_server_state(false);
    }
    with(|v| v.blink_timer += frametime);
}

extern "C" fn show_banned() {
    let e = engine();
    e.print("------- BANNED PLAYERS -------\n");
    let lines: Vec<String> = with(|v| v.bans.iter().enumerate().map(|(n, id)| ban_line(n, id, cfg!(windows))).collect());
    for l in lines {
        e.print(&l);
    }
    e.print("------------------------------\n");
}

// ---------------------------------------------------------------------------------------------
// Messages

/// `VoiceMask` (`HandleVoiceMaskMsg` 10018780): audible and server-banned masks. TS then reads a
/// byte the 8-byte message doesn't have; the failed read (-1) makes the next state update resend
/// `VModEnable`, which the port keeps.
pub fn on_voice_mask(buf: &[u8]) -> Option<(u32, u32, i32)> {
    let long = |o: usize| buf.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    Some((long(0)?, long(4)?, buf.get(8).map_or(-1, |&b| b as i32)))
}

fn msg_voice_mask(buf: &[u8]) {
    let Some((audible, banned, mod_enable)) = on_voice_mask(buf) else { return };
    with(|v| {
        v.audible = audible;
        v.server_banned = banned;
        v.server_mod_enable = mod_enable;
    });
    if debug() {
        let e = engine();
        e.print("CVoiceStatus::HandleVoiceMaskMsg\n");
        e.print(&format!("    - m_AudiblePlayers[0] = {audible}\n"));
        e.print(&format!("    - m_ServerBannedPlayers[0] = {banned}\n"));
    }
}

/// `ReqState` (`HandleReqStateMsg` 10018830): resend everything.
fn msg_req_state(_buf: &[u8]) {
    if debug() {
        engine().print("CVoiceStatus::HandleReqStateMsg\n");
    }
    update_server_state(true);
}

/// The `vban` command for a ban mask (one 32-player dword).
pub fn vban_command(mask: u32) -> String {
    format!("vban {mask:x}")
}

/// `CVoiceStatus::UpdateServerState` (10018360): `VModEnable` when `voice_modenable` changed (or
/// `force`), `vban` when the local ban mask differs from the server's (or `force`).
pub fn update_server_state(force: bool) {
    let e = engine();
    if goldsrc::ext::render::level_name().is_empty() {
        if debug() {
            e.print("CVoiceStatus::UpdateServerState: pLevelName[0]==0\n");
        }
        return;
    }
    let dbg = debug();
    let enable = with(|v| v.modenable.is_some_and(|c| c.value() != 0.0)) as i32;
    if force || with(|v| v.server_mod_enable) != enable {
        with(|v| v.server_mod_enable = enable);
        let cmd = format!("VModEnable {enable}");
        e.server_cmd(&cmd);
        if dbg {
            e.print(&format!("CVoiceStatus::UpdateServerState: Sending '{cmd}'\n"));
        }
    }
    let ids: Vec<Option<[u8; 16]>> = (1..=MAX_PLAYERS as i32).map(|i| e.player_unique_id(i)).collect();
    let (ban_mask, server_mask) = with(|v| {
        let (mut ban, mut server) = (0u32, 0u32);
        for (i, id) in ids.iter().enumerate() {
            let Some(id) = id else { continue };
            if v.bans.contains(id) {
                ban |= bit(i);
            }
            server |= v.server_banned & bit(i);
        }
        (ban, server)
    });
    let cmd = vban_command(ban_mask);
    if force || ban_mask != server_mask {
        if dbg {
            e.print(&format!("CVoiceStatus::UpdateServerState: Sending '{cmd}'\n"));
        }
        e.server_cmd_unreliable(&cmd);
    } else if dbg {
        e.print("CVoiceStatus::UpdateServerState: no change\n");
    }
    with(|v| v.last_update_server_state = e.client_time());
}

// ---------------------------------------------------------------------------------------------
// Speakers

/// Pure part of `UpdateSpeakerStatus`. Returns the client that just got a speaker label.
fn speaker_status(v: &mut Voice, entindex: i32, talking: bool) -> Option<usize> {
    match entindex {
        // The local player (only the HUD talk icon).
        -1 => v.local_talking = talking,
        // The server acknowledged our voice data.
        -2 => v.server_acked = talking,
        // TS checks `entindex < 32` (the SDK `<= VOICE_MAX_PLAYERS`), so player 32 never shows.
        1..=31 => {
            let c = entindex as usize - 1;
            let label = v.labels.iter().position(|l| *l == Some(c));
            if talking {
                v.talking |= bit(c);
                v.spoken |= bit(c);
                if label.is_none()
                    && let Some(free) = v.labels.iter().position(|l| l.is_none())
                {
                    v.labels[free] = Some(c);
                    return Some(c);
                }
            } else {
                v.talking &= !bit(c);
                if let Some(l) = label {
                    v.labels[l] = None;
                }
            }
        }
        _ => {}
    }
    None
}

/// `HUD_VoiceStatus` → `CVoiceStatus::UpdateSpeakerStatus` (100180c0). `entindex` -1 is the local
/// player, -2 the server's acknowledgement, otherwise a player index.
pub fn update_speaker_status(entindex: i32, talking: bool) {
    let e = engine();
    if debug() {
        e.print(&format!("CVoiceStatus::UpdateSpeakerStatus: ent {entindex} talking = {}\n", talking as i32));
    }
    if entindex == -1 && talking {
        // Talking turns voice on for this mod.
        e.client_cmd("voice_modenable 1");
    }
    with(|v| speaker_status(v, entindex, talking));
}

/// `CVoiceStatus::IsInSquelchMode` (100188b0): the scoreboard has the cursor for muting.
pub fn in_squelch_mode() -> bool {
    with(|v| v.squelch)
}

/// `StartSquelchMode` (10018870).
pub fn start_squelch_mode() {
    with(|v| v.squelch = true);
}

/// `StopSquelchMode` (10018890), when the scoreboard hides.
pub fn stop_squelch_mode() {
    with(|v| v.squelch = false);
}

/// `IsPlayerBlocked` (10018c80): the player's unique id is in the ban list.
pub fn is_player_blocked(player: i32) -> bool {
    engine().player_unique_id(player).is_some_and(|id| with(|v| v.bans.contains(&id)))
}

/// `IsPlayerAudible` (10018cc0): the server's rules let us hear the player.
pub fn is_player_audible(player: i32) -> bool {
    player >= 1 && with(|v| v.audible & bit(player as usize - 1) != 0)
}

/// Icon for a scoreboard row (`UpdateBanButton` 100185f0).
pub fn speaker_icon(player: i32) -> SpeakerIcon {
    let Some(id) = (player >= 1).then(|| engine().player_unique_id(player)).flatten() else { return SpeakerIcon::None };
    let c = player as usize - 1;
    with(|v| {
        if v.audible & bit(c) == 0 {
            SpeakerIcon::None
        } else if v.bans.contains(&id) {
            SpeakerIcon::Banned
        } else if v.talking & bit(c) != 0 {
            SpeakerIcon::Talking { bright: v.blink_timer % (BLINK_FREQUENCY * 2.0) < BLINK_FREQUENCY }
        } else if v.spoken & bit(c) == 0 {
            SpeakerIcon::NeverSpoken
        } else {
            SpeakerIcon::NotTalking
        }
    })
}

/// `CVoiceStatus::~CVoiceStatus` (`0x100177e0`, the static `g_VoiceStatus` torn down when the
/// library unloads) saves the bans (`CVoiceBanMgr::SaveState` `0x10017300`). The port has no
/// static destructors and does it from `HUD_Shutdown`, which the engine calls just before
/// unloading the library.
pub fn shutdown() {
    save_bans();
}

fn save_bans() {
    let (path, bytes) = with(|v| (v.ban_path.clone(), v.bans.serialize()));
    let tmp = format!("{path}.tmp");
    if std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
        engine().print(&format!("[ts] couldn't write {path}\n"));
    }
}

/// `SetPlayerBlockedState` (10018cf0): TS toggles the ban, and only for audible players.
/// `voice_ban.dt` is written when the library shuts down ([`shutdown`]).
pub fn toggle_block(player: i32) {
    let e = engine();
    let dbg = debug();
    if dbg {
        e.print("CVoiceStatus::SetPlayerBlockedState part 1\n");
    }
    let Some(id) = e.player_unique_id(player) else { return };
    if dbg {
        e.print("CVoiceStatus::SetPlayerBlockedState part 2\n");
    }
    if !is_player_audible(player) {
        return;
    }
    let banned = with(|v| {
        let now = !v.bans.contains(&id);
        v.bans.set(&id, now);
        now
    });
    if dbg {
        e.print(&format!("CVoiceStatus::SetPlayerBlockedState: setting player {player} ban to {}\n", banned as i32));
    }
    update_server_state(false);
}

/// `ScorePanel::mousePressed` (1005e660) on a player row in squelch mode: toggle the mute and say
/// so in the chat area (`"%c** %s\n"` through `CHudTextMessage`).
pub fn scoreboard_toggle(player: i32) {
    let e = engine();
    let name = e.player_info(player).name;
    if name.is_empty() {
        return;
    }
    let was_blocked = is_player_blocked(player);
    toggle_block(player);
    let fmt = |key: &str| hud::text::format(&hud::text::localise(key), &[hud::text::Arg::S(&name)]);
    let line = if was_blocked {
        format!("** {}\n", fmt("#Unmuted"))
    } else {
        format!("** {} {}\n", fmt("#Muted"), hud::text::localise("#No_longer_hear_that_player"))
    };
    hud::print_text_message(PRINT_TALK, &line);
}

// ---------------------------------------------------------------------------------------------
// Drawing

/// `CVoiceStatus::CreateEntities` (10017f30), from `HUD_CreateEntities`: a voice icon above each
/// talking player in the PVS who is drawn, and above the local player only in third person.
pub fn create_entities() {
    let (sprite, talking) = with(|v| (v.head_sprite, v.talking));
    if sprite == 0 {
        return;
    }
    let e = engine();
    let Some(local) = e.local_player() else { return };
    let third = crate::view::is_third_person();
    let mut slot = 0;
    for i in 0..MAX_PLAYERS {
        if talking & bit(i) == 0 {
            continue;
        }
        let Some(p) = e.entity(i as i32 + 1) else { continue };
        if p.curstate.messagenum < local.curstate.messagenum || p.curstate.effects & EF_NODRAW_BIT != 0 {
            continue;
        }
        if p.index == local.index && !third {
            continue;
        }
        let origin = [p.origin[0], p.origin[1], p.origin[2] + HEAD_ICON_HEIGHT];
        add_sprite_entity(
            slot,
            &SpriteEntity {
                sprite,
                origin,
                scale: 0.5,
                rendermode: kRenderTransAdd as i32,
                renderfx: kRenderFxNoDissipation as i32,
                renderamt: 255,
            },
        );
        slot += 1;
    }
}

/// Speaker label layout (`RepositionLabels` 10018900): right-aligned boxes stacked down from
/// mid-screen. Returns (x, y, w, h) per label; text width is capped at 2/3 of the screen.
pub fn label_rects(screen_w: i32, screen_h: i32, text_sizes: &[(i32, i32)], icon: (i32, i32)) -> Vec<(i32, i32, i32, i32)> {
    let border = 2;
    let mut y = screen_h / 2;
    text_sizes
        .iter()
        .map(|&(tw, th)| {
            let tw = tw.min(screen_w * 2 / 3);
            let w = tw + icon.0 + border * 3;
            let h = th.max(icon.1) + border * 2;
            let r = (screen_w - w - 8, y, w, h);
            y += h + 2;
            r
        })
        .collect()
}

/// TS `GetPlayerTextColor` (helper 1001fcd0): the team colour table by team number.
fn player_color(player: i32) -> [i32; 3] {
    let team = hud::with(|h| h.players.get(player as usize).map_or(0, |p| p.team));
    hud::team_color(team)
}

/// A 16×16 speaker glyph drawn with fills (TS used `speaker4.tga` and the `640_speaker*` TGAs,
/// which the engine API can't draw).
pub fn draw_speaker(dc: &Dc, x: i32, y: i32, icon: SpeakerIcon) {
    let (rgb, waves) = match icon {
        SpeakerIcon::None => return,
        SpeakerIcon::Banned => ([255, 40, 40], 0),
        SpeakerIcon::Talking { bright: true } => ([255, 170, 0], 2),
        SpeakerIcon::Talking { bright: false } => ([255, 170, 0], 1),
        SpeakerIcon::NeverSpoken => ([100, 100, 100], 0),
        SpeakerIcon::NotTalking => ([200, 200, 200], 0),
    };
    let a = 255;
    dc.fill(x + 1, y + 6, 3, 4, rgb, a);
    dc.fill(x + 4, y + 4, 2, 8, rgb, a);
    dc.fill(x + 6, y + 2, 2, 12, rgb, a);
    for w in 0..waves {
        dc.fill(x + 10 + w * 3, y + 5 - w * 2, 1, 6 + w * 4, rgb, a);
    }
    if icon == SpeakerIcon::Banned {
        for i in 0..12 {
            dc.fill(x + 2 + i, y + 2 + i, 1, 1, rgb, a);
            dc.fill(x + 13 - i, y + 2 + i, 1, 1, rgb, a);
        }
    }
}

/// The VGUI-panel part of `CVoiceStatus`, drawn after the HUD: speaker labels (hidden while the
/// scoreboard is up, `CanShowSpeakerLabels` 1001fd60) and the local talk icon at
/// `GetAckIconHeight` (1001fd40: screen height − 3 font heights − 6).
pub fn redraw() {
    let (labels, local, acked, sprite) = with(|v| (v.labels, v.local_talking, v.server_acked, v.head_sprite));
    let show_labels = !crate::menus::scores_visible();
    let dbg = debug();
    hud::with_dc(|dc| {
        let e = engine();
        if show_labels {
            let shown: Vec<usize> = labels.iter().flatten().copied().collect();
            let names: Vec<String> = shown.iter().map(|&c| format!("{}   ", e.player_info(c as i32 + 1).name)).collect();
            let sizes: Vec<(i32, i32)> = names.iter().map(|n| e.console_string_size(n)).collect();
            let icon = (16, 16);
            for ((rect, name), &c) in label_rects(dc.w(), dc.h(), &sizes, icon).into_iter().zip(&names).zip(&shown) {
                let (x, y, w, h) = rect;
                dc.fill(x, y, w, h, player_color(c as i32 + 1), 100);
                let th = e.console_string_size(name).1;
                e.draw_console_string(x + 2, y + (h - th) / 2, [1.0; 3], name);
                draw_speaker(dc, x + w - 2 - icon.0, y + (h - icon.1) / 2, SpeakerIcon::Talking { bright: true });
            }
        }
        // The engine's "you are talking" icon (TS `icntlk_pl.tga`; `icntlk_sv.tga` when the server
        // acknowledged and voice_clientdebug is on).
        if (local || acked) && sprite != 0 {
            let (sw, sh) = (e.spr_width(sprite, 0), e.spr_height(sprite, 0));
            let ack_height = dc.h() - (dc.g.screen.char_height * 3 + 6);
            let rgb = if acked && dbg { [100, 255, 100] } else { [255, 255, 255] };
            dc.spr(sprite, dc.w() - sw - 10, ack_height - sh, rgb);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> [u8; 16] {
        let mut id = [0u8; 16];
        id[0] = b;
        id
    }

    #[test]
    fn ban_file_round_trip_and_version() {
        let mut l = BanList::default();
        l.set(&id(3), true);
        l.set(&id(1), true);
        l.set(&id(1), true);
        assert_eq!(l.len(), 2);
        let bytes = l.serialize();
        assert_eq!(&bytes[..4], &1i32.to_le_bytes());
        assert_eq!(bytes.len(), 4 + 32);
        // Written bucket by bucket (hash = byte sum): id(1) before id(3).
        assert_eq!(bytes[4], 1);
        let back = BanList::parse(&bytes);
        assert!(back.contains(&id(1)) && back.contains(&id(3)));
        let mut wrong = bytes.clone();
        wrong[0] = 2;
        assert!(BanList::parse(&wrong).is_empty());
        assert!(BanList::parse(&bytes[..20]).len() == 1, "partial trailing id dropped");
        l.set(&id(3), false);
        assert!(!l.contains(&id(3)));
    }

    #[test]
    fn showbanned_line_matches_sdk_printf() {
        let mut x = [0u8; 16];
        x[0] = 0x0a;
        x[1] = 0x80;
        let l = ban_line(0, &x, false);
        assert!(l.starts_with("Ban 0:  affffff80 0 0"), "{l}");
        assert!(ban_line(1, &x, true).starts_with("BAN 1:  AFFFFFF80"));
    }

    #[test]
    fn voice_mask_message() {
        let mut b = 5u32.to_le_bytes().to_vec();
        b.extend_from_slice(&2u32.to_le_bytes());
        assert_eq!(on_voice_mask(&b), Some((5, 2, -1)));
        b.push(1);
        assert_eq!(on_voice_mask(&b), Some((5, 2, 1)));
        assert_eq!(on_voice_mask(&b[..6]), None);
        assert_eq!(vban_command(0x1f), "vban 1f");
    }

    #[test]
    fn speaker_labels_follow_talkers() {
        let mut v = Voice { server_mod_enable: -1, ..Voice::default() };
        assert_eq!(speaker_status(&mut v, 3, true), Some(2));
        assert_eq!(v.talking, 0b100);
        assert_eq!(v.spoken, 0b100);
        assert_eq!(v.labels[0], Some(2));
        assert_eq!(speaker_status(&mut v, 3, true), None, "already labelled");
        speaker_status(&mut v, 3, false);
        assert_eq!(v.talking, 0);
        assert_eq!(v.spoken, 0b100, "keeps 'has spoken'");
        assert_eq!(v.labels[0], None);
        speaker_status(&mut v, 32, true);
        assert_eq!(v.talking, 0, "TS ignores player 32");
        speaker_status(&mut v, -1, true);
        assert!(v.local_talking && v.talking == 0);
        for i in 1..=8 {
            speaker_status(&mut v, i, true);
        }
        assert_eq!(v.labels.iter().flatten().count(), MAX_SPEAKERS);
        assert_eq!(v.talking, 0xff, "talking is tracked even without a free label");
    }

    #[test]
    fn label_layout() {
        let r = label_rects(640, 480, &[(100, 12), (900, 12)], (16, 16));
        assert_eq!(r[0], (640 - (100 + 16 + 6) - 8, 240, 122, 20));
        assert_eq!(r[1].1, 240 + 20 + 2);
        assert_eq!(r[1].2, 426 + 16 + 6, "text capped at 2/3 of the screen");
    }
}
