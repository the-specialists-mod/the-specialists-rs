//! HUD state and the message handlers that write it (TS `CHudTSStatus` 0x100d43cc,
//! `CHudTSWeapons` 0x100d0ca0, the viewport's `g_PlayerExtraInfo`, and the SDK elements TS
//! kept). Handlers are pure: engine side effects come back as [`Effect`]s so they can be tested.

use super::text::{Titles, localise_with};
use goldsrc::ext::hud::Screen;
use shared::protocol::Msg;
use shared::protocol::hud as p;
use shared::weapons::MAX_WEAPONS;

pub const MAX_CLIENTS: usize = 64;
/// Weapon grid: 17 rows × 11 buckets (`m_rgSlots` at `+0x164`, index `bucket + row*11`).
pub const GRID_ROWS: usize = 17;
pub const GRID_BUCKETS: usize = 11;
pub const MAX_TS_MESSAGES: usize = 16;
pub const MAX_OBJECTIVES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// TS weapon id, or -1 when empty.
    pub id: i32,
    pub clip: i32,
}
pub const EMPTY: Slot = Slot { id: -1, clip: -1 };

/// One timed TS message (`0x9c`-byte records at `+0x70`).
#[derive(Debug, Clone, Default)]
pub struct TsLine {
    pub expire: f32,
    pub rgb: [i32; 3],
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub flags: u8,
}

#[derive(Debug, Clone, Default)]
pub struct Objective {
    pub visible: bool,
    pub pos: [f32; 3],
    pub label: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FlagState {
    pub state: u8,
    pub carrier: u8,
    pub team: u8,
}

/// `g_PlayerExtraInfo` / `g_PlayerItems` per client.
#[derive(Debug, Clone, Default)]
pub struct PlayerExtra {
    pub frags: i32,
    pub deaths: i32,
    pub class: i32,
    pub team: i32,
    pub teamname: String,
    pub award: u8,
    pub observer: bool,
    pub active_items: u8,
    pub laser_color: u8,
}

#[derive(Debug, Clone, Default)]
pub struct TeamScore {
    pub name: String,
    pub frags: i32,
    pub deaths: i32,
}

/// `CHudMenu` (SDK menu.cpp): the `ShowMenu` text menu.
#[derive(Debug, Clone, Default)]
pub struct TextMenu {
    pub displayed: bool,
    pub text: String,
    pub pending: String,
    pub slots: u16,
    /// Absolute time to hide, 0 = until a selection.
    pub shutoff: f32,
}

/// Engine-side effects the caller applies after a handler.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Sound(&'static str),
    /// Run the `thirdperson` toggle (`0x1004fe00`) now, as TS's handlers do: the death cam
    /// (`TSHealth`), `cl_thirdpersonswitch` (`WeaponInfo`) and `Spectator`.
    ToggleThirdPerson,
    OpenBuyMenu,
    OpenTeamMenu,
    OpenMotd,
    /// `Spectator` for the local player (`0x100652fd` -> `0x1000b9d0`): clear the predicted spread
    /// penalty ([`crate::prediction::reset_spread`]).
    ResetSpread,
    /// Forward to the effects package (`crate::state::Shared::ts_effects`).
    Fx(crate::state::TsEffect),
    Fade(p::TsFade),
    ConsolePrint(String),
    CenterPrint(String),
    /// `CHudSayText::SayTextPrint(text, client)` ([`super::say_text_print`]).
    SayText(String, i32),
}

/// Context for handlers: what TS reads from globals while parsing.
pub struct Ctx<'a> {
    pub now: f32,
    pub screen: &'a Screen,
    pub titles: &'a dyn Titles,
    pub local_index: i32,
    pub local_name: String,
    pub third_person: bool,
    /// `cl_thirdpersonswitch == 1` (`0x100df3a4`, read only by `WeaponInfo`).
    pub thirdperson_switch: bool,
    /// `cl_thirdperson` (`0x100df544`, read only by `Spectator`).
    pub cl_thirdperson: f32,
    pub saytext_time: f32,
    /// `hud_deathnotice_time`. No handler here reads it any more (`DeathMsg` is handled by
    /// `hud::notice::on_death_msg`, which reads the cvar itself); kept so callers building a
    /// `Ctx` elsewhere (the menus) stay unchanged.
    pub deathnotice_time: f32,
}

