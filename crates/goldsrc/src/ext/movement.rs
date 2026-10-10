//! Engine helpers added by the movement work package. Keep additions here to avoid merge conflicts.
//!
//! Studio model access for player animation: GoldSrc has no engine call that looks up a
//! sequence by name, so (like the SDK's `animation.cpp`) we read the `studiohdr_t` returned by
//! `pfnGetModelPtr`. The studio file layout is the same on every target (ints and floats only).

/// One `mstudioseqdesc_t`, the fields animation code needs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeqInfo {
    pub fps: f32,
    pub flags: i32,
    pub numframes: i32,
    pub linear_movement: [f32; 3],
}

/// `STUDIO_LOOPING` sequence flag.
pub const STUDIO_LOOPING: i32 = 0x0001;

const HDR_NUMSEQ: usize = 164;
const HDR_SEQINDEX: usize = 168;
const SEQDESC_SIZE: usize = 176;
const SEQ_FPS: usize = 32;
const SEQ_FLAGS: usize = 36;
const SEQ_NUMFRAMES: usize = 56;
const SEQ_LINEARMOVEMENT: usize = 76;
const HDR_NUMBONECONTROLLERS: usize = 148;
const HDR_BONECONTROLLERINDEX: usize = 152;
const BONECONTROLLER_SIZE: usize = 24;
const SEQ_ACTIVITY: usize = 40;
const SEQ_ACTWEIGHT: usize = 44;
const SEQ_NUMEVENTS: usize = 48;
const SEQ_EVENTINDEX: usize = 52;
const EVENT_SIZE: usize = 76;
const SEQ_BBMIN: usize = 96;

/// One `mstudioevent_t` (frame and event number; the options string isn't needed server-side).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StudioEvent {
    pub frame: i32,
    pub event: i32,
}

/// One `mstudiobonecontroller_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoneController {
    /// `STUDIO_X..STUDIO_RLOOP` motion type bits.
    pub kind: i32,
    pub start: f32,
    pub end: f32,
    /// Which `entvars_t::controller` byte drives it (0..3, 4 = mouth).
    pub index: i32,
}

/// A studio header in memory (`studiohdr_t *`).
#[derive(Clone, Copy)]
pub struct StudioHdr(*const u8);

impl StudioHdr {
    /// # Safety
    /// `p` must be null or point to a loaded studio model header that outlives the value.
    pub unsafe fn from_ptr(p: *const std::ffi::c_void) -> Option<Self> {
        (!p.is_null()).then_some(StudioHdr(p.cast()))
    }

    fn i32_at(&self, off: usize) -> i32 {
        // SAFETY: `off` lies inside the engine-loaded studio header (header offsets from the studio format); the header outlives `self` per `from_ptr`.
        unsafe { self.0.add(off).cast::<i32>().read_unaligned() }
    }

    fn f32_at(&self, off: usize) -> f32 {
        // SAFETY: `off` lies inside the engine-loaded studio header (header offsets from the studio format); the header outlives `self` per `from_ptr`.
        unsafe { self.0.add(off).cast::<f32>().read_unaligned() }
    }

    pub fn num_sequences(&self) -> i32 {
        self.i32_at(HDR_NUMSEQ)
    }

    fn seq_offset(&self, seq: i32) -> Option<usize> {
        if seq < 0 || seq >= self.num_sequences() {
            return None;
        }
        Some(self.i32_at(HDR_SEQINDEX) as usize + seq as usize * SEQDESC_SIZE)
    }

    /// The sequence's label (`char[32]`).
    pub fn sequence_name(&self, seq: i32) -> Option<&[u8]> {
        let off = self.seq_offset(seq)?;
        // SAFETY: `off` lies inside the engine-loaded studio header (header offsets from the studio format); the header outlives `self` per `from_ptr`.
        let label = unsafe { std::slice::from_raw_parts(self.0.add(off), 32) };
        let len = label.iter().position(|&c| c == 0).unwrap_or(32);
        Some(&label[..len])
    }

