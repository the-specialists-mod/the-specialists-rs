use super::table::{Classes, DEFS};
use super::*;
use std::any::TypeId;
use std::cell::RefCell;
use ts_plugin_api::ham::action::{CONTINUE, OVERRIDE, SUPERCEDE};

// ---- a fake game ---------------------------------------------------------------------------------

/// Edicts are fake pointers (never dereferenced): entity `i` is `(i + 1) * 0x100`.
fn ed(i: c_int) -> *mut edict_t {
    ((i as usize + 1) * 0x100) as *mut edict_t
}

/// Entities 0..=4 are in use; 3 is held by a dispatch further up the stack (`BUSY`), and 4 isn't
/// a player.
#[derive(Default)]
struct Fake {
    errors: RefCell<Vec<String>>,
}

impl Env for Fake {
    fn index_of(&self, e: *mut edict_t) -> c_int {
        if e.is_null() { -1 } else { (e as usize / 0x100 - 1) as c_int }
    }
    fn edict_of(&self, index: c_int) -> Option<*mut edict_t> {
        (0..=4).contains(&index).then(|| ed(index))
    }
    fn class_of(&self, _: *mut edict_t) -> c_int {
        7
    }
    fn error(&self, msg: &str) {
        self.errors.borrow_mut().push(msg.to_owned());
    }
}

impl Exec for Fake {
    fn entity(&self, index: c_int) -> Option<(*mut edict_t, TypeId)> {
        let ty = if index == 4 { TypeId::of::<u8>() } else { TypeId::of::<Player>() };
        self.edict_of(index).map(|e| (e, ty))
    }

    /// Every function is "TakeDamage": it returns true, and records its arguments.
    fn run(&self, d: &table::Def, e: *mut edict_t, call: &mut HamCall, _: &mut Pointers, hooks: bool) -> Option<()> {
        if self.index_of(e) == 3 {
            return None;
        }
        let mut a = (Ent(std::ptr::null_mut()), Ent(std::ptr::null_mut()), 0.0f32, 0i32);
        if d.ham == id::TAKEDAMAGE {
            a.take(&call.args, self);
        }
        let run = |a: &mut (Ent, Ent, f32, i32)| {
            log(format!("run {}", a.2));
            Some(true)
        };
        let r = if hooks { H.with(|h| h.call(self, d.ham, e, &mut a, &mut plain, run)) } else { run(&mut a) };
        call.ret = r?.put(self);
        Some(())
    }
}

/// A test hook's behaviour.
type Script = Box<dyn FnMut(&mut HamCall, bool) -> c_int>;

thread_local! {
    static H: Hooks = const { Hooks::new() };
    static FAKE: Fake = Fake::default();
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// What [`script_hook`] does. Taken out while it runs, so nested calls see `None`.
    static SCRIPT: RefCell<Option<Script>> = const { RefCell::new(None) };
}

fn log(s: String) {
    LOG.with(|l| l.borrow_mut().push(s));
}

fn logged() -> Vec<String> {
    LOG.with(|l| l.borrow_mut().drain(..).collect())
}

fn script(f: impl FnMut(&mut HamCall, bool) -> c_int + 'static) {
    SCRIPT.with(|s| *s.borrow_mut() = Some(Box::new(f)));
}

fn script_hook(call: &mut HamCall, post: bool) -> c_int {
    let Some(mut f) = SCRIPT.with(|s| s.borrow_mut().take()) else { return CONTINUE };
    let act = f(call, post);
    SCRIPT.with(|s| *s.borrow_mut() = Some(f));
    act
}

type Dmg = (Ent, Ent, f32, i32);

fn dmg_args() -> Dmg {
    (Ent(ed(1)), Ent(ed(2)), 25.0, 2)
}

/// A hooked "TakeDamage" on entity 1 that records what it got and returns true.
fn take_damage(h: &Hooks, env: &Fake) -> Option<bool> {
    h.call(env, id::TAKEDAMAGE, ed(1), &mut dmg_args(), &mut plain, |a| {
        log(format!("run inflictor {} attacker {} damage {} bits {}", env.index_of(a.0.0), env.index_of(a.1.0), a.2, a.3));
        Some(true)
    })
}

// ---- numbering and signatures --------------------------------------------------------------------