#[derive(Debug, Clone)]
pub struct Hud {
    // ---- CHudTSStatus
    /// `CHudTSStatus::m_iHealth` (`+0x20`), drawn by the status panel; its `Reset` clears it.
    pub health: u8,
    /// `0x100cf9d8`: the last `TSHealth` value as an int. Only the `TSHealth` handler writes it
    /// (`0x1004b747`; `ResetHUD` leaves it), and `0x10026860` (`<= 0`) reads it for
    /// `CL_ButtonBits`' forced +score (`0x10026936`) and the prediction's `ItemPostFrame` gate
    /// (`0x10003d54`).
    pub last_hp: i32,
    pub armor: u8,
    pub stance: u8,
    pub cash: i32,
    pub slots: u8,
    pub pw_flags: u32,
    pub pw_value: i32,
    pub pw_expire: f32,
    pub kungfu_icon: bool,
    pub move_icon: bool,
    pub round_time: f32,
    pub round_next: f32,
    pub round_mode: u8,
    pub observer: bool,
    pub messages: Vec<TsLine>,
    pub objectives: Vec<Objective>,
    pub obj_max: i32,
    pub flags: [FlagState; 16],
    pub flag_count: i32,
    // ---- CHudTSWeapons
    pub grid: [[Slot; GRID_BUCKETS]; GRID_ROWS],
    pub menu_open: bool,
    pub menu_row: i32,
    pub menu_bucket: i32,
    pub reserve: [i32; 16],
    pub firemode: u8,
    pub last_firemode: u8,
    pub clip: i32,
    pub weapon: i32,
    pub attachments: u32,
    pub active_items: u32,
    pub next_stunt: f32,
    pub wires: [u8; 3],
    pub kfu_power: f32,
    pub kfu_prev: f32,
    pub invnext_time: f32,
    pub last_laser_color: i32,
    pub usepowerup_time: f32,
    pub custom_weapons: Vec<p::CustomWp>,
    // ---- game / viewport
    pub gamemode: i32,
    pub realbullet: i32,
    pub cashbased: i32,
    pub ammocount: i32,
    pub restriction: i32,
    pub bullettime: bool,
    pub players: Vec<PlayerExtra>,
    pub team_scores: Vec<TeamScore>,
    pub team_names: Vec<String>,
    pub server_name: String,
    pub motd: String,
    pub motd_final: bool,
    pub map_briefing: String,
    // ---- SDK elements kept by TS
    /// `CHudDeathNotice`.
    pub deaths: super::notice::DeathNotices,
    /// `CHudMessage` (`HudText`, `GameTitle`).
    pub message: super::message::HudMessage,
    /// `CHudStatusBar` (`StatusText`, `StatusValue`).
    pub status_bar: super::statusbar::StatusBar,
    /// `CHudAmmoSecondary` (`SecAmmoVal`, `SecAmmoIcon`).
    pub sec_ammo: super::tfc::AmmoSecondary,
    /// The viewport's TFC message values (`ValClass` … `AllowSpec`).
    pub tfc: super::tfc::ViewportTfc,
    /// `CHudSayText`.
    pub say_text: super::saytext::SayText,
    /// `CHudStatusIcons`.
    pub icons: super::icons::StatusIcons,
    /// `CHud::m_iConcuss` (`+0x1c`, the `Concuss` message), `v_idlescale`'s source.
    pub concuss: i32,
    /// `CHud::m_iLogo` (`+0x8`, the `Logo` message).
    pub logo: i32,
    pub center_text: String,
    pub center_expire: f32,
    pub menu: TextMenu,
    pub crosshair_id: Option<(i32, u8)>,
    pub intermission: bool,
    /// `CHudTrain::m_iPos` (`Train`): speed notch + 1 of the train being driven, 0 = hidden.
    pub train_pos: u8,
    /// `gHUD.m_iHideHUDDisplay` (`HideWeapon`): HIDEHUD_WEAPONS 1 (tank in use), HIDEHUD_ALL 4.
    pub hide_hud: u8,
}

impl Default for Hud {
    fn default() -> Self {
        let mut h = Hud {
            health: 0,
            last_hp: 0,
            armor: 0,
            stance: 0,
            cash: 0,
            slots: 0,
            pw_flags: 0,
            pw_value: 0,
            pw_expire: 0.0,
            kungfu_icon: false,
            move_icon: false,
            round_time: 0.0,
            round_next: 0.0,
            round_mode: 0,
            observer: false,
            messages: vec![TsLine::default(); MAX_TS_MESSAGES],
            objectives: vec![Objective::default(); MAX_OBJECTIVES],
            obj_max: 0,
            flags: [FlagState::default(); 16],
            flag_count: 0,
            grid: [[EMPTY; GRID_BUCKETS]; GRID_ROWS],
            menu_open: false,
            menu_row: 0,
            menu_bucket: 0,
            reserve: [0; 16],
            firemode: 0,
            last_firemode: 0,
            clip: 0,
            weapon: 0,
            attachments: 0,
            active_items: 0,
            next_stunt: 0.0,
            wires: [0; 3],
            kfu_power: 100.0,
            kfu_prev: 100.0,
            invnext_time: 0.0,
            last_laser_color: -1,
            usepowerup_time: 0.0,
            custom_weapons: Vec::new(),
            gamemode: 0,
            realbullet: 0,
            cashbased: 0,
            ammocount: 0,
            // `0x1018d174` is zero (`.bss`) until the server's `SrvSett 3` (sent in `InitHUD`),
            // which blocks every weapon in the buy menu until then, as in TS.
            restriction: 0,
            bullettime: false,
            players: vec![PlayerExtra::default(); MAX_CLIENTS + 1],
            team_scores: Vec::new(),
            team_names: Vec::new(),
            server_name: String::new(),
            motd: String::new(),
            motd_final: false,
            map_briefing: String::new(),
            deaths: Default::default(),
            message: Default::default(),
            status_bar: Default::default(),
            sec_ammo: Default::default(),
            tfc: Default::default(),
            say_text: Default::default(),
            icons: Default::default(),
            concuss: 0,
            logo: 0,
            center_text: String::new(),
            center_expire: 0.0,
            menu: TextMenu::default(),
            crosshair_id: None,
            intermission: false,
            train_pos: 0,
            hide_hud: 0,
        };
        h.reset_weapons();
        h
    }
}

/// Grid position (row, bucket) of a weapon: `row = info.pos (+0x85)`, `bucket = info.slot
/// (+0x84)`. TS skips ids whose position is outside the grid.
pub fn grid_pos(id: i32) -> Option<(usize, usize)> {
    let w = shared::weapons::tables().weapon(u8::try_from(id).ok()?)?;
    let (bucket, row) = (w.hud[0] as usize, w.hud[1] as usize);
    (row < GRID_ROWS && bucket < GRID_BUCKETS).then_some((row, bucket))
}

impl Hud {
    /// `CHudTSWeapons::Reset` (1004cd60): empty grid, kung fu in its slot.
    pub fn reset_weapons(&mut self) {
        self.grid = [[EMPTY; GRID_BUCKETS]; GRID_ROWS];
        self.menu_open = false;
        self.weapon = 0;
        self.menu_row = 0;
        self.menu_bucket = 0;
        if let Some((r, b)) = grid_pos(0) {
            self.grid[r][b] = Slot { id: 0, clip: 1 };
        }
        self.active_items = 0;
        self.next_stunt = 0.0;
        self.invnext_time = 0.0;
        self.wires = [0; 3];
        self.usepowerup_time = 0.0;
        self.last_laser_color = -1;
    }

