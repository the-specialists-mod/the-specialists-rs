//! Rules client commands (`ClientCommand` 0xad838: say, respawn, power-ups, teams, motd ...) and
//! the TS server commands (`GameDLLInit`: globalslow, restartround, addbot, weaponrestriction ...).

use super::lifecycle::{give_cash, respawn_command, send_motd};
use super::msg;
use super::powerup::{add_powerup, use_powerup};
use super::{alive, cvars, frame, netname, other, state, teams, votes};
use crate::commands::Args;
use crate::messages::Msg;
use crate::player::Player;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::hud_wire::{PwUp, Spectator};
use shared::protocol::rules::{SrvSett, VguiMenu, hud_dest};
use shared::ts::powerup as pw;

pub fn client_command(p: &mut Player, cx: &mut Cx, cmd: &str, args: &Args) -> bool {
    match cmd {
        "say" => host_say(p, cx, args, false),
        "say_team" => host_say(p, cx, args, true),
        "respawn" => respawn_command(p, cx),
        "usepowerup" => {
            use_powerup(p, cx);
        }
        "powerup" if cvars::cheats() && args.argc() > 1 => cheat_powerup(p, cx, args.int(1)),
        "cashala" if cvars::cheats() => give_cash(p, 30000),
        // The cheat sets `m_iFOV` (+0x9f0) and `pev->fov`; the reply prints `m_iFOV`, which only
        // this command sets (a scope zooms `pev->fov` alone, so it prints 0 then).
        "fov" => {
            if cvars::cheats() && args.argc() > 1 {
                p.ts.ifov = args.int(1);
                cx.pev().fov = p.ts.ifov as f32;
            } else {
                let fov = p.ts.ifov;
                engine().client_print(cx.edict, PRINT_TYPE::print_console, &format!("\"fov\" is \"{fov}\"\n"));
            }
        }
        // `CBasePlayer::SelectItem(argv(1))` (0xdade4): switch to the held item of that class
        // (`strcmp`) unless it is `m_pActiveItem`: between `weapon_tsgun` and a picked-up
        // `cycler_weapon`. With only the gun the command changes nothing.
        "use" => crate::weapons::select_item(p, cx, args.argv(1)),
        // No teamplay check (0xae49c): the TS client ignores menu 2 unless `GameMode` said
        // teamplay (`ShowVGUIMenu` 0x10064050 -> 0x10064280).
        "changeteam" => msg::one(Msg::VguiMenu, cx.edict, |w| VguiMenu { menu: 2 }.write(w)),
        // `ChangePlayerTeam(GetIndexedTeamName(n - 1), 0, 0)` with no other checks: rejoining
        // the same team, or an out-of-range number (the smallest team), still sends the player
        // to observer. Outside teamplay `CGameRules::ChangePlayerTeam` (0xc632c) is empty.
        "jointeam" if args.argc() > 1 => {
            if state::teamplay() {
                let name = state::with(|g| g.indexed_team_name(args.int(1).wrapping_sub(1)));
                teams::change_team(p, cx, &name, false, false);
            }
        }
        // Only when `IsMultiplayer()`.
        "motd" => {
            if !state::single_player() {
                send_motd(cx.edict);
            }
        }
        "fullupdate" => force_client_dll_update(p, cx),
        c if c.starts_with("cutwire_") => crate::entities::ts::objective::cut_wire_command(p, cx, &c[8..]),
        // Past `ClientCommand`'s own names, `CHalfLifeMultiplay::ClientCommand` (0xceee8): the voice
        // manager (`vban`/`VModEnable` by `strcasecmp`, the only case-insensitive names), then
        // `menuselect`. `CHalfLifeRules` keeps `CGameRules::ClientCommand`, which takes nothing.
        _ if state::single_player() => return false,
        _ if super::voice::client_command(cx.edict, p.respawn_at, cmd, args) => {}
        "menuselect" => {}
        _ => return false,
    }
    true
}

/// `CBasePlayer::ForceClientDllUpdate` (0x92594), the `fullupdate` command and impulse 204:
/// forget the player's "last sent" HUD values (not `TSPAward`, nor the gun's own caches), flag a
/// new train HUD, reset the throttle and run `UpdateClientData` now, then tell the player whether
/// it is an observer. Unlike the SDK, TS doesn't set `m_fInitHUD`, so no `ResetHUD` goes out
/// unless one was already pending.
pub fn force_client_dll_update(p: &mut Player, cx: &mut Cx) {
    crate::player::hud::force_reset(p);
    p.drive.train |= crate::entities::stock::drive::TRAIN_NEW;
    crate::player::hud::update_client_data(p, cx);
    let s = fullupdate_spectator(cx.index(), cx.pev().deadflag);
    msg::one(Msg::Spectator, cx.edict, |w| s.write(w));
}

/// The `Spectator` that ends `ForceClientDllUpdate`: observer is `deadflag == DEAD_RESPAWNABLE`
/// (0x9267b), which `StartObserver` sets, not `iuser1`.
fn fullupdate_spectator(idx: i32, deadflag: i32) -> Spectator {
    Spectator { entindex: idx as u8, observer: (deadflag == DEAD_RESPAWNABLE as i32) as u8 }
}

