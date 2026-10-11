//! Studio model (`.mdl`) data read straight from the bytes the engine loaded (studio.h layouts,
//! offsets confirmed against the TS renderer's field accesses). Every read is bounds-checked: a
//! truncated or corrupt model yields defaults instead of reading outside the block.

/// Little-endian reads at byte offsets; out of range reads return 0.
#[derive(Clone, Copy)]
pub struct Bytes<'a>(pub &'a [u8]);

/// The largest offset [`at`] produces. Offsets come from the model's own (untrusted) `int`
/// indexes and counts; capped, the small field offsets added after them can't overflow `usize`
/// (32 bits on i686), and a read there is simply out of range.
const MAX_OFFSET: usize = usize::MAX / 4;

/// `base + add` for offsets taken from model data, saturating at [`MAX_OFFSET`].
pub fn at(base: usize, add: usize) -> usize {
    base.saturating_add(add).min(MAX_OFFSET)
}

impl<'a> Bytes<'a> {
    pub fn i32(&self, off: usize) -> i32 {
        off.checked_add(4).and_then(|end| self.0.get(off..end)).map_or(0, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn f32(&self, off: usize) -> f32 {
        f32::from_bits(self.i32(off) as u32)
    }
    pub fn i16(&self, off: usize) -> i16 {
        off.checked_add(2).and_then(|end| self.0.get(off..end)).map_or(0, |b| i16::from_le_bytes([b[0], b[1]]))
    }
    pub fn u16(&self, off: usize) -> u16 {
        self.i16(off) as u16
    }
    pub fn u8(&self, off: usize) -> u8 {
        self.0.get(off).copied().unwrap_or(0)
    }
    pub fn vec3(&self, off: usize) -> [f32; 3] {
        [self.f32(off), self.f32(off.saturating_add(4)), self.f32(off.saturating_add(8))]
    }
    /// A NUL-terminated name in a fixed `char[len]` field.
    pub fn name(&self, off: usize, len: usize) -> &'a [u8] {
        let field = self.0.get(off..off.saturating_add(len).min(self.0.len())).unwrap_or(&[]);
        let end = field.iter().position(|&c| c == 0).unwrap_or(field.len());
        &field[..end]
    }
}

/// Offsets into `studiohdr_t`.
mod hdr {
    pub const NUMBONES: usize = 140;
    pub const BONEINDEX: usize = 144;
    pub const NUMBONECONTROLLERS: usize = 148;
    pub const BONECONTROLLERINDEX: usize = 152;
    pub const NUMSEQ: usize = 164;
    pub const SEQINDEX: usize = 168;
    pub const NUMSEQGROUPS: usize = 172;
    pub const SEQGROUPINDEX: usize = 176;
    pub const NUMBODYPARTS: usize = 204;
    pub const NUMATTACHMENTS: usize = 212;
    pub const ATTACHMENTINDEX: usize = 216;
}

pub const BONE_SIZE: usize = 112;
pub const BONECONTROLLER_SIZE: usize = 24;
pub const SEQDESC_SIZE: usize = 176;
pub const SEQGROUP_SIZE: usize = 104;
pub const ATTACHMENT_SIZE: usize = 88;
/// `mstudioanim_t`: `unsigned short offset[6]`.
pub const ANIM_SIZE: usize = 12;

/// `STUDIO_LOOPING` (seqdesc flags).
pub const STUDIO_LOOPING: i32 = 1;
/// Motion types (`STUDIO_X` ...) and the controller wrap flag.
pub const STUDIO_X: i32 = 1;
pub const STUDIO_Y: i32 = 2;
pub const STUDIO_Z: i32 = 4;
pub const STUDIO_XR: i32 = 8;
pub const STUDIO_YR: i32 = 0x10;
pub const STUDIO_ZR: i32 = 0x20;
pub const STUDIO_TYPES: i32 = 0x7fff;
pub const STUDIO_RLOOP: i32 = 0x8000;

/// A studio header and the data after it.
#[derive(Clone, Copy)]
pub struct Studio<'a> {
    pub b: Bytes<'a>,
}