    /// SDK `LookupSequence`: case-insensitive label match, -1 if missing.
    pub fn lookup_sequence(&self, name: &str) -> i32 {
        (0..self.num_sequences()).find(|&i| self.sequence_name(i).is_some_and(|l| l.eq_ignore_ascii_case(name.as_bytes()))).unwrap_or(-1)
    }

    pub fn sequence_info(&self, seq: i32) -> Option<SeqInfo> {
        let off = self.seq_offset(seq)?;
        Some(SeqInfo {
            fps: self.f32_at(off + SEQ_FPS),
            flags: self.i32_at(off + SEQ_FLAGS),
            numframes: self.i32_at(off + SEQ_NUMFRAMES),
            linear_movement: [
                self.f32_at(off + SEQ_LINEARMOVEMENT),
                self.f32_at(off + SEQ_LINEARMOVEMENT + 4),
                self.f32_at(off + SEQ_LINEARMOVEMENT + 8),
            ],
        })
    }
}

impl StudioHdr {
    /// The sequence's (`activity`, `actweight`), for SDK `LookupActivity`.
    pub fn sequence_activity(&self, seq: i32) -> Option<(i32, i32)> {
        let off = self.seq_offset(seq)?;
        Some((self.i32_at(off + SEQ_ACTIVITY), self.i32_at(off + SEQ_ACTWEIGHT)))
    }

    /// The sequence's animation events, in file order (SDK `GetAnimationEvent` walks them).
    pub fn sequence_events(&self, seq: i32) -> Vec<StudioEvent> {
        let Some(off) = self.seq_offset(seq) else { return Vec::new() };
        let (n, base) = (self.i32_at(off + SEQ_NUMEVENTS), self.i32_at(off + SEQ_EVENTINDEX) as usize);
        (0..n.max(0) as usize)
            .map(|i| StudioEvent { frame: self.i32_at(base + i * EVENT_SIZE), event: self.i32_at(base + i * EVENT_SIZE + 4) })
            .collect()
    }

    /// The sequence's bounding box (`bbmin`, `bbmax`), for SDK `ExtractBbox`.
    pub fn sequence_bbox(&self, seq: i32) -> Option<([f32; 3], [f32; 3])> {
        let off = self.seq_offset(seq)?;
        let v = |o: usize| [self.f32_at(off + o), self.f32_at(off + o + 4), self.f32_at(off + o + 8)];
        Some((v(SEQ_BBMIN), v(SEQ_BBMIN + 12)))
    }

    /// The model's bone controllers, in file order (SDK `SetController` searches them).
    pub fn bone_controllers(&self) -> Vec<BoneController> {
        let (n, base) = (self.i32_at(HDR_NUMBONECONTROLLERS), self.i32_at(HDR_BONECONTROLLERINDEX) as usize);
        (0..n.max(0) as usize)
            .map(|i| {
                let o = base + i * BONECONTROLLER_SIZE;
                BoneController { kind: self.i32_at(o + 4), start: self.f32_at(o + 8), end: self.f32_at(o + 12), index: self.i32_at(o + 20) }
            })
            .collect()
    }
}

impl SeqInfo {
    /// SDK `GetSequenceInfo`: (frame rate in 0..256 units per second, ground speed).
    pub fn rates(&self) -> (f32, f32) {
        if self.numframes > 1 {
            let n = (self.numframes - 1) as f32;
            let lm = self.linear_movement;
            let dist = (lm[0] * lm[0] + lm[1] * lm[1] + lm[2] * lm[2]).sqrt();
            (256.0 * self.fps / n, dist * self.fps / n)
        } else {
            (256.0, 0.0)
        }
    }
}

#[cfg(feature = "server")]
mod server {
    use super::StudioHdr;
    use crate::sys::edict_t;

    /// Studio header of the entity's model (`GET_MODEL_PTR`), if it has one.
    pub fn model_ptr(e: *mut edict_t) -> Option<StudioHdr> {
        if e.is_null() {
            return None;
        }
        let hdr = crate::eng!(pfnGetModelPtr(e));
        // SAFETY: `pfnGetModelPtr`/the caller give null or a studio header loaded for the model's lifetime.
        unsafe { StudioHdr::from_ptr(hdr) }
    }