#[test]
fn ham_numbers_are_ham_const_incs() {
    assert_eq!(COUNT, 481, "HAM_LAST_ENTRY_DONT_USE_ME_LOL");
    let ids = [
        (id::SPAWN, "Ham_Spawn"),
        (id::KEYVALUE, "Ham_Keyvalue"),
        (id::OBJECTCAPS, "Ham_ObjectCaps"),
        (id::ACTIVATE, "Ham_Activate"),
        (id::CLASSIFY, "Ham_Classify"),
        (id::TRACEATTACK, "Ham_TraceAttack"),
        (id::TAKEDAMAGE, "Ham_TakeDamage"),
        (id::TAKEHEALTH, "Ham_TakeHealth"),
        (id::KILLED, "Ham_Killed"),
        (id::BLOODCOLOR, "Ham_BloodColor"),
        (id::OVERRIDERESET, "Ham_OverrideReset"),
        (id::ISALIVE, "Ham_IsAlive"),
        (id::ISBSPMODEL, "Ham_IsBSPModel"),
        (id::ISINWORLD, "Ham_IsInWorld"),
        (id::ISPLAYER, "Ham_IsPlayer"),
        (id::ISNETCLIENT, "Ham_IsNetClient"),
        (id::THINK, "Ham_Think"),
        (id::TOUCH, "Ham_Touch"),
        (id::USE, "Ham_Use"),
        (id::BLOCKED, "Ham_Blocked"),
        (id::RESPAWN, "Ham_Respawn"),
        (id::CENTER, "Ham_Center"),
        (id::EYEPOSITION, "Ham_EyePosition"),
        (id::PLAYER_JUMP, "Ham_Player_Jump"),
        (id::PLAYER_DUCK, "Ham_Player_Duck"),
        (id::PLAYER_PRETHINK, "Ham_Player_PreThink"),
        (id::PLAYER_POSTTHINK, "Ham_Player_PostThink"),
        (id::PLAYER_GETGUNPOSITION, "Ham_Player_GetGunPosition"),
        (id::TS_BREAKABLERESPAWN, "Ham_TS_BreakableRespawn"),
        (id::TS_CANUSEDTHROUGHWALLS, "Ham_TS_CanUsedThroughWalls"),
        (id::TS_GIVESLOWMUL, "Ham_TS_GiveSlowMul"),
        (id::TS_GOSLOW, "Ham_TS_GoSlow"),
        (id::TS_INSLOW, "Ham_TS_InSlow"),
        (id::TS_ISOBJECTIVE, "Ham_TS_IsObjective"),
        (id::TS_ENABLEOBJECTIVE, "Ham_TS_EnableObjective"),
        (id::TS_ONFREEENTPRIVATEDATA, "Ham_TS_OnFreeEntPrivateData"),
        (id::TS_SHOULDCOLLIDE, "Ham_TS_ShouldCollide"),
    ];
    for (n, name) in ids {
        assert_eq!(NAMES[n], name, "Ham number {n}");
    }
    // Anchors counted by hand in ham_const.inc: the TS block and the last entry.
    assert_eq!(NAMES[94], "Ham_TS_BreakableRespawn");
    assert_eq!(NAMES[96], "Ham_TS_RespawnWait");
    assert_eq!(NAMES[418], "Ham_TS_Weapon_AlternateAttack");
    assert_eq!(NAMES[480], "Ham_SC_Player_IsObserver");
    let mut sorted = NAMES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), COUNT, "no name twice");
    assert_eq!(table::DEFS.iter().map(|d| NAMES[d.ham]).filter(|n| !ids.iter().any(|(_, i)| i == n)).count(), 0);
}

/// The names and signatures against AMX Mod X's own files, when `ref/amxmodx` is checked out next
/// to the repository (it isn't in CI).
#[test]
fn names_and_signatures_match_amxmodx_sources() {
    let Some(root) = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|p| p.join("ref/amxmodx")).find(|p| p.is_dir())
    else {
        eprintln!("ref/amxmodx not found: skipped");
        return;
    };
    let inc = std::fs::read_to_string(root.join("plugins/include/ham_const.inc")).unwrap();
    let body = &inc[inc.find("enum Ham").unwrap()..inc.find("HAM_LAST_ENTRY_DONT_USE_ME_LOL").unwrap()];
    let names: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("Ham_"))
        .map(|l| l.trim_end_matches(',').split([' ', ',', '=']).next().unwrap())
        .collect();
    assert_eq!(names, NAMES);
    let cpp = std::fs::read_to_string(root.join("modules/hamsandwich/hook_native.cpp")).unwrap();
    let list = &cpp[cpp.find("hook_t hooklist[]").unwrap()..];
    let sigs: Vec<String> = list
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("{ V(") || l.starts_with("{ V_REMOVED("))
        .map(|l| {
            if l.starts_with("{ V_REMOVED(") {
                return "Removed".to_owned();
            }
            let inner = &l[l.find(',').unwrap() + 1..l.rfind(')').unwrap()];
            inner.trim().to_owned()
        })
        .collect();
    assert_eq!(sigs, SIGS);
}