/// `powerup <n>` (cheat, `ClientCommand` 0xad838): 1 slow pause, 2 grenade, 3 kung fu + super
/// jump, 4 multiply clip, 5 double fire rate, other slow motion; +5 s if the same is held. Unlike
/// a pickup (`PlayerAddPowerUp`) it plays no sound, and 3 sets both and sends both `PwUp`s even
/// when the player already has them.
pub fn cheat_powerup(p: &mut Player, cx: &mut Cx, n: i32) {
    if !alive(cx.pev()) || cx.pev().iuser1 != 0 {
        return;
    }
    let bits = match n {
        1 => pw::SLOWPAUSE,
        2 => {
            // `PickUpWeapon(24, NULL, 0, 1, 0)`.
            crate::weapons::give_weapon(p, cx, 24, 0);
            return;
        }
        3 => {
            let e = cx.edict;
            p.kungfu = true;
            msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::KUNGFU as i16, secs: 0 }.write(w));
            p.superjump = true;
            engine().set_physics_key(e, c"mj", c"1");
            msg::one(Msg::PwUp, e, |w| PwUp { bits: pw::SUPERJUMP as i16, secs: 0 }.write(w));
            return;
        }
        4 => {
            crate::weapons::multiply_clip(p, cx);
            return;
        }
        5 => pw::DOUBLE_FIRERATE,
        _ => pw::SLOWMO,
    };
    if p.active_powerup == 0 {
        msg::one(Msg::PwUp, cx.edict, |w| PwUp { bits: bits as i16, secs: 0 }.write(w));
        p.ts.held_secs = if p.ts.held == bits { p.ts.held_secs + 5 } else { 5 };
        p.ts.held = bits;
        p.powerup_until = 0.0;
    }
}

/// `Host_Say`'s prefix, `"%c%s: "` or `"%c(TEAM) %s: "` with `\x02` and the sender's name (its
/// bytes as they are: TS `sprintf`s `STRING(pev->netname)`).
pub fn say_prefix(name: &[u8], team: bool) -> Vec<u8> {
    if team { shared::bformat!("\x02(TEAM) {}: ", name) } else { shared::bformat!("\x02{}: ", name) }
}

/// The text as `Host_Say` cuts it, in place, so that prefix and text fit in 126 bytes (the SDK's
/// `sizeof(text) - 2 - strlen(text)` with `char text[128]`): 3.0 `p[0x7f - (strlen(prefix) + 1)]
/// = 0` (0xad02c, the `repnz scasb` count includes the terminator), 2.1 `p[0x7e -
/// strlen(prefix)] = 0` (0x1000cf00). A byte count, which may split a multi-byte UTF-8 character
/// as TS does. The log line shows the cut text too.
pub fn clip_say_text<'a>(prefix: &[u8], text: &'a [u8]) -> &'a [u8] {
    &text[..126usize.saturating_sub(prefix.len()).min(text.len())]
}

/// The longest chat line (before its `\n`) that fits `Host_Say`'s 140-byte buffer with the `\n`
/// and the terminator.
const SAY_LINE_MAX: usize = 138;

/// Builds the chat line from [`say_prefix`] and the [`clip_say_text`] text: `%a` becomes
/// "no kevlar"/"with kevlar", `%h` the health as `(int)` truncates it (GCC's `fistp` under a
/// truncating control word, 0x9d2a8), any other `%` stays, then `\n`.
///
/// Guard (stack overflow in TS): TS `strcat`s the expansions into a 140-byte stack buffer
/// without a bound, so enough `%a`s overrun it. The port stops at the buffer's size, which also
/// keeps the `SayText` under the engines' 192-byte user-message limit (ReHLDS/HLDS `Host_Error`
/// on a longer one).
/// The text's bytes are copied unchanged (`strncat(p, c, 1)`), whatever their encoding.
pub fn format_say(prefix: &[u8], text: &[u8], armor: f32, health: f32) -> Vec<u8> {
    let mut out = prefix[..prefix.len().min(SAY_LINE_MAX)].to_vec();
    let push = |out: &mut Vec<u8>, s: &[u8]| {
        let room = SAY_LINE_MAX.saturating_sub(out.len());
        out.extend_from_slice(&s[..s.len().min(room)]);
    };
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c == b'%' {
            match text.get(i + 1) {
                Some(b'a') => {
                    i += 1;
                    push(&mut out, if armor == 0.0 { b"no kevlar" } else { b"with kevlar" });
                }
                Some(b'h') => {
                    i += 1;
                    push(&mut out, crate::entities::stock::maprules::x87_int(health).to_string().as_bytes());
                }
                _ => push(&mut out, b"%"),
            }
        } else {
            push(&mut out, &[c]);
        }
        i += 1;
    }
    out.push(b'\n');
    out
}

/// How a ruleset's `Host_Say` treats the chat text ([`ServerRules::say_rules`](crate::gameplay::ServerRules::say_rules)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SayRules {
    /// `%a` and `%h` in the text become the armour and health ([`format_say`]); false appends
    /// the text as it is ([`plain_say_line`]).
    pub expand_codes: bool,
    /// The whole text is lower-cased (A-Z) once the chat line is built, before the `timeleft`,
    /// `ff` and `vote` tests, so they and the log see it lowered. False lowers only the text
    /// after `vote`, once it matched.
    pub lower_all: bool,
    /// A text starting with `ff` answers the sender (a net client) "Friendly fire is on/off" and
    /// isn't broadcast, only logged.
    pub ff_reply: bool,
}

/// The chat line as the prefix, the [`clip_say_text`] text as it is and `\n` (a ruleset whose
/// [`SayRules::expand_codes`] is off). The prefix and the text fit in 126 bytes, so the line
/// needs no guard.
pub fn plain_say_line(prefix: &[u8], text: &[u8]) -> Vec<u8> {
    let mut out = prefix.to_vec();
    out.extend_from_slice(text);
    out.push(b'\n');
    out
}