/// `mstudiobone_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bone<'a> {
    pub name: &'a [u8],
    pub parent: i32,
    pub bonecontroller: [i32; 6],
    pub value: [f32; 6],
    pub scale: [f32; 6],
}

/// `mstudiobonecontroller_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoneController {
    pub bone: i32,
    pub kind: i32,
    pub start: f32,
    pub end: f32,
    pub index: i32,
}

/// `mstudioseqdesc_t` (the fields the renderer uses).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeqDesc {
    pub fps: f32,
    pub flags: i32,
    pub numframes: i32,
    pub motiontype: i32,
    pub motionbone: i32,
    pub linearmovement: [f32; 3],
    pub numblends: i32,
    pub animindex: i32,
    pub blendstart: [f32; 2],
    pub blendend: [f32; 2],
    pub seqgroup: i32,
}

/// `mstudioattachment_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Attachment {
    pub bone: i32,
    pub org: [f32; 3],
}

fn count(n: i32) -> usize {
    n.max(0) as usize
}

impl<'a> Studio<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Studio { b: Bytes(data) }
    }
    pub fn numbones(&self) -> usize {
        count(self.b.i32(hdr::NUMBONES)).min(goldsrc::ext::studio::MAX_BONES)
    }
    pub fn numseq(&self) -> i32 {
        self.b.i32(hdr::NUMSEQ).max(0)
    }
    pub fn numbodyparts(&self) -> i32 {
        self.b.i32(hdr::NUMBODYPARTS).max(0)
    }
    pub fn numattachments(&self) -> usize {
        count(self.b.i32(hdr::NUMATTACHMENTS))
    }
    pub fn numbonecontrollers(&self) -> usize {
        count(self.b.i32(hdr::NUMBONECONTROLLERS))
    }

    fn table(&self, index_off: usize, i: usize, size: usize) -> usize {
        at(count(self.b.i32(index_off)), i.saturating_mul(size))
    }

    pub fn bone(&self, i: usize) -> Bone<'a> {
        let o = self.table(hdr::BONEINDEX, i, BONE_SIZE);
        let b = self.b;
        Bone {
            name: b.name(o, 32),
            parent: b.i32(o + 32),
            bonecontroller: std::array::from_fn(|j| b.i32(o + 40 + 4 * j)),
            value: std::array::from_fn(|j| b.f32(o + 64 + 4 * j)),
            scale: std::array::from_fn(|j| b.f32(o + 88 + 4 * j)),
        }
    }

    pub fn bone_controller(&self, i: usize) -> BoneController {
        let o = self.table(hdr::BONECONTROLLERINDEX, i, BONECONTROLLER_SIZE);
        let b = self.b;
        BoneController { bone: b.i32(o), kind: b.i32(o + 4), start: b.f32(o + 8), end: b.f32(o + 12), index: b.i32(o + 20) }
    }

    /// Sequence `i`; None when out of range.
    pub fn seq(&self, i: i32) -> Option<SeqDesc> {
        if i < 0 || i >= self.numseq() {
            return None;
        }
        let o = self.table(hdr::SEQINDEX, i as usize, SEQDESC_SIZE);
        let b = self.b;
        Some(SeqDesc {
            fps: b.f32(o + 32),
            flags: b.i32(o + 36),
            numframes: b.i32(o + 56),
            motiontype: b.i32(o + 68),
            motionbone: b.i32(o + 72),
            linearmovement: b.vec3(o + 76),
            numblends: b.i32(o + 120),
            animindex: b.i32(o + 124),
            blendstart: [b.f32(o + 136), b.f32(o + 140)],
            blendend: [b.f32(o + 144), b.f32(o + 148)],
            seqgroup: b.i32(o + 156),
        })
    }

    /// Sequence group `i`: (file name, `data` offset). None when out of range.
    pub fn seqgroup(&self, i: i32) -> Option<(&'a [u8], i32)> {
        if i < 0 || i >= self.b.i32(hdr::NUMSEQGROUPS) {
            return None;
        }
        let o = self.table(hdr::SEQGROUPINDEX, i as usize, SEQGROUP_SIZE);
        Some((self.b.name(o + 32, 64), self.b.i32(o + 100)))
    }

    pub fn attachment(&self, i: usize) -> Attachment {
        let o = self.table(hdr::ATTACHMENTINDEX, i, ATTACHMENT_SIZE);
        Attachment { bone: self.b.i32(o + 36), org: self.b.vec3(o + 40) }
    }
}

