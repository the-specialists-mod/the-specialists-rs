//! The HEV suit's sentence queue: `CBasePlayer::SetSuitUpdate` (0xda410) and
//! `CBasePlayer::CheckSuitUpdate` (0xda2d0), unchanged from the SDK.
//!
//! Both work only for a player with the suit bit (`pev->weapons` bit 31) under rules whose
//! `IsMultiplayer` (vtable +0x1c) is 0: `CHalfLifeRules` (0xde734), the single-player rules
//! (`deathmatch` 0, [`state::single_player`]). There `item_suit` gives the suit, and damage, the
//! battery, the antidote and the time-based cures queue `!HEV_*` sentences, which PreThink plays
//! one at a time, 3.5 s apart. Under the multiplayer rules `SetSuitUpdate` returns at once and
//! `CheckSuitUpdate` only runs the geiger counter.

use super::Player;
use crate::entities::stock::sentences;
use crate::rules::state;
use crate::world::Cx;
use goldsrc::edict::EdictPtr;
use goldsrc::ext::entities::ATTN_NORM;
use goldsrc::server::engine;
use goldsrc::sys::*;
use std::ffi::{CStr, CString};

/// `CSUITPLAYLIST`.
pub const PLAY_LIST: usize = 4;
/// `CSUITNOREPEAT`.
pub const NO_REPEAT: usize = 32;
/// `SUITUPDATETIME`: seconds between two sentences.
const UPDATE_TIME: f32 = 3.5;

/// The suit fields of `CBasePlayer` (`m_flSuitUpdate` .. `m_rgflSuitNoRepeatTime`, +0x864 ..
/// +0x97c). Nothing resets them on spawn or death (`Killed` and `StartObserver` only empty the play
/// list), so the no-repeat times outlive a life, as in TS.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct SuitQueue {
    /// `m_flSuitUpdate` (+0x864): when the next sentence may play; 0 while the list is empty.
    pub update: f32,
    /// `m_rgSuitPlayList` (+0x868): sentence indices (`!NAME`), or minus a group index; 0 = empty.
    pub play_list: [i32; PLAY_LIST],
    /// `m_iSuitPlayNext` (+0x878): the next free play list slot.
    pub play_next: i32,
    /// `m_rgiSuitNoRepeat` (+0x87c) and `m_rgflSuitNoRepeatTime` (+0x8fc): sentences that
    /// mustn't be queued again until their time.
    pub no_repeat: [i32; NO_REPEAT],
    pub no_repeat_time: [f32; NO_REPEAT],
}

impl SuitQueue {
    /// `SetSuitUpdate(NULL, ...)`: empty the play list (the update time and the no-repeat
    /// tables stay).
    pub fn clear(&mut self) {
        self.play_list = [0; PLAY_LIST];
    }

    /// `SetSuitUpdate` (0xda410) after the lookup: refused while `isentence` is still in the
    /// no-repeat table; otherwise remembered there for `no_repeat` seconds (when non-zero; a full
    /// table loses a random slot), appended to the play list (overwriting the oldest when it is
    /// full), and the first sentence of an idle suit plays 0.1 s later.
    pub fn queue(&mut self, isentence: i32, no_repeat: i32, now: f32, random: impl FnOnce(i32, i32) -> i32) {
        let mut empty = -1;
        for i in 0..NO_REPEAT {
            if self.no_repeat[i] == isentence {
                if now <= self.no_repeat_time[i] {
                    return;
                }
                self.no_repeat[i] = 0;
                self.no_repeat_time[i] = 0.0;
                empty = i as i32;
                break;
            }
            if self.no_repeat[i] == 0 {
                empty = i as i32;
            }
        }
        if no_repeat != 0 {
            if empty < 0 {
                empty = random(0, NO_REPEAT as i32 - 1);
            }
            // Guard: TS indexes with whatever RandomLong(0, 31) returned.
            if let Some(i) = usize::try_from(empty).ok().filter(|&i| i < NO_REPEAT) {
                self.no_repeat[i] = isentence;
                self.no_repeat_time[i] = no_repeat as f32 + now;
            }
        }
        // Guard: TS writes `m_rgSuitPlayList[m_iSuitPlayNext]` unchecked; only a corrupt save
        // puts `m_iSuitPlayNext` outside 0..4.
        if let Some(slot) = usize::try_from(self.play_next).ok().and_then(|i| self.play_list.get_mut(i)) {
            *slot = isentence;
        }
        self.play_next += 1;
        if self.play_next == PLAY_LIST as i32 {
            self.play_next = 0;
        }
        if self.update <= now {
            // `time + 0.1` with a double constant (0xda5f7), stored as a float.
            self.update = if self.update == 0.0 { (now as f64 + 0.1) as f32 } else { now + UPDATE_TIME };
        }
    }

