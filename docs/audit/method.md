# Gameplay audit brief (read-only static audit)

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
