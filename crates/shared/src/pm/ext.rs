//! x87 extended precision (64-bit significand, round to nearest even) in software.
//!
//! TS's server (`ts_i386.so`, GCC 2.95, x87) evaluates each C expression on the FPU stack in
//! extended precision and rounds to `float` only where it stores: assignments to `float`
//! variables (the build spills them to memory, as `-ffloat-store` does), struct fields and
//! `float` arguments. Double constants (`0.001`, `0.333`, `0.8`, ...) stay doubles. Movement
//! reproduces this exactly with [`Ext`]: the operations are written in the order of the
//! machine code, and [`Ext::f`] is the `fstps`. `f64` would round twice (to 53 bits, then 24)
//! and misses cases TS hits every few frames, such as `600 * 0.001 <= 0.6` (false in extended
//! precision, true in `f64`), which decides the frame a duck finishes.
//!
//! The exponent is kept in an `i32`, so values never overflow or go subnormal inside the
//! emulation; that only differs from the FPU for magnitudes far outside anything movement
//! computes (beyond 1e±4900). Transcendentals ([`Ext::sin`], [`Ext::cos`]) go through `f64`,
//! which is enough because TS rounds `fsin`/`fcos` results to `float` straight away.

use std::cmp::Ordering;

/// An x87 extended-precision value.
#[derive(Clone, Copy, Debug)]
pub struct Ext {
    neg: bool,
    /// Value = `mant * 2^(exp - 63)`; `mant` has bit 63 set, or is 0 for a zero.
    exp: i32,
    mant: u64,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Finite,
    Inf,
    Nan,
}

impl Ext {
    pub const ZERO: Ext = Ext { neg: false, exp: 0, mant: 0, kind: Kind::Finite };
    const NAN: Ext = Ext { neg: false, exp: 0, mant: 0, kind: Kind::Nan };

    fn inf(neg: bool) -> Ext {
        Ext { neg, exp: 0, mant: 0, kind: Kind::Inf }
    }

    fn zero(neg: bool) -> Ext {
        Ext { neg, ..Ext::ZERO }
    }

    fn is_zero(self) -> bool {
        self.kind == Kind::Finite && self.mant == 0
    }

    pub fn is_nan(self) -> bool {
        self.kind == Kind::Nan
    }

    /// Rounds `m * 2^e` (plus a sticky remainder below bit 0) to a 64-bit significand.
    fn pack(neg: bool, e: i32, m: u128, sticky: bool) -> Ext {
        if m == 0 {
            return Ext::zero(neg);
        }
        let lz = m.leading_zeros() as i32;
        let m = m << lz;
        let e = e - lz;
        let mut mant = (m >> 64) as u64;
        let rest = m as u64;
        let half = 1u64 << 63;
        let up = rest > half || (rest == half && (sticky || mant & 1 == 1));
        let mut exp = e + 127;
        if up {
            mant = mant.wrapping_add(1);
            if mant == 0 {
                mant = half;
                exp += 1;
            }
        }
        Ext { neg, exp, mant, kind: Kind::Finite }
    }

    pub fn from_f32(v: f32) -> Ext {
        let b = v.to_bits();
        let neg = b >> 31 != 0;
        let e = ((b >> 23) & 0xff) as i32;
        let frac = (b & 0x7f_ffff) as u128;
        match e {
            0 => Ext::pack(neg, -149, frac, false),
            0xff if frac == 0 => Ext::inf(neg),
            0xff => Ext::NAN,
            _ => Ext::pack(neg, e - 150, frac | 1 << 23, false),
        }
    }

    pub fn from_f64(v: f64) -> Ext {
        let b = v.to_bits();
        let neg = b >> 63 != 0;
        let e = ((b >> 52) & 0x7ff) as i32;
        let frac = (b & 0xf_ffff_ffff_ffff) as u128;
        match e {
            0 => Ext::pack(neg, -1074, frac, false),
            0x7ff if frac == 0 => Ext::inf(neg),
            0x7ff => Ext::NAN,
            _ => Ext::pack(neg, e - 1075, frac | 1 << 52, false),
        }
    }

    /// `fild`.
    pub fn from_i32(v: i32) -> Ext {
        Ext::pack(v < 0, 0, v.unsigned_abs() as u128, false)
    }