    /// `CHudTSStatus::Reset` (10049e70).
    pub fn reset_status(&mut self) {
        self.stance = 0;
        self.health = 0;
        self.armor = 0;
        self.kungfu_icon = false;
        self.move_icon = false;
        self.pw_flags = 0;
        self.pw_expire = 0.0;
        self.round_time = 0.0;
        self.round_next = 0.0;
        self.round_mode = 0;
    }

    /// `ResetHUD`: every element's `Reset`.
    pub fn reset(&mut self) {
        self.reset_status();
        self.reset_weapons();
        // CHudAmmo::Reset clears m_iHideHUDDisplay; CHudTrain has no Reset.
        self.hide_hud = 0;
        self.menu = TextMenu::default();
    }

    /// `InitHUD`: `InitHUDData` (death notices and say text cleared).
    pub fn init_hud_data(&mut self) {
        self.deaths.clear();
        self.say_text.clear();
        self.menu = TextMenu::default();
    }

    pub fn ammo_type(&self, id: i32) -> usize {
        shared::weapons::tables().weapon(id.max(0) as u8).map_or(0, |w| w.ammo as usize).min(15)
    }

    /// `AddTSMessage` (1004b520).
    pub fn add_ts_message(&mut self, ctx: &Ctx, text: &str, rgb: [i32; 3], tenths: i32, flags: u8) {
        let now = ctx.now;
        let Some(i) = self.messages.iter().take(15).position(|m| m.expire < now) else { return };
        let mut line = TsLine { text: localise_with(ctx.titles, text).chars().take(127).collect(), flags, ..Default::default() };
        line.rgb = if !ctx.local_name.is_empty() && line.text.contains(&ctx.local_name) { [155; 3] } else { rgb };
        let (w, h) = (ctx.screen.width, ctx.screen.height);
        line.y = if flags & 1 != 0 {
            32
        } else if flags & 8 != 0 {
            h - 64
        } else {
            h / 2
        };
        let len = super::text::string_len(ctx.screen, &line.text);
        line.x = if flags & 2 != 0 {
            32
        } else if flags & 4 != 0 {
            w - len - 32
        } else {
            w / 2 - len / 2
        };
        // Stack below/above messages already at the same spot, or replace one (flag 0x10).
        let mut moved = 0;
        let mut j = 0;
        while j < MAX_TS_MESSAGES && moved <= 15 {
            let m = &self.messages[j];
            if j != i && m.expire > now && m.y == line.y && m.flags == line.flags {
                if flags & 0x10 != 0 {
                    self.messages[j].expire = 0.0;
                    break;
                }
                line.y += if line.flags & 1 != 0 { 16 } else { -16 };
                moved += 1;
                j = 0;
                continue;
            }
            j += 1;
        }
        line.expire = tenths as f32 * 0.1 + now;
        self.messages[i] = line;
    }

