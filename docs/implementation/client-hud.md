# Client: input, HUD, menus (work package `wp/hud`)

Port of the TS 3.0 `client.dll` input layer, HUD elements and menus. Source of truth: decomp
`docs/client/architecture-hud.md` (cited as §n) and `client/decomp/*.c`. Layout follows
`docs/design.md` §6.

| File | Contents |
|---|---|
| `crates/client/src/input.rs` | kbuttons, `CL_CreateMove`, anti-script gate, input cvars, mouse back ends, virtual cursor, `IN_ClientLookEvent`/`IN_ClientMoveEvent` exports |
| `crates/client/src/hud/mod.rs` | hooks for every message, HUD cvars/commands, per-frame think, FOV/zoom sensitivity, slow-mo easing, draw pass, `ts_hud_selftest` |
| `crates/client/src/hud/state.rs` | `Hud` state and the pure message handlers (return `Effect`s) |
| `crates/client/src/hud/{status,weapons,notice}.rs` | `CHudTSStatus`, `CHudTSWeapons`, SDK elements (HudText, death notices, say text, `CHudMenu`) |
| `crates/client/src/hud/{draw,text}.rs` | hud.txt sprites, `DrawHudNumberTS`, `DrawHudString`, titles.txt localisation, mini printf |
| `crates/client/src/menus/{mod,buy,scoreboard}.rs` | buy menu (`CTSBuyMenu`), scoreboard, team select, MOTD, mouse routing |
| `crates/client/src/voice.rs` | `CVoiceStatus`/`CVoiceBanMgr`: speaker state, head icons, speaker labels, talk icon, scoreboard voice icons and squelch mode, `voice_ban.dt`, `VModEnable`/`vban` |
| `crates/client/src/joystick.rs` | the SDK joystick code TS ships (`IN_JoyMove`, `IN_Commands`, `Joy_AdvancedUpdate_f`), Steam GoldSrc only |
| `crates/client/src/state.rs` | fields added for other packages (see Integration) |
| `crates/client-dll/src/vgui.rs` | `ClientFactory`/`CreateInterface` exports and the `IClientVGUI` object that makes Steam GoldSrc draw the world (see Deviations) |
| `crates/goldsrc/src/ext/hud.rs` | engine helpers: sprites (`spr_load`, the one `SPR_Load` wrapper), text, cvars, player info, `max_clients`/`client_time` (the one `GetMaxClients`/`GetClientTime` wrappers), demo API; client entities: `entity`/`local_player` (shared `&cl_entity_t`) and `with_entity_mut`/`with_local_player_mut` (a `&mut` scoped to a closure), the one `GetEntityByIndex`/`GetLocalPlayer` wrappers, with the aliasing rules in the module docs; `crosshair_trace` (TS `DrawCrosshairID` trace, built on `render::with_solid_players`/`player_trace`; `None` if the event API is null); `ev_play_sound`, the one `EV_PlaySound` wrapper (no-op if the slot is null); `ClEngine::event_api`/`efx_api`/`tri_api`/`demo_api` (null-checked bindgen `event_api_s`/`efx_api_s`/`triangleapi_s`/`demo_api_s`); `world_to_screen_ndc`, the one `WorldToScreen` wrapper (`None` if clipped or the slot is null; the laser dots use it), and TS's pixel `WorldToScreen` on top of it; `server_cmd`, the one `pfnServerCmd` wrapper (no-op if the slot is null); and `local_view_height`, the one `EV_LocalPlayerViewheight` wrapper (standing offset 0 0 28 if the slot is null; the events' gun position uses it too) |
| `crates/goldsrc/src/ext/voice.rs` | `GetPlayerUniqueID`, `pfnServerCmdUnreliable`, level check, the static head-icon entities for `CL_CreateVisibleEntity` (and the server's `Voice_SetClientListening`) |
| `crates/goldsrc/src/joystick.rs` | joystick device access: WinMM on Windows, SDL2 game controller (run-time lookup) elsewhere; `CheckParm` |
| `crates/shared/src/protocol/hud.rs` | `Reader`/`Writer` and decode/encode for all HUD payloads |

## 1. TS function → Rust

| TS (VA) | Rust |
|---|---|
| `InitInput` 10026b30 (cvars, `+cmds`) | `input::init`, `InputCvars::register`, `register_commands` |
| `CL_ButtonBits` 10026870 | `input::button_bits` |
| `CL_CreateMove` 10026570, `CL_AdjustAngles` 10026330 | `input::create_move`, `adjust_angles` |
| `+attack2` 10025f40 / `+alt2` 10026110 (`cl_attackswitch`) | `input::attack_switched` + the two command bodies |
| `HUD_Key_Event` 10025a80 gate, `HUD_Frame` gate clear | `input::key_event`, `input::frame` |
| `KB_Find` 10025880 | `input::find` |
| `IN_MouseMove` 100274d0 / `IN_MouseEvent` 100273d0 / `IN_Accumulate` | `input::apply_mouse`, `mouse_event`, `accumulate` (+ `native`, `win`, `sdl`) |
| `TS_ApproachTimescale` 10055ae0 | `hud::approach_timescale` (called from `HUD_Frame`) |
| `CHud::Think` 10049760, `UTIL_ApproachFOV` 10049620, `SetFOV` 10049570 | `hud::think_fov`, `Msg::SetFov` handler |
| `CHudTSStatus` handlers 1004b730..1004c1e0 | `Hud::apply` (status arms) |
| `AddTSMessage` 1004b520 | `Hud::add_ts_message` |
| `CHudTSStatus::Draw` 10049ea0 and `DrawRoundTimer` 1004a130, `DrawPowerup` 1004a2e0, `DrawStatusPanel` 1004a3e0, `DrawTeammateMarkers` 1004ab40, `DrawCrosshairID` 1004acc0, `DrawTSMessages` 1004af40, `DrawObjectiveText` 1004b0e0, `DrawObjectiveMarkers` 10049fc0 | `hud::status::*` |
| `CHudTSWeapons` handlers 1004f6f0..1004fb50 | `Hud::apply` (weapon arms) |
| `CHudTSWeapons::Reset` 1004cd60, `Think` 1004ce10 | `Hud::reset_weapons`; `hud::think`, `weapons::think_sounds`, `hud::weapon_menu_buttons` |
| `Draw` 1004d3f0, `DrawWirePanel` 1004d550, `DrawKungFuBar` 1004d5f0, `DrawKungFuLabel` 1004d700, `DrawWeaponInfo` 1004d7e0, `DrawWeaponList` 1004e030, `DrawLaserDots` 1004e6f0, `DrawCrosshair` 1004e9f0 + 1004ef50 | `hud::weapons::*` |
| `SelectSlot` 1004f220, `SelectNextPrev` 1004f500, `CmdBuy` 1004fda0 | `hud::select_slot` → `weapons::select_slot`; `weapons::select_next_prev`; `hud::cmd_buy` |
| `DrawHudNumberTS` 1004c290, `DrawHudString` 10021ce0, `DrawHudStringLen` 10021ca0 | `draw::draw_number`/`number_cells`, `text::draw_string`, `text::string_len` |
| `LocaliseTextString` 100348d0, `LookupString` 100349c0 | `text::localise`, `text::lookup` |
| `CHudDeathNotice` 1001b020, `CHudSayText` 1002eb40, `CHudMenu` 100285f0, `CHudMessage` 100290b0 | `hud::notice::*` |
| `CTSBuyMenu` 10036200..100398e0 (Init, Load/Save file, Open, Close, Draw pages, HandleKey 10038840, Add 10038f60, Remove 10038e40, ammo fill 100392b0, Submit 10039400, Random 100398e0, restriction 100365e0/1000b6d0, cost 10012ad0, slots 10038dc0) | `menus::buy::BuyMenu::*`, `menus::draw_buy`, `menus::save_file` |
| `CTSBuyMenu::MouseEvent` 10038360 | `menus::mouse_buttons` / `click` |
| `ScorePanel` 1005ce70/1005db70 | `menus::scoreboard` |
| `CTeamMenuPanel` 100655d0 (VGUIMenu 2) | `menus::draw_team_menu`, `team_menu_key` |
| `CVoiceStatus::Init` 100178c0 / `VidInit` 10017af0 / `Frame` 10017ea0 | `voice::init`, `vid_init`, `frame` |
| `CVoiceStatus::CreateEntities` 10017f30 (`HUD_CreateEntities`) | `voice::create_entities` |
| `UpdateSpeakerStatus` 100180c0 (`HUD_VoiceStatus` 1001ab40) | `voice::update_speaker_status` |
| `UpdateServerState` 10018360, `HandleVoiceMaskMsg` 10018780, `HandleReqStateMsg` 10018830 | `voice::update_server_state`, `msg_voice_mask`, `msg_req_state` |
| `RepositionLabels` 10018900, `UpdateBanButton` 100185f0 | `voice::redraw`/`label_rects`, `voice::speaker_icon`/`draw_speaker` (scoreboard voice column) |
| `Start/StopSquelchMode` 10018870/10018890, `IsInSquelchMode` 100188b0, `IsPlayerBlocked` 10018c80, `IsPlayerAudible` 10018cc0, `SetPlayerBlockedState` 10018cf0 | `voice::start_squelch_mode` … `voice::toggle_block` |
| `ScorePanel::mousePressed` 1005e660, `MouseOverCell` 1005e870, `HideScoreBoard` 10063760 | `menus::scoreboard_press`, `menus::mouse_buttons`/`squelch_row`, `menus::show_scores(false)` |
| `CVoiceBanMgr` (`voice_ban.dt`), `voice_showbanned` 10017660 | `voice::BanList`, `voice::show_banned` |
| `SayTextPrint` 1002ed00 (muted players' chat dropped) | `hud::on_message` (SayText) |
| `IN_StartupJoystick` 10027960, `IN_ReadJoystick` 10027de0 | `goldsrc::joystick::startup`/`read` |
| `Joy_AdvancedUpdate_f` 10027b00, `IN_Commands` 10027ca0, `IN_JoyMove` 10027e40 | `joystick::axis_map`/`joy_advanced_update`, `joystick::commands`, `joystick::joy_move`/`apply_axes` |
| `CMessageWindowPanel` (MOTD, VGUIMenu 5) | `menus::draw_motd` |
| `CHudSpectator` 10022c70 (Init), 10022fe0 (VidInit), `DirectorMessage` 10023380, `SetModes` 100234d0, `specmode` 10022c10, map camera 10022da0 | `spectator::{init, vid_init, director_event, set_modes}`; `HUD_DirectorEvent` 1001ab60 and `HUD_DirectorMessage` exports |

## 2. Message table

Every message in `shared::protocol::MESSAGES` is hooked except `ScreenShake` and `ScreenFade`:
FWGS (and GoldSrc) parse those two in the engine before client hooks are consulted, and TS
doesn't hook them either. `Logo` is also hooked (ignored), and the voice module hooks `VoiceMask`
(audible and server-side ban masks) and `ReqState` (answers `VModEnable` + `vban`). Malformed
payloads are logged and ignored.

| Message | Read order | Effect |
|---|---|---|
| TSHealth | B hp | health; hp 0 → third-person toggle request (death cam) |
| TSArmor / TSState / TSSpace | B | kevlar / stance / free slots |
| TSCash | S | credits |
| PwUp | S flags, B value | flags & 0x104 → kung-fu / movement icon, else timed power-up |
| PwUpTime | B secs | expiry = now + secs, value 0 |
| RoundTime | B secs, B mode | timer (mode 0: 1 s, 1: 0.1 s, 2: frozen) |
| Objective | Co×3, B idx, Str | idx 0 clears; bounds fixed (< 32) |
| ShowObj | B idx, B visible | |
| TSMessage | B r,g,b, B flags, B time (tenths, ≤100), Str | `AddTSMessage` (positions, stacking, 0x10 replace, grey if it names you) |
| PTakeDam | Co×6, B | queued `TsEffect::PlayerHit` |
| TSSlowMo | Co (×0.01, 0 → 1) | `state.time_scale_target`; `goslow.wav` when slow-mo starts |
| TSFrags | Co×6, B type [+Co×6, L, Co] | queued `TsEffect::Frags` |
| TSPAward | B cl, B award | scoreboard status |
| SrvSett | B key, S value | realbullet / cashbased / ammocount / restriction mask |
| TSBTime | B | bullet time |
| TSHVoice | B | `misc/successful.wav` / `misc/failed.wav` |
| TSFlag | B state, B idx, B team, B carrier | flag table |
| TSMBlur | B ent, B mode | queued `TsEffect::MotionBlur` |
| TSPSys | B type [type 3: L, L, Co] | queued `TsEffect::Wind` |
| PlInfo | B cl, B key, B value | laser colour (key 0) |
| TSFade | S, S, S, B×4 | `state.ts_fade` (seconds = value/4096) |
| WeaponInfo | B id (0xff keep), B clip, S reserve, B mode, B att | current gun, grid; `cl_thirdpersonswitch` |
| ClipInfo / WStatus / ActItems / NextStunt / Wires / KFuPower / CustomWP | §3.3 | grid, active items, stunt cooldown, wires, kung-fu power, custom weapon params |
| ScoreInfo / TeamInfo / TeamScore / TeamNames | stock | scoreboard data |
| DeathMsg | B, B, Str | kill feed + TS console line |
| SayText / TextMsg / HudText / ShowMenu / MOTD / ServerName / GameMode / Spectator / VGUIMenu / ResetHUD / InitHUD / SetFOV | stock (TS-modified Spectator/VGUIMenu) | as in the SDK/§3.2 (Spectator: observer flag, buy menu on entering) |
| Train | B notch | `CHudTrain`: `sprites/%d_train.spr` frame `notch - 1` in (255,160,0), x = w/3 + sprite w/4, y = h - sprite h - font height; hidden at 0 |
| HideWeapon | B bits | `hide_hud`; HIDEHUD_ALL (4) hides the train element. TS only resets the engine crosshair, which it doesn't use |
| CurWeapon, WeaponList, AmmoPickup, WeapPickup, ItemPickup, AmmoX, Geiger, Health, Battery, Damage, GameTitle, SelAmmo | stock | parsed by hook, no effect (TS's elements draw nothing) |

## 3. Cvars and commands

Cvars, names/defaults/flags as TS registers them (§2.1):
* InitInput/IN_Init: `lookstrafe lookspring cl_anglespeedkey cl_yawspeed cl_pitchspeed cl_upspeed
  cl_forwardspeed cl_backspeed cl_sidespeed cl_movespeedkey cl_vsmoothing m_pitch m_yaw m_forward
  m_side cl_fx cam_back cam_right cam_up cl_camtype hud_r hud_g hud_b hud_color cl_lasercolor
  cl_buykevlar nohud cl_realhud cl_shadow cl_wavedetail cl_thirdpersonswitch cl_usepowerup cl_laser
  cl_solidcross cl_thirdperson cl_attackswitch cl_thirdtransparency cl_motionblur cl_gore
  cl_showequipment m_filter sensitivity m_accel m_accelexp m_accelmax joystick joyname joyadv*
  joy*threshold joy*sensitivity joywwhack1/2`.
* Voice (`CVoiceStatus::Init`): `voice_modenable voice_clientdebug`.
* HUD: `hud_drawhistory_time hud_fastswitch hud_deathnotice_time
  hud_classautokill hud_takesshots zoom_sensitivity_ratio default_fov hud_capturemouse
  hud_saytext_time hud_centerid cl_drawnames`.
* Not registered here (other packages): `cl_bob…`/`cl_slowfade` (V_Init), `cam_*` (CAM_Init),
  `r_paintball`, `gl_fog*`.

Commands: `+/-moveup movedown left right forward back moveleft moveright lookup lookdown speed
attack attack2 use jump klook jlook duck reload alt1 alt2 score showscores graph break`, `impulse`,
`slot1..slot10`, `invnext`, `invprev`, `buy`, `cancelselect`, `special`, `specmode` (HLTV only),
`voice_showbanned`, `joyadvancedupdate`, and `ts_hud_selftest` (port-only development aid).
Server-side commands (`lastinv`, `drop`, `firemode`, `useitems`,
`jointeam`, `changeteam`) are forwarded to the server by the engine, as in TS.

Client → server commands sent: `weapon_<n>`, `weapon_0`, `drop_<n>`, `cutwire_<n>`,
`lasclr_<n>`, `usepowerup`, `rst_cfg` + `tkw …` + `tki …` + `tka …`, `chgteam <n>`,
`jointeam <n>`, `_special`, `menuselect <n>`, `VModEnable <0|1>` and `vban <hex mask>` (voice; the
mask goes unreliably, at most once a second unless the server asks with `ReqState`).

## 4. Input

* Bits (§5.1): attack 0x1, jump 0x2, duck 0x4, forward 0x8, back 0x10, use 0x20, alt2 0x40, left
  0x80, right 0x100, moveleft 0x200, moveright 0x400, attack2 0x800, reload 0x2000, alt1 0x4000,
  score 0x8000 (also forced while health < 1 or in intermission); `+speed` scales movement but
  isn't sent.
* `cl_attackswitch`: with a gun of kind other than grenade/knife/katana/briefcase held, `+attack2`
  drives 0x40 and `+alt2` drives `IN_ATTACK2`. `+attack2` also sends `_special`.
* Gate: `+attack +attack2 +alt2 +reload +forward +back +moveleft +moveright +jump` only act when
  the same frame saw a physical key bound to exactly that command (FWGS order checked:
  input events → `Cbuf_Execute` → `HUD_Frame`). So `autotest.cfg` can't press them, like TS.
* Turn clamps: `cl_yawspeed ≤ 200`, `cl_pitchspeed ≤ 220`, `cl_anglespeedkey ≥ 0.67`, pitch ±89,
  roll ±50. View angles frozen at the last live value while dead (not observing).
* **Mouse (two back ends).** FWGS is detected by its `host_ver` cvar. The client exports
  `IN_ClientLookEvent`/`IN_ClientMoveEvent` (FWGS extensions; same scheme as hlsdk-portable's
  `input_xash3d.cpp`). When they are exported FWGS stops adding mouse look itself
  (`IN_EngineAppendMove` returns early when `pfnLookEvent` is set), uses relative mouse input on
  every platform (`IN_CheckMouseState`: `use_raw_input = … || pfnLookEvent`), and calls our hook
  with `yaw = -dx*m_yaw`, `pitch = dy*m_pitch` once per frame. We convert back to counts and apply
  the TS chain (`m_filter`, TS accel `m_accel/m_accelexp/m_accelmax` or zoom sensitivity, then
  `sensitivity`, `m_yaw/m_pitch`), so there is no double input and `sensitivity` works (FWGS's
  own fallback ignores it). On Steam GoldSrc (exports ignored) the client reads the mouse like
  the SDK: Windows `GetCursorPos` minus `GetWindowCenterX/Y`, `SetCursorPos` re-centre,
  `SystemParametersInfo` acceleration save/restore in `IN_Activate/DeactivateMouse`, or SDL2
  relative motion when `m_rawinput 1` (SDL2.dll found with `GetModuleHandle`/`GetProcAddress`);
  Linux SDL2 relative motion with `SDL_SetRelativeMouseMode` on activate, looked up with
  `dlopen(RTLD_NOLOAD)`/`dlsym` in the engine's already-loaded libSDL2 (no link dependency).
  `IN_MouseEvent` forwards button edges as `K_MOUSE1+i` key events (the previous stub dropped
  mouse buttons entirely).
* Menus that show a cursor (buy, team, MOTD, the scoreboard in voice squelch mode) receive the
  mouse deltas as a virtual cursor (drawn with `sprites/cursor.spr`) while the view is held still;
  clicks go to the menu.
* **Joystick.** FWGS reads joysticks and gamepads itself and sends movement through
  `IN_ClientMoveEvent`, so the client's joystick code only runs on Steam GoldSrc. It is TS's copy
  of the classic SDK code: `IN_StartupJoystick` at init (skipped with `-nojoy`), the axis map from
  `joyadvanced`/`joyadvaxis*` (`joyadvancedupdate`, also run on first use), `IN_JoyMove` after the
  mouse in `CL_CreateMove` (thresholds, sensitivities, `+jlook`/`+strafe`/`lookstrafe`/`+speed`,
  relative axes, `joywwhack1/2`, pitch clamp ±89; active only with `joystick 1`), and
  `IN_Commands` from `HUD_UpdateClientData` (buttons 0-3 → `K_JOY1..4`, higher → `K_AUX1+i`, POV →
  `K_AUX29..32`). The device is read in `goldsrc::joystick`: WinMM (`joyGetPosEx`) on Windows as
  TS does, and on Linux the first SDL2 game controller (left stick X/Y, right stick Z/R, 21
  buttons, no POV), looked up at run time in the engine's SDL2 like the mouse, rescanned at most
  once a second for hot-plugging as the SDL-era SDK does.

## 4b. Voice

* **Server side** (`server/src/rules/voice.rs`, `CVoiceGameMgr`): see rules.md. It is required on
  Steam HLDS, which starts every client unable to hear anyone; FWGS defaults to all-hear.
* `HUD_VoiceStatus` updates who is talking: -1 the local player (talking turns `voice_modenable`
  on), -2 the server's acknowledgement, else a player (TS's check is `< 32`, so player 32 is never
  shown). Talking players get `sprites/voiceicon.spr` (scale 0.5, additive) 45 units above their
  origin when in the PVS and drawn (the local player only in third person), and a name label on the
  right half of the screen (up to 7, stacked down from mid-screen, team-coloured background,
  hidden while the scoreboard shows). The local talk icon sits at screen height − 3 font heights − 6.
* `VoiceMask` gives the audible (game rules) and server ban masks; `ReqState` makes the client
  resend `VModEnable` and `vban`. Every second the client sends `vban` if its ban mask for the
  connected players differs from what the server last reported.
* Scoreboard: a voice column shows, for players the rules let you hear, banned / talking
  (blinking every 0.3 s) / never spoken / not talking. With the scoreboard up, a mouse press enters
  squelch mode (cursor, no `+attack`); a press on an audible player's row toggles their mute, saves
  `<gamedir>/voice_ban.dt` (SDK format: version 1, then 16-byte unique ids) and prints
  `** #Muted #No_longer_hear_that_player` / `** #Unmuted` (titles.txt, `%s` = name) in the chat.
  Hiding the scoreboard ends squelch mode. Chat (`SayText`) from muted players is dropped.
  `voice_showbanned` lists the bans.

## 5. HUD draw pass (TS order)

HudText → death notices (`nohud` gate) → weapons element (laser dots, drawn even with `nohud`;
equipment/slot menu, kung-fu label or weapon info, crosshair or wire panel) → status element (round timer, TS messages, objective
text, teammate markers, crosshair ID, status panel, power-up, objective markers) → say text →
`CHudMenu` → (menus module) buy menu, team menu, MOTD, scoreboard, cursor. Colours from
`hud_r/g/b`, halved where TS halves them, red (195,45,45). Sprites: hud.txt `number_0..9`,
`d_*`; `sprites/player/*.spr`, `sprites/weapons/*.spr`, `cross_e/cross2/cross_t`, `divestep*`,
`wires/*`, `compass/uparrow/downarrow/cursor`. The crosshair gap is `22 (12 with the laser on) +
state.spread × 160` px (constant from the TS binary, `0x1007d7ac` = 160.0).

`DrawCrosshair` (1004e9f0) centres everything (lines, sprites, divesteps, laser-range number) on
`(cx, cy)`: the screen centre in first person; in third person the projection of the view
trace's end (`crosshair_centre`, as `DrawLaserDots`), or (−1, −1) when it projects behind the
view, which TS draws at as well. Its scoped flag (`0x1018a704`, FOV below 40) is cleared in
third person (`crosshair_scoped`), so a zoomed third-person crosshair keeps its lines.

Divesteps (1004ef50, drawn when not scoped or in third person; checked against the TS binary):
the three sprites at `(cx − 40, cy − 40)`. Sprite i is red (195, 45, 45) when `81 − free slots`
≥ 42 / 52 / 62; the threshold helpers (1004eec0/1004eef0/1004ef20) compute
`81 − slots − f()` with `f` (1004eeb0) a stub returning 0, so the decompile's
`HUD_GetUserEntity` term is that 0. Otherwise the sprite has the base colour (red while
`NextStunt ≥ 0.01`, else the HUD colour) ×1 for the band the pitch is in (< −10°, −10..10°,
> 10°) and ×0.5 for the other two. The pitch dot (2×2, alpha 100, base colour) is at
`(cx − 32·cos a, cy + 32·sin a)`, `a = clamp(3·pitch, ±90°)`.

Laser HUD (read from the TS binary, since the decompile hides the `__ftol` arguments):
* `DrawLaserDots` (1004e6f0): 2×2 fills, alpha 255, in the local player's `PlInfo` laser colour.
  When `state.items_ready` (TS `0x1018a628`: the `from` state's switched-on items, 0 while
  `m_flNextAttack > 0`, set in `HUD_PostRunCmd`) is non-zero and `cl_laser < 1`, the dot is
  pinned to the crosshair centre at `(cx−1+jx, cy−1+jy)`; akimbo dots at `(cx+3+jx, cy−1+jy)` and
  `(cx−5+jy, cy−1+jx)`, with `j = RandomLong(−10, 10) / 10` (±1 only at ±10). Otherwise each dot
  is at its projected hit point − (1, 1). No dot when the hit point projects behind the view.
* The traces (`effects::laser::draw_local`, `DrawLocalLaser` 0x1003bb00) start at the cull origin
  (±4 units along `v_right` per akimbo gun) along the beam direction; the projected point is the
  hit − `2·v_forward`, and `state.laser_range` = first hit distance × 0.025 (`0x1003d15f`).
* `DrawCrosshair` with the laser on draws `laser_range` (`DrawHudNumberTS` flag 1) at
  `(cx + 28, cy − fontHeight/2 − (8 when scoped in first person))`, in the crosshair colour ×0.7
  when the crosshair is on a player, ×0.5 otherwise (`0x1004ee02`).
* `DrawWeaponInfo` ends with `sprintf("%ix", 80 / m_iFOV)` at (24, 24) in the HUD colour when the
  FOV is below 40 (`0x1004dfbe`: `mov eax, 80; idiv fov`).

## 6. Buy menu

* `tsbuylist.tsa` is TS's **saved-config file**, not a buy list: nine 0x408-byte configs (ammo
  {rounds, buys}×16, 16 entries {item, id, attachments, cost, akimbo, count}, total, slots).
  Parsed and written byte-exactly (test against the shipped file). Writes go to a temp file then
  rename, so the xtask symlink to the TS content is replaced, never written through.
* Categories/rows are TS's hard-coded table (`+0x2d6c`). Pages: main, category, weapon list,
  options (attachments, akimbo / full load), accessories (Kevlar 1900, Stealth Shoes 700), remove,
  save, load, team (`chgteam`). Keys via `slot1..10` (10 = back) and `a` (= key 11, 10th row);
  left click selects any visible row (parent columns too), right click = back.
* Money is always 30000 and slots 81 (`CashBased()` is off in 3.0); ammo is auto-filled
  round-robin to the `ammocount` maximum. Restriction mask from `SrvSett 3`; grenades/stealth shoes
  only when `GameMode > 1`.
* Submit = `rst_cfg`, then `tkw <id>_<att|count> …` (akimbo = the gun twice, second copy without
  attachments), `tki <items> …`, `tka <type>_<buys> …`, each with TS's trailing space, after TS's
  bubble sort (melee/grenades first, then ascending price). Verified against the disassembly at
  0x10039620..0x10039810 (format strings `%i_%i `, `%i `, `tkw `/`tki `/`tka `).
* Opens only when dead or observing (`buy`, or automatically on becoming an observer); closes
  when alive again (except the team page); first open of the session random-buys like TS.

## 7. Deviations

* Fixed TS bugs: always-true `Objective`/`ShowObj` bounds; `Buy_SMGs` missing `#`; scoreboard rows
  are a `Vec` capped at 72 (no 74-entry overrun); `default_fov` registered as "90" (archived) and
  not forced to 80; buy-menu entry overflow and save/load slot 10/11 out-of-range reads guarded.
* Restriction mask starts at 0x1f instead of 0 until `SrvSett 3` arrives (0 would show every
  weapon as restricted).
* TS messages fade over their last second (TS fades too; the exact curve is hidden behind
  `__ftol` in the decompile). FOV smoothing rate is a guess for the same reason. Heartbeat/breath
  sounds use pitch 100.
* Clicking outside any buy-menu row does nothing (TS treats it as key 1 on the current page).
* Team select, MOTD and scoreboard are immediate-mode replacements for the VGUI1 panels (same
  commands and data, different look). Escape closes the buy/team menu, Enter/Space/Escape the MOTD.
* **No VGUI1 viewport; the client presents itself as a VGUI2 client instead** (issue #62). TS is
  a VGUI1 client: its viewport panel's `paintBackground` (`0x1005b3c0`) calls
  `VGui_ViewportPaintBackground`, and that engine call is what draws the 3D view and calls
  `HUD_Redraw` on Steam GoldSrc. Without an `IClientVGUI` the Steam engine (`VGui_Paint` in
  `hw.so`) treats the client as VGUI1 and, while the game UI is hidden, paints only the VGUI1
  root panel, expecting the client's viewport to do the rest. The port has no VGUI1 panels (C++
  ABI), so on the Steam client the world was never drawn: a black screen with the last frame of
  the "Loading… Parsing game info" dialog left on it, although the game ran. `client-dll`
  therefore exports `ClientFactory` and `CreateInterface` and answers `VClientVGUI001` with an
  `IClientVGUI` whose `UseVGUI1()` is false (`client-dll/src/vgui.rs`, a hand-laid C++ vtable:
  Itanium on Linux, MSVC `thiscall` on Win32). The engine then paints the viewport background
  itself every frame, as for Counter-Strike's VGUI2 client. In that mode the engine stops sending
  `IN_DeactivateMouse`/`IN_ActivateMouse` when the game UI opens and closes, so `HideClientUI`
  (from `CBaseUI::ActivateGameUI`) and `ActivateClientUI` (from `CBaseUI::HideGameUI`) forward
  to them. FWGS calls neither export and draws the world itself.
* Golden Colts are never restricted (server rule) where the client's own remap would treat them as
  pistols.
* `ts_hud_selftest [dead]` is port-only.
* Voice UI: TS drew the speaker labels, talk icon and scoreboard icons as VGUI1 panels with
  `gfx/vgui/*.tga` images, which the engine API can't draw. The port draws them in immediate mode:
  labels with the console font, `sprites/voiceicon.spr` as the local talk icon (green-tinted for
  the debug "server acked" icon), and a 16 px speaker glyph made of fills for the label and
  scoreboard icons (banned: red with a cross). Squelch mode centres the virtual cursor. The first
  scoreboard press (entering squelch mode) isn't passed to the engine as a key.
* `voice_ban.dt` is written whenever a mute changes (the SDK writes it when the library unloads).
  TS reads `scripts/voicemodel.txt` but never uses the height, so the port doesn't read it.
* Joystick on Linux uses an SDL2 game controller (TS only has the Windows WinMM code). The SDL
  rescan uses the client clock (`GetAbsoluteTime` isn't in the 2007 engine table). A failed device read releases
  the joystick buttons (TS's zeroed `JOYINFOEX` reads as POV "up"; the port treats it as centred).
* Director messages: TS exports only the old `HUD_DirectorEvent(command, primary, secondary,
  flags)`, which current engines (FWGS, Steam GoldSrc) never call. The port also handles
  `HUD_DirectorMessage`: `DRC_CMD_START` (1) becomes TS's `DRC_ACTIVE` reset and `DRC_CMD_EVENT`
  (2, the same `short, short, long` layout) TS's `DRC_CAMERA`; other commands are ignored, as TS
  ignores everything but those two.

## 8. Missing / not done

* `CHudSpectator`'s overview map and player name tags (`DrawOverview` `10024970`,
  `ParseOverviewFile` `10023720`, `Draw` `10023080`): TS draws them only for HLTV spectators
  (`IsSpectateOnly`). The rest of the element is in `client/src/spectator.rs`: director events,
  `specmode`, `SetModes` (crosshair, `#Spec_Mode%d`), the director-reset map camera. The VGUI
  spectator panel (`#Spec_TSHelp%d`) is not ported either.
* Server browser, command menu, TFC panels (dropped as allowed by §7).
* `HUD_UpdateClientData` writes the eased FOV (`view::update_client_data`, wired at integration).
* Death-cam / `cl_thirdpersonswitch` / observer third-person changes are only *requested*
  (`state.thirdperson_*`); the view package owns `cam_thirdperson`.

## 9. Tests and results

* `cargo test`: 35 client tests (message handlers on encoded buffers for status, power-ups, weapon
  grid, TS message stacking/replace, objectives, scoreboard, MOTD chunks, ShowMenu, spectator,
  TextMsg formatting; `DrawHudNumberTS` layout; localisation/printf; slot menu, fast switch, wires,
  next/prev skipping owned akimbos; buy menu file round trip on the shipped `tsbuylist.tsa`,
  add/cost/slot rules, restrictions, submit protocol strings, page machine, random config;
  scoreboard grouping; gate bindings; mouse acceleration; key-state fractions) and 6 protocol
  tests in `shared`. All pass.
* Builds: x86_64 Linux (dev), `i686-unknown-linux-gnu`, `i686-pc-windows-gnu`,
  `x86_64-pc-windows-gnu`. The Windows DLL imports only system DLLs (`USER32` added for the
  mouse); both exports `IN_ClientLookEvent`/`IN_ClientMoveEvent`.
* In engine (FWGS amd64, `~/games/xash_hud`, listen server on `ts_awaken`, `autotest.cfg`): the
  library loads (`[ts] input: FWGS mouse (IN_ClientLookEvent)`), no user-message errors, HUD draws.
  `ts_hud_selftest` showed the round timer, top/bottom TS messages, kill feed with `d_skull`,
  say text, status panel (stance sprite, health, kevlar, credits, free slots), kung-fu icon,
  equipment list sprites, AK47 weapon info with ammo name/fire mode/Scope row, crosshair lines with
  `cross2`, scoreboard (`+showscores`, "THE ONE" status). `ts_hud_selftest dead` + `buy` opened the
  buy menu (random first loadout Kevlar/Katana/M4A1 with Sil,Las,Sco; rifles red because they
  don't fit in 21 free slots); navigating with `slot` commands reached the options page, and
  "Use New Config" sent `rst_cfg`/`tkw`/`tki`/`tka` (the skeleton server answers "Unknown
  command") and showed "Using new config.". Screenshots: `img/client-hud-ingame.jpg`,
  `img/client-hud-buymenu.jpg` (the tiling WM resized the window, hence the odd aspect).
* Not tested in engine: physical keys/mouse (no input device in the automated run: the gate,
  mouse look, clicks and the virtual cursor are untested live), Steam GoldSrc mouse paths
  (Windows cursor / SDL2), team menu, MOTD, ShowMenu, death cam, effects queue consumers.
* Steam Half-Life 25th anniversary client (Linux, build 10211, `docs/validation.md` §5): the
  scripted `xtask play` scenes draw the world, HUD, buy menu, MOTD, view model, slow-motion indicator
  and bot fights. Under `gdb`, opening the console (`CBaseUI::ActivateGameUI`) reaches
  `input::deactivate_mouse` through `HideClientUI`, and the post-load `CBaseUI::HideGameUI` reaches
  `input::activate_mouse`, matching the calls the engine made in VGUI1 mode.

## 10. Integration notes

* `state::Shared` additions: `observer`, `spread` (prediction should write the gun's spread
  penalty), `ts_fade` (view applies TSFade), `active_items`/`laser_color` (effects: laser and
  flashlight), `ts_effects` queue (effects drains `PlayerHit`, `Frags`, `Wind`, `MotionBlur`),
  `thirdperson_toggles`/`thirdperson_leave`/`thirdperson_restore` (view consumes), `menu_open`.
  The HUD writes `health`, `dead`, `weapon`, `fov` (smoothed zoom FOV, default `default_fov`),
  `time_scale_target` (TSSlowMo) and eases `time_scale` in `HUD_Frame` (TS does it there).
* The FWGS `IN_ClientLookEvent`/`IN_ClientMoveEvent` exports are in `client-dll` and call
  `input::look_event`/`move_event`.
* The server should send `SrvSett 3` (restriction) and `GameMode` in `InitHUD` like TS, and
  implement `rst_cfg`/`tkw`/`tki`/`tka`, `weapon_<n>`, `drop_<n>`, `lasclr_<n>`, `usepowerup`.
* `shared::protocol::hud` encoders produce the exact TS byte layouts; the server's HUD sync can
  use them for its tests.
