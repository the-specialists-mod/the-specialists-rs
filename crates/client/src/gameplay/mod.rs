//! The client's gameplay plugins (`docs/implementation/gameplay-plugins.md`): the registry
//! `client.dll` fills at `Initialize`, and where each part of the client finds the active rules.
//!
//! * Movement prediction (`HUD_PlayerMove`): [`pm_rules`], from `pmove->physinfo`.
//! * Weapon prediction (`HUD_PostRunCmd`): [`gun_rules`], from `clientdata_t::physinfo`, per
//!   command.
//! * HUD: [`hud_rules`], from the server's `SrvSett` key 4 (the HUD's `gameplay`); the weapon
//!   events: [`hud_gun_rules`], from the same.
//!
//! When the server switches rulesets it re-sends `SrvSett` 4, and a changed value resets the
//! client to a brand-new state ([`reset_client`]) before the server's fresh state arrives. The
//! value the HUD holds is cleared by each `InitHUD` (a connect, a map, the fresh join after a
//! switch), whose own `SrvSett` 4 then only records it.

pub mod client;

pub use client::ClientRules;

use goldsrc::client::engine;
use goldsrc::pmove::PlayerMoveExt;
use goldsrc::sys::{clientdata_t, playermove_t};
use shared::gameplay::{CORE, GunRules, PHYSINFO_KEY, PHYSINFO_KEY_C, PmRules};
use std::sync::{Mutex, PoisonError, RwLock};
use ts_gameplay::Registry;

/// A gameplay plugin as the client sees it: its shared hooks and the client's. Each part is its
/// own object, so a plugin's client half can live in a package of its own (the client library
/// links it; the server library never does).
pub struct ClientPlugin {
    pub pm: &'static dyn PmRules,
    pub gun: &'static dyn GunRules,
    pub client: &'static dyn ClientRules,
}

impl ClientRules for shared::gameplay::Core {}

/// The core plugin (TS 3.0) on the client.
pub static CORE_PLUGIN: ClientPlugin = ClientPlugin { pm: &CORE, gun: &CORE, client: &CORE };

static REGISTRY: RwLock<Registry<ClientPlugin>> = RwLock::new(Registry::new(&CORE_PLUGIN));

/// The last unknown id warned about, so each is reported once.
static WARNED: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// Registers a plugin (`client.dll`'s `Initialize`). False for an id that isn't a short number.
pub fn register(id: &'static str, plugin: &'static ClientPlugin) -> bool {
    REGISTRY.write().unwrap_or_else(PoisonError::into_inner).add(id, plugin)
}

/// The plugin for an id the server sent; an unknown one is the core plugin, with a console
/// warning the first time.
fn resolve(id: &[u8]) -> &'static ClientPlugin {
    let r = REGISTRY.read().unwrap_or_else(PoisonError::into_inner).resolve_bytes(id);
    if r.unknown {
        let mut w = WARNED.lock().unwrap_or_else(PoisonError::into_inner);
        if w.as_deref() != Some(id) {
            *w = Some(id.to_vec());
            engine().print(&format!(
                "[ts] gameplay: the server's ruleset \"{}\" isn't in this client; predicting with \"{}\"\n",
                String::from_utf8_lossy(id),
                ts_gameplay::DEFAULT_ID
            ));
        }
    }
    r.plugin
}

/// The movement rules for one `HUD_PlayerMove`, from the player's physinfo.
pub fn pm_rules(pm: &playermove_t) -> &'static dyn PmRules {
    resolve(pm.physinfo_value(PHYSINFO_KEY_C).as_bytes()).pm
}

/// The gun rules for one predicted command, from the `clientdata_t` it starts from.
pub fn gun_rules(cd: &clientdata_t) -> &'static dyn GunRules {
    let buf = cd.physinfo.map(|c| c as u8);
    resolve(ts_gameplay::info_value(&buf, PHYSINFO_KEY.as_bytes()).unwrap_or_default()).gun
}

/// The HUD's rules, from the `SrvSett` 4 value it holds (`None` before the server sent one, or
/// from a server that doesn't: TS 3.0's).
pub fn hud_rules(id: Option<i16>) -> &'static dyn ClientRules {
    match id {
        Some(v) => resolve(v.to_string().as_bytes()).client,
        None => &CORE,
    }
}

/// The gun rules outside a predicted command, for the weapon events (`EV_TS_Fire*`): the HUD's
/// ruleset (`SrvSett` 4), as [`hud_rules`].
pub fn hud_gun_rules() -> &'static dyn GunRules {
    match crate::hud::with(|h| h.gameplay) {
        Some(v) => resolve(v.to_string().as_bytes()).gun,
        None => &CORE,
    }
}

/// The client's part of a ruleset switch: everything a new map resets (`HUD_VidInit`), so no HUD,
/// prediction, effects or spectator state carries over. The HUD keeps the new ruleset's id.
/// Voice settings aren't game state and stay.
pub fn reset_client(id: i16) {
    crate::hud::vid_init();
    crate::prediction::vid_init();
    crate::effects::vid_init();
    crate::spectator::vid_init();
    engine().print(&format!("[ts] gameplay: the server switched to ruleset {id}; HUD reset\n"));
}