/// The `ff` reply: `"Friendly fire is %s\n"` with "on" for any `mp_friendlyfire` but 0 (a NaN
/// included: `fcomp` against 0 and `jp`, 2.1 0x1000d19e..0x1000d1b3).
pub fn friendly_fire_text(friendlyfire: f32) -> String {
    format!("Friendly fire is {}\n", if friendlyfire != 0.0 { "on" } else { "off" })
}

/// `Host_Say`'s dead-chat gate, for a message to `other`: with the gate on, a sender out for the
/// round (dead, `respawn_at < 0`) only reaches other such players.
pub fn say_reaches(gated: bool, me_alive: bool, my_respawn: f32, other_alive: bool, other_respawn: f32) -> bool {
    !gated || me_alive || my_respawn >= 0.0 || (!other_alive && other_respawn < 0.0)
}

/// `Host_Say`'s chat text from `Cmd_Args`: a leading `"` is dropped together with the last
/// byte, whatever it is (`p[strlen(p) - 1] = 0` on the original string).
pub fn say_text(raw: &[u8]) -> &[u8] {
    match raw.strip_prefix(b"\"") {
        Some(t) => &t[..t.len().saturating_sub(1)],
        None => raw,
    }
}

/// `Host_Say`'s empty-line test: the text needs a printable, non-space character
/// (`__ctype_b[c] & (_ISprint | _ISspace) == _ISprint`, C locale, so bytes from 0x80 don't count).
pub fn say_has_text(text: &[u8]) -> bool {
    text.iter().any(|b| b.is_ascii_graphic())
}

/// `Host_Say` (0xad02c; 2.1 0x1000cf00, the differences in the ruleset's [`SayRules`] and
/// [`say_dead_chat_gated`](crate::gameplay::ServerRules::say_dead_chat_gated)). `vote …` lines go
/// to `PlayerVote` first.
fn host_say(p: &mut Player, cx: &mut Cx, args: &Args, team: bool) {
    let now = cx.time();
    if now < p.ts.next_say || args.argc() < 2 {
        return;
    }
    // The bytes of `Cmd_Args` as the client sent them (issue #81: no UTF-8 conversion).
    let raw = args.rest().to_bytes().to_vec();
    let text = say_text(&raw);
    if !say_has_text(text) {
        return;
    }
    let e = cx.edict;
    let idx = cx.index();
    let name = netname(e);
    let prefix = say_prefix(&name, team);
    let mut text = clip_say_text(&prefix, text).to_owned();
    let rules = crate::gameplay::active().server.say_rules();
    let line = if rules.expand_codes {
        let v = cx.pev();
        format_say(&prefix, &text, v.armorvalue, v.health)
    } else {
        plain_say_line(&prefix, &text)
    };
    if rules.lower_all {
        // `_strlwr(text)` after the line is built (2.1 0x1000d0ec): the tests and the log see the
        // text lowered, the chat line keeps its case.
        text.make_ascii_lowercase();
    }

    // `IsNetClient()` of the sender gates the `timeleft` reply and the vote: a bot's vote line is
    // broadcast as chat.
    let net_client = !p.cbot;
    let broadcast = if text.starts_with(b"timeleft") {
        // Not broadcast: in TS both exits of this branch jump to 0xad4dc, which clears the
        // broadcast flag, the same as a handled vote. `(int)mp_timeleft` truncates (0x9d3eb).
        if net_client {
            let left = crate::entities::stock::maprules::x87_int(cvars::f(c"mp_timeleft"));
            msg::text_one(e, hud_dest::TALK, &format!("Time left: {}:{:02}\n", left / 60, left % 60));
        }
        false
    } else if rules.ff_reply && text.starts_with(b"ff") {
        // Not broadcast, whoever sent it (2.1 0x1000d164..0x1000d1da: both exits skip the
        // broadcast to the log).
        if net_client {
            msg::text_one(e, hud_dest::TALK, &friendly_fire_text(cvars::f(c"mp_friendlyfire")));
        }
        false
    } else if text.starts_with(b"vote") {
        // `strlwr(text + 4)` lowers the text in place (A-Z only, C locale), so the log line shows
        // it lowered too; the chat line was built before.
        text[4..].make_ascii_lowercase();
        !(net_client && votes::player_vote(e, idx, &String::from_utf8_lossy(&text[4..])))
    } else {
        true
    };
    if broadcast {
        let gated = crate::gameplay::active().server.say_dead_chat_gated();
        let me_alive = alive(cx.pev());
        let my_respawn = p.respawn_at;
        // `UTIL_FindEntityByClassname(.., "player")` + `IsNetClient()`: every `CBasePlayer` but
        // the bots, players who left included (the engine drops a message to a free slot).
        for o in super::slot_players() {
            if o == e || !super::is_net_client(o) {
                continue;
            }
            if team && !teams::same_team(o, e) {
                continue;
            }
            // Round modes and LMS: players out for the round only talk to each other.
            let (o_alive, o_respawn) = (alive(o.v()), other(o, |q| q.respawn_at).unwrap_or(0.0));
            if say_reaches(gated, me_alive, my_respawn, o_alive, o_respawn) {
                msg::say_one(o, idx, &line);
            }
        }
        msg::say_one(e, idx, &line);
        engine().print(&line);
    }
    // `"name<uid><authid><uid>"`, the `model` key instead of the second user id in teamplay.
    msg::log(&shared::bformat!("{} {} \"{}\"\n", super::scoring::log_player(e), if team { "say_team" } else { "say" }, text));
    p.ts.next_say = now + 0.5;
}