    /// Rounds to `nbits` significant bits (to nearest even) with a minimum exponent for
    /// gradual underflow; returns (significand, exponent of its bit 0), or `None` on overflow.
    fn round_bits(self, nbits: u32, min_exp: i32, max_exp: i32) -> Option<(u64, i32)> {
        // The leading bit weighs 2^exp; below `min_exp` the format loses bits (subnormals).
        let mut shift = 64 - nbits as i32;
        if self.exp < min_exp {
            shift += min_exp - self.exp;
        }
        let mut e0 = self.exp - 63 + shift;
        let mut m = if shift > 65 {
            0 // below a quarter of the smallest step: rounds to zero
        } else {
            let wide = self.mant as u128;
            let kept = wide >> shift;
            let rem = wide & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            let up = rem > half || (rem == half && kept & 1 == 1);
            (kept + up as u128) as u64
        };
        if m >> nbits != 0 {
            m >>= 1;
            e0 += 1;
        }
        if m != 0 && e0 + nbits as i32 - 1 > max_exp {
            return None;
        }
        Some((m, e0))
    }

    /// `fstps`: rounds to `float`.
    pub fn f(self) -> f32 {
        let sign = (self.neg as u32) << 31;
        match self.kind {
            Kind::Nan => return f32::NAN,
            Kind::Inf => return f32::from_bits(sign | 0x7f80_0000),
            Kind::Finite if self.mant == 0 => return f32::from_bits(sign),
            Kind::Finite => {}
        }
        let Some((m, e0)) = self.round_bits(24, -126, 127) else { return f32::from_bits(sign | 0x7f80_0000) };
        if m == 0 {
            return f32::from_bits(sign);
        }
        if m >> 23 == 0 {
            // Subnormal: exponent field 0, bit 0 weighs 2^-149.
            return f32::from_bits(sign | m as u32);
        }
        let biased = (e0 + 23 + 127) as u32;
        f32::from_bits(sign | biased << 23 | (m as u32 & 0x7f_ffff))
    }

    /// `fstpl`: rounds to `double`.
    pub fn d(self) -> f64 {
        let sign = (self.neg as u64) << 63;
        match self.kind {
            Kind::Nan => return f64::NAN,
            Kind::Inf => return f64::from_bits(sign | 0x7ff0_0000_0000_0000),
            Kind::Finite if self.mant == 0 => return f64::from_bits(sign),
            Kind::Finite => {}
        }
        let Some((m, e0)) = self.round_bits(53, -1022, 1023) else { return f64::from_bits(sign | 0x7ff0_0000_0000_0000) };
        if m == 0 {
            return f64::from_bits(sign);
        }
        if m >> 52 == 0 {
            return f64::from_bits(sign | m);
        }
        let biased = (e0 + 52 + 1023) as u64;
        f64::from_bits(sign | biased << 52 | (m & 0xf_ffff_ffff_ffff))
    }

    /// `fistp` with the truncating control word GCC sets for a C `(int)` cast.
    pub fn trunc_i32(self) -> i32 {
        if self.kind != Kind::Finite {
            return i32::MIN; // the x87 "integer indefinite"
        }
        if self.mant == 0 || self.exp < 0 {
            return 0;
        }
        if self.exp > 31 {
            return i32::MIN;
        }
        let mag = self.mant >> (63 - self.exp);
        match (self.neg, i32::try_from(mag)) {
            (false, Ok(v)) => v,
            (true, _) if mag <= 1 << 31 => (mag as i64).wrapping_neg() as i32,
            _ => i32::MIN,
        }
    }

    fn add_signed(a: Ext, b: Ext, flip_b: bool) -> Ext {
        let bneg = b.neg ^ flip_b;
        match (a.kind, b.kind) {
            (Kind::Nan, _) | (_, Kind::Nan) => return Ext::NAN,
            (Kind::Inf, Kind::Inf) => return if a.neg == bneg { a } else { Ext::NAN },
            (Kind::Inf, _) => return a,
            (_, Kind::Inf) => return Ext::inf(bneg),
            _ => {}
        }
        if b.is_zero() {
            return if a.is_zero() { Ext::zero(a.neg && bneg) } else { a };
        }
        if a.is_zero() {
            return Ext { neg: bneg, ..b };
        }
        // 62 guard bits below the significand; a sticky LSB stands in for the rest.
        let (mut x, mut xn, mut y, mut yn) = (a, a.neg, b, bneg);
        if (y.exp, y.mant) > (x.exp, x.mant) {
            std::mem::swap(&mut x, &mut y);
            std::mem::swap(&mut xn, &mut yn);
        }
        let xm = (x.mant as u128) << 62;
        let d = (x.exp - y.exp) as u32;
        let ym = if d >= 126 {
            1
        } else {
            let full = (y.mant as u128) << 62;
            let shifted = full >> d;
            shifted | ((shifted << d) != full) as u128
        };
        let e = x.exp - 63 - 62;
        if xn == yn {
            Ext::pack(xn, e, xm + ym, false)
        } else {
            let r = xm - ym;
            if r == 0 { Ext::ZERO } else { Ext::pack(xn, e, r, false) }
        }
    }