    /// `LookupSequence(name)` on the entity's model: -1 if the model or sequence is missing.
    pub fn lookup_sequence(e: *mut edict_t, name: &str) -> i32 {
        model_ptr(e).map_or(-1, |h| h.lookup_sequence(name))
    }
}

#[cfg(feature = "server")]
pub use server::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sequences() {
        // A fake header with two sequences.
        let mut buf = vec![0u8; 1024];
        buf[HDR_NUMSEQ..HDR_NUMSEQ + 4].copy_from_slice(&2i32.to_le_bytes());
        buf[HDR_SEQINDEX..HDR_SEQINDEX + 4].copy_from_slice(&172i32.to_le_bytes());
        for (i, name) in [&b"idle"[..], b"run2"].iter().enumerate() {
            let o = 172 + i * SEQDESC_SIZE;
            buf[o..o + name.len()].copy_from_slice(name);
            buf[o + SEQ_FPS..o + SEQ_FPS + 4].copy_from_slice(&30.0f32.to_le_bytes());
            buf[o + SEQ_NUMFRAMES..o + SEQ_NUMFRAMES + 4].copy_from_slice(&31i32.to_le_bytes());
        }
        // SAFETY: `buf` is a local studio header image that outlives `h`.
        let h = unsafe { StudioHdr::from_ptr(buf.as_ptr().cast()) }.unwrap();
        assert_eq!(h.lookup_sequence("RUN2"), 1);
        assert_eq!(h.lookup_sequence("missing"), -1);
        let (fr, gs) = h.sequence_info(1).unwrap().rates();
        assert_eq!(fr, 256.0);
        assert_eq!(gs, 0.0);
    }

    #[test]
    fn reads_activities_events_controllers() {
        let mut buf = vec![0u8; 2048];
        let put_i = |b: &mut Vec<u8>, o: usize, v: i32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        let put_f = |b: &mut Vec<u8>, o: usize, v: f32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put_i(&mut buf, HDR_NUMSEQ, 1);
        put_i(&mut buf, HDR_SEQINDEX, 200);
        put_i(&mut buf, 200 + SEQ_ACTIVITY, 30);
        put_i(&mut buf, 200 + SEQ_ACTWEIGHT, 2);
        put_i(&mut buf, 200 + SEQ_NUMEVENTS, 2);
        put_i(&mut buf, 200 + SEQ_EVENTINDEX, 600);
        put_f(&mut buf, 200 + SEQ_BBMIN, -8.0);
        put_f(&mut buf, 200 + SEQ_BBMIN + 20, 72.0);
        put_i(&mut buf, 600, 5);
        put_i(&mut buf, 604, 1);
        put_i(&mut buf, 600 + EVENT_SIZE, 9);
        put_i(&mut buf, 604 + EVENT_SIZE, 5004);
        put_i(&mut buf, HDR_NUMBONECONTROLLERS, 1);
        put_i(&mut buf, HDR_BONECONTROLLERINDEX, 800);
        put_i(&mut buf, 804, 0x10);
        put_f(&mut buf, 808, -30.0);
        put_f(&mut buf, 812, 30.0);
        put_i(&mut buf, 820, 2);
        // SAFETY: `buf` is a local studio header image that outlives `h`.
        let h = unsafe { StudioHdr::from_ptr(buf.as_ptr().cast()) }.unwrap();
        assert_eq!(h.sequence_activity(0), Some((30, 2)));
        assert_eq!(h.sequence_activity(1), None);
        assert_eq!(h.sequence_events(0), [StudioEvent { frame: 5, event: 1 }, StudioEvent { frame: 9, event: 5004 }]);
        assert_eq!(h.sequence_bbox(0), Some(([-8.0, 0.0, 0.0], [0.0, 0.0, 72.0])));
        assert_eq!(h.bone_controllers(), [BoneController { kind: 0x10, start: -30.0, end: 30.0, index: 2 }]);
    }
}