// ---- server commands -------------------------------------------------------------------------

pub fn register_server_commands() {
    let e = engine();
    e.add_server_command(c"addbot", cmd_addbot);
    e.add_server_command(c"addcustombot", cmd_addcustombot);
    e.add_server_command(c"globalslow", cmd_globalslow);
    e.add_server_command(c"restartround", cmd_restartround);
    e.add_server_command(c"weaponrestriction", cmd_weaponrestriction);
    // Port additions for testing dedicated servers.
    e.add_server_command(c"ts_status", cmd_status);
    e.add_server_command(c"ts_hurt", cmd_hurt);
    e.add_server_command(c"ts_givepowerup", cmd_givepowerup);
    e.add_server_command(c"ts_tp", cmd_tp);
    e.add_server_command(c"ts_face", cmd_face);
    e.add_server_command(c"ts_observe", cmd_observe);
    e.add_server_command(c"ts_as", crate::commands::cmd_as);
    e.add_server_command(c"ts_dev_touch", cmd_dev_touch);
    e.add_server_command(c"ts_dev_hamlog", crate::ham::cmd_dev_hamlog);
}

/// `ts_dev_touch <file>` (sv_cheats): creates an empty file in the game directory. Scripted
/// tests end with it so `cargo xtask` sees the script finish at once; the engine log can sit in a
/// write buffer until exit (FWGS under Wine). Only plain file names are accepted.
extern "C" fn cmd_dev_touch() {
    use goldsrc::ext::weapons::WeaponsEngine;
    let e = engine();
    if !cvars::dev_cheats() || e.cmd_argc() < 2 {
        e.print("usage (sv_cheats 1): ts_dev_touch <file>\n");
        return;
    }
    let name = e.cmd_argv(1).to_string_lossy().into_owned();
    if name.is_empty() || name.starts_with('.') || !name.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)) {
        e.print("ts_dev_touch: plain file names only\n");
        return;
    }
    // The engines run with the game root as the working directory; the game dir may be relative.
    let path = std::path::Path::new(&e.game_dir()).join(&name);
    if let Err(err) = std::fs::write(&path, b"") {
        e.print(&format!("ts_dev_touch: {}: {err}\n", path.display()));
    }
}

/// Points a player's view (and body) at `angles`; `fixangle` makes the client take them.
fn set_view_angles(p: *mut edict_t, angles: [f32; 3]) {
    let v = p.v();
    v.v_angle = angles;
    v.angles = angles;
    v.fixangle = 1;
}

fn arg_f32(i: i32) -> f32 {
    engine().cmd_argv(i).to_string_lossy().trim().parse().unwrap_or(0.0)
}

/// `ts_tp <player#> <x> <y> <z> [yaw] [pitch]` (sv_cheats): teleports a player, drops them to the
/// floor and optionally sets the view. Lets scripted tests reach map features.
extern "C" fn cmd_tp() {
    let e = engine();
    if !cvars::dev_cheats() || e.cmd_argc() < 5 {
        e.print("usage (sv_cheats 1): ts_tp <player#> <x> <y> <z> [yaw] [pitch]\n");
        return;
    }
    let p = e.client(arg_f32(1) as i32);
    if p.is_null() {
        return;
    }
    (p.v().velocity = [0.0; 3]);
    e.set_origin(p, [arg_f32(2), arg_f32(3), arg_f32(4)]);
    e.drop_to_floor(p);
    if e.cmd_argc() > 5 {
        let pitch = if e.cmd_argc() > 6 { arg_f32(6) } else { 0.0 };
        set_view_angles(p, [pitch, arg_f32(5), 0.0]);
    }
    let o = p.v().origin;
    e.print(&format!("[ts] ts_tp: #{} at ({:.0} {:.0} {:.0})\n", e.index_of(p), o[0], o[1], o[2]));
}

/// `ts_face <player#> <target#>` or `ts_face <player#> <x> <y> <z>` (sv_cheats): aims a player's
/// view at another player's chest or at a point.
extern "C" fn cmd_face() {
    let e = engine();
    if !cvars::dev_cheats() || (e.cmd_argc() != 3 && e.cmd_argc() != 5) {
        e.print("usage (sv_cheats 1): ts_face <player#> <target#> | <player#> <x> <y> <z>\n");
        return;
    }
    let p = e.client(arg_f32(1) as i32);
    if p.is_null() {
        return;
    }
    let target = if e.cmd_argc() == 3 {
        let t = e.client(arg_f32(2) as i32);
        if t.is_null() {
            return;
        }
        let tv = t.v();
        [tv.origin[0], tv.origin[1], tv.origin[2] + tv.view_ofs[2] * 0.5]
    } else {
        [arg_f32(2), arg_f32(3), arg_f32(4)]
    };
    let v = p.v();
    let eye = [v.origin[0] + v.view_ofs[0], v.origin[1] + v.view_ofs[1], v.origin[2] + v.view_ofs[2]];
    let a = e.vec_to_angles([target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]]);
    // VecToAngles pitch is up-positive; view angles look down with positive pitch.
    set_view_angles(p, [-a[0], a[1], 0.0]);
}