    /// Dispatch one message. Returns false if the payload was malformed.
    pub fn apply(&mut self, msg: Msg, buf: &[u8], ctx: &Ctx, fx: &mut Vec<Effect>) -> bool {
        macro_rules! dec {
            ($t:ty) => {
                match <$t>::decode(buf) {
                    Some(v) => v,
                    None => return false,
                }
            };
        }
        let now = ctx.now;
        match msg {
            // ---- CHudTSStatus
            Msg::TsHealth => {
                let v = dec!(p::TsHealth);
                self.health = v.hp;
                self.last_hp = v.hp as i32;
                if !self.observer && v.hp == 0 && !ctx.third_person {
                    fx.push(Effect::ToggleThirdPerson);
                }
            }
            Msg::TsArmor => self.armor = dec!(p::TsArmor).armor,
            Msg::TsState => self.stance = dec!(p::TsState).stance,
            Msg::TsCash => self.cash = dec!(p::TsCash).cash as i32,
            Msg::TsSpace => self.slots = dec!(p::TsSpace).slots,
            Msg::PwUp => {
                let v = dec!(p::PwUp);
                let flags = v.flags as u32;
                if flags & 0x104 == 0 {
                    self.pw_flags = flags;
                    self.pw_value = v.value as i32;
                } else if flags & 4 != 0 {
                    self.kungfu_icon = true;
                } else {
                    self.move_icon = true;
                }
            }
            Msg::PwUpTime => {
                let v = dec!(p::PwUpTime);
                self.pw_expire = now + v.secs as f32;
                self.pw_value = 0;
            }
            Msg::RoundTime => {
                let v = dec!(p::RoundTime);
                self.round_time = v.secs as f32;
                self.round_mode = v.mode;
                self.round_next = 0.0;
            }
            Msg::Objective => {
                let v = dec!(p::Objective);
                let idx = v.idx as usize;
                // TS's check is `(i >= 0) || (i < 32)`, always true; the port bounds it.
                if idx < MAX_OBJECTIVES {
                    if idx == 0 {
                        self.objectives = vec![Objective::default(); MAX_OBJECTIVES];
                        self.obj_max = 0;
                    }
                    self.objectives[idx] = Objective { visible: true, pos: [v.x, v.y, v.z], label: v.label.chars().take(63).collect() };
                    self.obj_max = self.obj_max.max(idx as i32);
                }
            }
            Msg::ShowObj => {
                let v = dec!(p::ShowObj);
                if let Some(o) = self.objectives.get_mut(v.idx as usize) {
                    o.visible = v.visible != 0;
                }
            }
            Msg::TsMessage => {
                let v = dec!(p::TsMessage);
                let tenths = (v.time as i32).min(100);
                self.add_ts_message(ctx, &v.text, [v.r as i32, v.g as i32, v.b as i32], tenths, v.flags);
            }
            Msg::PTakeDam => {
                let v = dec!(p::PTakeDam);
                fx.push(Effect::Fx(crate::state::TsEffect::PlayerHit {
                    pos: [v.px, v.py, v.pz],
                    dir: [v.dx, v.dy, v.dz],
                    amount: v.amount,
                }));
            }
            Msg::TsSlowMo => {
                let v = dec!(p::TsSlowMo);
                let mut target = v.factor100 * 0.01;
                if target == 0.0 {
                    target = 1.0;
                }
                let current = crate::state::with(|s| {
                    let c = s.time_scale;
                    s.time_scale_target = target;
                    c
                });
                if current == 1.0 && target != 1.0 {
                    fx.push(Effect::Sound("goslow.wav"));
                }
            }
            Msg::TsFrags => fx.push(Effect::Fx(crate::state::TsEffect::Frags(dec!(p::TsFrags)))),
            Msg::TsPAward => {
                let v = dec!(p::TsPAward);
                if (1..=MAX_CLIENTS as u8).contains(&v.client) {
                    self.players[v.client as usize].award = v.award;
                }
            }
            Msg::SrvSett => {
                let v = dec!(p::SrvSett);
                match v.key {
                    0 => self.realbullet = v.value as i32,
                    1 => self.cashbased = v.value as i32,
                    2 => self.ammocount = v.value as i32,
                    3 => self.restriction = v.value as i32,
                    _ => {}
                }
            }
            Msg::TsBTime => self.bullettime = dec!(p::TsBTime).on != 0,
            Msg::TsHVoice => match dec!(p::TsHVoice).lost {
                0 => fx.push(Effect::Sound("misc/successful.wav")),
                1 => fx.push(Effect::Sound("misc/failed.wav")),
                _ => {}
            },
            Msg::TsFlag => {
                let v = dec!(p::TsFlag);
                if (1..16).contains(&v.idx) {
                    self.flags[v.idx as usize] =
                        FlagState { state: v.state, carrier: if v.state == 1 { v.carrier } else { 0 }, team: v.team };
                }
                self.flag_count = 16;
            }
            Msg::TsMBlur => {
                let v = dec!(p::TsMBlur);
                fx.push(Effect::Fx(crate::state::TsEffect::MotionBlur { entity: v.entity as i32 }));
            }
            Msg::TsPSys => {
                let v = dec!(p::TsPSys);
                if v.kind == 3 {
                    fx.push(Effect::Fx(crate::state::TsEffect::Wind { seed: v.seed, yaw: v.yaw, strength: v.strength }));
                }
            }
            Msg::PlInfo => {
                let v = dec!(p::PlInfo);
                if (1..=MAX_CLIENTS as u8).contains(&v.client) && v.key == 0 {
                    self.players[v.client as usize].laser_color = v.value;
                }
            }
            Msg::TsFade => fx.push(Effect::Fade(dec!(p::TsFade))),
            // ---- CHudTSWeapons
            Msg::WeaponInfo => {
                // TS's `READ_BYTE` (`0x10029a10`) zero-extends, so every id byte, 0xff included, is
                // the new weapon; it keeps the old id (`0x1004f822`) only on -1, a read past the end,
                // and the port drops a short message instead.
                let v = dec!(p::WeaponInfo);
                let old = self.weapon;
                self.weapon = v.id as i32;
                self.clip = v.clip as i32;
                // `GetWeaponInfo` (`0x100071c0`) answers ids outside 0..38 with entry 0, so such an
                // id takes kung fu's ammo type and slot.
                let info_id = if (0..MAX_WEAPONS as i32).contains(&self.weapon) { self.weapon } else { 0 };
                let t = self.ammo_type(info_id);
                self.reserve[t] = v.reserve as i32;
                self.firemode = v.mode;
                self.attachments = v.attachments as u32;
                if self.weapon >= 1
                    && let Some((r, b)) = grid_pos(info_id)
                {
                    self.grid[r][b] = Slot { id: self.weapon, clip: self.clip };
                }
                // `cl_thirdpersonswitch` 1 (`0x1004f93b..0x1004f970`): kung fu goes to third person
                // whatever the old weapon was; a gun goes back to first person only when the
                // weapon before this message was kung fu (id 0), not on a gun-to-gun change.
                let toggle = if self.weapon < 1 { !ctx.third_person } else { ctx.third_person && old == 0 };
                if ctx.thirdperson_switch && toggle {
                    fx.push(Effect::ToggleThirdPerson);
                }
            }
            Msg::ClipInfo => {
                self.clip = dec!(p::ClipInfo).clip as i32;
                if self.weapon != 0
                    && let Some((r, b)) = grid_pos(self.weapon)
                {
                    self.grid[r][b].clip = self.clip;
                }
            }
            Msg::WStatus => {
                let v = dec!(p::WStatus);
                if let Some((r, b)) = grid_pos(v.id as i32) {
                    self.grid[r][b] = Slot { id: if v.owned == 0 { -1 } else { v.id as i32 }, clip: v.clip as i32 };
                }
            }
            Msg::ActItems => {
                let v = dec!(p::ActItems);
                if v.client <= 32 {
                    self.players[v.client as usize].active_items = v.bits;
                }
            }
            Msg::NextStunt => self.next_stunt = dec!(p::NextStunt).tenths as f32 * 0.1,
            Msg::Wires => {
                let v = dec!(p::Wires);
                self.wires = [v.w0, v.w1, v.w2];
            }
            Msg::CustomWp => {
                let v = dec!(p::CustomWp);
                self.custom_weapons.retain(|c| c.id != v.id);
                self.custom_weapons.push(v);
            }
            Msg::KFuPower => {
                self.kfu_prev = self.kfu_power;
                self.kfu_power = (dec!(p::KFuPower).power as f32).clamp(-100.0, 100.0);
            }
            // ---- CHud / viewport
            // `0x100219a0`: every element's `Reset` (`CHudStatusIcons` too), then `m_iConcuss`
            // = 0; the caller zeroes the zoom sensitivity (`+0x18`).
            Msg::ResetHUD => {
                self.reset();
                self.icons.reset();
                self.message.reset();
                self.status_bar.reset();
                self.sec_ammo.reset();
                self.concuss = 0;
            }
            Msg::InitHUD => self.init_hud_data(),
            Msg::GameMode => {
                let mut r = p::Reader::new(buf);
                self.gamemode = r.byte() as i32;
            }
            // The caller applies it (`hud::set_fov`): the zoom lives in `crate::state`.
            Msg::SetFov => {}
            Msg::ScoreInfo => {
                let v = dec!(p::ScoreInfo);
                if (1..=MAX_CLIENTS as u8).contains(&v.client) {
                    let pl = &mut self.players[v.client as usize];
                    pl.frags = v.frags as i32;
                    pl.deaths = v.deaths as i32;
                    pl.class = v.class as i32;
                    pl.team = v.team as i32;
                }
            }
            Msg::TeamInfo => {
                let v = dec!(p::TeamInfo);
                if (1..=MAX_CLIENTS as u8).contains(&v.client) {
                    self.players[v.client as usize].teamname = v.team.chars().take(15).collect();
                }
            }
            Msg::TeamScore => {
                let v = dec!(p::TeamScore);
                match self.team_scores.iter_mut().find(|t| t.name.eq_ignore_ascii_case(&v.team)) {
                    Some(t) => {
                        t.frags = v.frags as i32;
                        t.deaths = v.deaths as i32;
                    }
                    None => self.team_scores.push(TeamScore { name: v.team, frags: v.frags as i32, deaths: v.deaths as i32 }),
                }
            }
            Msg::TeamNames => {
                let Some(names) = p::decode_team_names(buf) else { return false };
                self.team_names = names;
                for n in &self.team_names {
                    if !self.team_scores.iter().any(|t| t.name.eq_ignore_ascii_case(n)) {
                        self.team_scores.push(TeamScore { name: n.clone(), ..Default::default() });
                    }
                }
            }
            Msg::ServerName => self.server_name = p::Reader::new(buf).string(),
            Msg::Motd => {
                let v = dec!(p::Motd);
                if self.motd_final {
                    self.motd.clear();
                    self.motd_final = false;
                }
                // Viewport buffer at +0x240 holds 0x600 bytes.
                let room = 0x5ffusize.saturating_sub(self.motd.len());
                self.motd.extend(v.text.chars().take(room));
                if v.last != 0 {
                    self.motd_final = true;
                    fx.push(Effect::OpenMotd);
                }
            }
            Msg::Spectator => {
                let v = dec!(p::Spectator);
                let cl = v.client as usize;
                if (1..=MAX_CLIENTS).contains(&cl) {
                    // TS keeps the raw byte (`0x101900ec[i]`, copied to `0x101901f5`), but its
                    // readers, `HUD_AddEntity` (`0x10044846`) and the scoreboard (`0x1005e3e8`),
                    // only test it for zero; the local `== 1` test below is the one that differs.
                    self.players[cl].observer = v.observer != 0;
                    self.players[cl].active_items = 0;
                }
                // The local player (`10065294..100652f8`), on every message, not only on a change:
                // observer (`0x100d4401`) is `observer == 1`. Entering leaves third person and
                // opens the buy menu; otherwise `cl_thirdperson` 1 or 0 picks the camera (other
                // values keep it), so the death cam doesn't outlive the respawn.
                if cl as i32 == ctx.local_index {
                    self.observer = v.observer == 1;
                    let toggle = if self.observer {
                        ctx.third_person
                    } else if ctx.cl_thirdperson == 1.0 {
                        !ctx.third_person
                    } else {
                        ctx.cl_thirdperson == 0.0 && ctx.third_person
                    };
                    if toggle {
                        fx.push(Effect::ToggleThirdPerson);
                    }
                    if self.observer {
                        fx.push(Effect::OpenBuyMenu);
                    }
                    fx.push(Effect::ResetSpread);
                }
            }
            Msg::VguiMenu => {
                let mut r = p::Reader::new(buf);
                let id = r.byte();
                if r.bad {
                    return false;
                }
                match id {
                    // `ShowVGUIMenu` (0x10064050): the team menu (0x10064280) needs
                    // `m_Teamplay >= 1`, so a non-teamplay server's `changeteam` reply is ignored.
                    2 if self.gamemode >= 1 => fx.push(Effect::OpenTeamMenu),
                    4 => self.map_briefing = r.string().chars().take(63).collect(),
                    5 => fx.push(Effect::OpenMotd),
                    _ => {}
                }
            }
            // `hud::notice::on_death_msg` handles it: it needs the engine's player names.
            Msg::DeathMsg => {
                dec!(p::DeathMsg);
            }
            // `MsgFunc_SayText` (`0x1002ecc0`): `SayTextPrint(text, size - 1, client)`.
            Msg::SayText => {
                let v = dec!(p::SayText);
                fx.push(Effect::SayText(v.text, v.client as i32));
            }
            Msg::TextMsg => {
                let v = dec!(p::TextMsg);
                let mut args: Vec<String> = v.strings.iter().map(|s| localise_with(ctx.titles, s)).collect();
                if args.is_empty() {
                    return true;
                }
                let fmt = args.remove(0).replace('\r', "\n");
                let a: Vec<super::text::Arg> = args.iter().map(|s| super::text::Arg::S(s.trim_end_matches('\n'))).collect();
                let text = super::text::format(&fmt, &a);
                match v.dest {
                    4 => {
                        self.center_text = text.clone();
                        self.center_expire = now + 5.0;
                        fx.push(Effect::CenterPrint(text));
                    }
                    // HUD_PRINTTALK: `SayTextPrint(text, 128, -1)` (`0x10034c01`).
                    3 => fx.push(Effect::SayText(text, -1)),
                    _ => fx.push(Effect::ConsolePrint(text)),
                }
            }
            // `CHudMessage::MsgFunc_HudText` (`0x100295c0`). A short message is TS's empty
            // `READ_STRING`, a custom message with no text.
            Msg::HudText => {
                let text = super::saytext::bytes(&p::Reader::new(buf).string());
                self.message.msg_hud_text(&text, now, ctx.titles);
            }
            // `CHudMessage::MsgFunc_GameTitle` (`0x10029610`, hooked at `0x100288e0`).
            Msg::GameTitle => self.message.msg_game_title(now, ctx.titles),
            // CHudTrain::MsgFunc_Train (10034df0).
            Msg::Train => self.train_pos = dec!(p::Train).pos,
            // CHudAmmo::MsgFunc_HideWeapon (1001a110). TS only resets the engine crosshair,
            // which it doesn't use; the port keeps the bits for HIDEHUD_ALL.
            Msg::HideWeapon => self.hide_hud = dec!(p::HideWeapon).flags,
            Msg::ShowMenu => {
                let v = dec!(p::ShowMenu);
                if v.slots == 0 {
                    self.menu = TextMenu::default();
                    return true;
                }
                self.menu.pending.push_str(&v.text);
                if v.more == 0 {
                    self.menu.text = localise_with(ctx.titles, &std::mem::take(&mut self.menu.pending));
                    self.menu.slots = v.slots;
                    self.menu.displayed = true;
                    self.menu.shutoff = if v.time > 0 { now + v.time as f32 } else { 0.0 };
                }
            }
            // Stock/TFC bookkeeping messages TS keeps hooked; their elements draw nothing.
            Msg::CurWeapon
            | Msg::WeaponList
            | Msg::AmmoPickup
            | Msg::WeapPickup
            | Msg::ItemPickup
            | Msg::AmmoX
            | Msg::Geiger
            | Msg::Health
            | Msg::Battery
            | Msg::Damage
            | Msg::SelAmmo => {}
            // Engine-handled; never hooked.
            Msg::ScreenShake | Msg::ScreenFade => {}
        }
        true
    }