    /// `CheckSuitUpdate` (0xda2d0) once `m_flSuitUpdate` is due: the first queued sentence from
    /// `m_iSuitPlayNext` on (wrapping), taken off the list, and the next update 3.5 s later; with
    /// the list empty the suit goes idle (`update` 0). `m_iSuitPlayNext` doesn't move.
    pub fn next(&mut self, now: f32) -> Option<i32> {
        if !(self.update <= now && self.update > 0.0) {
            return None;
        }
        // Guard: as in `queue`, only a corrupt save puts `m_iSuitPlayNext` outside 0..4.
        let mut search = self.play_next.rem_euclid(PLAY_LIST as i32) as usize;
        let mut found = 0;
        for _ in 0..PLAY_LIST {
            found = self.play_list[search];
            if found != 0 {
                break;
            }
            search += 1;
            if search == PLAY_LIST {
                search = 0;
            }
        }
        if found == 0 {
            self.update = 0.0;
            return None;
        }
        self.play_list[search] = 0;
        self.update = now + UPDATE_TIME;
        Some(found)
    }
}

/// The sentence `SetSuitUpdate` queues for `name`: `SENTENCEG_Lookup(name)` (`!NAME`; not found
/// queues nothing), or for a group `-SENTENCEG_GetIndex(name)` (an unknown group is -(-1) = 1,
/// as in the SDK).
fn sentence_of(name: &CStr, group: bool) -> Option<i32> {
    if group { Some(-sentences::group_index(name)) } else { sentences::lookup(name).map(|i| i as i32) }
}

/// `SetSuitUpdate`'s gate: the suit bit, then `IsMultiplayer()` false.
fn suit_works(v: &entvars_t) -> bool {
    v.weapons < 0 && state::single_player()
}

fn queue_at(q: &mut SuitQueue, name: Option<&CStr>, group: bool, no_repeat: i32, now: f32) {
    let Some(name) = name else {
        q.clear();
        return;
    };
    let Some(isentence) = sentence_of(name, group) else { return };
    q.queue(isentence, no_repeat, now, |lo, hi| engine().random_long(lo, hi));
}

/// `CBasePlayer::SetSuitUpdate` (0xda410): queue `!NAME` (or a random sentence of group `name`)
/// on the suit, not again for `no_repeat` seconds; `None` empties the play list.
pub fn set_suit_update(p: &mut Player, cx: &mut Cx, name: Option<&CStr>, group: bool, no_repeat: i32) {
    if !suit_works(cx.pev()) {
        return;
    }
    let now = cx.time();
    queue_at(&mut p.suit, name, group, no_repeat, now);
}

/// [`set_suit_update`] for entities that hold only the player's edict (the battery and antidote
/// touches). It reaches a player lent to its slot at once; one held by a dispatch further up the
/// stack gets the sentence when that returns, queued at the time of the call.
pub fn set_suit_update_edict(e: *mut edict_t, name: &CStr, group: bool, no_repeat: i32) {
    if e.is_null() || !suit_works(e.v()) {
        return;
    }
    let (name, now) = (CString::from(name), engine().time());
    crate::world::run_now_or_defer(e, move || {
        super::with_player(e, |p, _| queue_at(&mut p.suit, Some(&name), group, no_repeat, now));
    });
}

/// `CBasePlayer::CheckSuitUpdate` (0xda2d0), from PreThink: with the suit, the geiger counter,
/// then (single player only) the next queued sentence once it is due: `!NAME` through
/// `EMIT_SOUND_SUIT`, a group through `EMIT_GROUPID_SUIT`.
pub fn check_suit_update(p: &mut Player, cx: &mut Cx) {
    let e = cx.edict;
    if cx.pev().weapons >= 0 {
        return;
    }
    crate::entities::stock::triggers::update_geiger_counter(e);
    if !state::single_player() {
        return;
    }
    let Some(sentence) = p.suit.next(cx.time()) else { return };
    if sentence > 0 {
        // `"!" + gszallsentencenames[sentence]`. Guard: TS reads the name table at any index;
        // only a corrupt save (or a sentences.txt changed under a restore) holds one past its end.
        if let Some(name) = sentences::name(sentence as usize).and_then(|n| CString::new(format!("!{n}")).ok()) {
            crate::entities::stock::items::emit_sound_suit(e, &name);
        }
    } else {
        emit_groupid_suit(e, -sentence);
    }
}