/// `ts_observe <player#> <mode> [target#]` (sv_cheats, port addition): sets an observer's mode
/// (1 chase locked, 2 chase free, 3 roaming, 5 in-eye) and target.
extern "C" fn cmd_observe() {
    let e = engine();
    if !cvars::dev_cheats() || e.cmd_argc() < 3 {
        e.print("usage (sv_cheats 1): ts_observe <player#> <mode> [target#]\n");
        return;
    }
    let p = e.client(arg_f32(1) as i32);
    if p.is_null() {
        return;
    }
    let (mode, target) = (arg_f32(2) as i32, if e.cmd_argc() > 3 { arg_f32(3) as i32 } else { 0 });
    let ok = crate::player::with_player(p, |pl, cx| super::lifecycle::dev_observe(pl, cx, mode, target)).unwrap_or(false);
    let v = p.v();
    e.print(&format!(
        "[ts] ts_observe: #{} {} mode={} target={}\n",
        e.index_of(p),
        if ok { "set" } else { "not observing" },
        v.iuser1,
        v.iuser2
    ));
}

/// `ts_status`: one line per player (state, health, frags, power-ups, time scale).
extern "C" fn cmd_status() {
    let e = engine();
    let (mode, teamplay, lms, the_one) = state::mode();
    e.print(&format!("[ts] status: {mode:?} teamplay={teamplay} lms={lms} theone={the_one} time={:.1}\n", e.time()));
    for p in super::players() {
        let v = p.v();
        // The name's bytes as they are, padded to 16 like `{:<16}` (port dev aid).
        let name = netname(p);
        let name_col = shared::bformat!("{}{}", name, " ".repeat(16usize.saturating_sub(name.len())));
        let line = other(p, |q| {
            let head = format!(
                "{} hp={} ar={} frags={} deaths={} cash={}",
                if v.iuser1 != 0 {
                    "observer"
                } else if alive(v) {
                    "alive"
                } else {
                    "dead"
                },
                v.health,
                v.armorvalue,
                v.frags,
                q.deaths,
                q.cash,
            );
            let tail = format!(
                "held={:#x}/{}s active={:#x} slow={:.2}->{:.2} award={} spree={} gp={} org=({:.0} {:.0} {:.0})\n",
                q.ts.held,
                q.ts.held_secs,
                q.active_powerup,
                v.fuser1,
                q.slow_target,
                q.ts.award,
                q.ts.spree,
                // Port addition: the ruleset the player moves by, `-` before its first spawn.
                Some(e.physics_key(p, shared::gameplay::PHYSINFO_KEY_C)).filter(|g| !g.is_empty()).unwrap_or_else(|| "-".to_owned()),
                v.origin[0],
                v.origin[1],
                v.origin[2]
            );
            // The team name's bytes as they are (#188).
            shared::bformat!("{} team='{}' {}", head, q.ts.team_name, tail)
        });
        if let Some(l) = line {
            e.print(&shared::bformat!("[ts]  #{} {} {}", e.index_of(p), name_col, l));
        }
    }
}

/// `ts_hurt <player#> <damage> [hitgroup] [attacker#]`: bullet damage, for testing.
extern "C" fn cmd_hurt() {
    let e = engine();
    if e.cmd_argc() < 3 {
        e.print("usage: ts_hurt <player#> <damage> [hitgroup] [attacker#]\n");
        return;
    }
    let arg = |i| e.cmd_argv(i).to_string_lossy().trim().parse::<f32>().unwrap_or(0.0);
    let victim = e.client(arg(1) as i32);
    if victim.is_null() {
        return;
    }
    let attacker = if e.cmd_argc() > 4 { e.client(arg(4) as i32) } else { e.entity_of_index(0) };
    let attacker = if attacker.is_null() { e.entity_of_index(0) } else { attacker };
    let mut info = crate::damage::DamageInfo::new(attacker, attacker, arg(2), crate::damage::bits::BULLET).traced();
    info.hitgroup = if e.cmd_argc() > 3 { arg(3) as i32 } else { crate::damage::hitgroup::CHEST };
    info.pos = victim.v().origin;
    info.dir = [1.0, 0.0, 0.0];
    let took = crate::damage::apply(victim, &info);
    e.print(&format!("[ts] ts_hurt: applied={took} health={}\n", victim.v().health));
}

/// `ts_givepowerup <player#> <bits>`: applies a power-up pickup, for testing.
extern "C" fn cmd_givepowerup() {
    let e = engine();
    if e.cmd_argc() < 3 {
        return;
    }
    let p = e.client(e.cmd_argv(1).to_string_lossy().trim().parse().unwrap_or(0));
    let bits: u16 = e.cmd_argv(2).to_string_lossy().trim().parse().unwrap_or(0);
    crate::player::with_player(p, |pl, cx| {
        let secs = crate::gameplay::active().server.powerups().base(bits);
        add_powerup(pl, cx, bits, secs, 100.0);
    });
}

/// `addbot [model]` (Windows `mp.dll` `0x10026110`): `addbot help` prints the usage and anything
/// else adds a bot (`BotCreate`, [`crate::bots::add_bot`] for the native backend, through
/// [`crate::bot_backend::add_bot`]: the argument is the model, not the name). Deviation (chosen): TS refuses on a dedicated server with "Bots only in listen
/// servers." (all the Linux `ts_i386.so`, which is dedicated-only, has: 0xc5520); the port adds
/// the bot there too. As a server command it only runs from the server's console, command line
/// and configs, or `rcon`: a client's command goes to `ClientCommand`, which doesn't have it.
extern "C" fn cmd_addbot() {
    let e = engine();
    if e.cmd_argv(1) == c"help" {
        e.alert(ALERT_TYPE::at_console, "Usage: addbot [botname]\n");
    } else {
        // `BotCreate(CMD_ARGV(1))`: the bytes as typed (#190).
        let model = e.cmd_argv(1).to_bytes().to_vec();
        crate::bot_backend::add_bot(crate::bot_backend::AddBot::Addbot { model });
    }
}