/// hamsandwich's signature (`Int_Entvar_Entvar_Float_Int`) as kinds: return, then arguments.
fn parse_sig(sig: &str) -> (Vec<c_int>, c_int) {
    let k = |t: &str| match t {
        "Void" => kind::VOID,
        "Int" | "Bool" | "Short" => kind::INT,
        "Float" => kind::FLOAT,
        "Entvar" | "Cbase" | "Edict" => kind::ENTITY,
        "Vector" => kind::VECTOR,
        "pVector" => kind::VECTOR_OUT,
        "pFloat" => kind::FLOAT_OUT,
        "Trace" => kind::TRACE,
        "Str" => kind::STRING,
        "ItemInfo" => kind::ITEMINFO,
        t => panic!("unknown signature type {t}"),
    };
    let mut parts = sig.split('_');
    let ret = k(parts.next().unwrap());
    let args: Vec<c_int> = parts.map(k).filter(|&a| a != kind::VOID).collect();
    (args, ret)
}

#[test]
fn every_function_takes_hamsandwichs_arguments() {
    for d in DEFS {
        let (args, ret) = (d.sig)();
        assert_eq!((args.to_vec(), ret), parse_sig(SIGS[d.ham]), "{} ({})", NAMES[d.ham], SIGS[d.ham]);
        assert!(!matches!((&d.fires, &d.exec), (Classes::None, Classes::None)), "{}", NAMES[d.ham]);
    }
    assert_eq!(parse_sig("Int_Entvar_Entvar_Float_Int"), (vec![kind::ENTITY, kind::ENTITY, kind::FLOAT, kind::INT], kind::INT));
    let mut hams: Vec<usize> = DEFS.iter().map(|d| d.ham).collect();
    hams.dedup();
    assert_eq!(hams.len(), DEFS.len(), "one entry per function");
}

#[test]
fn classes_say_where_hooks_fire() {
    let player = TypeId::of::<Player>();
    let other = TypeId::of::<crate::entities::stock::breakable::FuncBreakable>();
    let kit = TypeId::of::<crate::entities::stock::items::HealthKit>();
    let support = |ham: usize, ty| table::def(ham as c_int).unwrap().support(ty);
    assert_eq!(support(id::SPAWN, other), 1);
    assert_eq!(support(id::KILLED, player), 1);
    assert_eq!(support(id::KILLED, other), 0, "CBaseEntity::Killed runs on request only");
    assert_eq!(support(id::PLAYER_JUMP, player), 1);
    assert_eq!(support(id::PLAYER_JUMP, other), UNSUPPORTED);
    assert_eq!(support(id::ISALIVE, player), 0, "a getter is execute-only");
    assert_eq!(support(id::RESPAWN, kit), 1);
    assert_eq!(support(id::RESPAWN, player), UNSUPPORTED);
    assert_eq!(support(id::TS_BREAKABLERESPAWN, other), 1);
    assert_eq!(support(id::TS_BREAKABLERESPAWN, player), 0, "CBaseEntity's is empty");
    assert!(table::def(1).is_none(), "Precache: never a separate call in the port");
    assert!(table::def(-1).is_none() && table::def(COUNT as c_int).is_none());
}

