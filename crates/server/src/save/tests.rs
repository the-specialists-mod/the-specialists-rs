//! The save chains against TS's tables, and the global state's bytes.

use super::*;
use goldsrc::ext::saverestore::testing::{Buf, FakeHost};

/// Linked classnames whose TS chain the port doesn't map yet (their `save_fields` is the
/// `CBaseEntity` default). Remove a class from here when its mapping is done.
const REMAINING: &[&str] = &[];

/// Linked classnames whose TS chain is `CBaseEntity`'s alone and whose `BASE` member pointers
/// (and entvars) the port doesn't map yet. Remove a class when done.
const REMAINING_BASE: &[&str] = &[];

fn all_linked() -> Vec<world::Linked> {
    let mut v = crate::entities::stock::linked_entities();
    v.extend(crate::entities::ts::linked_entities());
    v.extend(crate::entities::stock::blood::linked_entities());
    v.extend(crate::entities::stock::monsters::linked_entities());
    v.extend(crate::entities::stock::xen::linked_entities());
    v.extend(crate::player::linked_entities());
    v.extend(crate::weapons::linked_entities());
    v
}

/// Runs `save_fields` in probe mode on a fresh object over a scratch edict.
fn probe(ctor: fn() -> Box<dyn world::Entity>) -> (Vec<(&'static CStr, Vec<FieldType>)>, Vec<String>) {
    let mut ent = ctor();
    let mut ed = edict_t::default();
    let mut cx = Cx { edict: &mut ed };
    let mut f = Fields::probe();
    ent.save_fields(&mut cx, &mut f);
    f.finish();
    let (log, errors) = f.probe_log();
    (log.to_vec(), errors.to_vec())
}

/// How `probe`'s walk differs from TS's chain for `ts_class`, if it does.
fn chain_mismatch(ts_class: &str, log: &[(&'static CStr, Vec<FieldType>)], errors: &[String]) -> Option<String> {
    let chain = tables::chain(ts_class)?;
    let want: Vec<&CStr> = chain.iter().map(|t| t.name).collect();
    let got: Vec<&CStr> = log.iter().map(|(n, _)| *n).collect();
    if want != got {
        return Some(format!("tables {got:?}, TS writes {want:?}"));
    }
    for (t, (_, types)) in chain.iter().zip(log) {
        let tt: Vec<FieldType> = t.fields.iter().map(|d| d.ty).collect();
        if &tt != types {
            return Some(format!("{:?}: field types {types:?}, TS has {tt:?}", t.name));
        }
    }
    (!errors.is_empty()).then(|| errors.join("; "))
}

#[test]
fn every_linked_class_walks_its_ts_chain() {
    let mut bad = Vec::new();
    for (classname, ctor) in all_linked() {
        let Some(ts_class) = tables::class_of(classname.as_bytes()) else {
            bad.push(format!("{classname}: not a TS classname"));
            continue;
        };
        if tables::chain(ts_class).is_none() {
            bad.push(format!("{classname}: no chain for {ts_class}"));
            continue;
        }
        let (log, errors) = probe(ctor);
        let mismatch = chain_mismatch(ts_class, &log, &errors);
        let remaining = REMAINING.contains(&classname);
        match (mismatch, remaining) {
            (Some(m), false) => bad.push(format!("{classname} ({ts_class}): {m}")),
            (None, true) if tables::chain(ts_class).unwrap().len() > 2 => {
                bad.push(format!("{classname} ({ts_class}) matches TS: take it off REMAINING"))
            }
            _ => {}
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

#[test]
fn chains_follow_ts() {
    let names = |c: &str| tables::chain(c).unwrap().iter().map(|t| t.name.to_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(names("CBaseEntity"), ["ENTVARS", "BASE"]);
    assert_eq!(names("CRotDoor"), ["ENTVARS", "BASE", "CBaseDelay", "CBaseAnimating", "CBaseToggle", "CBaseDoor"]);
    assert_eq!(names("CBasePlayer"), ["ENTVARS", "BASE", "CBaseDelay", "CBaseAnimating", "CBaseToggle", "PLAYER"]);
    // TS's own parents, which differ from the SDK's.
    assert_eq!(names("CFuncTrackTrain"), ["ENTVARS", "BASE", "CFuncTrackTrain"]);
    assert_eq!(names("CBreakable"), ["ENTVARS", "BASE", "CBreakable"]);
    assert_eq!(names("CTSPowerUp"), ["ENTVARS", "BASE", "CBaseDelay", "CBaseAnimating", "CTSPowerUp"]);
    assert_eq!(names("CTSGun"), ["ENTVARS", "BASE", "CBaseDelay", "CBaseAnimating", "CBasePlayerItem", "CBasePlayerWeapon"]);
    assert_eq!(tables::class_of(b"func_door_rotating"), Some("CRotDoor"));
    assert_eq!(tables::ENTVARS.fields.len(), 86);
    assert_eq!(tables::CBasePlayer.fields.len(), 36);
}

#[test]
fn global_state_bytes() {
    use crate::entities::stock::base::{GlobalState, global_add, global_clear, global_list};
    global_clear();
    global_add("door1", "c1a0", GlobalState::On);
    global_add("door2", "c1a1", GlobalState::Dead);
    let host = FakeHost::default();
    let mut b = Buf::new(1024, 64, vec![]);
    save_global_state_with(&mut b.sd(), &host);
    // GLOBAL count record, then GENT blocks newest first: count record (3 fields) + name[64] +
    // levelName[32] + state (dead: 2, so non-empty).
    let w = b.written().to_vec();
    assert_eq!(&w[..2], &4i16.to_le_bytes());
    assert_eq!(&w[4..8], &1i32.to_le_bytes(), "one non-empty field");
    assert_eq!(&w[8..10], &4i16.to_le_bytes());
    assert_eq!(&w[12..16], &2i32.to_le_bytes(), "m_listCount");
    assert_eq!(&w[20..24], &3i32.to_le_bytes(), "GENT: three non-empty fields");
    assert_eq!(&w[24..26], &64i16.to_le_bytes(), "name[64]");
    assert_eq!(&w[28..33], b"door2", "newest first");
    assert_eq!(&w[28 + 64..28 + 66], &32i16.to_le_bytes(), "levelName[32]");
    assert_eq!(&w[w.len() - 4..], &1i32.to_le_bytes(), "the oldest entry (door1, on) comes last");
    // Back again: each block is EntityAdd'ed at the head, so the order flips.
    b.rewind_all();
    global_clear();
    restore_global_state_with(&mut b.sd(), &host);
    let list = global_list();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], ("door1".to_owned(), "c1a0".to_owned(), GlobalState::On));
    assert_eq!(list[1], ("door2".to_owned(), "c1a1".to_owned(), GlobalState::Dead));
    global_clear();
}

/// A tiny class for round trips through the real codec.
#[derive(Default, Debug, PartialEq)]
struct Thing {
    delay: f32,
    kill: string_t,
    anim: Animating,
    think: Option<u8>,
}

impl Thing {
    fn walk(&mut self, cx: &mut Cx, f: &mut Fields) {
        let cleared = f.base_funcs(cx, BaseFuncs { think: self.think.is_some(), ..Default::default() });
        if cleared.think {
            self.think = None;
        }
        f.delay(&mut self.delay, &mut self.kill);
        f.animating(&mut self.anim);
        f.finish();
    }
}

#[test]
fn a_set_function_cuts_the_restore_short() {
    // With no member pointer set, the whole chain comes back.
    let host = FakeHost::default();
    let mut ed = edict_t::default();
    ed.v.health = 5.0;
    let mut cx = Cx { edict: &mut ed };
    let mut b = Buf::new(4096, 256, vec![]);
    let mut t = Thing { delay: 2.0, anim: Animating { frame_rate: 1.5, sequence_loops: true, ..Default::default() }, ..Default::default() };
    t.walk(&mut cx, &mut Fields::save(&mut b.sd(), &host));
    let saved = b.data.size;
    b.rewind_all();
    let mut back = Thing::default();
    cx.pev().health = 0.0;
    back.walk(&mut cx, &mut Fields::restore(&mut b.sd(), &host, false));
    assert_eq!(back, t);
    assert_eq!(cx.pev().health, 5.0);
    assert_eq!(b.data.size, saved);

    // A set think: counted, not written, and the restore loses everything after "BASE".
    let mut b = Buf::new(4096, 256, vec![]);
    let mut t = Thing { delay: 2.0, think: Some(1), ..Default::default() };
    t.walk(&mut cx, &mut Fields::save(&mut b.sd(), &host));
    b.rewind_all();
    let mut back = Thing { delay: 9.0, ..Default::default() };
    let mut sd = b.sd();
    let mut f = Fields::restore(&mut sd, &host, false);
    back.walk(&mut cx, &mut f);
    assert!(!f.restore_ok());
    drop(f);
    assert_eq!(back.delay, 9.0, "CBaseDelay wasn't read");
    assert_eq!(cx.pev().health, 5.0, "entvars were");
}

/// Saves `ent` into `b`.
fn save_into(ent: &mut dyn world::Entity, cx: &mut Cx, host: &FakeHost, b: &mut Buf) {
    let mut sd = b.sd();
    let mut f = Fields::save(&mut sd, host);
    ent.save_fields(cx, &mut f);
    f.finish();
}

/// A host whose string pool has the ids `Mode::Fill` uses.
fn fill_host() -> FakeHost {
    let host = FakeHost::default();
    for i in 1..=8 {
        host.alloc_string(&std::ffi::CString::new(format!("s{i}")).unwrap());
    }
    host
}

#[test]
fn every_linked_class_round_trips() {
    // Fill every mapped field, save, restore into a fresh object, save again: the same bytes.
    let mut bad = Vec::new();
    for (classname, ctor) in all_linked() {
        let host = fill_host();
        let mut ed = edict_t::default();
        let mut cx = Cx { edict: &mut ed };
        let mut a = ctor();
        let mut f = Fields::fill();
        a.save_fields(&mut cx, &mut f);
        f.finish();
        drop(f);
        // A saved model would make the restore set it through the engine.
        cx.pev().modelindex = 0;
        let mut b = Buf::new(1 << 16, 1024, vec![]);
        save_into(&mut *a, &mut cx, &host, &mut b);
        let first = b.written().to_vec();
        b.rewind_all();
        let mut again = ctor();
        let mut ed2 = edict_t::default();
        let mut cx2 = Cx { edict: &mut ed2 };
        let ok = {
            let mut sd = b.sd();
            let mut f = Fields::restore(&mut sd, &host, false);
            again.save_fields(&mut cx2, &mut f);
            let ok = f.restore_ok();
            f.finish();
            ok
        };
        let mut b2 = Buf::new(1 << 16, 1024, vec![]);
        save_into(&mut *again, &mut cx2, &host, &mut b2);
        if !ok {
            bad.push(format!("{classname}: the restore stopped early"));
        } else if first != b2.written() {
            bad.push(format!("{classname}: the second save differs"));
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}