/// `addcustombot [name] [model] [skill]` (Windows `mp.dll` `0x100261a0`; Linux 0xc5550 as
/// `addbot`): as [`cmd_addbot`], dedicated servers included, with `BotCreateCustom`'s name, model
/// and skill ([`crate::bots::add_custom_bot`] for the native backend). The arguments are read last to first, as TS does.
extern "C" fn cmd_addcustombot() {
    let e = engine();
    if e.cmd_argv(1) == c"help" {
        e.alert(ALERT_TYPE::at_console, "Usage: addcustombot [botname] [team] [skill]\n");
    } else {
        let skill = crate::bots::files::scan_f32(e.cmd_argv(3).to_bytes()).unwrap_or(0.0); // (float)atof
        let model = e.cmd_argv(2).to_bytes().to_vec();
        let name = e.cmd_argv(1).to_bytes().to_vec();
        crate::bot_backend::add_bot(crate::bot_backend::AddBot::Custom { name, model, skill });
    }
}

/// `globalslow [percent]` (0xc5580). The argument is `(float)strtod(argv(1))`, so trailing text
/// is ignored and garbage is 0. The query and the "is at" text read the stored amount
/// (`TSGameRules+0x564`), not the active flag, and `SetGlobalSlowMotion` only acts on a value
/// that differs from it. The text goes through `ALERT(at_console)`, which the engines print only
/// with `developer` on.
extern "C" fn cmd_globalslow() {
    if crate::gameplay::active().server.server_command("globalslow") {
        return;
    }
    let e = engine();
    let amount = if e.cmd_argc() < 2 {
        state::with(|g| g.global_slow.amount)
    } else {
        let amount = globalslow_amount(c_strtod(&e.cmd_argv(1).to_string_lossy()) as f32);
        frame::set_global_slow(amount, 0.0);
        amount
    };
    e.alert(ALERT_TYPE::at_console, &globalslow_text(amount));
}

/// `cmd_globalslow`'s reply for the amount: `%.0f` of `amount × 100` (exact in the double TS
/// passes).
fn globalslow_text(amount: f32) -> String {
    if amount != 0.0 { format!("Globalslow is at {:.0}%.\n", f64::from(amount) * 100.0) } else { "Globalslow is off.\n".to_owned() }
}

/// glibc's `strtod` read of the longest prefix that is a number: white space, a sign, then
/// `inf`/`infinity`/`nan`, a hexadecimal float (`0x`, digits, `.`, `p` exponent) or a decimal one
/// (digits, `.`, an `e` exponent only when digits follow); 0 when nothing is read.
pub fn c_strtod(s: &str) -> f64 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let lower = s.get(..8).unwrap_or(s).to_ascii_lowercase();
    let v = if lower.starts_with("inf") {
        f64::INFINITY
    } else if lower.starts_with("nan") {
        f64::NAN
    } else if (lower.starts_with("0x")) && s[2..].starts_with(|c: char| c.is_ascii_hexdigit() || c == '.') {
        hex_float(&s[2..])
    } else {
        let b = s.as_bytes();
        let digits = |mut i: usize| {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            i
        };
        let int_end = digits(0);
        let mut end = int_end;
        if b.get(end) == Some(&b'.') {
            end = digits(end + 1);
        }
        if end == 0 || (end == 1 && int_end == 0) {
            return 0.0;
        }
        if matches!(b.get(end), Some(b'e' | b'E')) {
            let sign = usize::from(matches!(b.get(end + 1), Some(b'+' | b'-')));
            let exp_end = digits(end + 1 + sign);
            if exp_end > end + 1 + sign {
                end = exp_end;
            }
        }
        s[..end].parse().unwrap_or(0.0)
    };
    if neg { -v } else { v }
}

/// The digits of a hexadecimal float after `0x` (`0x.8` is 0.5, `0x1p4` 16). A `0x` with no
/// digits reads as just the `0`.
fn hex_float(s: &str) -> f64 {
    let (mut mant, mut exp, mut seen, mut dot) = (0.0f64, 0i32, false, false);
    let mut rest = s;
    for (i, c) in s.char_indices() {
        if c == '.' && !dot {
            dot = true;
        } else if let Some(d) = c.to_digit(16) {
            mant = mant * 16.0 + f64::from(d);
            seen = true;
            if dot {
                exp -= 4;
            }
        } else {
            rest = &s[i..];
            break;
        }
        rest = &s[i + c.len_utf8()..];
    }
    if !seen {
        return 0.0;
    }
    if let Some(p) = rest.strip_prefix(['p', 'P']) {
        let (neg, digits) = match p.as_bytes().first() {
            Some(b'-') => (true, &p[1..]),
            Some(b'+') => (false, &p[1..]),
            _ => (false, p),
        };
        let n = digits.bytes().take_while(u8::is_ascii_digit).fold(0i32, |a, d| a.saturating_mul(10).saturating_add(i32::from(d - b'0')));
        if digits.starts_with(|c: char| c.is_ascii_digit()) {
            exp = exp.saturating_add(if neg { -n } else { n });
        }
    }
    mant * 2f64.powi(exp)
}

/// `cmd_globalslow` argument → factor: ×0.01, clamped to [0, 2], ≈1 means off, tiny → 0.01.
pub fn globalslow_amount(pct: f32) -> f32 {
    let mut a = (pct * 0.01).clamp(0.0, 2.0);
    if a > 0.999 && a < 1.001 {
        a = 0.0;
    }
    if a > 0.0 && a < 0.01 {
        a = 0.01;
    }
    a
}