#[test]
fn classes_are_one_id_per_rust_type() {
    struct A;
    struct B;
    struct C;
    let mut r = classes::Registry::new(vec![("a", TypeId::of::<A>()), ("b", TypeId::of::<B>()), ("a2", TypeId::of::<A>())]);
    assert_eq!(r.class_id(c"a"), Some(1));
    assert_eq!(r.class_id(c"a2"), Some(1), "names of one type share its class");
    assert_eq!(r.class_id(c"b"), Some(2));
    assert_eq!(r.class_id(c"A"), None, "case-sensitive, as the export lookup");
    assert_eq!(r.id_of(TypeId::of::<C>()), 3, "an unlinked type gets the next id");
    assert_eq!(r.id_of(TypeId::of::<C>()), 3, "and keeps it");
    assert_eq!(r.type_of(1), Some(TypeId::of::<A>()));
    assert_eq!(r.type_of(0), None);
    assert_eq!(r.type_of(4), None);

    // The game's links: every name has a class, doors share theirs, the player has its own.
    let game = classes::Registry::new(classes::linked());
    let id = |n: &CStr| game.class_id(n).unwrap_or_else(|| panic!("{n:?}"));
    assert_eq!(id(c"func_door"), id(c"func_water"));
    assert_eq!(id(c"weapon_shotgun"), id(c"ts_groundweapon"));
    assert_ne!(id(c"player"), id(c"func_door"));
    assert_eq!(game.type_of(id(c"player")), Some(TypeId::of::<Player>()));
    assert_eq!(id(c"worldspawn"), 1, "ids follow link order");
}

// ---- marshalling ----------------------------------------------------------------------------------

#[test]
fn arguments_marshal_by_kind() {
    let env = Fake::default();
    let mut i = 5i32;
    assert_eq!(i.put(&env), HamArg { kind: kind::INT, i: 5, ..HamArg::default() });
    i.take(&HamArg { i: -9, ..HamArg::default() }, &env);
    assert_eq!(i, -9);

    let mut f = 2.5f32;
    assert_eq!(f.put(&env).f, 2.5);
    f.take(&HamArg { f: f32::NAN, ..HamArg::default() }, &env);
    assert_eq!(f, 2.5, "NaN is ignored");
    f.take(&HamArg { f: 7.0, ..HamArg::default() }, &env);
    assert_eq!(f, 7.0);

    let mut v = [1.0f32, 2.0, 3.0];
    assert_eq!(v.put(&env).kind, kind::VECTOR);
    v.take(&HamArg { v: [0.0, f32::INFINITY, 0.0], ..HamArg::default() }, &env);
    assert_eq!(v, [1.0, 2.0, 3.0]);
    v.take(&HamArg { v: [4.0, 5.0, 6.0], ..HamArg::default() }, &env);
    assert_eq!(v, [4.0, 5.0, 6.0]);

    let mut e = Ent(std::ptr::null_mut());
    assert_eq!(e.put(&env), HamArg { kind: kind::ENTITY, i: -1, ..HamArg::default() }, "NULL is -1");
    e.take(&HamArg { i: 2, ..HamArg::default() }, &env);
    assert_eq!(e, Ent(ed(2)));
    e.take(&HamArg { i: 99, ..HamArg::default() }, &env);
    assert_eq!(e, Ent(ed(2)), "an entity not in use is ignored");
    assert_eq!(env.errors.borrow().len(), 1);
    e.take(&HamArg { i: -1, ..HamArg::default() }, &env);
    assert_eq!(e, Ent(std::ptr::null_mut()));

    let mut u = UseType::Toggle;
    assert_eq!(u.put(&env).i, 3);
    u.take(&HamArg { i: 2, ..HamArg::default() }, &env);
    assert_eq!(u, UseType::Set);
    u.take(&HamArg { i: 7, ..HamArg::default() }, &env);
    assert_eq!(u, UseType::Set, "no such use type");

    let mut tr = trace_of(ed(1), &DamageInfo { hitgroup: 2, pos: [1.0, 2.0, 3.0], ..DamageInfo::new(ed(2), ed(2), 10.0, 0) });
    let a = tr.put(&env);
    assert_eq!((a.kind, a.p), (kind::TRACE, std::ptr::from_mut(&mut tr).cast::<c_void>().cast_const()));
    assert_eq!((tr.iHitgroup, tr.vecEndPos, tr.pHit), (2, [1.0, 2.0, 3.0], ed(1)));

    let mut kv = Kvd(KeyValueData {
        szClassName: std::ptr::null_mut(),
        szKeyName: c"k".as_ptr().cast_mut(),
        szValue: c"v".as_ptr().cast_mut(),
        fHandled: 0,
    });
    let a = kv.put(&env);
    assert_eq!((a.kind, a.p), (kind::INT, std::ptr::from_mut(&mut kv.0).cast::<c_void>().cast_const()));
    assert_eq!(a.i, a.p as usize as c_int, "hamsandwich's int is the pointer");

    let mut args = [HamArg::default(); ts_plugin_api::ham::MAX_ARGS];
    let mut d = dmg_args();
    assert_eq!(Dmg::KINDS, [kind::ENTITY, kind::ENTITY, kind::FLOAT, kind::INT]);
    d.put(&mut args, &env);
    assert_eq!(args.map(|a| (a.kind, a.i))[..5], [(kind::ENTITY, 1), (kind::ENTITY, 2), (kind::FLOAT, 0), (kind::INT, 2), (kind::VOID, 0)]);
    assert_eq!(args[2].f, 25.0);
}

