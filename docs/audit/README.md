# Gameplay audit against the original TS 3.0 and TS 2.1

A static audit (October 2026) of the port's gameplay logic against the original binaries, as
the [decomp](https://github.com/the-specialists-mod/the-specialists-decomp) exports them: TS 3.0's
`ts_i386.so`, Windows `mp.dll` and `client.dll`, and TS 2.1's `mp.dll` and `client.dll`. Each port
function was compared with the original it implements: constants, branches, order of operations,
rounding (f32, f64, x87), timers, edge cases and messages, for the 3.0 path and for the 2.1
plugin's overrides (and the 3.0 code 2.1 mode still runs). [method.md](method.md) is the brief
every area followed.

| Area | Coverage table |
|---|---|
| Movement and stunts (incl. the SDK movement code, instruction-level) | [movement.md](movement.md) |
| Weapons, tables and client prediction | [weapons.md](weapons.md) |
| Damage, melee and explosives | [damage.md](damage.md) |
| Rules, scoring, cash, commands, the gameplay switch | [rules.md](rules.md) |
| Power-ups, slow motion and map entities | [powerups-entities.md](powerups-entities.md) |
| Client HUD, input, effects and events | [client.md](client.md) |
| Bots | [bots.md](bots.md) |

Every verified mismatch was filed as an issue titled "Audit 3.0:" or "Audit 2.1:" (#118-#165),
or added to an existing issue (#62, #74, #80, #89, #105, #112, #128). Out of scope: purely
cosmetic code (sprite/beam rendering fields, ambient sound modulation, gibs, sentences) and the
HUD modules TS left as stock SDK (listed in client.md).

`scripts/` holds the comparison helpers the auditors wrote (normalised disassembly diffs, FPU
operation sequences, SDK and 3.0-vs-2.1 constant scans). They read the decomp's exports and
local disassembly dumps; paths are relative to a checkout next to this repo.