/// The animation of one sequence blend: `numbones` `mstudioanim_t`s at `base` in `data` (the model,
/// or a demand-loaded sequence group file).
#[derive(Clone, Copy)]
pub struct Anim<'a> {
    pub data: Bytes<'a>,
    pub base: usize,
}

impl<'a> Anim<'a> {
    /// The anim of blend `blend` (`panim += numbones` per blend).
    pub fn blend(self, blend: usize, numbones: usize) -> Self {
        Anim { data: self.data, base: at(self.base, blend.saturating_mul(numbones).saturating_mul(ANIM_SIZE)) }
    }

    /// Byte offset of bone `bone`'s channel `ch` (0..2 position, 3..5 rotation) value run, or
    /// None when the channel is constant (`offset[ch] == 0`).
    fn channel(&self, bone: usize, ch: usize) -> Option<usize> {
        let panim = at(self.base, bone.saturating_mul(ANIM_SIZE));
        let off = self.data.u16(panim + 2 * ch) as usize;
        (off != 0).then_some(at(panim, off))
    }

    /// Finds the run that holds `frame` (`num.valid`, `num.total` in each run header, followed by
    /// `valid` values). Returns (byte offset of the run header, k = frame within the run).
    fn span(&self, mut p: usize, frame: i32) -> (usize, i32) {
        let d = self.data;
        let mut k = frame;
        let (valid, total) = (d.u8(p) as i32, d.u8(p + 1) as i32);
        // DEBUG (SDK): a corrupt header restarts at its first value.
        if total < valid {
            k = 0;
        }
        let mut total = total;
        let mut guard = 0;
        while total <= k {
            k -= total;
            let valid = d.u8(p) as i32;
            p = at(p, (valid as usize + 1) * 2);
            let (v, t) = (d.u8(p) as i32, d.u8(p + 1) as i32);
            if t < v {
                k = 0;
            }
            total = t;
            guard += 1;
            // A run with total 0 (or data past the end, which reads as 0) would never end.
            if total == 0 || guard > 4096 {
                return (p, 0);
            }
        }
        (p, k)
    }

    /// Value `i` of the run at `p` (`panimvalue[i].value`).
    fn value(&self, p: usize, i: i32) -> f32 {
        self.data.i16(at(p, 2 * i.max(0) as usize)) as f32
    }

    /// The two raw rotation samples (frame, frame + 1) of a channel, as `StudioCalcBoneQuaterion`
    /// (`0x10030380`) reads them.
    pub fn rotation_samples(&self, bone: usize, ch: usize, frame: i32) -> Option<(f32, f32)> {
        let p = self.channel(bone, ch)?;
        let (p, k) = self.span(p, frame);
        let (valid, total) = (self.data.u8(p) as i32, self.data.u8(p + 1) as i32);
        Some(if valid > k {
            let a1 = self.value(p, k + 1);
            let a2 = if valid > k + 1 {
                self.value(p, k + 2)
            } else if total > k + 1 {
                a1
            } else {
                self.value(p, valid + 2)
            };
            (a1, a2)
        } else {
            let a1 = self.value(p, valid);
            let a2 = if total > k + 1 { a1 } else { self.value(p, valid + 2) };
            (a1, a2)
        })
    }