#[test]
fn killed_maps_gib_to_damage_bits() {
    use crate::damage::bits::{ALWAYSGIB, BULLET, NEVERGIB};
    assert_eq!([gib_of(BULLET), gib_of(NEVERGIB), gib_of(ALWAYSGIB | NEVERGIB)], [0, 1, 2]);
    let d = DamageInfo::new(ed(1), ed(1), 10.0, BULLET | NEVERGIB);
    let k = killed_info(&d, ed(2), 2);
    assert_eq!((k.attacker, k.bits, k.inflictor), (ed(2), BULLET | ALWAYSGIB, ed(1)));
    assert_eq!(killed_info(&d, ed(1), 0).bits, BULLET);
}

// ---- the hook calls ---------------------------------------------------------------------------------

#[test]
fn only_watched_functions_with_a_hook_are_armed() {
    let h = Hooks::new();
    let env = Fake::default();
    h.watch(id::TAKEDAMAGE, true);
    assert!(!h.armed(id::TAKEDAMAGE), "no hook installed");
    h.set_hook(Some(script_hook));
    assert!(h.armed(id::TAKEDAMAGE));
    assert!(!h.armed(id::TOUCH) && !h.armed(id::TAKEDAMAGE + 64), "only that bit");
    h.watch(id::TS_SHOULDCOLLIDE, true);
    assert!(h.armed(id::TS_SHOULDCOLLIDE) && h.armed(id::TAKEDAMAGE));
    h.watch(id::TAKEDAMAGE, false);
    assert!(!h.armed(id::TAKEDAMAGE) && h.armed(id::TS_SHOULDCOLLIDE));
    h.set_hook(None);
    assert!(!h.armed(id::TS_SHOULDCOLLIDE));
    // No hook: `call` runs the function alone.
    script(|_, _| panic!("no hook installed"));
    assert_eq!(take_damage(&h, &env), Some(true));
    assert_eq!(logged(), ["run inflictor 1 attacker 2 damage 25 bits 2"]);
}

#[test]
fn pre_hook_changes_reach_the_function_and_the_post_hook() {
    let (h, env) = (Hooks::new(), Fake::default());
    h.set_hook(Some(script_hook));
    script(|c, post| {
        assert_eq!((c.ham, c.this, c.class, c.argc), (id::TAKEDAMAGE as c_int, 1, 7, 4));
        assert_eq!((c.ret.kind, c.orig_ret.kind), (kind::INT, kind::INT));
        if !post {
            assert_eq!((c.args[0].i, c.args[1].i, c.args[2].f, c.args[3].i), (1, 2, 25.0, 2));
            c.args[1].i = 4;
            c.args[2].f = 50.0;
            c.args[3].i = 8;
            c.argc = 0; // scribbles on what isn't an argument are undone
            c.this = 3;
        } else {
            log(format!("post attacker {} damage {} orig {} ret {}", c.args[1].i, c.args[2].f, c.orig_ret.i, c.ret.i));
        }
        CONTINUE
    });
    assert_eq!(take_damage(&h, &env), Some(true));
    assert_eq!(logged(), ["run inflictor 1 attacker 4 damage 50 bits 8", "post attacker 4 damage 50 orig 1 ret 1"]);
    assert_eq!(h.depth(), 0);
}