    /// `CHudMenu::SelectMenuItem`: returns the `menuselect` command if the slot is valid.
    pub fn menu_select(&mut self, slot: i32) -> Option<String> {
        if !self.menu.displayed || !(1..=10).contains(&slot) {
            return None;
        }
        if self.menu.slots & (1 << (slot - 1)) == 0 {
            return None;
        }
        self.menu.displayed = false;
        Some(format!("menuselect {slot}\n"))
    }

    /// Kung-fu power regenerates client-side between `KFuPower` updates (`Think` 1004ce10):
    /// 15/s when non-negative, 1/s while negative, scaled by frame time / time scale.
    pub fn regen_kungfu(&mut self, dt: f32) {
        if self.kfu_power < 100.0 {
            let step = if self.kfu_power >= -0.001 { dt * 15.0 } else { dt };
            self.kfu_power = (self.kfu_power + step).min(100.0);
        }
    }

    /// `DrawRoundTimer` tick (1004a130): one second (or 0.1 s in mode 1) per tick; frozen in
    /// mode 2. Returns the seconds to show.
    pub fn tick_round_timer(&mut self, now: f32) -> i32 {
        if self.round_next < now {
            if self.round_mode != 2 {
                self.round_time -= 1.0;
            }
            if self.round_time <= 0.0 {
                self.round_time = 0.0;
            }
            self.round_next = now + if self.round_mode == 1 { 0.1 } else { 1.0 };
        }
        self.round_time as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::protocol::hud::{
        KFuPower, Motd, PwUp, PwUpTime, ScoreInfo, ShowMenu, ShowObj, Spectator, TeamInfo, TextMsg, TsCash, TsHealth, TsMessage, TsPAward,
        TsSpace, TsState, WStatus, WeaponInfo,
    };

    struct NoTitles;
    impl Titles for NoTitles {
        fn get(&self, key: &str) -> Option<String> {
            (key == "HUD_Credits").then(|| "Credits".to_owned())
        }
    }

    fn ctx(screen: &Screen) -> Ctx<'_> {
        Ctx {
            now: 10.0,
            screen,
            titles: &NoTitles,
            local_index: 1,
            local_name: "me".into(),
            third_person: false,
            thirdperson_switch: false,
            cl_thirdperson: 0.0,
            saytext_time: 5.0,
            deathnotice_time: 6.0,
        }
    }