    pub fn sqrt(self) -> Ext {
        match self.kind {
            Kind::Nan => return Ext::NAN,
            Kind::Inf if self.neg => return Ext::NAN,
            Kind::Inf => return self,
            Kind::Finite => {}
        }
        if self.mant == 0 {
            return self;
        }
        if self.neg {
            return Ext::NAN;
        }
        // sqrt(R * 2^E) with E even; the root of R * 2^68 has 66 bits.
        let mut r = self.mant as u128;
        let mut e = self.exp - 63;
        if e & 1 != 0 {
            r <<= 1;
            e -= 1;
        }
        let (mut rem, mut root) = (0u128, 0u128);
        for i in (0..67).rev() {
            let pos = 2 * i - 68;
            let pair = if pos >= 0 { (r >> pos) & 3 } else { 0 };
            rem = (rem << 2) | pair;
            let t = (root << 2) | 1;
            if rem >= t {
                rem -= t;
                root = (root << 1) | 1;
            } else {
                root <<= 1;
            }
        }
        Ext::pack(false, (e - 68) / 2, root, rem != 0)
    }

    pub fn abs(self) -> Ext {
        Ext { neg: false, ..self }
    }

    /// `fsin` on a value that came from a `float`.
    pub fn sin(self) -> Ext {
        Ext::from_f64(self.d().sin())
    }

    /// `fcos` on a value that came from a `float`.
    pub fn cos(self) -> Ext {
        Ext::from_f64(self.d().cos())
    }
}

impl From<f32> for Ext {
    fn from(v: f32) -> Ext {
        Ext::from_f32(v)
    }
}

impl std::ops::Add for Ext {
    type Output = Ext;
    fn add(self, b: Ext) -> Ext {
        Ext::add_signed(self, b, false)
    }
}

impl std::ops::Sub for Ext {
    type Output = Ext;
    fn sub(self, b: Ext) -> Ext {
        Ext::add_signed(self, b, true)
    }
}

impl std::ops::Neg for Ext {
    type Output = Ext;
    fn neg(self) -> Ext {
        Ext { neg: !self.neg, ..self }
    }
}

impl std::ops::Mul for Ext {
    type Output = Ext;
    fn mul(self, b: Ext) -> Ext {
        let neg = self.neg ^ b.neg;
        match (self.kind, b.kind) {
            (Kind::Nan, _) | (_, Kind::Nan) => Ext::NAN,
            (Kind::Inf, _) | (_, Kind::Inf) => {
                if self.is_zero() || b.is_zero() {
                    Ext::NAN
                } else {
                    Ext::inf(neg)
                }
            }
            _ if self.mant == 0 || b.mant == 0 => Ext::zero(neg),
            _ => Ext::pack(neg, self.exp + b.exp - 126, self.mant as u128 * b.mant as u128, false),
        }
    }
}

impl std::ops::Div for Ext {
    type Output = Ext;
    fn div(self, b: Ext) -> Ext {
        let neg = self.neg ^ b.neg;
        match (self.kind, b.kind) {
            (Kind::Nan, _) | (_, Kind::Nan) | (Kind::Inf, Kind::Inf) => return Ext::NAN,
            (Kind::Inf, _) => return Ext::inf(neg),
            (_, Kind::Inf) => return Ext::zero(neg),
            _ => {}
        }
        if b.mant == 0 {
            return if self.mant == 0 { Ext::NAN } else { Ext::inf(neg) };
        }
        if self.mant == 0 {
            return Ext::zero(neg);
        }
        // 66 quotient bits by restoring division; the remainder is the sticky bit.
        let (a, d) = (self.mant as u128, b.mant as u128);
        let (mut q, mut r) = (0u128, a);
        for _ in 0..66 {
            q <<= 1;
            if r >= d {
                r -= d;
                q |= 1;
            }
            r <<= 1;
        }
        Ext::pack(neg, self.exp - b.exp - 65, q, r != 0)
    }
}