extern "C" fn cmd_restartround() {
    if crate::gameplay::active().server.server_command("restartround") {
        return;
    }
    frame::reset_restart_round(2);
}

/// `weaponrestriction [n]`: 1 none, 2 knives, 3 knives+pistols, 4 knives+pistols+shotguns,
/// 5 reset, other everything; masked by the map's base set and broadcast as SrvSett 3.
extern "C" fn cmd_weaponrestriction() {
    if crate::gameplay::active().server.server_command("weaponrestriction") {
        return;
    }
    let e = engine();
    if e.cmd_argc() >= 2 {
        // `strtol(argv(1), 0, 10)`: the leading integer, trailing text ignored.
        let n = super::mapcycle::atoi(&e.cmd_argv(1).to_string_lossy());
        let (old, new) = state::with(|g| {
            let old = g.allowed_weapons;
            g.allowed_weapons = if n == 5 { 0x1f } else { restriction_by_id(n) & g.base_weapons };
            (old, g.allowed_weapons)
        });
        if old != new {
            msg::all(Msg::SrvSett, |w| SrvSett { key: 3, value: new as i16 }.write(w));
        }
    }
    e.alert(ALERT_TYPE::at_console, &weaponrestriction_text(state::with(|g| g.allowed_weapons)));
}

/// `display_weaponrestriction` (0xc5770): the categories from `g_szRestrictions` (0x109034, the
/// strings at 0xfcec0..), capitalised, printed with `ALERT(at_console)`.
fn weaponrestriction_text(mask: i32) -> String {
    if mask == 0 {
        return "All weapons are restricted.\n".to_owned();
    }
    let names = ["Knives ", "Pistols ", "SMGs ", "Rifles ", "Shotguns "];
    let s: String = (0..5).filter(|i| mask & (1 << i) != 0).map(|i| names[i]).collect();
    format!("Weapons allowed: {s}\n")
}

