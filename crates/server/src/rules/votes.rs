//! Chat map votes: `vote map <name>`, `vote stats`, `vote reset` (`CTSGameRules::PlayerVote`
//! 0x7a1c8), the 10 s tally (`CheckVotes` 0x7a5c4) and the vote that ends the map
//! (`CHalfLifeMultiplay::Think` 0xcf038 → `GoToIntermission`, `ChangeLevel` uses the chosen map).
//!
//! TS layout (`TSGameRules`): per-player records at +0x684 (32 × 0x48: voted map[64] and three
//! toggle bytes), tallies at +0xf88 (32 × map[64]) and +0x1788 (32 counts), voter count at
//! +0x1820, chosen map at +0x1824. The toggle bytes (slow match, teamplay, friendly fire) are
//! tallied by `CheckVotes` but nothing ever sets them, so only map votes are ported.

use super::{msg, netname, state};
use goldsrc::server::engine;
use goldsrc::sys::*;
use shared::protocol::rules::hud_dest;

/// Player record slots: edict indices 0..31 (index 32 can't vote in TS).
pub const SLOTS: usize = 32;
/// `char[64]` map names.
const MAX_NAME: usize = 63;
/// Sentinel the (dead) teamplay toggle writes: "change level through the normal map cycle".
pub const NEXTMAP: &str = "nextmap";

/// Vote state; part of `GameRules`, so it starts empty on every map (`CTSGameRules::Init`).
#[derive(Debug, Default)]
pub struct Votes {
    /// Voted map per player index (empty = none).
    pub records: [String; SLOTS],
    /// Tally from the last `CheckVotes`: (map, votes) in slot order.
    pub tallies: Vec<(String, i32)>,
    /// Players counted by the last `CheckVotes` (+0x1820).
    pub voters: i32,
    /// Next `CheckVotes` (+0xf84).
    pub next_check: f32,
    /// The map a passed vote chose (+0x1824); empty until then.
    pub chosen: String,
}

/// A parsed `vote …` chat line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoteCmd {
    Reset,
    Stats,
    /// The first token after `map` (may be empty).
    Map(String),
}

/// `CalcMiniminVotants` (0x7a1b0): half the players, at least 1.
pub fn min_votes(players: i32) -> i32 {
    (players / 2).max(1)
}

/// `PlayerVote`'s parsing of the (lowercased) text after `vote`. The keywords are found anywhere
/// in the text, in TS's order: `reset`, `stats`, then `map`.
pub fn parse(text: &str) -> Option<VoteCmd> {
    if text.contains("reset") {
        return Some(VoteCmd::Reset);
    }
    if text.contains("stats") {
        return Some(VoteCmd::Stats);
    }
    let i = text.find("map")?;
    // The token is the first run of printable, non-space characters (`__ctype_b & 0x6000`).
    let token: String =
        text[i + 3..].chars().skip_while(|c| !c.is_ascii_graphic()).take_while(|c| c.is_ascii_graphic()).take(MAX_NAME).collect();
    Some(VoteCmd::Map(token))
}

impl Votes {
    /// `CheckVotes` tally: counts every player (`IsNetClient`, so bots too) in `voters`, groups
    /// the records of those players by map, and sets `chosen` when the leading map has
    /// `min_votes(players)`. On a tie the map that reached the count first wins.
    pub fn tally(&mut self, voters: impl IntoIterator<Item = usize>) {
        self.tallies.clear();
        let mut players = 0;
        let mut best: Option<(usize, i32)> = None;
        for idx in voters {
            if idx >= SLOTS {
                continue;
            }
            players += 1;
            let map = &self.records[idx];
            if map.is_empty() {
                continue;
            }
            let slot = match self.tallies.iter().position(|(m, _)| m == map) {
                Some(s) => s,
                None if self.tallies.len() < SLOTS => {
                    self.tallies.push((map.clone(), 0));
                    self.tallies.len() - 1
                }
                None => continue,
            };
            self.tallies[slot].1 += 1;
            let n = self.tallies[slot].1;
            if best.is_none_or(|(_, b)| n > b) {
                best = Some((slot, n));
            }
        }
        self.voters = players;
        if let Some((slot, n)) = best
            && n >= min_votes(players)
        {
            self.chosen = self.tallies[slot].0.clone();
        }
    }

    /// `vote stats` lines (against the last tally), or `None` when nothing is being voted on.
    pub fn stats(&self) -> Option<Vec<String>> {
        let need = min_votes(self.voters);
        let lines: Vec<String> = self.tallies.iter().map(|(m, n)| format!("({n}/{need}) votes for {m}\n")).collect();
        if lines.is_empty() { None } else { Some(lines) }
    }

    /// Clears one player's vote (`vote reset`, and `CTSGameRules::InitHUD` for a joiner).
    pub fn reset(&mut self, idx: usize) {
        if let Some(r) = self.records.get_mut(idx) {
            r.clear();
        }
    }

    /// The level `ChangeLevel` should load instead of the map cycle's choice, if any.
    pub fn override_map(&self) -> Option<&str> {
        (!self.chosen.is_empty() && self.chosen != NEXTMAP).then_some(self.chosen.as_str())
    }
}