/// `EMIT_GROUPID_SUIT` (0xe0c44): a random sentence of group `group` at `suitvolume`, pitch 100
/// or, half the time, 98..104 (the rolls happen even when the volume is too low to play).
fn emit_groupid_suit(e: *mut edict_t, group: i32) {
    let eng = engine();
    let vol = eng.cvar_float(c"suitvolume");
    let pitch = if eng.random_long(0, 1) != 0 { eng.random_long(0, 6) + 98 } else { PITCH_NORM as i32 };
    if vol > 0.05 {
        sentences::play_random_index(e, group, vol, ATTN_NORM, 0, pitch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_random(_: i32, _: i32) -> i32 {
        panic!("RandomLong only runs with a full no-repeat table")
    }

    #[test]
    fn first_sentence_waits_a_tenth_then_three_and_a_half() {
        let mut q = SuitQueue::default();
        q.queue(7, 0, 10.0, no_random);
        assert_eq!(q.play_list, [7, 0, 0, 0]);
        assert_eq!(q.play_next, 1);
        assert_eq!(q.update, (10.0f64 + 0.1) as f32, "idle suit: time + 0.1 (double constant)");
        assert_eq!(q.next(10.05), None, "not due yet");
        assert_eq!(q.next(q.update), Some(7));
        assert_eq!(q.update, (10.0f64 + 0.1) as f32 + 3.5);
        // A sentence queued while the update is pending doesn't move it.
        q.queue(8, 0, 11.0, no_random);
        assert_eq!(q.update, (10.0f64 + 0.1) as f32 + 3.5);
        assert_eq!(q.next(13.0), None);
        assert_eq!(q.next(13.6), Some(8));
        // Empty list: the suit goes idle, and the next sentence again plays after 0.1 s.
        assert_eq!(q.next(17.1), None);
        assert_eq!(q.update, 0.0);
        assert_eq!(q.next(100.0), None, "idle stays idle");
        q.queue(9, 0, 20.0, no_random);
        assert_eq!(q.update, (20.0f64 + 0.1) as f32);
    }

    #[test]
    fn queued_while_update_is_overdue_waits_three_and_a_half() {
        let mut q = SuitQueue { update: 5.0, ..Default::default() };
        q.queue(3, 0, 6.0, no_random);
        assert_eq!(q.update, 9.5, "update not 0 and already due: time + 3.5");
    }

    #[test]
    fn no_repeat_refuses_until_its_time_has_passed() {
        let mut q = SuitQueue::default();
        q.queue(4, 30, 10.0, no_random);
        assert_eq!((q.no_repeat[31], q.no_repeat_time[31]), (4, 40.0), "the last empty slot");
        q.queue(4, 30, 40.0, no_random);
        assert_eq!(q.play_next, 1, "refused at exactly the no-repeat time");
        q.queue(4, 0, 40.5, no_random);
        assert_eq!(q.play_list, [4, 4, 0, 0], "expired: queued again");
        assert_eq!((q.no_repeat[31], q.no_repeat_time[31]), (0, 0.0), "and the entry cleared (no new time given)");
    }

    #[test]
    fn no_repeat_takes_the_last_empty_slot_before_a_match() {
        let mut q = SuitQueue::default();
        q.no_repeat[5] = 9;
        q.no_repeat_time[5] = 1.0;
        q.queue(9, 10, 2.0, no_random);
        assert_eq!((q.no_repeat[5], q.no_repeat_time[5]), (9, 12.0), "expired match reuses its own slot");
        q.queue(11, 10, 2.0, no_random);
        assert_eq!(q.no_repeat[31], 11, "no match: the last empty slot");
    }

    #[test]
    fn full_no_repeat_table_overwrites_a_random_slot() {
        let mut q = SuitQueue { no_repeat: [100; NO_REPEAT], no_repeat_time: [50.0; NO_REPEAT], ..Default::default() };
        q.queue(1, 60, 2.0, |lo, hi| {
            assert_eq!((lo, hi), (0, 31));
            17
        });
        assert_eq!((q.no_repeat[17], q.no_repeat_time[17]), (1, 62.0));
    }

    #[test]
    fn play_list_wraps_and_overwrites_the_oldest() {
        let mut q = SuitQueue::default();
        for s in 1..=5 {
            q.queue(s, 0, 1.0, no_random);
        }
        assert_eq!(q.play_list, [5, 2, 3, 4]);
        assert_eq!(q.play_next, 1);
        // CheckSuitUpdate searches from m_iSuitPlayNext, so it plays 2 3 4 5.
        let mut t = 10.0;
        let mut played = vec![];
        while let Some(s) = q.next(t) {
            played.push(s);
            t += UPDATE_TIME;
        }
        assert_eq!(played, [2, 3, 4, 5]);
        assert_eq!(q.play_next, 1, "CheckSuitUpdate doesn't move m_iSuitPlayNext");
    }

    #[test]
    fn clear_empties_only_the_play_list() {
        let mut q = SuitQueue::default();
        q.queue(6, 30, 1.0, no_random);
        q.clear();
        assert_eq!(q.play_list, [0; PLAY_LIST]);
        assert_eq!(q.no_repeat[31], 6);
        assert_eq!(q.update, (1.0f64 + 0.1) as f32);
        assert_eq!(q.next(2.0), None);
        assert_eq!(q.update, 0.0, "the due update finds nothing and idles");
    }

    #[test]
    fn corrupt_play_next_is_guarded() {
        let mut q = SuitQueue { play_next: 9, update: 1.0, play_list: [0, 0, 3, 0], ..Default::default() };
        assert_eq!(q.next(2.0), Some(3));
        q.queue(2, 0, 2.0, no_random);
        assert_eq!(q.play_next, 10);
    }
}