#[test]
fn supercede_skips_the_function_but_not_the_post_hook() {
    let (h, env) = (Hooks::new(), Fake::default());
    h.set_hook(Some(script_hook));
    for post_action in [CONTINUE, OVERRIDE] {
        script(move |c, post| {
            if !post {
                c.ret.i = 1;
                return SUPERCEDE;
            }
            log(format!("post orig {:?} ret {}", (c.orig_ret.kind, c.orig_ret.i), c.ret.i));
            post_action
        });
        let r = take_damage(&h, &env);
        assert_eq!(logged(), [format!("post orig {:?} ret 1", (kind::INT, 0))], "no run; a zero orig_ret");
        // The post call decides: CONTINUE returns orig_ret (0), OVERRIDE the pre hook's ret.
        assert_eq!(r, Some(post_action == OVERRIDE));
    }
}

#[test]
fn the_post_calls_action_decides_the_return() {
    let (h, env) = (Hooks::new(), Fake::default());
    h.set_hook(Some(script_hook));
    for post_action in [CONTINUE, OVERRIDE] {
        script(move |c, post| {
            if !post {
                c.ret.i = 0;
                return OVERRIDE;
            }
            log(format!("post orig {} ret {}", c.orig_ret.i, c.ret.i));
            post_action
        });
        let r = take_damage(&h, &env);
        assert_eq!(logged()[1], "post orig 1 ret 0", "the pre hook's ret is kept for the post call");
        assert_eq!(r, Some(post_action == CONTINUE), "only the post call's OVERRIDE uses it");
    }
    // A post OVERRIDE of a void function changes nothing; of a vector one, gives its `v`.
    script(|c, post| {
        if post {
            c.ret.v = [9.0; 3];
            return OVERRIDE;
        }
        CONTINUE
    });
    assert_eq!(h.call(&env, id::CENTER, ed(1), &mut (), &mut plain, |_| Some([1.0f32; 3])), Some([9.0; 3]));
    assert_eq!(h.call(&env, id::THINK, ed(1), &mut (), &mut plain, |_| Some(())), Some(()));
    // A function that couldn't reach its object returns `None`, unless the post hook overrides.
    script(|_, _| CONTINUE);
    assert_eq!(h.call(&env, id::TAKEDAMAGE, ed(1), &mut dmg_args(), &mut plain, |_| None::<bool>), None);
}

#[test]
fn hooks_run_through_reach() {
    let (h, env) = (Hooks::new(), Fake::default());
    h.set_hook(Some(script_hook));
    script(|_, post| {
        log(format!("hook {post}"));
        CONTINUE
    });
    let mut reach = |f: &mut dyn FnMut()| {
        log("lend".into());
        f();
        log("back".into());
    };
    h.call(&env, id::PLAYER_JUMP, ed(1), &mut (), &mut reach, |_| {
        log("run".into());
        Some(())
    });
    assert_eq!(logged(), ["lend", "hook false", "back", "run", "lend", "hook true", "back"]);
}

/// `ExecuteHamB(Ham_Spawn, id)` from a Spawn hook: each level runs, the nesting stops at
/// `MAX_DEPTH` with one error, and the stack unwinds.
fn respawning_hook(c: &mut HamCall, post: bool) -> c_int {
    if !post {
        log(format!("pre {}", H.with(|h| h.depth())));
        FAKE.with(|env| H.with(|h| h.call(env, c.ham as usize, ed(c.this), &mut (), &mut plain, |_| Some(()))));
    }
    CONTINUE
}

#[test]
fn nested_hooks_stop_at_the_limit() {
    H.with(|h| h.set_hook(Some(respawning_hook)));
    let runs = std::cell::Cell::new(0);
    FAKE.with(|env| {
        H.with(|h| {
            h.call(env, id::SPAWN, ed(1), &mut (), &mut plain, |_| {
                runs.set(runs.get() + 1);
                Some(())
            })
        })
    });
    let pre: Vec<String> = (1..=MAX_DEPTH).map(|d| format!("pre {d}")).collect();
    assert_eq!(logged(), pre, "one hook call per level, each one deeper");
    assert_eq!(runs.get(), 1, "the outer function ran once");
    assert_eq!(H.with(|h| h.depth()), 0);
    FAKE.with(|env| assert_eq!(env.errors.borrow().len(), 1, "{:?}", env.errors.borrow()));
}

// ---- ham_execute ------------------------------------------------------------------------------------

fn dmg_call(this: c_int) -> HamCall {
    let mut c = HamCall { ham: id::TAKEDAMAGE as c_int, this, argc: 4, ..HamCall::default() };
    FAKE.with(|env| dmg_args().put(&mut c.args, env));
    c
}

