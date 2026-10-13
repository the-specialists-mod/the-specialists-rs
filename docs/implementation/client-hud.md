# Client: input, HUD, menus (work package `wp/hud`)

Port of the TS 3.0 `client.dll` input layer, HUD elements and menus. Source of truth: decomp
`docs/client/architecture-hud.md` (cited as §n) and `client/decomp/*.c`. Layout follows
`docs/design.md` §6.

| File | Contents |
|---|---|
| `crates/client/src/input.rs` | kbuttons, `CL_CreateMove`, anti-script gate, input cvars, mouse back ends, virtual cursor, `IN_ClientLookEvent`/`IN_ClientMoveEvent` exports |
| `crates/client/src/hud/mod.rs` | hooks for every message, HUD cvars/commands, per-frame think, FOV/zoom sensitivity, slow-mo easing, draw pass, `ts_hud_selftest` |
| `crates/client/src/hud/state.rs` | `Hud` state and the pure message handlers (return `Effect`s) |
| `crates/client/src/hud/{status,weapons}.rs` | `CHudTSStatus`, `CHudTSWeapons` |
| `crates/client/src/hud/notice.rs` | `CHudDeathNotice` (TS's text weapon names), `GetClientColor`, `CHudMenu` |
| `crates/client/src/hud/message.rs` | `CHudMessage`: `HudText` titles.txt messages (fade, flicker, write-out effects) and the `GameTitle` sprites |
| `crates/client/src/hud/statusbar.rs` | `CHudStatusBar` (`StatusText`/`StatusValue`) |
| `crates/client/src/hud/tfc.rs` | the TFC messages TS still hooks: the viewport's `ValClass`/`Feign`/`Detpack`/`BuildSt`/`RandomPC`/`AllowSpec`, `CHudAmmoSecondary` (`SecAmmoVal`/`SecAmmoIcon`) |
| `crates/client/src/hud/{draw,text}.rs` | hud.txt sprites, `DrawHudNumberTS`, `DrawHudString`, `ftol`, the `Canvas` the SDK elements draw through, titles.txt localisation (`LocaliseTextString` with its step budget for the status bar), mini printf |
| `crates/client/src/hud/saytext.rs` | `CHudSayText`: chat lines, wrap, scroll, console print on scroll-off |
| `crates/client/src/hud/icons.rs` | `CHudStatusIcons` (`StatusIcon`, `Concuss`'s `dmg_concuss`) |
| `crates/client/src/hud/vgui.rs` | `CSchemeManager` (text schemes with all their colours, VGUI1 bitmap fonts from `gfx/vgui/fonts/*.tga`), VGUI1 `BitmapTGA` images, and VGUI1 panel painting with engine fills (clipping, insets, `LineBorder`, label alignment, `TextImage` text, modulated images) |
| `crates/client/src/menus/{mod,buy}.rs` | buy menu (`CTSBuyMenu`), the viewport's VGUI menu queue, key and mouse routing |
| `crates/client/src/menus/{team,motd,scoreboard,widgets}.rs` | `CTeamMenuPanel`, the MOTD's `CMessageWindowPanel`, `ScorePanel` (with `g_TeamInfo`), and the shared `CommandButton`, `CTransparentPanel`, team colours and viewport scheme colours |
| `crates/client/src/voice.rs` | `CVoiceStatus`/`CVoiceBanMgr`: speaker state, head icons, speaker labels, talk icon, scoreboard voice icons and squelch mode, `voice_ban.dt`, `VModEnable`/`vban` |
| `crates/client/src/joystick.rs` | the SDK joystick code TS ships (`IN_JoyMove`, `IN_Commands`, `Joy_AdvancedUpdate_f`), Steam GoldSrc only |
| `crates/client/src/state.rs` | fields added for other packages (see Integration) |
| `crates/client-dll/src/vgui.rs` | `ClientFactory`/`CreateInterface` exports and the `IClientVGUI` object that makes Steam GoldSrc draw the world (see Deviations) |
| `crates/goldsrc/src/ext/hud.rs` | engine helpers: sprites (`spr_load`, the one `SPR_Load` wrapper), text (`screen`, `draw_console_string` and `fill`, the one `pfnGetScreenInfo`/`pfnDrawConsoleString`/`pfnFillRGBA` wrappers), cvar strings, player info, `max_clients`/`client_time` (the one `GetMaxClients`/`GetClientTime` wrappers), demo API; client entities: `entity`/`local_player` (shared `&cl_entity_t`) and `with_entity_mut`/`with_local_player_mut` (a `&mut` scoped to a closure), the one `GetEntityByIndex`/`GetLocalPlayer` wrappers, with the aliasing rules in the module docs; `crosshair_trace` (TS `DrawCrosshairID` trace, built on `render::with_solid_players`/`player_trace`; `None` if the event API is null); `ev_play_sound`, the one `EV_PlaySound` wrapper (no-op if the slot is null); `ClEngine::event_api`/`efx_api`/`tri_api`/`demo_api` (null-checked bindgen `event_api_s`/`efx_api_s`/`triangleapi_s`/`demo_api_s`); `world_to_screen_ndc`, the one `WorldToScreen` wrapper (`None` if clipped or the slot is null; the laser dots use it), and TS's pixel `WorldToScreen` on top of it; `server_cmd`, the one `pfnServerCmd` wrapper (no-op if the slot is null); and `local_view_height`, the one `EV_LocalPlayerViewheight` wrapper (standing offset 0 0 28 if the slot is null; the events' gun position uses it too) |
| `crates/goldsrc/src/ext/voice.rs` | `GetPlayerUniqueID`, `pfnServerCmdUnreliable`, the static head-icon entities for `CL_CreateVisibleEntity` (and the server's `Voice_SetClientListening`) |
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
| `KB_Find` 10025880, `KB_Add` 100258c0, `KB_Init` 10025930 (end of `InitInput`, before `V_Init`) | `input::find`, `kb_add`, `kb_init`: the list holds only `in_graph`, `in_mlook`, `in_jlook` (newest first), matched with `_stricmp`; any other name is null |
| `HUD_Shutdown` 10027250 → `ShutdownInput` 10027240: `IN_Shutdown` (on Windows only `IN_DeactivateMouse` 100272a0), `KB_Shutdown` 10025970 | `client-dll` `HUD_Shutdown` → `input::shutdown` (mouse deactivated, kbutton list emptied, so `KB_Find` answers null afterwards), then `voice::shutdown` (see Deviations) |
| `IN_MouseMove` 100274d0 / `IN_MouseEvent` 100273d0 / `IN_Accumulate` 100278b0 | `input::apply_mouse`, `mouse_event`, `accumulate` (+ `native`, `win`, `sdl`); all three stand down while `camera::mouse_in_use` (`iMouseInUse` 0x100df394), leaving the motion to `CAM_Think` (`input::take_cursor_offset`) |
| `InterpolateSlowMotion` 10055ae0 | `hud::approach_timescale` (from `HUD_Frame`, `hud::frame`; TS's only time-scale easer) |
| `CHud::Think` 10049760 (from `CHud::UpdateClientData` 10024b00), `UTIL_ApproachFOV` 10049620, `HUD_GetFOV` 100209f0, `SetFOV` 10049570 | `hud::think_fov` (from `view::update_client_data`), `view::ease_fov`, `hud::get_fov` (demo record type 0), `hud::set_fov` (ignored while `cl_lw` is on; sets the target and `m_iFOV`, and the sensitivity `fov·0.0125·sensitivity·zoom_sensitivity_ratio`) |
| `CHudTSStatus` handlers 1004b730..1004c1e0 | `Hud::apply` (status arms) |
| `AddTSMessage` 1004b520 | `Hud::add_ts_message` |
| `CHudTSStatus::Draw` 10049ea0 and `DrawRoundTimer` 1004a130, `DrawPowerup` 1004a2e0, `DrawStatusPanel` 1004a3e0, `DrawTeammateMarkers` 1004ab40, `DrawCrosshairID` 1004acc0, `DrawTSMessages` 1004af40, the flag panel 1004b0e0, `DrawObjectiveMarkers` 10049fc0 | `hud::status::*` |
| `CHudTSWeapons` handlers 1004f6f0..1004fb50 | `Hud::apply` (weapon arms) |
| `CHudTSWeapons::Reset` 1004cd60, `Think` 1004ce10 | `Hud::reset_weapons`; `hud::think`, `weapons::think_sounds`, `hud::weapon_menu_buttons` |
| `Draw` 1004d3f0, `DrawWirePanel` 1004d550, `DrawKungFuBar` 1004d5f0, `DrawKungFuLabel` 1004d700, `DrawWeaponInfo` 1004d7e0, `DrawWeaponList` 1004e030, `DrawLaserDots` 1004e6f0, `DrawCrosshair` 1004e9f0 + 1004ef50 | `hud::weapons::*` |
| `SelectSlot` 1004f220, `SelectNextPrev` 1004f500, `CmdBuy` 1004fda0 | `hud::select_slot` → `weapons::select_slot`; `weapons::select_next_prev`; `hud::cmd_buy` |
| `DrawHudNumberTS` 1004c290, `DrawHudString` 10021ce0, `DrawHudStringLen` 10021ca0 | `draw::draw_number`/`number_cells` (the top place is `n / 10000` without `% 10`, as TS, so from 100000 on it draws the hud.txt entry that many after `number_0`; an index past the table draws nothing), `text::draw_string`, `text::string_len` |
| `LocaliseTextString` 100348d0, `BufferedLocaliseTextString` 100349a0, `LookupString` 100349c0 | `text::localise_text_string(_in)` (TS's word bytes `A`..`z` and digits, its step budget, the unterminated unknown `#`), `text::buffered_localise` / `localise_with` / `localise` (the shared 0x400-byte buffer: `AddTSMessage`, `ShowMenu`, buy menu, HUD labels), `text::lookup_string` (`TextMsg`: whole-string keys, a negative `effect` picks the destination), `text::lookup` |
| `MsgFunc_TextMsg` 10034a60 (`StripEndNewlineFromString` 10034a10, `ConvertCRtoNL` 10034a40) | `Hud::apply` (`Msg::TextMsg`) |
| `CHudMenu::MsgFunc_ShowMenu` 10028760 | `Hud::apply` (`Msg::ShowMenu`) |
| `KB_ConvertString` 10025740 (only caller: `ShowMenu`, 10028843), `Key_LookupBinding` | `input::kb_convert_string`, through `Ctx::key_binding` (`ClEngine::key_lookup_binding`) |
| `CHudGeiger` `Init` 1001f450 (`srand(time(NULL))`), `MsgFunc_Geiger` 1001f4a0, `Draw` 1001f4e0 | `hud::init`, `Hud::apply` (`Msg::Geiger`), `hud::geiger::draw` (from `hud::redraw`, before `CHudTrain`) |
| `CHudDeathNotice` Init 1001afa0, VidInit 1001b000 (`d_skull`), InitHUDData 1001afe0, `Draw` 1001b020, `MsgFunc_DeathMsg` 1001b360; `GetClientColor` 1001af40 | `hud::notice::DeathNotices` (`add`, `draw`, `clear`), `notice::on_death_msg`, `notice::client_color` |
| `CHudMessage` Init 10028900, VidInit 10028940, Reset 10028980, `FadeBlend` 100289b0, `XPosition`/`YPosition` 10028a50/10028ad0, `MessageScanNextChar` 10028b60, `MessageScanStart` 10028d40, `MessageDrawScan` 10028eb0, `Draw` 100290b0, `MessageAdd` 100293f0, `MsgFunc_HudText` 100295c0, `MsgFunc_GameTitle` 10029610 | `hud::message::HudMessage` and its helpers |
| `CHudStatusBar` Init 1002f3c0, Reset 1002f430, `ParseStatusString` 1002f480, `Draw` 1002f6f0, `MsgFunc_StatusText` 1002f850, `MsgFunc_StatusValue` 1002f8d0 | `hud::statusbar::StatusBar` |
| `CHudAmmoSecondary` Init 1001a5e0, Reset 1001a640, `MsgFunc_SecAmmoIcon` 1001a650, `MsgFunc_SecAmmoVal` 1001a690 (`Draw` is the stub 1001f8d0) | `hud::tfc::AmmoSecondary` |
| `ValClass`/`Feign`/`Detpack`/`BuildSt`/`RandomPC`/`AllowSpec` hooks (`CHud::Init` 10020170) → viewport 10064da0/10064ea0/10064ed0/10064ff0/10065020/100653a0; `Initialize` 10060aa0 | `hud::tfc::ViewportTfc` (`apply`, `initialize` from `hud::vid_init`) |
| `CHudSayText` 1002eb40, `CHudMenu` 100285f0 | `hud::saytext`, `hud::notice::draw_menu` |
| `CTSBuyMenu` 10036200..100398e0 (Init, Load/Save file, Open, Close, Draw pages, HandleKey 10038840, Add 10038f60, Remove 10038e40, ammo fill 100392b0, Submit 10039400, Random 100398e0, restriction 100365e0/1000b6d0, cost 10012ad0, slots 10038dc0) | `menus::buy::BuyMenu::*`, `menus::draw_buy`, `menus::save_file` |
| `CTSBuyMenu::MouseEvent` 10038360 | `menus::mouse_buttons` / `click` |
| `ScorePanel` 1005ce70 (`CreateScoreBoard` 10063a50), `Update` 1005d5f0, `SortTeams` 1005d6a0, `SortPlayers` 1005d8f0, `RebuildTeams` 1005da20, `FillGrid` 1005db70, `DeathMsg` 1005e5d0, `Open` 1005e630, `CLabelHeader` paint 1005e930/1005e9b0, `ShowScoreBoard` 10063710 (multiplayer only) | `menus::scoreboard::ScoreBoard::*`, `scoreboard::layout`, `menus::show_scores` |
| Viewport `TeamNames` 10064df0, `ScoreInfo` 10065090 (`UpdateOnPlayerInfo` 10064750), `TeamScore` 10065120, `TeamInfo` 100651c0; `gViewPort->DeathMsg` from 1001b360 | `menus::on_message` (called from `hud::on_message` after the HUD state took the message) |
| `CTeamMenuPanel` 100655d0 (VGUIMenu 2; opened only when `GameMode` said teamplay, `0x10064280`, since the server sends it for `changeteam` in any mode), `Update` 10066020, `SlotInput` 100665a0, `SetActiveInfo` 10066690, `Open` 10066610, `paintBackground` 10066640; `CMenuHandler_StringCommandWatch` 10064a20 | `menus::team::TeamMenu::*`, `menus::team_menu_key` |
| `CommandButton` 1005a0b0 (`Init` 1005a170, `RecalculateText` 1005a1f0, `paint` 1005a2f0, `paintBackground` 1005a370), `CTransparentPanel`, `iTeamColors` 10096974, viewport scheme colours 100607f0 | `menus::widgets` |
| `ShowVGUIMenu` 10064050, `HideTopMenu` 10064200, `SetCurrentMenu` 10063bd0, `KeyInput` 10064b90 | `menus::show_vgui_menu`, `hide_top_menu`, `key_event` |
| `MsgFunc_VGUIMenu` 10064f00 (menu 4's `m_sMapName`), `CreateTextWindow` 10063c00 (MOTD and map briefing), `CMessageWindowPanel` 1005bb90 | `menus::vgui_menu_message`, `menus::motd::{Motd, map_briefing}` |
| `CTFScrollPanel` 1005a860 (`CTFScrollButton` 1005a620/paint 1005a6b0, `CTFSlider::paintBackground` 1005a750, `LoadTGAForRes` 1005a070); VGUI1 `ScrollPanel`/`ScrollBar`/`Slider` (`vgui.so`); `CListBox` 1005b490 (`InternalLayout` 1005b730, `SetScrollRange` 1005b860, `setPixelScroll` 1005b700), `ScrollBar2` 100153f0, `Slider2` 10016690 | `menus::scroll::{ScrollPanel, ScrollBar, Slider, ListBox}` |
| `CVoiceStatus::Init` 100178c0 / `VidInit` 10017af0 / `Frame` 10017ea0 | `voice::init`, `vid_init`, `frame` |
| `CVoiceStatus::CreateEntities` 10017f30 (`HUD_CreateEntities`) | `voice::create_entities` |
| `UpdateSpeakerStatus` 100180c0 (`HUD_VoiceStatus` 1001ab40) | `voice::update_speaker_status` |
| `UpdateServerState` 10018360, `HandleVoiceMaskMsg` 10018780, `HandleReqStateMsg` 10018830 | `voice::update_server_state`, `msg_voice_mask`, `msg_req_state` |
| `RepositionLabels` 10018900, `UpdateBanButton` 100185f0 (`UpdateSpeakerImage` 100185d0) | `voice::redraw`/`label_rects`, `voice::speaker_icon`/`icon_image`/`scoreboard_icon` (scoreboard voice column) |
| `Start/StopSquelchMode` 10018870/10018890, `IsInSquelchMode` 100188b0, `IsPlayerBlocked` 10018c80, `IsPlayerAudible` 10018cc0, `SetPlayerBlockedState` 10018cf0 | `voice::start_squelch_mode` … `voice::toggle_block` |
| `ScorePanel::mousePressed` 1005e660, `cursorMoved` 1005e7f0, `MouseOverCell` 1005e870, `HideScoreBoard` 10063760 | `menus::scoreboard_press`, `menus::mouse_buttons`, `ScoreBoard::cursor_moved`, `menus::show_scores(false)` |
| `CVoiceBanMgr` (`voice_ban.dt`), `voice_showbanned` 10017660 | `voice::BanList`, `voice::show_banned` |
| `SayTextPrint` 1002ed00 (muted players' chat dropped) | `hud::on_message` (SayText) |
| `IN_StartupJoystick` 10027960, `IN_ReadJoystick` 10027de0 | `goldsrc::joystick::startup`/`read` |
| `Joy_AdvancedUpdate_f` 10027b00, `IN_Commands` 10027ca0, `IN_JoyMove` 10027e40 | `joystick::axis_map`/`joy_advanced_update`, `joystick::commands`, `joystick::joy_move`/`apply_axes` |
| `CMessageWindowPanel` 1005bb90 (MOTD, VGUIMenu 5), `CreateTextWindow` 10063c00 | `menus::motd::Motd` |
| `CHud::MsgFunc_Concuss` 10021a40 (hooked at 1001fe40), `MsgFunc_Logo` 100209c0, `MsgFunc_ResetHUD` 100219a0, logo draw 10021bcd | `hud::msg_concuss` (`m_iConcuss` → `hud::concussion` → `v_idlescale` in `view::update_client_data`, as `0x10024b63`), `hud::msg_logo`/`draw_logo`, `Hud::apply` (ResetHUD) |
| `CHudStatusIcons` 1002f0e0 (Init), `MsgFunc_StatusIcon` 1002f1a0, `EnableIcon` 1002f210, `DisableIcon` 1002f330, `Draw` 1002f130, `Reset` 1002f110 | `hud::icons::StatusIcons`, `hud::msg_status_icon` |
| `CHudSayText` `SayTextPrint` 1002ed00, `MsgFunc_SayText` 1002ecc0, `Draw` 1002eb40, `ScrollTextUp` 1002ead0, `EnsureTextFitsInOneLineAndWrapIfHaveTo` 1002ee90, `InitHUDData` 1002eaa0; `GetClientColor` 1001af40 | `hud::say_text_print`, `hud::saytext::SayText`, `hud::draw_say_text` |
| `TeamFortressViewport::AllowedToPrintText` 10064240 | `menus::allowed_to_print_text` |
| `CSchemeManager` 1005c300, `LoadFileByResolution` 1005caf0 | `hud::vgui::Schemes::load`, `hud::vgui::BitmapFont` |
| `vgui_LoadTGA` 1005b8d0, `vgui_LoadTGANoInvertAlpha` 1005ba50 (VGUI1 `BitmapTGA`) | `hud::vgui::Image::from_tga`, `hud::vgui::image` |
| `CHudSpectator` 10022c70 (Init), 10022fe0 (VidInit), `DirectorMessage` 10023380, `SetModes` 100234d0, `specmode` 10022c10, map camera 10022da0, `Draw` (HLTV name tags) 10023080, `ParseOverviewFile` 10023720, `LoadMapSprites` 10023c60 | `spectator::{init, vid_init, director_event, set_modes, redraw}`, `spectator::overview::{draw_names, Overview::reset, parse_overview}`; `HUD_DirectorEvent` 1001ab60 and `HUD_DirectorMessage` exports. `VidInit` also zeroes the 0.2 s click timer (`+0xc94`, `input::reset_key_tap`): the client clock restarts with each map, and a timer left from the old map blocked click-to-spawn after a `changelevel` |

## 2. Message table

Every message in `shared::protocol::MESSAGES` is hooked except `ScreenShake` and `ScreenFade`
(FWGS and GoldSrc parse those two in the engine before client hooks are consulted) and `SelAmmo`,
none of which TS hooks. The client-only hooks `Logo` (`m_iLogo`, the animated
`sprites/%d_logo.spr`), `Concuss` (`m_iConcuss` and the `dmg_concuss` icon), `StatusIcon`
(`CHudStatusIcons`), `StatusText`/`StatusValue` (`CHudStatusBar`), `SecAmmoVal`/`SecAmmoIcon`
(`CHudAmmoSecondary`) and the viewport's `ValClass`, `Feign`, `Detpack`, `BuildSt`, `RandomPC`
and `AllowSpec` are registered as TS registers them. Neither TS's server (`mp.dll`/`ts_i386.so`
contain none of these names except `Logo`) nor the port's registers them, so they never arrive
and the server's message table and order are unchanged. The TFC handlers only store their
values (`-1` past the end of a short message, as the SDK's `READ_*` give it); their readers are
the command menu's buttons, which the port doesn't have. The voice module
hooks `VoiceMask` (audible and server-side ban masks) and `ReqState` (answers `VModEnable` +
`vban`). Malformed payloads are logged and ignored (see Deviations).

| Message | Read order | Effect |
|---|---|---|
| TSHealth | B hp | health; also the int `0x100cf9d8` (`Hud::last_hp`, mirrored to `state.health`), which only this handler writes (`ResetHUD` leaves it) and which `0x10026860` (`<= 0`) reads for `CL_ButtonBits`' forced +score (`0x10026936`) and the predicted `ItemPostFrame` gate (`0x10003d54`); hp 0, not observing and not already in third person → `thirdperson` toggle (TS `1004b730`). This toggle is the whole death cam: TS has no other dead-camera flag, and everything that checks third person reads `cam_thirdperson` |
| TSArmor / TSState / TSSpace | B | kevlar / stance / free slots |
| TSCash | S | credits |
| PwUp | S flags, B value | flags & 0x104 → kung-fu / movement icon, else timed power-up |
| PwUpTime | B secs | expiry = now + secs, value 0. `DrawPowerup`'s name sits in a static buffer rewritten only for flags 1/2/8/0x10, so another bit shows the last name again (`Hud::pw_label`, issue #96) |
| RoundTime | B secs, B mode | timer (mode 0: 1 s, 1: 0.1 s, 2: frozen) |
| Objective | Co×3, B idx, Str | idx 0 clears; bounds fixed (< 32) |
| ShowObj | B idx, B visible | |
| TSMessage | B r,g,b, B flags, B time (tenths, ≤100), Str | `AddTSMessage` (positions, stacking, 0x10 replace, grey if it names you) |
| PTakeDam | Co×6, B | queued `TsEffect::PlayerHit` |
| TSSlowMo | Co (×0.01, 0 → 1) | `state.time_scale_target`; `goslow.wav` when slow-mo starts, through `EV_PlaySound(-1, local origin, 0, …, 1.0, 0.8, 0, 100)` (`0x1004bf53`; **was** `PlaySoundByName`, issue #96) |
| TSFrags | Co×6, B type [+Co×6, L, Co] | queued `TsEffect::Frags` |
| TSPAward | B cl, B award | scoreboard status |
| SrvSett | B key, S value | realbullet / cashbased / ammocount / restriction mask |
| TSBTime | B | bullet time |
| TSHVoice | B | `misc/successful.wav` / `misc/failed.wav` |
| TSFlag | B state, B idx, B team, B carrier | flag table, drawn by the flag panel (`0x1004b0e0`): each flag with a state and team 1..4 at `(w − 56, team·48 + 40)` in `iTeamColors[team]`, blinking (× 1 / 0.5 every 0.1 s while carried, else × 0.75 × 1 / 0.5 every 0.5 s, timers on frame time × time scale); the carrier's name (24 bytes) right-aligned against it, red (175,40,40) for the other team and green (40,175,40) for yours, and "Take the Briefcase to the base!" at y 320 (HUD colour, red + 35) when you carry it. TS has no objective-text block and never draws flags over carriers (its test reads a field nothing writes). **Was**: an invented wrapped objective-label block at the bottom, flags over carriers, no panel (issue #94) |
| TSMBlur | B ent, B mode | queued `TsEffect::MotionBlur` |
| TSPSys | B type [type 3: L, L, Co] | queued `TsEffect::Wind` |
| PlInfo | B cl, B key, B value | laser colour (key 0) |
| TSFade | S, S, S, B×4 | `state.ts_fade` (seconds = value/4096) |
| WeaponInfo | B id, B clip, S reserve, B mode, B att | current gun, grid. Every id byte is a weapon, 0xff included: TS's `READ_BYTE` (`0x10029a10`) zero-extends and the handler keeps the old id only on -1, a read past the end (`0x1004f822`; the port drops a short message). An id outside 0..38 takes entry 0's ammo type and grid slot, as `GetWeaponInfo` (`0x100071c0`) answers it with kung fu; `cl_thirdpersonswitch` 1 (TS `1004f93b`, the only reader): kung fu (id < 1) toggles to third person if not already there; a gun toggles back to first person only when the weapon before the message was kung fu (id 0). Between messages the prediction writes the clip, fire mode, attachments and menu cells (`Hud::predicted_weapon`, TS `0x100581c0` from `0x1000acd0`, on first predictions with a record), so a mode switch plays `switch.wav` from `Think` straight away; the reserve is this message's alone. **Was**: a predicted overlay on the weapon box only, with the predicted reserve, and no switch sound until the next `WeaponInfo` (issue #12) |
| ClipInfo / WStatus / ActItems / NextStunt / Wires / KFuPower / CustomWP | §3.3 | grid, active items, stunt cooldown, wires, kung-fu power, custom weapon params. `Think` (`0x1004ce10`) runs the stunt cooldown and the kung-fu regeneration (15/s, 1/s while negative) on `frame time × max(time scale, 0.01)`, so both slow down in slow motion (**was** ÷, issue #90). `CustomWP` (`0x1004fb50`) writes accuracy, view kick, accuracy kick, damage and cost into the client's weapon table for the rest of the session (`shared::weapons::table::set_weapon_params`), which prediction, the events and the buy menu read (**was** stored only, issue #91). A fire-mode change plays `weapons/switch.wav` through `EV_PlaySound` on the local player (channel 3, volume `RandomFloat(0.92, 1)`, attenuation 1.25, pitch `RandomLong(−5, 5) + 25 − ftol(ts × −75)`); while dead or observing the change isn't taken, so it clicks on respawn |
| ScoreInfo / TeamInfo / TeamScore / TeamNames | stock | scoreboard data |
| DeathMsg | B, B, Str | `CHudSpectator::DeathMessage(victim)`, then a kill-feed entry (TS `1001b360`): names from `GetPlayerInfo` (31 bytes) with `GetClientColor` colours fixed at message time; victim 0xff is a non-player kill (the weapon string is the object's name); suicide = killer 0 or killer = victim; team kill = weapon `teammate`; the display time is `(int)hud_deathnotice_time` s. TS's change from the SDK: no weapon sprite. A plain kill keeps the weapon name and draws it as text in (1, 0.5, 0.2) between the names; suicides, team kills and non-player kills draw `d_skull`, white (green 10,240,10 for a team kill; the SDK's 255,80,0 is gone). Up to four, right-aligned, from y 20 in 20 px steps, only while `AllowedToPrintText`. Console: `<k> killed <v>`, `<v> killed self` / `<v> died` (weapon `world`), `<k> killed his teammate <v>`, `<k> killed a <object>`, then ` with <weapon>` unless `world` or a team kill (`egon`/`gauss` become `gluon gun`/`tau cannon`) |
| HudText / GameTitle | Str / – | `CHudMessage` as the SDK ships it: up to 16 titles.txt messages (the `#` dropped for the lookup), each replacing any message at the same position and skipped when the same text already shows; an unknown name becomes the single shared custom message (write-out, centred at 0.7, the name as text, so a second custom message rewrites the first). Drawn a character at a time with `pfnDrawCharacter`: effect 0 fade in/out, 1 the same with a 10 % chance per draw (CRT `rand`) of the second colour drawn under it, 2 write-out per `fadein` with an `fxtime` colour sweep. Start times in the future (level change) move back with 0.2 s of grace. `GameTitle` draws `title_half`+`title_life` faded by `FadeBlend` |
| SayText | B client, Str | `SayTextPrint(text, client)` (TS `1002ecc0`): dropped for a muted player; console only while `AllowedToPrintText` is false (team menu or MOTD up and no player class); otherwise a chat line (`\x02` lines colour the speaker's name with `GetClientColor`: white for the local player, else the team colour of teams 1-4, the rest in the default text colour), wrapped at the screen width − 40, `misc/talk.wav`. Lines reach the console only as they scroll off. `TextMsg` HUD_PRINTTALK goes the same way with client −1, and every `TextMsg` is skipped while `AllowedToPrintText` is false |
| StatusText / StatusValue (never sent) | B line, Str / B index, S value | `CHudStatusBar`: line 0 or 1 (cut to 127 bytes, turns the element on), values 1..7. On the next draw each line is localised (`LocaliseTextString` into 128 bytes, whose step budget cuts long lines at `128 − steps − 1`), split at `\n`, and the parts whose leading number names a non-zero value are kept with `%pN` (player name, its `GetClientColor` for the line; `******` without one), `%iN` and `%%` substituted. Line 0 at (5, h − 55) (h − 45 below 480 px), line 1 a text height higher, or with `hud_centerid` centred at h/2 + `hud_centerid` text heights |
| TextMsg / ShowMenu / MOTD / ServerName / GameMode / Spectator / VGUIMenu / ResetHUD / InitHUD / SetFOV | stock (TS-modified Spectator/VGUIMenu) | as in the SDK/§3.2. ResetHUD resets every element (status icons, `CHudMessage`, the status bar and `CHudAmmoSecondary` too), then `m_flMouseSensitivity` and `m_iConcuss` (TS `100219a0`) (Spectator, TS `10065250`, for the local player on every message: observer is `observer == 1`; an observer leaves third person and gets the buy menu, otherwise `cl_thirdperson` 1 or 0 picks the camera and other values keep it, so the death cam doesn't outlive the respawn. This handler is the only reader of `cl_thirdperson`. Then, observer or not, `0x100652fd` runs `0x1000b9d0` on the client `player`: the predicted spread penalty (`player+0x764`) and the crosshair's copy (`0x1018d188`) go to 0 (`prediction::reset_spread`); the other fields it zeroes, `player+0x768..0x770` and `ev_punchangle` (`0x10191480`), are never read or never non-zero in the TS client. The per-player flag is stored as `observer != 0`: TS keeps the raw byte (`0x101900ec[i]`, `0x101901f5`), but `HUD_AddEntity` (`0x10044846`) and the scoreboard (`0x1005e3e8`) only test it for zero) |
| Train | B notch | `CHudTrain`: `sprites/%d_train.spr` frame `notch - 1` in (255,160,0), x = w/3 + sprite w/4, y = h - sprite h - `m_iFontHeight`; hidden at 0. `m_iFontHeight` (`0x100cf988`, `CHud::VidInit` `0x100208a3`) is the height of hud.txt's `number_0` (16 in TS's), not the engine's character height; its only other readers are the laser range and the talk icon (issue #37) |
| HideWeapon | B bits | `hide_hud`; HIDEHUD_ALL (4) hides the train element. TS only resets the engine crosshair, which it doesn't use |
| CurWeapon, WeaponList, AmmoPickup, WeapPickup, ItemPickup, AmmoX, Health, Battery, Damage, SelAmmo | stock | parsed by hook, no effect (TS's elements draw nothing) |
| Geiger | B | `CHudGeiger`: range = byte × 4, element on for good. Each frame (before `CHudTrain`, not at intermission or under `HIDEHUD_ALL`), with the range in 1..999: CRT `rand` gives the volume (`(r & 127) × scale / 255 + 0.25`), a click when either of two more `rand`s (`& 127`) is under the range's chance (0 past 800, then 2, 4, 8, 8, 28, 40, 60, 80, 90, 95 out of 128 as it falls), and one or two more choose `player/geiger1..3.wav` (`PlaySoundByName`). The CRT `rand` is the one the HudText flicker uses, seeded by `srand(time(NULL))` in `CHudGeiger::Init` |
| TextMsg | B dest, Str × 5 | `LookupString` on each (a whole-string `#key`; a negative titles `effect` on the first sets the destination), one trailing `\n`/`\r` off each argument, `sprintf(msg, a1..a4)`, `\r` → `\n`; 1 notify console (leading `\x01`), 2 console, 3 chat (`SayTextPrint(…, -1)`), 4 centre print, others nothing; all skipped while `AllowedToPrintText` is false |
| ShowMenu | S slots, C time, B more, Str | slots and shutoff at once; no slots hides the menu (keeping the "more" flag); otherwise the text starts (or after a "more" chunk extends) the 511-byte prelocalised string, the last chunk localises it whole (`BufferedLocaliseTextString`) and swaps each `+command` for `[key]`, the key bound to it (`KB_ConvertString`; unbound commands stay as written), and the menu shows after every chunk. `CHudMenu::Draw` (`0x100285f0`) draws it plain: each line white at x 20 (clipped at x 320), 12 px apart from `h/2 − (newlines/2)·12 − 40`, with no `\w \d \y \r \R` escapes (TS's menu predates them; they show as text), and not while the scoreboard is up. **Was**: escapes parsed into colours and right alignment, font-height lines (issue #34) |
| VGUIMenu | B id [, Str map] | menu 4's map name goes to `m_sMapName` (63 bytes); every id to `ShowVGUIMenu`: 2 team menu (teamplay only), 4 map briefing (`maps/<m_sMapName>.txt`, or the level's when the name is empty, which then fills it; nothing without the file), 5 MOTD |

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
* Not registered here (other packages):
  * V_Init (`view::init`): `cl_bob…`, `cl_slowfade`, `v_center*`, and `scr_ofsx/y/z` (0, never
    read by TS; the SDK adds them to the view origin).
  * CAM_Init (`view::cam_init`, called from `input::init` as `InitInput` calls it):
    `cam_command cam_snapto cam_idealyaw cam_idealpitch` 0, `cam_idealdist` 100, `cam_contain` 0,
    `c_maxpitch/c_minpitch` 90.0/0.0, `c_maxyaw/c_minyaw` 135.0/-135.0,
    `c_maxdistance/c_mindistance` 200.0/30.0, all flags 0. `CAM_Think` reads `cam_command`,
    `cam_snapto` and `cam_ideal*` (client-rendering.md); `c_*` bound the `+cammousemove`/
    `+camdistance` adjustments (`camera`); `cam_contain` is never read. `CAM_Init` also registers
    the camera commands (`camera::register_commands`), `thirdperson` and `firstperson`.
  * CHud::Init cvars that `effects` reads (`effects::init`): `r_paintball` and `gl_fog` (0,
    archived), `gl_fog_start` 0, `gl_fog_end` 2048, `gl_fog_red/green/blue` .5 (flags 0; the
    `gl_fog 2` user fog).
  * Scheme_Init (`menus::init`, right after the HUD): `bitmapfonts` 1, flags 0. The scheme
    manager (`hud::vgui`) reads it when it loads the fonts.
* Engine-owned names: FWGS's `ref_gl` registers `gl_fog` itself (client-rendering.md,
  Deviations); none of the other names above are engine cvars in FWGS.

Each cvar is registered once, with `goldsrc::client::Cvar::register` (a plain
`pfnRegisterVariable`, as TS does), where TS registers it. Other modules share that handle:
`view` and `effects` take the `InitInput` cvars from `input::cvars()` and `default_fov` from
`hud::cvars()`, so `init` order matters (input, hud, then view and effects). A second registration
is not harmless: FWGS's `Cvar_Get` resets an existing cvar to the new default and ORs in the new
flags, and GoldSrc's `Cvar_RegisterVariable` (ReHLDS) refuses it ("already defined"). A unit test
(`input::tests::each_cvar_is_registered_once`) scans the sources for repeats and for the
CAM_Init, fog, `scr_ofs*` and `bitmapfonts` names. Cvars registered by
the engine or another module that this code only reads (`host_ver`, `sv_cheats`,
`r_drawentities`) use `Cvar::find` (`pfnGetCvarPointer`).

Commands: `+/-moveup movedown left right forward back moveleft moveright lookup lookdown speed
attack attack2 use jump klook jlook duck reload alt1 alt2 score showscores graph break`, `impulse`,
`slot1..slot10`, `invnext`, `invprev`, `buy`, `cancelselect` (`UserCmd_Close` `0x1001a4e0`: closes the
weapon menu with `common/wpn_hudoff.wav`, else sends `escape`; Escape's hard binding, which Steam
GoldSrc and FWGS 0.21 run for Escape in game, so this is how Escape opens the main menu there),
`special`, `+commandmenu`/`-commandmenu`, `ForceCloseCommandMenu` and `togglebrowser` (registered as
TS's `CHud::Init` does, but no-ops: TS's command menu opens only for a player with a class, which
TS's server never gives, and has no `commandmenu.txt`; the server browser isn't ported; issue
#88), `specmode` (HLTV only),
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
  drives 0x40 and `+alt2` drives `IN_ATTACK2`. `+attack2` also sends `_special`. The `-` commands
  release only the kbutton the current mapping picks (`0x10025fc0`, `0x10026180`), so switching
  weapons while the key is held can leave the other one down, as in TS. Key numbers come from
  `Cmd_Argv(1)` through MSVC `atoi` (none: -1).
* Gate: `+attack +attack2 +alt2 +reload +forward +back +moveleft +moveright +jump` only act when
  the same frame saw a physical key bound to exactly that command (FWGS order checked:
  input events → `Cbuf_Execute` → `HUD_Frame`). So `autotest.cfg` can't press them, like TS.
* Turn clamps (`CL_AdjustAngles` 0x10026330): `cl_yawspeed` in [0, 200], `cl_pitchspeed` in
  [0, 220] (not above 0 → 0), `cl_anglespeedkey ≥ 0.67`, pitch ±89, roll ±50. The yaw goes
  through the SDK's `anglemod` (truncated to 1/65536 of a turn) on every command unless
  `in_strafe` is held, so the previous frame's mouse yaw is quantised too.
* `CL_CreateMove` (0x10026570): the movement vector is scaled down to `GetClientMaxspeed` before
  `IN_Move` (the joystick and FWGS move events come after the clamp); with the `joystick` cvar set
  a forward/backward move adds `IN_FORWARD`/`IN_BACK`. `g_iAlive` (0x100936a4, set in
  `HUD_TxferPredictionData` to `iuser1 != 0 || deadflag == 0`) freezes only the *command's* view
  angles at the last live ones; the engine's view angles keep following the mouse. TS's server
  never writes `deadflag`, so in practice nothing freezes.
* `IN_MouseMove` (0x100274d0): `m_filter` averages with the previous *filtered* value (a filtered
  x within ±0.01 is 0); the acceleration adds `sign(raw) · min(|raw|^m_accelexp · m_accel,
  m_accelmax)` (power and sign of the raw count, sign −1 for 0, any `m_accel`); `sensitivity` is
  not clamped.
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
  clicks go to the menu. The hold is only against the player's own turning: angles the server
  sets meanwhile (`svc_setangle` on spawn, teleports, `ts_face`) replace the held ones, as in TS,
  whose menus only stop the mouse. (Holding them too left the Steam client facing the old way,
  and shooting there, after a spawn or teleport with the MOTD open.)
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
  right half of the screen (up to 7, stacked down from mid-screen: a team-coloured label at VGUI
  alpha 100 with the name in white "Scoreboard Text" and `gfx/vgui/speaker4.tga`). While the
  scoreboard shows, `Frame` (`0x10017ea0`) hides only the names; the box and icon (the name's
  siblings) get their visibility once, when the label is given out (`0x10018240`), so a label
  already up keeps its box and icon over the scoreboard, and one given out under the scoreboard
  stays hidden until that player stops and starts talking again (**was**: the whole label hidden
  with the scoreboard, issue #95). The local talk icon (`gfx/vgui/icntlk_pl.tga`, `icntlk_sv.tga` once the server
  acknowledged with `voice_clientdebug` on) sits at screen height − 3 × `m_iFontHeight` (the `number_0` height) − 6.
* `VoiceMask` gives the audible (game rules) and server ban masks; `ReqState` makes the client
  resend `VModEnable` and `vban`. Every second the client sends `vban` if its ban mask for the
  connected players differs from what the server last reported.
* Scoreboard: a voice column shows, for players the rules let you hear, banned
  (`640_voiceblocked.tga`) / talking (`640_speaker4`/`640_speaker3`, blinking every 0.3 s, orange)
  / never spoken (`640_speaker1`, grey) / not talking (`640_speaker2`), in the row's colour; the
  local player's row has none unless spectate-only. With the scoreboard up, a mouse press enters
  squelch mode (cursor, no `+attack`); the cursor lights an audible player's row and a press on a
  player row toggles the lit player's mute (saved
  to `<gamedir>/voice_ban.dt` at shutdown; SDK format: version 1, then 16-byte unique ids) and prints
  `** #Muted #No_longer_hear_that_player` / `** #Unmuted` (titles.txt, `%s` = name) in the chat.
  Hiding the scoreboard ends squelch mode. Chat (`SayText`) from muted players is dropped.
  `voice_showbanned` lists the bans.

## 5. HUD draw pass (TS order)

Train → HudText (`CHudMessage`) → status bar → death notices (`nohud` gate) → status icons → HLTV name tags → weapons
element (laser dots, drawn even with `nohud`; then without `nohud` the buy menu, which is also
closed here once you're alive; equipment/slot menu, kung-fu label or weapon info,
crosshair or wire panel) → status element (round timer, TS messages, flag panel, teammate
markers, crosshair ID, status panel, power-up, objective markers) → say text → `CHudMenu` → the
logo → (VGUI pass) spectator panel, voice labels, then the viewport's
children in their order: team menu, scoreboard, MOTD; then the cursor. As in `CHud::Redraw` (`10021aa0`), `HIDEHUD_ALL` hides every element, and
during intermission only the say text (`HUD_INTERMISSION`) and the logo draw. TS messages fade
over their last second: each channel is `__ftol(base × min(expire − now, 1))`, the base being the
message colour or, for a black message, the `hud_r/g/b` cvar values (`0x1004afc9`). Colours from
`hud_r/g/b`, halved where TS halves them, red (195,45,45). Sprites: hud.txt `number_0..9`,
`d_*`; `sprites/player/*.spr`, `sprites/weapons/*.spr`, `cross_e/cross2/cross_t`, `divestep*`,
`wires/*`, `compass/uparrow/downarrow/cursor`. The crosshair gap is `22 (12 with the laser on) +
state.spread × 160` px (constant from the TS binary, `0x1007d7ac` = 160.0).

`DrawCrosshair` (1004e9f0) centres everything (lines, sprites, divesteps, laser-range number) on
`(cx, cy)`: the screen centre in first person; in third person the projection of the view
trace's end (`crosshair_centre`, as `DrawLaserDots`), or (−1, −1) when it projects behind the
view, which TS draws at as well. Its scoped flag (`0x1018a704`, FOV below 40) is cleared in
third person (`crosshair_scoped`), so a zoomed third-person crosshair keeps its lines.

Crosshair ID trace (`DrawCrosshairID` 1004acc0, read from the TS binary; `ClEngine::crosshair_trace`,
which `crosshair_centre` shares): `EV_SetSolidPlayers(local − 1)`, `EV_SetTraceHull(2)`,
`EV_PlayerTrace(start, end, PM_STUDIO_BOX, ignore_pe = local, &tr)` with `local` the local
player's entity index (`push [0x1018a53c]` at `0x1004ada3`), then `EV_IndexFromTrace(&tr)` and
`GetEntityByIndex` on the result whether or not the trace hit; a name is shown if that entity is
non-null and a player. The port does the same (issue #61). `ignore_pe` is not an entity index:
FWGS (`PM_PlayerTraceExt`, `pm_trace.c`) and the shared GoldSrc pmove code (ReHLDS
`_PM_PlayerTrace`) skip `physents[ignore_pe]`. The client's physent list is the world (slot 0),
then the solid non-player entities in this frame's packet entities (`CL_AddLinksToPmove`: brush
models with clip hulls and studio models, so only what the server sent, i.e. roughly the PVS),
then the other players that `EV_SetSolidPlayers` appends (the local player is already left out).
So TS's value skips one unrelated physent:
* when at least `local` such entities are in the frame (doors, breakables, walls): one of them, so
  a player behind that one entity can be identified through it, and in third person the crosshair
  projects through it;
* otherwise one of the other players (the `local − count`-th solid one in slot order), who then
  can't be identified until more entities come into the frame. A listen-server host (index 1) only
  hits this with no solid entity in the frame; high slots on a full server hit it more often.

Both are what TS players saw, so the port keeps them rather than recording a deviation (the
issue's alternative, `ignore_pe` −1, would differ from TS on every map). If the second case turns
out to matter in play, passing −1 again is a one-line change in `crosshair_trace`. On a miss
`tr.ent` stays −1; FWGS's `EV_IndexFromTrace` returns −1 and `GetEntityByIndex(−1)` null, so
nothing is shown, as before.

Divesteps (1004ef50, drawn when not scoped or in third person; checked against the TS binary):
the three sprites at `(cx − 40, cy − 40)`. Sprite i is red (195, 45, 45) when `81 − free slots`
≥ 42 / 52 / 62 (free slots: the `TSSpace` byte, `MsgFunc_TSSpace` 1004b9b0 writing element
`0x100d43cc` + 0x28, read at 1004eec5); the threshold helpers (1004eec0/1004eef0/1004ef20) compute
`81 − slots − f()` with `f` (1004eeb0) a stub returning 0, so the decompile's
`HUD_GetUserEntity` term is that 0. Otherwise the sprite has the base colour (red while
`NextStunt ≥ 0.01`, else the HUD colour) ×1 for the band the pitch is in (< −10°, −10..10°,
> 10°) and ×0.5 for the other two. The pitch dot (2×2, alpha 100, base colour) is at
`(cx − 32·cos a, cy + 32·sin a)`, `a = clamp(3·pitch, ±90°)`.

Laser HUD (read from the TS binary, since the decompile hides the `__ftol` arguments):
* `DrawLaserDots` (1004e6f0) runs before `Draw` writes `+0x14`, so the dots use the previous frame's items (issue #96). 2×2 fills, alpha 255, in the local player's `PlInfo` laser colour.
  When `state.items_ready` (TS `0x1018a628`: the `from` state's switched-on items, 0 while
  `m_flNextAttack > 0`, set in `HUD_PostRunCmd`) is non-zero and `cl_laser < 1`, the dot is
  pinned to the crosshair centre at `(cx−1+jx, cy−1+jy)`; akimbo dots at `(cx+3+jx, cy−1+jy)` and
  `(cx−5+jy, cy−1+jx)`, with `j = RandomLong(−10, 10) / 10` (±1 only at ±10). Otherwise each dot
  is at its projected hit point − (1, 1). No dot when the hit point projects behind the view.
* The traces (`effects::laser::draw_local`, `DrawLocalLaser` 0x1003bb00) start at the cull origin
  (±4 units along `v_right` per akimbo gun) along the beam direction; the projected point is the
  hit − `2·v_forward`, and `state.laser_range` = first hit distance × 0.025 (`0x1003d15f`).
* `DrawCrosshair` with the laser on draws `laser_range` (`DrawHudNumberTS` flag 1) at
  `(cx + 28, cy − m_iFontHeight/2 − (8 when scoped in first person))` (`m_iFontHeight` = the `number_0` height), in the crosshair colour ×0.7
  when the crosshair is on a player, ×0.5 otherwise (`0x1004ee02`).
* `DrawWeaponInfo` ends with `sprintf("%ix", 80 / m_iFOV)` at (24, 24) in the HUD colour when the
  FOV is below 40 (`0x1004dfbe`: `mov eax, 80; idiv fov`).

Weapon element colours (issue #92; read from the TS binary). Many are `ftol(colour × k)`:
* `DrawWeaponInfo`: the clip and reserve numbers (flags 5) are HUD × 2/3 (`0x1007d770`), red
  (195, 45, 45) at 0 or less; the reserve, and a stackable melee weapon's count, sit at
  `ftol(x + 94.8148)`. Fire mode, ammo name and weapon name are HUD × 2/3; attachment labels
  HUD / 2, the `|` HUD / 4. `DrawKungFuLabel` writes "Kung-Fu" in HUD × 2/3, and
  `DrawKungFuBar` fills `p − 2` px with `p = ftol(power × 1.28f)` in (r·128/255, p·g/255,
  p·b/255) at alpha 255 − p.
* `DrawWeaponList`: colours are `ScaleColors(c, a)` (`c × a/255`) then × `m` (1 with the menu
  open, 0.25/0.5/1 for `cl_showequipment` 1/2/3). Bucket numbers `a` = 200 selected / 100; the
  selection's name and `clip/clip size` text `a` = 200; sprites red with an empty clip, `a` =
  255 selected / 100; the clip bar in the sprite's colour. Closed, kung fu isn't drawn but keeps
  its 48 px.
* `DrawCrosshair`: `cross_t` × 0.7, `cross_e` × 0.75, `cross2` as is.

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
* Opens only when dead or observing (`buy`, or automatically on each local `Spectator` message with observer 1); closes
  when alive again (except the team page), from `CHudTSWeapons::Draw` (`0x1004d3f0`), so not
  while `nohud`, `HIDEHUD_ALL` or intermission hides the HUD (it stays open, unseen, with its keys
  and cursor); first open of the session random-buys like TS, and every random config
  (`100398e0`, also key 7) becomes the last config, so the next open shows it again.
* Drawing (`FUN_1004ff10`): each row is a 127×19 box in the row colour at alpha `50·scale` with the
  text in the row colour × `scale`, the scale 1.0 for the page's own column and 0.5 for the parent
  columns, except the chosen category (`0x1003788c`) and the chosen weapon (`0x10037ad4`), which
  keep 1.0. Ordinary rows are the HUD colour / 2, affordable "Buy"/akimbo rows green (45,195,45),
  the save/load and team headers green, unaffordable rows red (195,45,45), "Go back" (100,100,205).
  Weapon rows are keyed `1`..`9`, `A`; attachment rows show the attachment's own price (the
  colour tests the gun plus it). The loadout bar shows an akimbo entry as the gun in its bucket
  and the akimbo weapon in its own, both with the gun's ammo. **Was** (issue #82): text at twice
  the row colour, whole parent columns dimmed, red headers, base + attachment prices, `a`, the
  akimbo weapon alone; random configs not kept as the last one; the menu drawn after the HUD
  and closed whatever `nohud`.
* Mouse (`MouseEvent` 10038360, only while no VGUI menu is up): a left release runs
  `FUN_10038410`'s hit test (`BuyMenu::hit_test`: 128 px columns from x 8, 20 px rows from
  mid-screen, each column answering for its own page) and a miss is key 1 of the current page; a
  right release is key 10. Escape doesn't close it: TS's key handler (`0x10064b90`) never looks
  at the buy menu.
* The restriction mask starts at 0 (every weapon restricted) until `SrvSett 3`, and the weapon
  type is `FUN_1000b670`'s, Golden Colts counting as pistols (`shared::weapons::is_restricted`).

## 7. Deviations

Port-only aids and engine requirements:
* `ts_hud_selftest [dead]` is port-only.
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
* Director messages: TS exports only the old `HUD_DirectorEvent(command, primary, secondary,
  flags)`, which current engines (FWGS, Steam GoldSrc) never call. The port also handles
  `HUD_DirectorMessage`: `DRC_CMD_START` (1) becomes TS's `DRC_ACTIVE` reset and `DRC_CMD_EVENT`
  (2, the same `short, short, long` layout) TS's `DRC_CAMERA`; other commands are ignored, as TS
  ignores everything but those two.
* Joystick on Linux uses an SDL2 game controller (TS only has the Windows WinMM code); its rescan
  uses the client clock (`GetAbsoluteTime` isn't in the 2007 engine table).
* `voice_ban.dt` is saved from `HUD_Shutdown`: TS saves it in `g_VoiceStatus`'s static destructor
  (`0x100177e0`) when the library unloads, which Rust has no equivalent for; the engine calls
  `HUD_Shutdown` just before unloading the library.

Drawing that TS leaves to the engine's VGUI1 surface:
* Spectator panel (`TeamFortressViewport` `+0x884`, created by `0x10064410`, filled by
  `UpdateSpectatorPanel` `0x10063870`): the same panel and labels painted with engine fills: the
  `CTransparentPanel` background (black, VGUI alpha 100), the "Primary Button Text" mode label
  (`YRES(25)`) and "Briefing Text" help label (`YRES(15)` at `YRES(25)`), each `a_north`
  (centred, top) in its scheme's foreground colour and font, clipped to the label, with VGUI1's
  `TextImage` rules (`hud::vgui`). Two engine limits remain. The fills are alpha-blended with
  `pfnFillRGBABlend`, which only engines from the 2013 update on have (FWGS, current Steam); on
  an older engine the port falls back to the additive `pfnFillRGBA`, under which the black
  background vanishes. And TS draws a scheme with the system's Arial when its
  `gfx\vgui\fonts\<screen width>_<scheme>.tga` is missing (every width but 640, 800, 1024,
  1152, 1280 and 1600, or `bitmapfonts 0`); the port can't use system fonts, so it draws the
  bitmap rendered from the same scheme file (`<scheme res>_<scheme>.tga`, the same Arial size).
* Team menu, MOTD window, scoreboard and voice labels/icons (`CTeamMenuPanel`,
  `CMessageWindowPanel`, `ScorePanel`, `CVoiceStatus`'s labels) are the same VGUI1 panels painted
  with engine fills (`hud::vgui`, `menus::{team,motd,scoreboard}`, `voice::redraw`): TS's
  geometry (`XRES`/`YRES` with TS's float constants, which sit a hair above 1/640 and 1/480, so a
  few sizes come out one pixel short of the 640×480 numbers, as in TS; the few values TS spills to a
  `float` are rounded the same way), colours, scheme fonts, `gfx/vgui/*.tga` images, texts,
  sorting, hover/arm and click behaviour, keys (`KeyInput` `0x10064b90`) and the viewport's menu
  queue. Images follow VGUI1's TGA alpha rules (Valve's `vgui.so`, `BitmapTGA::loadTGA` at
  `0x23880`, as freevgui documents them): uncompressed 32-bit images keep their alpha with
  `vgui_LoadTGA` and are inverted with `vgui_LoadTGANoInvertAlpha`, RLE ones the other way round.
  So the speaker label's `speaker4.tga` (uncompressed, loaded without inversion) draws as a white
  square with the speaker cut out, in TS as in the port. What remains different:
  * Scroll bars follow VGUI1 (`menus::scroll`): `CTFScrollPanel` (team menu briefing, MOTD and
    map briefing) is VGUI1's `ScrollPanel` as `vgui.so` codes it (bars shown automatically
    unless the slider has the full range, horizontal decided first, the client clip losing a
    shown bar's 16 pixels, `validate` = four `setSize`s, ranges and windows from the client size,
    the proportional `size / window × size` nob with x87 truncation, `0x80000000` for an empty
    span) with TS's buttons (the `640_`/`320_` `arrowup`/`arrowdn`/`arrowlt`/`arrowrt` TGAs at
    their own (0, 0), white, half transparent unless armed; with the stock Half-Life files only
    `640_arrowup.tga` exists, so the other buttons are blank in TS as in the port) and
    `CTFSlider` (`sc_secondary1` frame, nob in `sc_primary2` outlined in `sc_primary1`). The team
    menu's default "#Map_Description_not_available" panel is larger than its client clip, so TS
    shows both bars there and the port does too. The scoreboard's `CListBox` scrolls by rows (range
    = row count, `ScrollBar2` with `arrowup.tga`/`arrowdown.tga` buttons in (140, 140, 140) and a
    `Slider2` with a 50-pixel nob, step 1), shown when a row is scrolled off the top or the rows
    run past the bottom. Input is VGUI1's: a press in a nob drags it (the slider keeps the mouse
    until the release), a left release over a button steps the value, a `CTFScrollButton` arms
    under the cursor; the scoreboard's bar takes input in squelch mode, when the cursor is up. No
    panel takes the mouse wheel, as in TS (no VGUI1 signal here handles `mouseWheeled`). The port
    arms and hit-tests by the cursor position each frame rather than VGUI's enter/exit events.
  * The engine's VGUI surface draws bitmap-font glyphs and images as textured quads; the port
    draws the same pixels as fills (glyph ink opaque in the text colour's opacity, images per
    pixel run with VGUI1's TGA alpha and colour modulation), and VGUI's own cursor is the TS sprite
    cursor the port already draws. A textured path was considered and not taken: the 2007 client
    API has no texture upload, `LoadMapSprite` + TriAPI would load the TGA through the engine's
    image loader (losing VGUI1's per-type alpha rules) and filter it, and the raw GL wrapper
    (`goldsrc::ext::gl`) works only on desktop GL renderers and behind the engine's state. With
    `pfnFillRGBABlend` the fills give VGUI1's pixels exactly; only engines older than the 2013
    update (no blend fill) lose fidelity, as noted for the spectator panel above.
  * The MOTD's full-screen shade is the whole `ScreenWidth`×`ScreenHeight` panel (`0x100d5908`/
    `0x100d590c`, `gHUD.m_scrinfo`, passed by `CreateTextWindow`), as in the port. A report of it
    covering only the left ~825 pixels at 1896×924 on FWGS came from a map whose near wall ends
    there: in that screenshot every world pixel, on both sides, is at most 100/255 bright (white
    shaded by VGUI alpha 100), and the view model is shaded evenly across that edge.
  * Squelch mode centres the virtual cursor, and the first scoreboard press (entering squelch mode)
    isn't passed to the engine as a key. Panels are laid out from the current screen size each
    frame (TS builds them once with the viewport).
  * `jointeam` from the team menu doesn't set the local `g_iTeamNumber` (TS does, `0x10064a82`,
    until the next player state); Escape's "has a team" test reads the player state only.

Checks that only matter on input TS mishandles (TS writes or reads out of bounds):
* `Objective`/`ShowObj` indexes outside 0..31 (TS's check `i >= 0 || i < 32` is always true and
  writes past the 32 objectives) and labels over 63 characters (TS copies them into a 64-byte
  stack buffer unbounded) are dropped or cut.
* The scoreboard's sorted rows stop at its 74 grids (TS writes on past its `m_iSortedRows` with
  more rows than that); its 74-byte `m_bHasBeenSorted` clear, which runs 2 bytes past TS's object
  (the ts-patcher crash fix), is a plain array.
* Buy menu: an add with all sixteen entries in use is refused (TS writes past them); key 11 (`a`)
  on the save and load pages is ignored (TS reads config 10, past the nine); a click on a path
  where `FUN_10038410` leaves the page unwritten (a full sixteen-entry remove list) does nothing
  (TS uses the uninitialised stack value).
* `StatusIcon` names over 23 characters are cut (TS's `strcpy` runs over its 24-byte slot), and an
  unknown sprite name gets an empty rectangle (TS copies the one at index −1).
* Chat: overlong wrap appends are cut at the 256-byte line (TS's `strcat` runs into the next
  line), and a wrap at a line's first character breaks at 0 (TS reads the byte before the line).
* Malformed (short) user messages are logged and ignored; TS's `READ_*` return −1 past the end
  and its handlers carry on with those values. `WeaponInfo` likewise drops a short message, and
  so does `DeathMsg` (TS would index the player list with −1). `HudText`, `StatusText`,
  `StatusValue` and the TFC messages follow TS instead (−1 values, empty strings).
* Death notices: the weapon name is cut at 29 bytes (`"d_" + strncat(.., 32)` runs over TS's
  32-byte buffer); a killer or victim number outside 1..63 has no name (TS reads past
  `g_PlayerInfoList`, refreshed only for 1..63, and `GetClientColor` past `g_PlayerExtraInfo`);
  with no `d_skull` in hud.txt no sprite is drawn and it has no width (TS uses index −1's
  rectangle).
* `CHudMessage`: a custom message's text is cut at 1023 bytes (`strcpy` into the 1024-byte
  `g_pCustomText`); a line longer than 79 characters is drawn whole (TS copies it into an
  80-byte stack buffer); the line-width count uses the byte's width for bytes 0x80..0xff (TS
  indexes `charWidths` with a signed `char`, reading before the table); a null `pMessage` reads
  as empty; a `GameTitle` that found no `GAMETITLE` entry while an earlier title still runs skips
  the title (TS dereferences the null pointer); missing `title_half`/`title_life` sprites have
  empty rectangles and aren't drawn.
* Status bar: output past 128 bytes is dropped (TS writes plain characters unchecked into the
  next line's buffer or past the last into the values); reads past the source (a line ending in
  `%`) and past a 32-byte name stop; a `%p` value outside 0..64 has no name (TS calls
  `GetPlayerInfo` into `g_PlayerInfoList[value]`, out of bounds); `LocaliseTextString`'s output
  is bounded and its `[-1]` terminator (written when the step budget runs out) is skipped.
* `KB_ConvertString` (`0x10025740`): TS's `isalnum` (`0x1006c45a`) indexes `_pctype` with the
  sign-extended byte, so a byte 0x80..0xff reads before the table (unrelated data and
  relocated pointers); the port counts those bytes as not alphanumeric, as the C locale's table
  does for the bytes it covers. Output past 4095 bytes is dropped (TS writes into its 4096-byte
  stack buffer unbounded). `ShowMenu` then `strcpy`s the result, like the up-to-1023-byte
  localised text before it, into the 512-byte `g_szMenuString` (`0x100df918`), running into the
  globals after it; the port keeps the whole string, as it already did for the localised text.

Undefined in TS:
* `CHudMessage::Draw` keeps each message's end time in a stack slot that only effects 0-2 write;
  a titles.txt effect outside 0..2 reuses the previous message's end time from the same draw, or
  for the first such message, uninitialised stack. The port uses 0 there (the message ends).
* The flicker (effect 1) and the Geiger clicks share MSVC's CRT `rand`, seeded as TS seeds it
  (`srand(time(NULL))` in `CHudGeiger::Init`), so the sequence is TS's for the same start time.
* `BufferedLocaliseTextString` copies an unknown `#` without a terminator, so a text ending in
  one shows the rest of the previous buffered text; the port keeps the shared buffer and shows the
  same bytes. `LocaliseTextString` into stack buffers (the status bar's) starts from zeros in the
  port, where TS has whatever the stack held.
* The viewport is allocated without clearing; `Initialize` zeroes the valid classes and
  `RandomPC`. The port starts `Feign`/`Detpack`/`BuildSt`/`AllowSpec` at 0 too.
* **Panic guards (the game must never abort).** Both profiles build with `panic = "abort"`, so
  bad input from the server, game files or cvars is dropped or bounded instead of panicking.
  Where TS would only read or write out of bounds, the port also differs:
  * Integer maths on HUD positions, panel and VGUI coordinates (`hud/vgui.rs` `Rect`/`Panel`),
    ammo, scores, buy-list totals and joystick and camera offsets wraps as TS's C `int` maths
    does, instead of tripping debug overflow checks.
  * `text::format` pads to at most 0x400 columns. A SayText whose `%s` substitutions keep
    expanding stops wrapping after 256 levels of recursion. The crosshair gap is clamped to
    ±65536 px.
  * A TGA (`gfx/vgui/*.tga`) whose pixel data can't fill its header size, or that is larger than
    2^24 pixels, is refused rather than allocated. `Image::pixel` is bounds-checked.
  * `TeamNames` keeps the first four names (TS's handler `0x10064df0` only stores teams 1..4). A
    longer list used to index past the team menu's buttons.
  * The buy menu's sort stops after 256 passes. A loaded `tsbuylist.tsa` uses wrapping sums. The
    scroll panels clamp the nob to ±2^30 px for painting. `voice_ban.dt` is deduplicated through a
    set as it loads.
  * `hud::screen()` takes a shared borrow, with a default if the HUD state is already borrowed.
    `ConCuss`, `StatusIcon` and the secondary-ammo icon read sprites through a shared borrow
    (`gfx_ref`), so a handler that runs while the HUD is drawing doesn't double-borrow.
  * The client-dll exports null-check every engine pointer, including the temp-entity free list.
    `goldsrc::ext::studio` reads a bone index past `MAXSTUDIOBONES`, or a missing engine matrix,
    as a zero matrix and drops writes to it. Event hooks past 32 and temp-entity callbacks past
    four are ignored rather than asserted.
* **Map name for the team menu and map briefing.** The port used to wrap `pfnGetLevelName` as
  `maps/{}.bsp`, but the engine already returns `maps/<map>.bsp`, so the briefing file was never
  found. It now uses the name as it comes, as TS's `Update` (`0x10066020`) does. This is a return
  to TS behaviour.

Notes (no observable difference):
* HLTV name tags (`CHudSpectator::Draw` `0x10023080`) use the console font like TS; TS's
  overview zoom and pan deltas (`+0xc98`/`+0xc9c`) are never set, so that code isn't ported.
* TS reads `scripts/voicemodel.txt` into `m_VoiceHeadModelHeight` (`+0x177e0`), which nothing
  reads (`CreateEntities` uses a constant 45, `0x10018053`), so the port doesn't read it.

## 8. Missing / not done

* Spectating (issue #7) is complete; what remains is HLTV-only and unverified in game (it needs
  an HLTV proxy on a Steam server): the overview map, name tags and director view are ported but
  have only unit tests, and the inset view is unreachable in TS itself (see client-rendering.md).
* Server browser, command menu, TFC panels (dropped as allowed by §7). The TFC message values
  (`hud::tfc::ViewportTfc`) are kept for a command menu; `UpdateCommandMenu` (`0x10063840`),
  which the TFC handlers call, has nothing to update.
* Player names: the HUD, menus and voice code use `PlayerInfo::name_bytes`, the engine's raw
  bytes, so the kill feed, status bar, crosshair ID, chat, scoreboard, team menu, voice labels,
  HLTV name tags (`0x100232e0`: `sprintf("%s", name)`, 3 pixels left per byte) and the
  spectator panel's target name (`0x10063952`) draw TS's glyphs for any byte. No client code
  reads `PlayerInfo::name` (decoded as UTF-8) any more.
* `HUD_UpdateClientData` runs `CHud::Think`'s zoom easing and writes the eased FOV
  (`view::update_client_data` → `hud::think_fov`), as TS's `CHud::UpdateClientData` does.
* The death-cam / `cl_thirdpersonswitch` / `Spectator` camera switches run TS's `thirdperson`
  toggle (`view::toggle_third_person`, with its observer and demo checks) right after the handler,
  as TS calls it inside the handler; the view package owns `cam_thirdperson`.

## 9. Tests and results

* `cargo test`: 35 client tests (message handlers on encoded buffers for status, power-ups, weapon
  grid, TS message stacking/replace, objectives, scoreboard, MOTD chunks, ShowMenu, spectator,
  TextMsg formatting; `DrawHudNumberTS` layout; localisation/printf; slot menu, fast switch, wires,
  next/prev skipping owned akimbos; buy menu file round trip on the shipped `tsbuylist.tsa`,
  add/cost/slot rules, restrictions, submit protocol strings, page machine, random config;
  scoreboard grouping; gate bindings; mouse acceleration; key-state fractions) and 6 protocol
  tests in `shared`. All pass. Issue #92 added: the kbutton list and its shutdown
  (`input::tests::kb_list_holds_only_ts_buttons_until_shutdown`); death-notice console lines,
  entries, layout, expiry and clamping (`hud::notice::tests`); `CHudMessage` positions,
  `FadeBlend`, `MessageAdd` replacement/duplicates/the shared custom message, fade and
  write-out colours per character, level-transition fix-up, game title sprites
  (`hud::message::tests`); status-bar messages, parsing, caps and layout
  (`hud::statusbar::tests`); `LocaliseTextString`'s budget, `BufferedLocaliseTextString`'s word
  bytes and shared buffer, `LookupString` (`hud::text::tests`); `TextMsg` destinations,
  `ShowMenu` chunks, `VGUIMenu` ids, `ScoreInfo`'s team clamp and `Geiger` (`hud::state::tests`);
  the Geiger click/volume/sample choice on the CRT sequence (`hud::geiger::tests`); the TFC and
  `SecAmmo*` values (`hud::tfc::tests`); `HudText`/`ResetHUD` through `Hud::apply`. Issue #103
  added `KB_ConvertString`'s name scan, lookups, NUL end and bounds
  (`input::tests::kb_convert_string_swaps_bound_commands`), `ShowMenu`'s key names
  (`hud::state::tests::show_menu_key_names`) and the name tag's raw-byte offset
  (`spectator::overview::tests::tag_x_counts_raw_bytes`).
  The VGUI panels have their own: scheme parsing and colour defaults,
  bitmap-font metrics and wrapping, TGA alpha rules (also on the shipped `gfx/vgui` icons when
  installed), panel clipping/insets/borders, label alignment and `XRES`/`YRES` truncation;
  `ScorePanel` sorting (`SortTeams`/`SortPlayers`, game-mode score rule, `TeamScore` overrides,
  `RebuildTeams` slot reuse), `FillGrid` cells (colours, fonts, status column, killer fade, voice
  column), layout at 640/1024/512/400 and squelch highlighting; the team menu's `Update` layout,
  info text, map briefing, slots, hover and clicks; the MOTD window's layout, OK button, scroll
  set-up and full-screen shade, and `CreateTextWindow`'s map briefing file and name; VGUI1
  slider maths (nob, value, drag, full range, empty span), `CTFScrollPanel` layout, input and
  paint, and the `CListBox` layout and scrolling (`menus::scroll::tests`); the
  speaker label layout.
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
  (Windows cursor / SDL2), team menu, MOTD, ShowMenu, death cam, effects queue consumers. The
  VGUI panel ports (issue #93) have unit tests only.
* Steam Half-Life 25th anniversary client (Linux, build 10211, `docs/validation.md` §5): the
  scripted `xtask play` scenes draw the world, HUD, buy menu, MOTD, view model, slow-motion indicator
  and bot fights. Under `gdb`, opening the console (`CBaseUI::ActivateGameUI`) reaches
  `input::deactivate_mouse` through `HideClientUI`, and the post-load `CBaseUI::HideGameUI` reaches
  `input::activate_mouse`, matching the calls the engine made in VGUI1 mode.

## 10. Integration notes

* `state::Shared` additions: `observer`, `spread` (prediction should write the gun's spread
  penalty), `ts_fade` (view applies TSFade), `active_items`/`laser_color` (effects: laser and
  flashlight), `ts_effects` queue (effects drains `PlayerHit`, `Frags`, `Wind`, `MotionBlur`),
  `menu_open`.
  The HUD writes `health`, `dead`, `weapon`, `fov` (smoothed zoom FOV, default `default_fov`;
  `fov_target` from `SetFOV`/prediction),
  `time_scale_target` (TSSlowMo) and eases `time_scale` in `HUD_Frame` (TS does it there, after
  the emitters and before clearing the key gate; nothing else eases it). `CHud::VidInit`
  (`0x100208a9`) resets `time_scale` to 1 but not the target, which only `TSSlowMo` writes
  (`0x1004bf10`), so slow motion still active after a level change eases back in. The target
  starts at 0, as in TS (`0x1009ff04` is past the 0x8000 initialised bytes of `.data`), so until
  the first `TSSlowMo` the scale eases from 1 toward 0. The port's server sends it on the first
  client update, which hides this in play; on a server that never sends it, or playing back a
  demo recorded after it was sent (in a fresh client), the scale stays near 0 (most readers clamp
  it to 0.01), in TS and in the port alike.
* Gameplay rulesets (issue #100): where another ruleset can differ, the HUD, input, FOV, map
  effects and buy menu ask `ClientRules` hooks whose defaults are the TS 3.0 behaviour described
  here (crosshair parts and gap, divestep limits and cooldown, `default_fov` pin and FOV snap, key
  gate, `cl_attackswitch`, `respawn` on `+attack`, map effects, kung-fu bar, slow-motion icon,
  `CashBased()`, buy-menu lists and item rules). The table is in
  [gameplay-plugins.md](gameplay-plugins.md) §5, and the TS 2.1 overrides in [ts21.md](ts21.md).
  `state::Shared::slowmo_factor100` keeps the raw `TSSlowMo` value; `Hud::next_stunt_msg` and
  `Hud::think_time` are for those hooks.
* The FWGS `IN_ClientLookEvent`/`IN_ClientMoveEvent` exports are in `client-dll` and call
  `input::look_event`/`move_event`.
* The server should send `SrvSett 3` (restriction) and `GameMode` in `InitHUD` like TS, and
  implement `rst_cfg`/`tkw`/`tki`/`tka`, `weapon_<n>`, `drop_<n>`, `lasclr_<n>`, `usepowerup`.
* `shared::protocol::hud` encoders produce the exact TS byte layouts; the server's HUD sync can
  use them for its tests.