    #[test]
    fn status_messages() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        assert!(h.apply(Msg::TsHealth, &TsHealth { hp: 73 }.encode(), &c, &mut fx));
        assert!(h.apply(Msg::TsCash, &TsCash { cash: 1200 }.encode(), &c, &mut fx));
        assert!(h.apply(Msg::TsSpace, &TsSpace { slots: 61 }.encode(), &c, &mut fx));
        assert!(h.apply(Msg::TsState, &TsState { stance: 2 }.encode(), &c, &mut fx));
        assert_eq!((h.health, h.cash, h.slots, h.stance), (73, 1200, 61, 2));
        assert!(fx.is_empty());
        h.reset();
        assert_eq!((h.health, h.last_hp), (0, 73), "ResetHUD leaves 0x100cf9d8");
        assert!(h.apply(Msg::TsHealth, &TsHealth { hp: 0 }.encode(), &c, &mut fx));
        assert_eq!(fx, [Effect::ToggleThirdPerson], "death cam");
        assert_eq!(h.last_hp, 0);
        assert!(!h.apply(Msg::TsCash, &[1], &c, &mut fx), "short payload rejected");
    }

    #[test]
    fn powerups() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        h.apply(Msg::PwUp, &PwUp { flags: 1, value: 3 }.encode(), &c, &mut fx);
        assert_eq!((h.pw_flags, h.pw_value), (1, 3));
        h.apply(Msg::PwUp, &PwUp { flags: 4, value: 0 }.encode(), &c, &mut fx);
        assert!(h.kungfu_icon && !h.move_icon);
        assert_eq!(h.pw_flags, 1, "permanent flags don't replace the timed power-up");
        h.apply(Msg::PwUpTime, &PwUpTime { secs: 12 }.encode(), &c, &mut fx);
        assert_eq!((h.pw_expire, h.pw_value), (22.0, 0));
    }

    #[test]
    fn weapon_grid() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        let (r, b) = grid_pos(0).unwrap();
        assert_eq!(h.grid[r][b], Slot { id: 0, clip: 1 });
        h.apply(Msg::WeaponInfo, &WeaponInfo { id: 13, clip: 30, reserve: 90, mode: 0, attachments: 8 }.encode(), &c, &mut fx);
        let (r, b) = grid_pos(13).unwrap();
        assert_eq!((r, b), (3, 4), "AK47 hud = [4, 3]");
        assert_eq!(h.grid[r][b], Slot { id: 13, clip: 30 });
        assert_eq!(h.reserve[6], 90, "7.62x39 is ammo type 6");
        h.apply(Msg::WeaponInfo, &WeaponInfo { id: 13, clip: 29, reserve: 90, mode: 1, attachments: 8 }.encode(), &c, &mut fx);
        assert_eq!((h.weapon, h.clip, h.firemode), (13, 29, 1));
        // 0xff is weapon 255, as in TS: entry 0's slot (kung fu) and ammo type take it.
        h.apply(Msg::WeaponInfo, &WeaponInfo { id: 0xff, clip: 7, reserve: 5, mode: 0, attachments: 0 }.encode(), &c, &mut fx);
        assert_eq!(h.weapon, 255);
        let (kr, kb) = grid_pos(0).unwrap();
        assert_eq!(h.grid[kr][kb], Slot { id: 255, clip: 7 });
        assert_eq!(h.grid[r][b], Slot { id: 13, clip: 29 });
        h.apply(Msg::WStatus, &WStatus { id: 13, owned: 0, clip: 0 }.encode(), &c, &mut fx);
        assert_eq!(h.grid[r][b].id, -1);
        h.apply(Msg::KFuPower, &KFuPower { power: -120 }.encode(), &c, &mut fx);
        assert_eq!(h.kfu_power, -100.0);
    }

    #[test]
    fn ts_messages_stack_and_replace() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        let m = TsMessage { r: 1, g: 2, b: 3, flags: 1, time: 30, text: "#HUD_Credits".into() };
        h.apply(Msg::TsMessage, &m.encode(), &c, &mut fx);
        h.apply(Msg::TsMessage, &m.encode(), &c, &mut fx);
        assert_eq!(h.messages[0].text, "Credits");
        assert_eq!(h.messages[0].y, 32);
        assert_eq!(h.messages[1].y, 48, "second top message goes below the first");
        assert!((h.messages[0].expire - 13.0).abs() < 1e-4, "time is in tenths");
        // 0x10 replaces an active message at the same spot with the same flags.
        let r = TsMessage { flags: 1 | 0x10, text: "x".into(), ..m.clone() };
        h.apply(Msg::TsMessage, &r.encode(), &c, &mut fx);
        h.apply(Msg::TsMessage, &r.encode(), &c, &mut fx);
        assert_eq!(h.messages[2].expire, 0.0, "first 0x11 message replaced");
        assert_eq!((h.messages[3].y, h.messages[3].text.as_str()), (32, "x"));
        let mine = TsMessage { flags: 0, text: "hello me".into(), ..m };
        h.apply(Msg::TsMessage, &mine.encode(), &c, &mut fx);
        assert!(h.messages.iter().any(|l| l.text == "hello me" && l.rgb == [155; 3]), "grey when it contains your name");
    }

    #[test]
    fn objectives_bounds_fixed() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        h.apply(Msg::Objective, &p::Objective { x: 1.0, y: 2.0, z: 3.0, idx: 5, label: "Base".into() }.encode(), &c, &mut fx);
        assert!(h.objectives[5].visible);
        assert_eq!(h.obj_max, 5);
        h.apply(Msg::Objective, &p::Objective { idx: 200, label: "bad".into(), ..Default::default() }.encode(), &c, &mut fx);
        assert_eq!(h.obj_max, 5);
        h.apply(Msg::ShowObj, &ShowObj { idx: 5, visible: 0 }.encode(), &c, &mut fx);
        assert!(!h.objectives[5].visible);
        h.apply(Msg::Objective, &p::Objective { idx: 0, label: "a".into(), ..Default::default() }.encode(), &c, &mut fx);
        assert!(h.objectives[5].label.is_empty(), "idx 0 clears all");
    }

    #[test]
    fn scoreboard_messages() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        h.apply(Msg::ScoreInfo, &ScoreInfo { client: 3, frags: 7, deaths: 2, class: 0, team: 1 }.encode(), &c, &mut fx);
        h.apply(Msg::TeamInfo, &TeamInfo { client: 3, team: "Specialists".into() }.encode(), &c, &mut fx);
        h.apply(Msg::TsPAward, &TsPAward { client: 3, award: 10 }.encode(), &c, &mut fx);
        h.apply(Msg::ScoreInfo, &ScoreInfo { client: 65, frags: 1, deaths: 1, class: 0, team: 0 }.encode(), &c, &mut fx);
        let p = &h.players[3];
        assert_eq!((p.frags, p.deaths, p.team, p.award), (7, 2, 1, 10));
        assert_eq!(p.teamname, "Specialists");
        h.apply(Msg::TeamScore, &p::TeamScore { team: "Specialists".into(), frags: 9, deaths: 3 }.encode(), &c, &mut fx);
        assert_eq!(h.team_scores[0].frags, 9);
    }

    #[test]
    fn team_menu_only_in_teamplay() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        let menu = [2u8];
        assert!(h.apply(Msg::VguiMenu, &menu, &c, &mut fx));
        assert!(fx.is_empty(), "the server sends it for changeteam in DM too");
        h.apply(Msg::GameMode, &[1], &c, &mut fx);
        h.apply(Msg::VguiMenu, &menu, &c, &mut fx);
        assert_eq!(fx, [Effect::OpenTeamMenu]);
    }

    #[test]
    fn motd_chunks_and_menus() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        h.apply(Msg::Motd, &Motd { last: 0, text: "Hello ".into() }.encode(), &c, &mut fx);
        h.apply(Msg::Motd, &Motd { last: 1, text: "world".into() }.encode(), &c, &mut fx);
        assert_eq!(h.motd, "Hello world");
        assert_eq!(fx, [Effect::OpenMotd]);
        h.apply(Msg::ShowMenu, &ShowMenu { slots: 0b11, time: -1, more: 1, text: "1. A\n".into() }.encode(), &c, &mut fx);
        assert!(!h.menu.displayed);
        h.apply(Msg::ShowMenu, &ShowMenu { slots: 0b11, time: -1, more: 0, text: "2. B".into() }.encode(), &c, &mut fx);
        assert_eq!(h.menu.text, "1. A\n2. B");
        assert_eq!(h.menu_select(3), None);
        assert_eq!(h.menu_select(2).as_deref(), Some("menuselect 2\n"));
        assert!(!h.menu.displayed);
    }

    #[test]
    fn spectator_opens_buy_menu() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        h.apply(Msg::Spectator, &Spectator { client: 1, observer: 1 }.encode(), &c, &mut fx);
        assert!(h.observer);
        assert_eq!(fx, [Effect::OpenBuyMenu, Effect::ResetSpread], "first person stays");
        fx.clear();
        let third = Ctx { third_person: true, ..ctx(&s) };
        h.apply(Msg::Spectator, &Spectator { client: 1, observer: 1 }.encode(), &third, &mut fx);
        assert_eq!(fx, [Effect::ToggleThirdPerson, Effect::OpenBuyMenu, Effect::ResetSpread], "every message, not only on a change");
        fx.clear();
        h.apply(Msg::Spectator, &Spectator { client: 2, observer: 1 }.encode(), &c, &mut fx);
        assert!(fx.is_empty());
        assert!(h.players[2].observer);
        h.apply(Msg::Spectator, &Spectator { client: 1, observer: 2 }.encode(), &c, &mut fx);
        assert!(!h.observer, "only 1 is an observer");
    }

    #[test]
    fn spectator_applies_cl_thirdperson() {
        let s = Screen::default();
        let mut h = Hud::default();
        let mut toggles = |third: bool, cl_thirdperson: f32| {
            let c = Ctx { third_person: third, cl_thirdperson, ..ctx(&s) };
            let mut fx = Vec::new();
            h.apply(Msg::Spectator, &Spectator { client: 1, observer: 0 }.encode(), &c, &mut fx);
            fx.contains(&Effect::ToggleThirdPerson)
        };
        assert!(toggles(true, 0.0), "the death cam doesn't survive the respawn");
        assert!(toggles(false, 1.0));
        assert!(!toggles(true, 1.0) && !toggles(false, 0.0));
        assert!(!toggles(true, 2.0) && !toggles(false, 2.0), "other values keep the camera");
    }

    #[test]
    fn thirdperson_switch_on_weapon_info() {
        let s = Screen::default();
        let mut h = Hud::default();
        let mut toggles = |old: i32, new: u8, third: bool| {
            h.weapon = old;
            let c = Ctx { third_person: third, thirdperson_switch: true, ..ctx(&s) };
            let mut fx = Vec::new();
            h.apply(Msg::WeaponInfo, &WeaponInfo { id: new, clip: 0, reserve: 0, mode: 0, attachments: 0 }.encode(), &c, &mut fx);
            fx == [Effect::ToggleThirdPerson]
        };
        assert!(toggles(13, 0, false) && toggles(0, 0, false), "kung fu: third person, changed or not");
        assert!(!toggles(13, 0, true));
        assert!(toggles(0, 13, true), "kung fu to a gun: first person");
        assert!(!toggles(13, 13, true) && !toggles(12, 13, true), "not on a gun-to-gun change");
        assert!(!toggles(0, 13, false));
    }

    #[test]
    fn round_timer_ticks() {
        let mut h = Hud { round_time: 3.0, ..Default::default() };
        assert_eq!(h.tick_round_timer(1.0), 2);
        assert_eq!(h.tick_round_timer(1.5), 2);
        assert_eq!(h.tick_round_timer(2.1), 1);
        h.round_mode = 2;
        assert_eq!(h.tick_round_timer(5.0), 1, "mode 2 is frozen");
    }

    #[test]
    fn text_msg_formats() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        let t = TextMsg { dest: 4, strings: vec!["%s has #HUD_Credits".into(), "Bob".into()] };
        h.apply(Msg::TextMsg, &t.encode(), &c, &mut fx);
        assert_eq!(h.center_text, "Bob has Credits");
    }

    /// `HudText` reaches `CHudMessage` (a short message is an empty custom message, as TS's
    /// `READ_STRING` gives ""), and `ResetHUD` runs the SDK elements' `Reset`s.
    #[test]
    fn hud_text_and_reset_hud() {
        let s = Screen::default();
        let c = ctx(&s);
        let mut h = Hud::default();
        let mut fx = Vec::new();
        assert!(h.apply(Msg::HudText, b"#Unknown\0", &c, &mut fx));
        assert_eq!(h.message.custom.text, b"#Unknown");
        assert!(h.message.active);
        assert!(h.apply(Msg::HudText, b"", &c, &mut fx));
        assert_eq!(h.message.custom.text, b"", "a short message still adds");
        h.status_bar.msg_status_text(&[0, b'0', b'x', 0]);
        h.sec_ammo.msg_val(&[0, 3]);
        assert!(h.apply(Msg::ResetHUD, &[0], &c, &mut fx));
        assert!(h.message.slots.iter().all(Option::is_none));
        assert!(!h.status_bar.active && h.status_bar.text[0].is_empty());
        assert_eq!(h.sec_ammo.fade, 0.0);
    }
}