fn exec(c: &mut HamCall, hooks: bool) -> c_int {
    FAKE.with(|env| H.with(|h| execute_in(env, h, c, &mut Pointers::default(), hooks)))
}

#[test]
fn execute_checks_in_order() {
    let mut c = dmg_call(9);
    assert_eq!(exec(&mut c, false), NOT_PLAYER);
    c.this = 1;
    c.ham = 1; // Precache
    assert_eq!(exec(&mut c, false), UNSUPPORTED);
    c.ham = COUNT as c_int;
    assert_eq!(exec(&mut c, false), UNSUPPORTED);
    let mut j = HamCall { ham: id::PLAYER_JUMP as c_int, this: 4, ..HamCall::default() };
    assert_eq!(exec(&mut j, false), UNSUPPORTED, "not a player");
    j.this = 1;
    assert_eq!(exec(&mut j, false), OK);

    let mut c = dmg_call(1);
    c.argc = 3;
    assert_eq!(exec(&mut c, false), INVALID, "argc");
    let mut c = dmg_call(1);
    c.args[2].kind = kind::INT;
    assert_eq!(exec(&mut c, false), INVALID, "kind");
    let mut c = dmg_call(1);
    c.args[1].i = 77;
    assert_eq!(exec(&mut c, false), INVALID, "an entity not in use");
    let mut c = dmg_call(1);
    c.args[1].i = -1;
    assert_eq!(exec(&mut c, false), OK, "-1 is NULL");
    assert_eq!(c.ret, HamArg { kind: kind::INT, i: 1, ..HamArg::default() });

    let mut c = dmg_call(3);
    assert_eq!(exec(&mut c, false), BUSY, "held further up the stack");
    assert_eq!(logged(), ["run 0", "run 25"]);

    let mut tr = HamCall { ham: id::TRACEATTACK as c_int, this: 1, argc: 5, ..HamCall::default() };
    for (a, k) in tr.args.iter_mut().zip([kind::ENTITY, kind::FLOAT, kind::VECTOR, kind::TRACE, kind::INT]) {
        a.kind = k;
    }
    assert_eq!(exec(&mut tr, false), INVALID, "a TRACE argument without its TraceResult");
    let mut kv = HamCall { ham: id::KEYVALUE as c_int, this: 1, argc: 1, ..HamCall::default() };
    kv.args[0].kind = kind::INT;
    assert_eq!(exec(&mut kv, false), INVALID, "Keyvalue without its KeyValueData");
}

/// A TakeDamage hook that runs `ExecuteHamB(Ham_TakeDamage)` on its own entity, and records what
/// that returned.
fn re_executing_hook(c: &mut HamCall, post: bool) -> c_int {
    if !post {
        let mut nested = dmg_call(c.this);
        nested.args[2].f = c.args[2].f + 1.0;
        let r = exec(&mut nested, true);
        log(format!("nested {r}"));
    }
    CONTINUE
}

#[test]
fn execute_from_a_hook_reenters_up_to_the_limit() {
    H.with(|h| {
        h.set_hook(Some(re_executing_hook));
        h.watch(id::TAKEDAMAGE, true);
    });
    let mut c = dmg_call(1);
    assert_eq!(exec(&mut c, true), OK);
    let got = logged();
    // Hooks at depths 1..=8 each execute once: the one at the limit is refused, the rest ran
    // (innermost first), and every level's function ran.
    let mut want = vec!["nested -4".to_owned()];
    for d in (1..MAX_DEPTH).rev() {
        want.push(format!("run {}", 25 + d));
        want.push("nested 0".to_owned());
    }
    want.push("run 25".to_owned());
    assert_eq!(got, want);
    assert_eq!(H.with(|h| h.depth()), 0);

    // ExecuteHam (no hooks) from the hook: only that call skips them.
    H.with(|h| h.set_hook(Some(script_hook)));
    script(|c, post| {
        if !post && c.args[2].f == 25.0 {
            let mut nested = dmg_call(c.this);
            nested.args[2].f = 26.0;
            log(format!("nested {}", exec(&mut nested, false)));
        }
        log(format!("hook {post} {}", c.args[2].f));
        CONTINUE
    });
    assert_eq!(exec(&mut dmg_call(1), true), OK);
    assert_eq!(logged(), ["run 26", "nested 0", "hook false 25", "run 25", "hook true 25"]);
}