impl PartialEq for Ext {
    fn eq(&self, b: &Ext) -> bool {
        self.partial_cmp(b) == Some(Ordering::Equal)
    }
}

impl PartialOrd for Ext {
    /// `fcom`: `None` is "unordered" (a NaN).
    fn partial_cmp(&self, b: &Ext) -> Option<Ordering> {
        if self.is_nan() || b.is_nan() {
            return None;
        }
        let key = |v: &Ext| -> (i32, i32, u64) {
            // Sort key for non-negative magnitudes; infinities above all finite values.
            match v.kind {
                Kind::Inf => (1, 0, 0),
                _ if v.mant == 0 => (-1, 0, 0),
                _ => (0, v.exp, v.mant),
            }
        };
        let (az, bz) = (self.is_zero(), b.is_zero());
        if az && bz {
            return Some(Ordering::Equal);
        }
        let an = self.neg && !az;
        let bn = b.neg && !bz;
        Some(match (an, bn) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => key(self).cmp(&key(b)),
            (true, true) => key(b).cmp(&key(self)),
        })
    }
}

/// `flds`: a `float` operand.
#[cfg(test)]
pub fn x(v: f32) -> Ext {
    Ext::from_f32(v)
}

/// `fldl`: a `double` constant.
#[cfg(test)]
pub fn xd(v: f64) -> Ext {
    Ext::from_f64(v)
}

impl Ext {
    /// The 10-byte x87 memory format (`fstp tbyte`), for the hardware cross-check in the tests.
    #[cfg(all(test, target_arch = "x86_64"))]
    pub(crate) fn to_bits80(self) -> [u8; 10] {
        let (exp, mant): (u16, u64) = match self.kind {
            Kind::Nan => (0x7fff, 0xc000_0000_0000_0000),
            Kind::Inf => (0x7fff, 1 << 63),
            Kind::Finite if self.mant == 0 => (0, 0),
            Kind::Finite => ((self.exp + 16383) as u16, self.mant),
        };
        let se = exp | (self.neg as u16) << 15;
        let mut b = [0u8; 10];
        b[..8].copy_from_slice(&mant.to_le_bytes());
        b[8..].copy_from_slice(&se.to_le_bytes());
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for v in [0.0f32, -0.0, 1.0, -1.5, 0.1, 3.4e38, 1e-40, f32::MIN_POSITIVE, 236.643_19, 1.0e-45] {
            assert_eq!(x(v).f().to_bits(), v.to_bits(), "{v}");
            assert_eq!(x(v).d(), v as f64, "{v}");
        }
        for v in [0.001f64, 0.333, 0.7, 1e300, 5e-324] {
            assert_eq!(xd(v).d().to_bits(), v.to_bits(), "{v}");
        }
        assert!(x(f32::NAN).f().is_nan());
        assert_eq!(x(f32::INFINITY).f(), f32::INFINITY);
    }

    #[test]
    fn single_operations_match_f32() {
        // One operation on two floats, rounded once to float, is the correctly rounded result.
        let vals = [1.0f32, 3.0, 0.1, 0.7, -2.5, 1e-3, 123.456, 1e30, 7.0e-39, 330.0, 0.333];
        for &a in &vals {
            for &b in &vals {
                assert_eq!((x(a) + x(b)).f(), a + b, "{a}+{b}");
                assert_eq!((x(a) - x(b)).f(), a - b, "{a}-{b}");
                assert_eq!((x(a) * x(b)).f(), a * b, "{a}*{b}");
                assert_eq!((x(a) / x(b)).f(), a / b, "{a}/{b}");
            }
            assert_eq!(x(a).abs().sqrt().f(), a.abs().sqrt(), "sqrt {a}");
        }
    }

    #[test]
    fn extended_differs_from_double() {
        // 600 * 0.001 in extended precision is just above the double 0.6 (PM_Duck's test).
        let t = x(600.0) * xd(0.001);
        assert!(t > xd(0.6));
        assert!(std::hint::black_box(600.0f64) * 0.001 <= 0.6);
        assert_eq!(Ext::from_i32(-7).f(), -7.0);
        assert_eq!((x(1.0) / x(0.3) * x(400.0)).trunc_i32(), 1333);
    }
}