/// `PlayerVote(edict, text)`: `text` is the chat line after `vote`. Returns true when handled
/// (then `Host_Say` doesn't broadcast the line).
pub fn player_vote(e: *mut edict_t, idx: i32, text: &str) -> bool {
    let Ok(idx) = usize::try_from(idx) else { return false };
    if e.is_null() || idx >= SLOTS {
        return false;
    }
    let Some(cmd) = parse(&text.to_ascii_lowercase()) else { return false };
    let reply = match cmd {
        VoteCmd::Reset => {
            state::with(|g| g.votes.reset(idx));
            "Your votes are resetted.\n".to_owned()
        }
        VoteCmd::Stats => {
            match state::with(|g| g.votes.stats()) {
                Some(lines) => lines.iter().for_each(|l| msg::text_one(e, hud_dest::TALK, l)),
                None => msg::text_one(e, hud_dest::TALK, "No votes in progress.\n"),
            }
            return true;
        }
        VoteCmd::Map(map) => {
            let eng = engine();
            if eng.string(eng.globals().mapname).to_bytes() == map.as_bytes() {
                format!("{map} is the current map!\n")
            } else if eng.is_map_valid(&map) {
                state::with(|g| g.votes.records[idx] = map.clone());
                msg::text_all(hud_dest::TALK, &format!("{} vote for map {map}\n", netname(e)));
                return true;
            } else {
                format!("{map} is an invalid map\n")
            }
        }
    };
    msg::text_one(e, hud_dest::TALK, &reply);
    true
}

/// `CheckVotes` (every 10 s from `CTSGameRules::Think`).
pub fn check_votes(now: f32) {
    let due = state::with(|g| {
        if g.votes.next_check <= now {
            g.votes.next_check = now + 10.0;
            true
        } else {
            false
        }
    });
    if !due {
        return;
    }
    let eng = engine();
    // `UTIL_FindEntityByClassname("player")` + `IsNetClient`: players who left still count (with
    // their vote cleared), and index 32 never does (`< 0x20`).
    let voters: Vec<usize> = super::slot_players().into_iter().map(|e| eng.index_of(e) as usize).filter(|&i| i < 0x20).collect();
    state::with(|g| g.votes.tally(voters));
}

/// The map a passed vote chose (`CHalfLifeMultiplay::Think` goes to intermission while set).
pub fn vote_passed() -> bool {
    state::with(|g| !g.votes.chosen.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voted(records: &[(usize, &str)]) -> Votes {
        let mut v = Votes::default();
        for &(i, m) in records {
            v.records[i] = m.to_owned();
        }
        v
    }

    #[test]
    fn parsing() {
        assert_eq!(parse(" map ts_awaken"), Some(VoteCmd::Map("ts_awaken".into())));
        assert_eq!(parse(" map   ts_lobby extra words"), Some(VoteCmd::Map("ts_lobby".into())));
        assert_eq!(parse("map"), Some(VoteCmd::Map(String::new())));
        assert_eq!(parse(" mapts_dojo"), Some(VoteCmd::Map("ts_dojo".into())));
        assert_eq!(parse(" stats"), Some(VoteCmd::Stats));
        assert_eq!(parse(" reset"), Some(VoteCmd::Reset));
        // TS looks for the keywords anywhere, reset first.
        assert_eq!(parse(" map resetmap"), Some(VoteCmd::Reset));
        assert_eq!(parse(" map ts_stats"), Some(VoteCmd::Stats));
        assert_eq!(parse(" kick bob"), None);
        assert_eq!(parse(&format!(" map {}", "x".repeat(100))), Some(VoteCmd::Map("x".repeat(63))));
    }

    #[test]
    fn minimum() {
        assert_eq!(min_votes(0), 1);
        assert_eq!(min_votes(1), 1);
        assert_eq!(min_votes(3), 1);
        assert_eq!(min_votes(4), 2);
        assert_eq!(min_votes(9), 4);
    }

    #[test]
    fn tally_passes_at_half() {
        let mut v = voted(&[(1, "ts_a"), (2, "ts_b"), (3, "ts_a")]);
        v.tally([1, 2, 3, 4, 5, 6]);
        assert_eq!(v.voters, 6);
        assert_eq!(v.tallies, [("ts_a".to_owned(), 2), ("ts_b".to_owned(), 1)]);
        assert!(v.chosen.is_empty(), "2 of 6 is below 3");
        v.records[4] = "ts_a".into();
        v.tally([1, 2, 3, 4, 5, 6]);
        assert_eq!(v.chosen, "ts_a");
        assert_eq!(v.override_map(), Some("ts_a"));
    }

    #[test]
    fn tally_ignores_absent_players_and_ties_go_first() {
        // Player 9 voted but isn't counted (left); index 32 never counts.
        let mut v = voted(&[(1, "ts_b"), (2, "ts_a"), (9, "ts_a")]);
        v.tally([1, 2, 32]);
        assert_eq!(v.voters, 2);
        assert_eq!(v.tallies, [("ts_b".to_owned(), 1), ("ts_a".to_owned(), 1)]);
        assert_eq!(v.chosen, "ts_b");
    }

    #[test]
    fn stats_and_reset() {
        let mut v = voted(&[(1, "ts_a")]);
        assert_eq!(v.stats(), None);
        v.tally([1, 2, 3, 4]);
        assert_eq!(v.stats().unwrap(), ["(1/2) votes for ts_a\n"]);
        v.reset(1);
        v.tally([1, 2, 3, 4]);
        assert_eq!(v.stats(), None);
        v.chosen = NEXTMAP.into();
        assert_eq!(v.override_map(), None);
    }
}