    /// The raw position sample at `frame + s`, as `StudioCalcBonePosition` (`0x10030550`) reads it.
    pub fn position_sample(&self, bone: usize, ch: usize, frame: i32, s: f32) -> Option<f32> {
        let p = self.channel(bone, ch)?;
        let (p, k) = self.span(p, frame);
        let (valid, total) = (self.data.u8(p) as i32, self.data.u8(p + 1) as i32);
        Some(if valid > k {
            if valid > k + 1 { self.value(p, k + 1) * (1.0 - s) + s * self.value(p, k + 2) } else { self.value(p, k + 1) }
        } else if total <= k + 1 {
            self.value(p, valid) * (1.0 - s) + s * self.value(p, valid + 2)
        } else {
            self.value(p, valid)
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a minimal model in memory for tests.
    #[derive(Default)]
    pub struct Builder {
        pub bones: Vec<(&'static str, i32, [f32; 6], [f32; 6])>,
        pub seqs: Vec<SeqDesc>,
        /// Per sequence: anim data per bone, `[channel] -> Option<run bytes>`, one blend only.
        pub anims: Vec<Vec<[Option<Vec<u8>>; 6]>>,
        pub controllers: Vec<BoneController>,
        pub attachments: Vec<Attachment>,
    }

    fn put_i32(v: &mut [u8], o: usize, x: i32) {
        v[o..o + 4].copy_from_slice(&x.to_le_bytes());
    }
    fn put_f32(v: &mut [u8], o: usize, x: f32) {
        put_i32(v, o, x.to_bits() as i32)
    }

    /// An RLE run: `valid` values, `total` frames.
    pub fn run(valid: u8, total: u8, values: &[i16]) -> Vec<u8> {
        let mut v = vec![valid, total];
        for x in values {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    impl Builder {
        pub fn build(&self) -> Vec<u8> {
            let mut v = vec![0u8; 244];
            let nb = self.bones.len();
            put_i32(&mut v, hdr::NUMBONES, nb as i32);
            let at = v.len() as i32;
            put_i32(&mut v, hdr::BONEINDEX, at);
            for (name, parent, value, scale) in &self.bones {
                let o = v.len();
                v.resize(o + BONE_SIZE, 0);
                v[o..o + name.len()].copy_from_slice(name.as_bytes());
                put_i32(&mut v, o + 32, *parent);
                for j in 0..6 {
                    put_i32(&mut v, o + 40 + 4 * j, -1);
                    put_f32(&mut v, o + 64 + 4 * j, value[j]);
                    put_f32(&mut v, o + 88 + 4 * j, scale[j]);
                }
            }
            for c in &self.controllers {
                let b = c.bone as usize;
                let o = 244 + b * BONE_SIZE;
                // Bind the controller to the channel its type names (rotation types use 3..5).
                let ch = match c.kind & STUDIO_TYPES {
                    STUDIO_X => 0,
                    STUDIO_Y => 1,
                    STUDIO_Z => 2,
                    STUDIO_XR => 3,
                    STUDIO_YR => 4,
                    _ => 5,
                };
                put_i32(&mut v, o + 40 + 4 * ch, c.index);
            }
            put_i32(&mut v, hdr::NUMBONECONTROLLERS, self.controllers.len() as i32);
            let at = v.len() as i32;
            put_i32(&mut v, hdr::BONECONTROLLERINDEX, at);
            for c in &self.controllers {
                let o = v.len();
                v.resize(o + BONECONTROLLER_SIZE, 0);
                put_i32(&mut v, o, c.bone);
                put_i32(&mut v, o + 4, c.kind);
                put_f32(&mut v, o + 8, c.start);
                put_f32(&mut v, o + 12, c.end);
                put_i32(&mut v, o + 20, c.index);
            }
            put_i32(&mut v, hdr::NUMATTACHMENTS, self.attachments.len() as i32);
            let at = v.len() as i32;
            put_i32(&mut v, hdr::ATTACHMENTINDEX, at);
            for a in &self.attachments {
                let o = v.len();
                v.resize(o + ATTACHMENT_SIZE, 0);
                put_i32(&mut v, o + 36, a.bone);
                for j in 0..3 {
                    put_f32(&mut v, o + 40 + 4 * j, a.org[j]);
                }
            }
            // one sequence group (data 0)
            put_i32(&mut v, hdr::NUMSEQGROUPS, 1);
            let at = v.len() as i32;
            put_i32(&mut v, hdr::SEQGROUPINDEX, at);
            v.resize(v.len() + SEQGROUP_SIZE, 0);
            put_i32(&mut v, hdr::NUMSEQ, self.seqs.len() as i32);
            let seqindex = v.len();
            put_i32(&mut v, hdr::SEQINDEX, seqindex as i32);
            v.resize(seqindex + self.seqs.len() * SEQDESC_SIZE, 0);
            for (i, s) in self.seqs.iter().enumerate() {
                let o = seqindex + i * SEQDESC_SIZE;
                put_f32(&mut v, o + 32, s.fps);
                put_i32(&mut v, o + 36, s.flags);
                put_i32(&mut v, o + 56, s.numframes);
                put_i32(&mut v, o + 68, s.motiontype);
                put_i32(&mut v, o + 72, s.motionbone);
                for j in 0..3 {
                    put_f32(&mut v, o + 76 + 4 * j, s.linearmovement[j]);
                }
                put_i32(&mut v, o + 120, s.numblends.max(1));
                put_f32(&mut v, o + 136, s.blendstart[0]);
                put_f32(&mut v, o + 144, s.blendend[0]);
                put_i32(&mut v, o + 156, 0);
                // anim block
                let anim = v.len();
                put_i32(&mut v, o + 124, anim as i32);
                v.resize(anim + nb * ANIM_SIZE, 0);
                if let Some(bones) = self.anims.get(i) {
                    for (b, chans) in bones.iter().enumerate() {
                        let panim = anim + b * ANIM_SIZE;
                        for (ch, data) in chans.iter().enumerate() {
                            if let Some(d) = data {
                                let at = v.len();
                                let rel = (at - panim) as u16;
                                v[panim + 2 * ch..panim + 2 * ch + 2].copy_from_slice(&rel.to_le_bytes());
                                v.extend_from_slice(d);
                            }
                        }
                    }
                }
            }
            let len = v.len() as i32;
            put_i32(&mut v, 72, len);
            v
        }
    }

    #[test]
    fn header_tables() {
        let b = Builder {
            bones: vec![("Bip01", -1, [1.0, 2.0, 3.0, 0.1, 0.2, 0.3], [0.5; 6]), ("Bip01 Spine", 0, [0.0; 6], [1.0; 6])],
            seqs: vec![SeqDesc { fps: 30.0, numframes: 10, flags: STUDIO_LOOPING, ..Default::default() }],
            attachments: vec![Attachment { bone: 1, org: [4.0, 5.0, 6.0] }],
            ..Default::default()
        };
        let data = b.build();
        let m = Studio::new(&data);
        assert_eq!(m.numbones(), 2);
        assert_eq!(m.bone(1).name, b"Bip01 Spine");
        assert_eq!(m.bone(1).parent, 0);
        assert_eq!(m.bone(0).value[2], 3.0);
        assert_eq!(m.bone(0).bonecontroller, [-1; 6]);
        assert_eq!(m.seq(0).unwrap().numframes, 10);
        assert!(m.seq(1).is_none() && m.seq(-1).is_none());
        assert_eq!(m.attachment(0), Attachment { bone: 1, org: [4.0, 5.0, 6.0] });
        assert_eq!(m.seqgroup(0).map(|g| g.1), Some(0));
    }

    #[test]
    fn rle_samples_follow_the_sdk_walk() {
        // Run 1: 2 values covering 4 frames (values repeat the last), run 2: 3 values, 3 frames.
        let mut d = run(2, 4, &[10, 20]);
        d.extend(run(3, 3, &[30, 40, 50]));
        let b = Builder {
            bones: vec![("b", -1, [0.0; 6], [1.0; 6])],
            seqs: vec![SeqDesc { numframes: 7, ..Default::default() }],
            anims: vec![vec![[Some(d.clone()), None, None, Some(d), None, None]]],
            ..Default::default()
        };
        let data = b.build();
        let m = Studio::new(&data);
        let seq = m.seq(0).unwrap();
        let a = Anim { data: m.b, base: seq.animindex as usize };
        // frame 0: inside the valid values
        assert_eq!(a.rotation_samples(0, 3, 0), Some((10.0, 20.0)));
        // frame 1: last valid value, next frame is a repeat
        assert_eq!(a.rotation_samples(0, 3, 1), Some((20.0, 20.0)));
        // frame 3: repeat at the end of the run, next frame is the next run's first value
        assert_eq!(a.rotation_samples(0, 3, 3), Some((20.0, 30.0)));
        // frame 4: second run
        assert_eq!(a.rotation_samples(0, 3, 4), Some((30.0, 40.0)));
        assert_eq!(a.rotation_samples(0, 4, 0), None, "constant channel");
        assert_eq!(a.position_sample(0, 0, 0, 0.5), Some(15.0));
        assert_eq!(a.position_sample(0, 0, 2, 0.5), Some(20.0), "repeat inside the run");
        assert_eq!(a.position_sample(0, 0, 3, 0.5), Some(25.0), "lerp into the next run");
        // Garbage offsets read as zero and never loop forever.
        let bad = Anim { data: Bytes(&[1, 0, 0xff, 0xff]), base: 0 };
        assert!(bad.rotation_samples(0, 0, 200).is_some());
    }

    /// Random, all-0xff and truncated model data and huge offsets: every read stays in bounds and
    /// no offset arithmetic overflows (`usize` is 32 bits on i686).
    #[test]
    fn garbage_models_never_panic() {
        let mut x = 0x9e37_79b9_u32;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let mut bufs: Vec<Vec<u8>> = vec![vec![], vec![0xff; 300], vec![0x7f; 4096]];
        for n in 0..40 {
            bufs.push((0..(n * 37) % 2000).map(|_| rnd() as u8).collect());
        }
        for b in &bufs {
            let m = Studio::new(b);
            for i in [0, 1, m.numbones().saturating_sub(1), usize::MAX / 2, usize::MAX] {
                m.bone(i);
                m.bone_controller(i);
                m.attachment(i);
            }
            for i in [-1, 0, 1, i32::MAX, i32::MIN, m.numseq().saturating_sub(1)] {
                m.seq(i);
                m.seqgroup(i);
            }
            for base in [0, 7, usize::MAX / 2, usize::MAX - 3, usize::MAX] {
                let a = Anim { data: Bytes(b), base };
                for blend in [0, 3, usize::MAX] {
                    let a = a.blend(blend, 128);
                    for (bone, frame) in [(0, 0), (127, 255), (usize::MAX, i32::MAX), (5, i32::MIN), (1, -1)] {
                        a.rotation_samples(bone, 3, frame);
                        a.position_sample(bone, 2, frame, 0.5);
                    }
                }
            }
        }
        let b = Bytes(&[1, 2, 3, 4, 5]);
        assert_eq!((b.i32(usize::MAX), b.i16(usize::MAX - 1), b.u8(usize::MAX)), (0, 0, 0));
        assert_eq!(b.vec3(usize::MAX), [0.0; 3]);
        assert_eq!(b.name(usize::MAX, 32), b"");
        assert_eq!(b.name(3, usize::MAX), [4, 5]);
    }
}
