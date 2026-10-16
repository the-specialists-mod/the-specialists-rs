# Gameplay audit briefs (read-only static audit)

Round 1 brief:

Goal: validate the Rust port's gameplay logic against the ORIGINAL logic, for BOTH TS 3.0 and TS 2.1.
- Port: this repo (main). 3.0 logic lives in crates/shared, crates/server, crates/client; the 2.1 ruleset is the plugin in crates/ts21 (shared/server/client), plugged in through hooks whose defaults are the 3.0 behaviour (docs/implementation/gameplay-plugins.md, docs/implementation/ts21.md). Bots: crates/server/src/bots (+ bots/rules.rs hooks; 2.1 in crates/ts21/server/src/bots).
- Original 3.0: the decomp repo (the-specialists-decomp) (Ghidra exports of ts_i386.so, Windows mp.dll and client.dll; docs/). 
- Original 2.1: the decomp repo (the-specialists-decomp)/ts21/ (server-win = mp.dll, client = client.dll exports) and ~/dev/hl/ts30-decomp/docs/ts21/ reports (reports can be wrong: the code is the truth; the port's ts21.md lists known corrections).
- Repo docs/ describe the port's design; AGENTS.md has the rules (match TS exactly, including oddities; only deliberate deviations are the ones documented, plus guards against crashes/hangs).

Method: go function by function through your area. For every port function that implements original logic, find the original function (address in the port's doc comments, or by name/strings in the decomp), and compare: constants, branch conditions, order of operations, rounding/types (f32 vs f64 vs x87, truncation vs rounding), timers, edge cases (NaN, negative, zero, overflow), side effects and messages sent. Do this for the 3.0 path AND, where the 2.1 plugin overrides it, the 2.1 path (and check that hooks the 2.1 plugin does NOT override really are identical in 2.1 — that's where silent gaps hide). Use the disassembly when the decompile is ambiguous. Do not trust the port's comments; check them.

Do NOT modify code in the port repo (read-only audit; you may write scratch files under /tmp/audit/).

Output:
1. For every VERIFIED mismatch (you checked both sides and are confident): file a GitHub issue on the-specialists-mod/the-specialistsmod-rs. First search existing issues (`gh issue list -R the-specialists-mod/the-specialistsmod-rs --state all --search "<keywords>" --limit 50`) to avoid duplicates (comment on an existing one instead if it covers it). Combine closely related mismatches in one issue. Title prefix "Audit 3.0:" or "Audit 2.1:". Label `bug`. Body: what the original does (address + snippet), what the port does (file:line), the observable effect, and the fix. Not-a-bug documented deviations: don't file.
2. Write /tmp/audit/<area>.md: coverage table (port function → original function(s) for 3.0 and 2.1 → status: match / mismatch #issue / unverifiable + why), and a list of anything you couldn't verify.
3. Final reply (under ~300 words): counts (functions checked, matches, issues filed with numbers), the most significant findings, and coverage gaps.

No sub-agents (API rate limits). If you get rate-limited you will be resumed; keep notes in /tmp/audit/<area>.md as you go so progress isn't lost.

## Round 2 (after #200)

The audit brief:


Goal: a full validation of the Rust port's TS 3.0 module and TS 2.1 module against the ORIGINAL code, for YOUR AREA (below).

- Port: the main checkout, this repo (branch main, commit 661d918). Read AGENTS.md, docs/implementation/gameplay-plugins.md and docs/implementation/ts21.md first.
- Layout since #200: the core crates (crates/shared, crates/server, crates/client, crates/gameplay) hold ruleset-neutral engine/TS mechanics that both rulesets use; TS 3.0 is a plugin in crates/ts30 (shared hooks: pm/gun/melee/recoil/tables), crates/ts30/server (ServerRules, NativeBotRules: rules, scoring, players, observer, items, entities, powerup, stunt, damage, melee, explode, bots), crates/ts30/client (ClientRules: view, events, fx, spectator); TS 2.1 is the plugin in crates/ts21/{src/shared,server,client}. Hook traits (shared/src/gameplay, server/src/gameplay/server.rs, server/src/bots/rules.rs, client/src/gameplay/client.rs) have no default bodies; each ruleset implements every hook.
- Original 3.0: the decomp checkout (../ts30-decomp) (server/ = ts_i386.so export, server-win/ = Windows mp.dll (only build with the bot AI), client/ = client.dll; docs/). Server Ghidra address = objdump address + 0x10000. mp.dll offsets: CBasePlayer = .so - 0x14, CTSGun = .so - 0x10.
- Original 2.1: the decomp checkout (../ts30-decomp)/ts21/{server-win (mp.dll), client (client.dll)} and docs/ts21/ reports (reports can be wrong: the code is the truth; the port's ts21.md lists known corrections).
- The previous audit (round 1, early Oct 2026) is in the port's docs/audit/ (<area>.md coverage tables, method.md, scripts/). Its issues were #118-#165 plus some existing ones; most are fixed. Use its tables as a checklist, but RE-CHECK: code moved in #200 and many fixes landed since. Also look at what round 1 marked "unverifiable" and at functions it didn't list.
- Rules: match TS exactly, including oddities; the only deliberate deviations are the documented ones (AGENTS.md, docs) plus guards against crashes/hangs.

Method: go function by function through your area. For every port function that implements original logic (in the core AND in the ts30 and ts21 plugins), find the original function (address in the port's doc comments, or by name/strings in the decomp) and compare: constants, branch conditions, order of operations, rounding/types (f32 vs f64 vs x87, truncation vs rounding), timers, edge cases (NaN, negative, zero, overflow), side effects and messages sent. Do this for the 3.0 path AND the 2.1 path. Pay special attention to core code that BOTH rulesets run with no hook (the 2.1 plugin can't override it): check that 2.1's original really does the same there — that's where silent gaps hide. Use the disassembly when the decompile is ambiguous. Do not trust the port's comments; check them.

Known and being fixed right now, don't file: #198 (2.1 mode fires no melee_attack/kill_scored plugin callbacks), #201 (2.1 server PreThink weapon step uses 3.0's double-fire-rate step). Known open, #200 phase 2 (3.0 code core still runs for every ruleset with no hook: client commands, parts of death/spawn/observer handling, team changes, explosion pitch `events::fire_pitch`, `TextureLimits::LARGE`, stunt-timer default bodies, RoundTdm/Objective branches in core): if you VERIFY that 2.1's original differs on one of those paths, file it as a concrete issue (and mention #200).

Do NOT modify code in the port repo (read-only; scratch files go under /tmp/audit2/). Don't run in-engine xtask commands. `cargo test` is fine if you need it, but don't build in the main checkout's target dir concurrently with others — prefer reading code.

Output:
1. For every VERIFIED mismatch (you checked both sides and are confident): file a GitHub issue on the-specialists-mod/the-specialistsmod-rs. First search existing issues (`gh issue list -R the-specialists-mod/the-specialistsmod-rs --state all --search "<keywords>" --limit 50`) to avoid duplicates (comment on an open one instead if it covers it; if a closed one regressed, reopen-worthy: file a new issue referencing it). Combine closely related mismatches in one issue. Title prefix "Audit 3.0:" or "Audit 2.1:". Label `bug`. Body: what the original does (address + snippet), what the port does (file:line), the observable effect, and the fix. End the body with "🤖 Generated with [Claude Code](https://claude.com/claude-code)". Documented deviations: don't file.
2. Write /tmp/audit2/<area>.md: a coverage table in the same format as docs/audit/<area>.md (port function → original function(s) for 3.0 and 2.1 → status: match / mismatch #issue / unverifiable + why), updated for the new file locations, plus a list of anything you couldn't verify. This will replace docs/audit/<area>.md.
3. Final reply (under ~300 words): counts (functions checked, matches, issues filed with numbers), the most significant findings, coverage gaps.

No sub-agents. Keep notes in /tmp/audit2/<area>.md as you go so progress isn't lost if you are interrupted.

The table-validation brief (the second, independent pass):


An auditor just compared one area of the TS Rust port against the original TS 3.0 and TS 2.1 code and wrote a coverage table at /tmp/audit2/<area>.md (its brief: /tmp/audit2-brief.md — read it for context, paths and layout). Your job: independently check that table before it replaces docs/audit/<area>.md in the port repo (this repo, main). Originals: the decomp checkout (../ts30-decomp) (3.0: server/, server-win/, client/; 2.1: ts21/server-win, ts21/client).

1. Completeness: enumerate the port functions in this area (core crates AND crates/ts30 AND crates/ts21) that implement original logic (grep for fn items, decomp-address doc comments, hook impls). List every one with no row in the table (or covered only vaguely). For the most important missing ones (gameplay-relevant), do the comparison yourself and add rows.
2. "match" rows: re-check a sample yourself against the decompile/disassembly — at least 15 rows or 25%, whichever is more, biased to the risky ones (rounding/x87 vs f32, timers, branch order, edge cases, core code that 2.1 runs without a hook, and rows that cite the wrong original address). Record each re-check's verdict.
3. "unverifiable" rows: is the reason real? Verify the row if you can.
4. "mismatch" rows: the cited issue exists (`gh issue view N -R the-specialists-mod/the-specialistsmod-rs`) and describes the same thing; port file:line references are right.
5. Addresses/paths: spot-check that cited original addresses and port paths exist.

For any wrongly marked row that hides a real mismatch: verify it, then file a GitHub issue (search duplicates first; title prefix "Audit 3.0:"/"Audit 2.1:", label bug, body with original address+snippet, port file:line, effect, fix, ending "🤖 Generated with [Claude Code](https://claude.com/claude-code)").

Write the corrected table to /tmp/audit2/<area>.md (edit in place: fix rows, add missing rows, mark re-checked rows, add a short "Validation" section at the end: what was re-checked, results, remaining gaps). Don't modify the port repo. No sub-agents. Don't run in-engine commands.

Final reply (under ~250 words): rows before/after, missing functions found, sample re-checked and how many were wrong, issues filed, remaining gaps.
