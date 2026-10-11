//! The two TS builds of `pm_shared.c` and their floating point.
//!
//! TS 3.0 shipped movement three times: in the Linux server `ts_i386.so` (GCC 2.95) and in the
//! Windows `client.dll` and `mp.dll` (MSVC 6; their movement code is byte for byte the same,
//! apart from the client's spectator changes). Both compilers use the x87, but:
//!
//! * **GCC** ([`Gcc`]) runs the FPU at its Linux default, 64-bit precision, and rounds to `float`
//!   wherever the C stores (every `float` local is spilled). [`Ext`] emulates that.
//! * **MSVC** ([`Msvc`]) runs at 53-bit precision: the MSVC CRT of each DLL sets `_PC_53` when it
//!   loads (`_controlfp(_PC_53, _MCW_PC)` from `_fpmath`: client.dll 0x1006acdb → 0x1006d7c7,
//!   mp.dll 0x1007d2cf → 0x1007ea5a) and nothing in TS or the engines changes it afterwards
//!   (GoldSrc only switches to 64 bits inside `Sys_FloatTime` and restores the old word, FWGS
//!   never touches it). At 53 bits every x87 add/sub/mul/div/sqrt rounds exactly like an IEEE
//!   `double` (the wider exponent only matters beyond ±1e308), so the number type is `f64`. MSVC
//!   also keeps `float` locals on the FPU stack across statements, reassociates some products
//!   and uses `float` constants where GCC uses `double` ones, so the spill points differ: the
//!   movement code branches on [`Fpu::MSVC`] where the two machine codes differ.
//!
//! Each function names the operations in machine-code order with [`Fpu::x`] (`fld dword`, a
//! `float` operand), [`Fpu::xd`] (`fld qword`, a `double`), [`Fpu::xi`] (`fild`) and [`Num::f`]
//! (`fstp dword`, rounding to `float`). Both builds are monomorphised and picked once per call
//! by [`Arith`]: the client always runs `Msvc` on native `f64`; the server runs `Msvc` too by
//! default (`mp.dll`'s arithmetic, which every client predicts with) and `Gcc` when its
//! `ts_pm_arith` cvar (a port addition) is `gcc`.

pub use super::ext::Ext;
use std::fmt::Debug;
use std::ops::{Add, Div, Mul, Neg, Sub};

/// A value on the x87 stack under one build's precision.
pub trait Num:
    Copy + Debug + PartialOrd + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + Div<Output = Self> + Neg<Output = Self>
{
    fn from_f32(v: f32) -> Self;
    fn from_f64(v: f64) -> Self;
    fn from_i32(v: i32) -> Self;
    /// `fstp dword`: rounds to `float`.
    fn f(self) -> f32;
    fn sqrt(self) -> Self;
    fn abs(self) -> Self;
    /// `fsin` of an angle in radians, through `f64` (see [`Fpu`] for the limits).
    fn sin(self) -> Self;
    /// `fcos` of an angle in radians, through `f64`.
    fn cos(self) -> Self;
    /// A C `(int)` cast: a truncating `fistp` (GCC: dword, 0x80000000 when out of range; MSVC's
    /// `_ftol`: qword, keeping the low dword).
    fn trunc_i32(self) -> i32;
}

impl Num for Ext {
    fn from_f32(v: f32) -> Ext {
        Ext::from_f32(v)
    }
    fn from_f64(v: f64) -> Ext {
        Ext::from_f64(v)
    }
    fn from_i32(v: i32) -> Ext {
        Ext::from_i32(v)
    }
    fn f(self) -> f32 {
        Ext::f(self)
    }
    fn sqrt(self) -> Ext {
        Ext::sqrt(self)
    }
    fn abs(self) -> Ext {
        Ext::abs(self)
    }
    fn sin(self) -> Ext {
        Ext::sin(self)
    }
    fn cos(self) -> Ext {
        Ext::cos(self)
    }
    fn trunc_i32(self) -> i32 {
        Ext::trunc_i32(self)
    }
}

/// The x87 at 53-bit precision.
impl Num for f64 {
    fn from_f32(v: f32) -> f64 {
        v as f64
    }
    fn from_f64(v: f64) -> f64 {
        v
    }
    fn from_i32(v: i32) -> f64 {
        v as f64
    }
    fn f(self) -> f32 {
        // Round to nearest even, overflow to infinity, NaN stays NaN: the `fstp dword`.
        self as f32
    }
    fn sqrt(self) -> f64 {
        f64::sqrt(self)
    }
    fn abs(self) -> f64 {
        f64::abs(self)
    }
    fn sin(self) -> f64 {
        f64::sin(self)
    }
    fn cos(self) -> f64 {
        f64::cos(self)
    }
    /// MSVC's `_ftol` (client 0x1006ad1c): a truncating `fistp qword`, then the low dword.
    fn trunc_i32(self) -> i32 {
        let t = self.trunc();
        // Out of the 64-bit range or NaN: the "integer indefinite" 0x8000000000000000, low dword 0
        // (Rust's `as` would saturate).
        let two63 = (1u64 << 63) as f64;
        if (-two63..two63).contains(&t) { t as i64 as i32 } else { 0 }
    }
}

/// One TS build's floating point.
pub trait Fpu: 'static {
    type N: Num;
    /// True for the MSVC builds (`client.dll`, `mp.dll`).
    const MSVC: bool;
    const ZERO: Self::N;

    /// `fld dword`: a `float` operand.
    #[inline]
    fn x(v: f32) -> Self::N {
        Self::N::from_f32(v)
    }

    /// `fld qword`: a `double` constant.
    #[inline]
    fn xd(v: f64) -> Self::N {
        Self::N::from_f64(v)
    }

    /// `fild`.
    #[inline]
    fn xi(v: i32) -> Self::N {
        Self::N::from_i32(v)
    }
}

/// `ts_i386.so`: GCC 2.95, x87 extended precision.
pub enum Gcc {}

impl Fpu for Gcc {
    type N = Ext;
    const MSVC: bool = false;
    const ZERO: Ext = Ext::ZERO;
}

/// `client.dll` / `mp.dll`: MSVC 6, x87 at 53-bit precision.
pub enum Msvc {}

impl Fpu for Msvc {
    type N = f64;
    const MSVC: bool = true;
    const ZERO: f64 = 0.0;
}

/// Which TS build's arithmetic movement reproduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arith {
    /// `ts_i386.so` (GCC, x87 extended precision).
    Gcc,
    /// `client.dll` and `mp.dll` (MSVC, x87 at 53-bit precision).
    Msvc,
}

impl Arith {
    /// The server's arithmetic unless the port's `ts_pm_arith` cvar says otherwise: `mp.dll`'s on
    /// every platform, so the server agrees bit for bit with every client's prediction (TS's
    /// `client.dll` and this port's). `ts_pm_arith gcc` selects `ts_i386.so`'s instead.
    pub const SERVER_DEFAULT: Arith = Arith::Msvc;
    /// TS's only client, `client.dll`.
    pub const CLIENT: Arith = Arith::Msvc;

    /// The `ts_pm_arith` cvar's value (port addition): `gcc` (any case, surrounding blanks
    /// ignored) is `ts_i386.so`'s arithmetic; anything else, `msvc` included, is the default.
    pub fn from_cvar(value: &[u8]) -> Arith {
        if value.trim_ascii().eq_ignore_ascii_case(b"gcc") { Arith::Gcc } else { Arith::SERVER_DEFAULT }
    }
}