/// `WeaponRestrictionByID`.
pub fn restriction_by_id(n: i32) -> i32 {
    match n {
        1 => 0,
        2 => 1,
        3 => 3,
        4 => 0x13,
        _ => 0x1f,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_command_arguments_parse_like_c() {
        // Issue #140: `strtod`/`strtol` read the leading number and ignore the rest.
        assert_eq!(c_strtod("50%"), 50.0);
        assert_eq!(globalslow_amount(c_strtod("50%") as f32), 0.5);
        assert_eq!(c_strtod("  -12.5e1x"), -125.0);
        assert_eq!(c_strtod("1e"), 1.0);
        assert_eq!(c_strtod("1e+"), 1.0);
        assert_eq!(c_strtod(".5"), 0.5);
        assert_eq!(c_strtod("5."), 5.0);
        assert_eq!(c_strtod("."), 0.0);
        assert_eq!(c_strtod("abc"), 0.0);
        assert_eq!(c_strtod(""), 0.0);
        assert_eq!(c_strtod("0x10"), 16.0);
        assert_eq!(c_strtod("0x.8p1"), 1.0);
        assert_eq!(c_strtod("0xg"), 0.0);
        assert_eq!(c_strtod("Infinity"), f64::INFINITY);
        assert!(c_strtod("nan").is_nan());
        assert_eq!(globalslow_amount(c_strtod("inf") as f32), 2.0);
        assert_eq!(super::super::mapcycle::atoi("3x"), 3);
        assert_eq!(super::super::mapcycle::atoi("2;"), 2);
        assert_eq!(super::super::mapcycle::atoi("x3"), 0);
    }

    #[test]
    fn server_command_replies_are_ts_text() {
        assert_eq!(globalslow_text(0.5), "Globalslow is at 50%.\n");
        assert_eq!(globalslow_text(0.0), "Globalslow is off.\n");
        // `%.0f` of the double: 0.285f × 100 is 28.499999...
        assert_eq!(globalslow_text(0.285), "Globalslow is at 28%.\n");
        assert_eq!(weaponrestriction_text(0), "All weapons are restricted.\n");
        assert_eq!(weaponrestriction_text(3), "Weapons allowed: Knives Pistols \n");
        assert_eq!(weaponrestriction_text(0x1f), "Weapons allowed: Knives Pistols SMGs Rifles Shotguns \n");
    }

    #[test]
    fn fullupdate_spectator_follows_deadflag() {
        let s = fullupdate_spectator(3, DEAD_RESPAWNABLE as i32);
        assert_eq!((s.entindex, s.observer), (3, 1));
        assert_eq!(fullupdate_spectator(3, DEAD_NO as i32).observer, 0);
        // A corpse that hasn't become an observer yet (DEAD_DEAD) isn't one.
        assert_eq!(fullupdate_spectator(3, DEAD_DEAD as i32).observer, 0);
    }

    #[test]
    fn say_format() {
        let say = |name: &str, team: bool, text: &str, armor: f32, health: f32| {
            let prefix = say_prefix(name.as_bytes(), team);
            String::from_utf8(format_say(&prefix, clip_say_text(&prefix, text.as_bytes()), armor, health)).unwrap()
        };
        assert_eq!(say("bob", false, "hi", 0.0, 100.0), "\x02bob: hi\n");
        assert_eq!(say("bob", true, "hp %h %a 100%", 50.0, 87.4), "\x02(TEAM) bob: hp 87 with kevlar 100%\n");
        // `(int)health` truncates: 87.6 is 87, -0.5 is 0.
        assert_eq!(say("bob", false, "%h", 0.0, 87.6), "\x02bob: 87\n");
        assert_eq!(say("bob", false, "%h", 0.0, -0.5), "\x02bob: 0\n");
        // The text is cut so the unexpanded line is 126 bytes (0xad02c, 2.1 0x1000cf00), 127 with
        // the `\n`.
        let long = "x".repeat(300);
        let prefix = say_prefix(b"bob", false);
        assert_eq!(prefix.len() + clip_say_text(&prefix, long.as_bytes()).len(), 126);
        assert_eq!(say("bob", false, &long, 0.0, 0.0).len(), 127);
        assert_eq!(clip_say_text(&prefix, &[b'x'; 120]).len(), 120, "a shorter text is kept whole");
        // Guard: expansions stop at TS's 140-byte buffer, well under the 192-byte message limit.
        let line = say("bob", false, &"%a".repeat(60), 1.0, 0.0);
        assert_eq!(line.len(), SAY_LINE_MAX + 1);
        assert!(line.ends_with('\n') && line.starts_with("\x02bob: with kevlarwith kevlar"));
    }

    /// Issue #81: a CP1252 name and chat (`é` = 0xE9, not UTF-8) go into the `SayText` and the log
    /// line byte for byte, as `Host_Say` (0xad02c) copies `Cmd_Args` and `STRING(pev->netname)`.
    #[test]
    fn say_keeps_latin1_bytes() {
        use shared::protocol::hud_wire::{Bytes, WireRead};
        let raw: &[u8] = b"\"caf\xe9 cr\xe8me \xe9\"";
        let text = say_text(raw);
        assert_eq!(text, b"caf\xe9 cr\xe8me \xe9");
        assert!(say_has_text(text));
        let prefix = say_prefix(b"Ren\xe9", false);
        let line = format_say(&prefix, clip_say_text(&prefix, text), 0.0, 100.0);
        assert_eq!(line, b"\x02Ren\xe9: caf\xe9 cr\xe8me \xe9\n");
        let mut w = Bytes::default();
        msg::say_text(3, &line).write(&mut w);
        assert_eq!(w.buf, b"\x03\x02Ren\xe9: caf\xe9 cr\xe8me \xe9\n\0", "SayText: byte sender, then the line unconverted");
        assert_eq!(WireRead::byte(&mut w), 3);
        assert_eq!(WireRead::string(&mut w), line);
        // A leading quote takes the last byte, not the last UTF-8 character.
        assert_eq!(say_text(b"\"ab\xc3\xa9"), b"ab\xc3");
        // The 126-byte cut counts bytes.
        let long = [0xe9u8; 300];
        assert_eq!(prefix.len() + clip_say_text(&prefix, &long).len(), 126);
    }

    /// The 2.1 shape of `Host_Say` (0x1000cf00): no `%a`/`%h`, the `ff` reply and the dead-chat
    /// gate (0x1000d2d5..0x1000d33a).
    #[test]
    fn say_without_codes_ff_and_dead_chat() {
        let prefix = say_prefix(b"Bob", true);
        // The text as it is, then "\n" (0x1000d0dd..0x1000d0e3); its case is kept in the line.
        assert_eq!(plain_say_line(&prefix, b"HP %h %a 100%"), b"\x02(TEAM) Bob: HP %h %a 100%\n");
        let long = [b'x'; 300];
        assert_eq!(plain_say_line(&prefix, clip_say_text(&prefix, &long)).len(), 127);
        // "Friendly fire is %s\n" (0x10091b70), "on" (0x10091b8c) unless the cvar is 0.
        assert_eq!(friendly_fire_text(0.0), "Friendly fire is off\n");
        assert_eq!(friendly_fire_text(1.0), "Friendly fire is on\n");
        assert_eq!(friendly_fire_text(-0.0), "Friendly fire is off\n");
        assert_eq!(friendly_fire_text(f32::NAN), "Friendly fire is on\n");
        // Gate off: everyone hears everyone.
        assert!(say_reaches(false, false, -1.0, true, 0.0));
        // Gate on: a sender out for the round reaches only players out for the round.
        assert!(!say_reaches(true, false, -1.0, true, 0.0));
        assert!(!say_reaches(true, false, -1.0, false, 2.0), "a dead player who respawns this round");
        assert!(say_reaches(true, false, -1.0, false, -1.0));
        // A living sender, or one still to respawn, reaches everyone.
        assert!(say_reaches(true, true, -1.0, true, 0.0));
        assert!(say_reaches(true, false, 0.0, true, 0.0));
    }

    #[test]
    fn say_text_parsing() {
        // A leading quote takes the last character with it, quote or not.
        assert_eq!(say_text(b"\"hello\""), b"hello");
        assert_eq!(say_text(b"\"hello"), b"hell");
        assert_eq!(say_text(b"\""), b"");
        assert_eq!(say_text(b"hello\""), b"hello\"");
        // Only printable non-space ASCII counts as text.
        assert!(!say_has_text(b" \t\x01"));
        assert!(!say_has_text(b"\xe9"));
        assert!(say_has_text(b" a "));
    }

    #[test]
    fn globalslow_clamps() {
        assert_eq!(globalslow_amount(50.0), 0.5);
        assert_eq!(globalslow_amount(100.0), 0.0, "100% is off");
        assert_eq!(globalslow_amount(500.0), 2.0);
        assert_eq!(globalslow_amount(0.5), 0.01);
        assert_eq!(globalslow_amount(-5.0), 0.0);
    }

    #[test]
    fn restrictions() {
        assert_eq!(restriction_by_id(4), 0x13);
        assert_eq!(restriction_by_id(9), 0x1f);
    }
}
