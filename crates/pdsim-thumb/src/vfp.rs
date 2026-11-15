//! Single-precision floating point (FPv5-SP), bit-exact with Unicorn's
//! QEMU softfloat as configured for Arm: tininess before rounding, Arm NaN
//! propagation, FZ flushing inputs (IDC) and outputs (UFC), DN, and
//! FPSCR's four rounding modes.
//!
//! Every operation computes the exact result as `m * 2^e` in a `u128`
//! (or a sticky approximation that rounds the same way) and rounds once.

use crate::decode::{Op, Rm, K, V2, V3};
use crate::{err, Exit, ThumbCpu};
use std::cmp::Ordering;

/// The FPSCR bits M-profile keeps (QEMU's `vfp_set_fpscr`).
pub(crate) const FPSCR_MASK: u32 = 0xf7c0_009f;

const IOC: u32 = 1;
const DZC: u32 = 2;
const OFC: u32 = 4;
const UFC: u32 = 8;
const IXC: u32 = 16;
const IDC: u32 = 0x80;

const DNAN: u32 = 0x7fc0_0000;
const QUIET: u32 = 0x0040_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Nearest, ties to even.
    N,
    /// Nearest, ties away from zero.
    A,
    /// Towards +infinity.
    P,
    /// Towards -infinity.
    M,
    /// Towards zero.
    Z,
}

/// A destination format.
#[derive(Clone, Copy)]
struct Fmt {
    fbits: u32,
    bias: i32,
    /// The largest finite magnitude's bits.
    max: u32,
    /// Infinity's bits.
    inf: u32,
    /// Arm's alternative half precision: no infinities or NaNs.
    ahp: bool,
}

const F32: Fmt = Fmt {
    fbits: 23,
    bias: 127,
    max: 0x7f7f_ffff,
    inf: 0x7f80_0000,
    ahp: false,
};
const F16: Fmt = Fmt {
    fbits: 10,
    bias: 15,
    max: 0x7bff,
    inf: 0x7c00,
    ahp: false,
};
const AHP: Fmt = Fmt {
    fbits: 10,
    bias: 15,
    max: 0x7fff,
    inf: 0x7fff,
    ahp: true,
};

/// An unpacked single.
#[derive(Debug, Clone, Copy)]
enum Val {
    Zero(bool),
    Inf(bool),
    /// The single's bits and whether it signals.
    Nan(u32, bool),
    /// `(-1)^sign * m * 2^e`, `m != 0`.
    Num(bool, i32, u128),
}

fn zero(s: bool) -> u32 {
    (s as u32) << 31
}

fn inf(s: bool) -> u32 {
    (s as u32) << 31 | 0x7f80_0000
}

fn is_nan(x: u32) -> bool {
    x & 0x7fff_ffff > 0x7f80_0000
}

fn neg(x: u32) -> u32 {
    x ^ 0x8000_0000
}

/// Shifts `m` right by `sh`, rounding as `mode` says for a value of sign
/// `sign`: the result and whether it was inexact.
fn round_shift(m: u128, sh: u32, sign: bool, mode: Mode) -> (u128, bool) {
    if sh == 0 {
        return (m, false);
    }
    // Every `m` here is below 2^112, so beyond 120 it is under half.
    let (keep, rem, cmp) = if sh > 120 {
        (0, m, Ordering::Less)
    } else {
        let rem = m & ((1u128 << sh) - 1);
        (m >> sh, rem, rem.cmp(&(1u128 << (sh - 1))))
    };
    let inexact = rem != 0;
    let up = match mode {
        Mode::N => cmp == Ordering::Greater || (cmp == Ordering::Equal && keep & 1 != 0),
        Mode::A => cmp != Ordering::Less,
        Mode::P => inexact && !sign,
        Mode::M => inexact && sign,
        Mode::Z => false,
    };
    (keep + up as u128, inexact)
}

fn isqrt(n: u128) -> u128 {
    let mut rem = n;
    let mut x = 0u128;
    let mut bit = 1u128 << 126;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= x + bit {
            rem -= x + bit;
            x = (x >> 1) + bit;
        } else {
            x >>= 1;
        }
        bit >>= 2;
    }
    x
}

/// The state of one operation: FPSCR's controls and the flags raised.
struct Fp {
    mode: Mode,
    fz: bool,
    dn: bool,
    ahp: bool,
    flags: u32,
}

impl Fp {
    fn new(fpscr: u32) -> Fp {
        Fp {
            mode: [Mode::N, Mode::P, Mode::M, Mode::Z][(fpscr >> 22 & 3) as usize],
            fz: fpscr & 1 << 24 != 0,
            dn: fpscr & 1 << 25 != 0,
            ahp: fpscr & 1 << 26 != 0,
            flags: 0,
        }
    }

    fn mode(&self, rm: Rm) -> Mode {
        match rm {
            Rm::A => Mode::A,
            Rm::N => Mode::N,
            Rm::P => Mode::P,
            Rm::M => Mode::M,
            Rm::Z => Mode::Z,
            Rm::Fpscr => self.mode,
        }
    }

    /// Unpacks a single, flushing a denormal to zero (raising IDC) under FZ.
    fn unpack(&mut self, x: u32) -> Val {
        self.unpack_with(x, self.fz)
    }

    fn unpack_with(&mut self, x: u32, flush: bool) -> Val {
        let s = x >> 31 != 0;
        let e = (x >> 23 & 0xff) as i32;
        let f = x & 0x7f_ffff;
        match e {
            0 if f == 0 => Val::Zero(s),
            0 if flush => {
                self.flags |= IDC;
                Val::Zero(s)
            }
            0 => Val::Num(s, -149, f as u128),
            255 if f == 0 => Val::Inf(s),
            255 => Val::Nan(x, f & QUIET == 0),
            _ => Val::Num(s, e - 150, (f | 0x80_0000) as u128),
        }
    }

    /// Rounds `(-1)^sign * m * 2^e` into `fmt`.
    fn round(&mut self, sign: bool, e: i32, m: u128, f: Fmt, mode: Mode, ftz: bool) -> u32 {
        let s = (sign as u32) << (f.fbits + if f.fbits == 23 { 8 } else { 5 });
        if m == 0 {
            return s;
        }
        let p = 127 - m.leading_zeros() as i32;
        let fb = f.fbits as i32;
        let be = p + e + f.bias;
        let tiny = be < 1;
        if tiny && ftz {
            self.flags |= UFC;
            return s;
        }
        let shift = if tiny { p - fb + 1 - be } else { p - fb };
        let (keep, inexact) = if shift <= 0 {
            (m << -shift, false)
        } else {
            round_shift(m, shift as u32, sign, mode)
        };
        let bits = if tiny {
            keep as u64
        } else {
            (((be - 1) as u64) << fb) + keep as u64
        };
        if bits > f.max as u64 {
            if f.ahp {
                self.flags |= IOC;
                return s | f.max;
            }
            self.flags |= OFC | IXC;
            let to_inf = match mode {
                Mode::N | Mode::A => true,
                Mode::Z => false,
                Mode::P => !sign,
                Mode::M => sign,
            };
            return s | if to_inf { f.inf } else { f.max };
        }
        if inexact {
            self.flags |= IXC;
            if tiny {
                self.flags |= UFC;
            }
        }
        s | bits as u32
    }

    fn pack(&mut self, sign: bool, e: i32, m: u128) -> u32 {
        self.round(sign, e, m, F32, self.mode, self.fz)
    }

    fn invalid(&mut self) -> u32 {
        self.flags |= IOC;
        DNAN
    }

    /// The result of an operation on one NaN.
    fn nan1(&mut self, x: u32, snan: bool) -> u32 {
        if snan {
            self.flags |= IOC;
        }
        if self.dn {
            DNAN
        } else {
            x | QUIET
        }
    }

    /// Arm's NaN propagation for two operands: the first signalling NaN,
    /// else the first quiet one.
    fn nan2(&mut self, a: Val, b: Val) -> Option<u32> {
        let pick = match (a, b) {
            (Val::Nan(x, true), _) => x,
            (_, Val::Nan(y, true)) => y,
            (Val::Nan(x, _), _) => x,
            (_, Val::Nan(y, _)) => y,
            _ => return None,
        };
        let snan = matches!(a, Val::Nan(_, true)) || matches!(b, Val::Nan(_, true));
        Some(self.nan1(pick, snan))
    }

    /// The sum of two nonzero finite values.
    #[allow(clippy::too_many_arguments)]
    fn sum(&mut self, sa: bool, ea: i32, ma: u128, sb: bool, eb: i32, mb: u128) -> u32 {
        let ((sa, ea, ma), (sb, eb, mb)) = if ea >= eb {
            ((sa, ea, ma), (sb, eb, mb))
        } else {
            ((sb, eb, mb), (sa, ea, ma))
        };
        let d = ea - eb;
        // Far below the larger operand, the smaller one only matters as a
        // sticky bit.
        let (ma, mb, e) = if d > 60 {
            (ma << 62, 1, ea - 62)
        } else {
            (ma << d, mb, eb)
        };
        match (sa == sb, ma.cmp(&mb)) {
            (true, _) => self.pack(sa, e, ma + mb),
            (false, Ordering::Equal) => zero(self.mode == Mode::M),
            (false, Ordering::Greater) => self.pack(sa, e, ma - mb),
            (false, Ordering::Less) => self.pack(sb, e, mb - ma),
        }
    }

    fn add(&mut self, x: u32, y: u32) -> u32 {
        let (a, b) = (self.unpack(x), self.unpack(y));
        if let Some(n) = self.nan2(a, b) {
            return n;
        }
        match (a, b) {
            (Val::Inf(sa), Val::Inf(sb)) if sa != sb => self.invalid(),
            (Val::Inf(s), _) | (_, Val::Inf(s)) => inf(s),
            (Val::Zero(sa), Val::Zero(sb)) => zero(if sa == sb { sa } else { self.mode == Mode::M }),
            (Val::Zero(_), Val::Num(s, e, m)) | (Val::Num(s, e, m), Val::Zero(_)) => self.pack(s, e, m),
            (Val::Num(sa, ea, ma), Val::Num(sb, eb, mb)) => self.sum(sa, ea, ma, sb, eb, mb),
            _ => unreachable!(),
        }
    }

    /// `x - y`: a NaN `y` keeps its sign.
    fn sub(&mut self, x: u32, y: u32) -> u32 {
        if is_nan(x) || is_nan(y) {
            let (a, b) = (self.unpack(x), self.unpack(y));
            return self.nan2(a, b).unwrap();
        }
        self.add(x, neg(y))
    }

    fn mul(&mut self, x: u32, y: u32) -> u32 {
        let (a, b) = (self.unpack(x), self.unpack(y));
        if let Some(n) = self.nan2(a, b) {
            return n;
        }
        let s = (x ^ y) >> 31 != 0;
        match (a, b) {
            (Val::Inf(_), Val::Zero(_)) | (Val::Zero(_), Val::Inf(_)) => self.invalid(),
            (Val::Inf(_), _) | (_, Val::Inf(_)) => inf(s),
            (Val::Zero(_), _) | (_, Val::Zero(_)) => zero(s),
            (Val::Num(_, ea, ma), Val::Num(_, eb, mb)) => self.pack(s, ea + eb, ma * mb),
            _ => unreachable!(),
        }
    }

    fn div(&mut self, x: u32, y: u32) -> u32 {
        let (a, b) = (self.unpack(x), self.unpack(y));
        if let Some(n) = self.nan2(a, b) {
            return n;
        }
        let s = (x ^ y) >> 31 != 0;
        match (a, b) {
            (Val::Inf(_), Val::Inf(_)) | (Val::Zero(_), Val::Zero(_)) => self.invalid(),
            (Val::Inf(_), _) => inf(s),
            (_, Val::Inf(_)) | (Val::Zero(_), _) => zero(s),
            (_, Val::Zero(_)) => {
                self.flags |= DZC;
                inf(s)
            }
            (Val::Num(_, ea, ma), Val::Num(_, eb, mb)) => {
                let n = ma << 64;
                let (q, r) = (n / mb, n % mb);
                self.pack(s, ea - eb - 65, q << 1 | (r != 0) as u128)
            }
            _ => unreachable!(),
        }
    }

    fn sqrt(&mut self, x: u32) -> u32 {
        match self.unpack(x) {
            Val::Nan(x, sn) => self.nan1(x, sn),
            Val::Zero(s) => zero(s),
            Val::Inf(false) => inf(false),
            Val::Inf(true) | Val::Num(true, ..) => self.invalid(),
            Val::Num(false, mut e, mut m) => {
                if e & 1 != 0 {
                    m <<= 1;
                    e -= 1;
                }
                let n = m << 80;
                let r = isqrt(n);
                self.pack(false, (e - 80) / 2 - 1, r << 1 | (r * r != n) as u128)
            }
        }
    }

    /// `a * b + c`, rounded once.
    fn muladd(&mut self, x: u32, y: u32, z: u32) -> u32 {
        let (a, b, c) = (self.unpack(x), self.unpack(y), self.unpack(z));
        let infzero = matches!((a, b), (Val::Inf(_), Val::Zero(_)) | (Val::Zero(_), Val::Inf(_)));
        let nan = |v: Val, sig: bool| matches!(v, Val::Nan(_, s) if s == sig);
        if [a, b, c].iter().any(|v| matches!(v, Val::Nan(..))) {
            if infzero && nan(c, false) {
                return self.invalid();
            }
            let snan = nan(a, true) || nan(b, true) || nan(c, true);
            let order = [
                (c, true),
                (a, true),
                (b, true),
                (c, false),
                (a, false),
                (b, false),
            ];
            let pick = order
                .iter()
                .find_map(|&(v, sig)| match v {
                    Val::Nan(bits, s) if s == sig => Some(bits),
                    _ => None,
                })
                .unwrap();
            return self.nan1(pick, snan);
        }
        if infzero {
            return self.invalid();
        }
        let ps = (x ^ y) >> 31 != 0;
        match (a, b, c) {
            (Val::Inf(_), _, _) | (_, Val::Inf(_), _) => match c {
                Val::Inf(sc) if sc != ps => self.invalid(),
                _ => inf(ps),
            },
            (_, _, Val::Inf(sc)) => inf(sc),
            (Val::Zero(_), _, _) | (_, Val::Zero(_), _) => match c {
                Val::Zero(sc) => zero(if sc == ps { ps } else { self.mode == Mode::M }),
                Val::Num(sc, ec, mc) => self.pack(sc, ec, mc),
                _ => unreachable!(),
            },
            (Val::Num(_, ea, ma), Val::Num(_, eb, mb), Val::Zero(_)) => self.pack(ps, ea + eb, ma * mb),
            (Val::Num(_, ea, ma), Val::Num(_, eb, mb), Val::Num(sc, ec, mc)) => {
                self.sum(ps, ea + eb, ma * mb, sc, ec, mc)
            }
            _ => unreachable!(),
        }
    }

    /// VMAXNM / VMINNM (IEEE 754-2008 maxNum / minNum).
    fn minmax(&mut self, x: u32, y: u32, min: bool) -> u32 {
        let (a, b) = (self.unpack(x), self.unpack(y));
        let (an, bn) = (matches!(a, Val::Nan(..)), matches!(b, Val::Nan(..)));
        let snan = matches!(a, Val::Nan(_, true)) || matches!(b, Val::Nan(_, true));
        if an || bn {
            if !snan && an != bn {
                return self.canon(if an { b } else { a });
            }
            return self.nan2(a, b).unwrap();
        }
        let key = |v: Val| -> (bool, u64) {
            match v {
                Val::Zero(s) => (s, 0),
                Val::Inf(s) => (s, u64::MAX),
                Val::Num(s, e, m) => (s, ((e + 200) as u64) << 25 | m as u64),
                Val::Nan(..) => unreachable!(),
            }
        };
        let ((sa, ka), (sb, kb)) = (key(a), key(b));
        let pick_b = if sa != sb { sa ^ min } else { sa ^ (ka < kb) ^ min };
        self.canon(if pick_b { b } else { a })
    }

    /// A non-NaN value packed back (after any input flush).
    fn canon(&mut self, v: Val) -> u32 {
        match v {
            Val::Zero(s) => zero(s),
            Val::Inf(s) => inf(s),
            Val::Num(s, e, m) => self.pack(s, e, m),
            Val::Nan(..) => unreachable!(),
        }
    }

    /// VCMP(E): the NZCV flags.
    fn cmp(&mut self, x: u32, y: u32, signal: bool) -> u32 {
        let (a, b) = (self.unpack(x), self.unpack(y));
        let (an, bn) = (matches!(a, Val::Nan(..)), matches!(b, Val::Nan(..)));
        if an || bn {
            if signal || matches!(a, Val::Nan(_, true)) || matches!(b, Val::Nan(_, true)) {
                self.flags |= IOC;
            }
            return 0b0011;
        }
        let ord = |v: Val| -> i64 {
            let bits = self.peek(v);
            let mag = (bits & 0x7fff_ffff) as i64;
            if bits >> 31 != 0 {
                -mag
            } else {
                mag
            }
        };
        match ord(a).cmp(&ord(b)) {
            Ordering::Equal => 0b0110,
            Ordering::Less => 0b1000,
            Ordering::Greater => 0b0010,
        }
    }

    /// The bits of an exact non-NaN value (no flags).
    fn peek(&self, v: Val) -> u32 {
        match v {
            Val::Zero(s) => zero(s),
            Val::Inf(s) => inf(s),
            Val::Num(s, e, m) => {
                let m = m as u32;
                if e == -149 && m < 0x80_0000 {
                    zero(s) | m
                } else {
                    zero(s) | ((e + 150) as u32) << 23 | (m & 0x7f_ffff)
                }
            }
            Val::Nan(..) => unreachable!(),
        }
    }

    /// VRINT: rounds to an integral value; Inexact only for VRINTX.
    fn rint(&mut self, x: u32, mode: Mode, exact: bool) -> u32 {
        match self.unpack(x) {
            Val::Nan(x, sn) => self.nan1(x, sn),
            Val::Zero(s) => zero(s),
            Val::Inf(s) => inf(s),
            Val::Num(s, e, m) if e >= 0 => self.pack(s, e, m),
            Val::Num(s, e, m) => {
                let (k, inexact) = round_shift(m, (-e) as u32, s, mode);
                if inexact && exact {
                    self.flags |= IXC;
                }
                if k == 0 {
                    zero(s)
                } else {
                    self.pack(s, 0, k)
                }
            }
        }
    }

    /// Float to (fixed-point when `fbits > 0`) integer of `width` bits,
    /// saturating with Invalid; the result is extended to 32 bits.
    fn f2i(&mut self, x: u32, signed: bool, mode: Mode, fbits: u32, width: u32) -> u32 {
        if is_nan(x) {
            self.flags |= IOC;
            return 0;
        }
        let (max_pos, max_neg): (u128, u128) = if signed {
            ((1 << (width - 1)) - 1, 1 << (width - 1))
        } else {
            ((1 << width) - 1, 0)
        };
        let (s, mag, inexact) = match self.unpack(x) {
            Val::Zero(_) => return 0,
            Val::Inf(s) => (s, u128::MAX, false),
            Val::Num(s, e, m) => {
                let e = e + fbits as i32;
                if e >= 64 {
                    (s, u128::MAX, false)
                } else if e >= 0 {
                    (s, m << e, false)
                } else {
                    let (k, i) = round_shift(m, (-e) as u32, s, mode);
                    (s, k, i)
                }
            }
            Val::Nan(..) => unreachable!(),
        };
        let r = if s {
            if mag > max_neg {
                self.flags |= IOC;
                return (max_neg as i64).wrapping_neg() as u32;
            } else {
                (mag as i64).wrapping_neg() as u32
            }
        } else if mag > max_pos {
            self.flags |= IOC;
            return max_pos as u32;
        } else {
            mag as u32
        };
        if inexact {
            self.flags |= IXC;
        }
        r
    }

    /// Integer `v` scaled by 2^-fbits to single.
    fn i2f(&mut self, v: i64, fbits: u32) -> u32 {
        if v == 0 {
            return 0;
        }
        self.pack(v < 0, -(fbits as i32), v.unsigned_abs() as u128)
    }

    /// VCVTB/VCVTT half to single (no input flush).
    fn h2f(&mut self, h: u32) -> u32 {
        let s = h >> 15 != 0;
        let e = (h >> 10 & 0x1f) as i32;
        let f = h & 0x3ff;
        if e == 31 && !self.ahp {
            if f == 0 {
                return inf(s);
            }
            let bits = (s as u32) << 31 | 0x7f80_0000 | f << 13;
            return self.nan1(bits, f & 0x200 == 0);
        }
        match (e, f) {
            (0, 0) => zero(s),
            (0, _) => self.round(s, -24, f as u128, F32, self.mode, false),
            _ => self.round(s, e - 25, (f | 0x400) as u128, F32, self.mode, false),
        }
    }

    /// VCVTB/VCVTT single to half (no output flush).
    fn f2h(&mut self, x: u32) -> u32 {
        let s = (x >> 31) << 15;
        match self.unpack(x) {
            Val::Nan(..) if self.ahp => {
                self.flags |= IOC;
                s
            }
            Val::Nan(x, sn) => {
                if sn {
                    self.flags |= IOC;
                }
                if self.dn {
                    0x7e00
                } else {
                    s | 0x7e00 | (x & 0x7f_ffff) >> 13
                }
            }
            Val::Inf(_) if self.ahp => {
                self.flags |= IOC;
                s | 0x7fff
            }
            Val::Inf(_) => s | 0x7c00,
            Val::Zero(_) => s,
            Val::Num(sg, e, m) => {
                let f = if self.ahp { AHP } else { F16 };
                self.round(sg, e, m, f, self.mode, false)
            }
        }
    }
}

impl ThumbCpu {
    fn vaddr(&self, rn: u8) -> u32 {
        if rn == 15 {
            self.r[15] & !3
        } else {
            self.r[rn as usize]
        }
    }

    /// One 4- or 8-byte VFP load or store of `s[d]` (and `s[d + 1]`).
    fn vxfer(&mut self, load: bool, dbl: bool, d: u8, a: u32, pc: u32) -> Result<(), Exit> {
        let d = d as usize;
        if load {
            if dbl {
                if let Some(v) = self.access(self.mem.ld64(a), false, pc)? {
                    self.s[d] = v as u32;
                    self.s[d + 1] = (v >> 32) as u32;
                }
            } else if let Some(v) = self.access(self.mem.ld32(a), false, pc)? {
                self.s[d] = v;
            }
        } else if dbl {
            let v = self.s[d] as u64 | (self.s[d + 1] as u64) << 32;
            let r = self.mem.st64(a, v);
            self.access(r, true, pc)?;
        } else {
            let r = self.mem.st32(a, self.s[d]);
            self.access(r, true, pc)?;
        }
        Ok(())
    }

    /// Runs a floating-point instruction (after its IT check).
    pub(crate) fn exec_vfp(&mut self, op: &Op) -> Result<(), Exit> {
        // QEMU's lazy FP state: the first FP instruction creates a new FP
        // context with FPSCR from FPDSCR (zero).
        if !self.fpca {
            self.fpca = true;
            self.fpscr = 0;
        }
        let mut fp = Fp::new(self.fpscr);
        let s = self.s;
        match op.k {
            K::V3 { op: v, d, n, m } => {
                let (d, a, b) = (d as usize, s[n as usize], s[m as usize]);
                let c = s[d];
                self.s[d] = match v {
                    V3::Mla => {
                        let p = fp.mul(a, b);
                        fp.add(c, p)
                    }
                    V3::Mls => {
                        let p = fp.mul(a, b);
                        fp.add(c, neg(p))
                    }
                    V3::Nmls => {
                        let p = fp.mul(a, b);
                        fp.add(neg(c), p)
                    }
                    V3::Nmla => {
                        let p = fp.mul(a, b);
                        fp.add(neg(c), neg(p))
                    }
                    V3::Mul => fp.mul(a, b),
                    V3::Nmul => neg(fp.mul(a, b)),
                    V3::Add => fp.add(a, b),
                    V3::Sub => fp.sub(a, b),
                    V3::Div => fp.div(a, b),
                    V3::Fma { neg_n, neg_d } => {
                        let a = if neg_n { neg(a) } else { a };
                        let c = if neg_d { neg(c) } else { c };
                        fp.muladd(a, b, c)
                    }
                    V3::MaxNm => fp.minmax(a, b, false),
                    V3::MinNm => fp.minmax(a, b, true),
                    V3::Sel(cond) => {
                        if self.cond(cond) {
                            a
                        } else {
                            b
                        }
                    }
                };
            }
            K::V2 { op: v, d, m } => {
                let (d, x) = (d as usize, s[m as usize]);
                self.s[d] = match v {
                    V2::Mov => x,
                    V2::Abs => x & 0x7fff_ffff,
                    V2::Neg => neg(x),
                    V2::Sqrt => fp.sqrt(x),
                    V2::Rint(rm) => {
                        let mode = fp.mode(rm);
                        fp.rint(x, mode, false)
                    }
                    V2::RintX => fp.rint(x, fp.mode, true),
                    V2::ToInt { signed, rm } => {
                        let mode = fp.mode(rm);
                        fp.f2i(x, signed, mode, 0, 32)
                    }
                    V2::FromInt { signed } => {
                        let v = if signed { x as i32 as i64 } else { x as i64 };
                        fp.i2f(v, 0)
                    }
                    V2::FromHalf { top } => fp.h2f(if top { x >> 16 } else { x & 0xffff }),
                    V2::ToHalf { top } => {
                        let h = fp.f2h(x);
                        if top {
                            (s[d] & 0xffff) | h << 16
                        } else {
                            (s[d] & 0xffff_0000) | h
                        }
                    }
                };
            }
            K::VFix {
                to_fixed,
                signed,
                half,
                d,
                fbits,
            } => {
                let (d, x) = (d as usize, s[d as usize]);
                let width = if half { 16 } else { 32 };
                self.s[d] = if to_fixed {
                    fp.f2i(x, signed, Mode::Z, fbits as u32, width)
                } else {
                    let v = match (half, signed) {
                        (true, true) => x as i16 as i64,
                        (true, false) => (x & 0xffff) as i64,
                        (false, true) => x as i32 as i64,
                        (false, false) => x as i64,
                    };
                    fp.i2f(v, fbits as u32)
                };
            }
            K::VMovI { d, v } => self.s[d as usize] = v,
            K::VCmp { e, d, m } => {
                let y = if m == crate::decode::NOREG {
                    0
                } else {
                    s[m as usize]
                };
                let nzcv = fp.cmp(s[d as usize], y, e);
                self.fpscr = (self.fpscr & 0x0fff_ffff) | nzcv << 28;
            }
            K::VMovR { to_core, rt, n } => {
                if to_core {
                    self.r[rt as usize] = s[n as usize];
                } else {
                    self.s[n as usize] = self.r[rt as usize];
                }
            }
            K::VMov2 { to_core, rt, rt2, m } => {
                let m = m as usize;
                if to_core {
                    self.r[rt as usize] = s[m];
                    self.r[rt2 as usize] = s[m + 1];
                } else {
                    self.s[m] = self.r[rt as usize];
                    self.s[m + 1] = self.r[rt2 as usize];
                }
            }
            K::Vmrs { rt } => {
                if rt == 15 {
                    let f = self.fpscr;
                    self.n = f & 1 << 31 != 0;
                    self.z = f & 1 << 30 != 0;
                    self.c = f & 1 << 29 != 0;
                    self.v = f & 1 << 28 != 0;
                } else {
                    self.r[rt as usize] = self.fpscr;
                }
            }
            K::Vmsr { rt } => self.fpscr = self.r[rt as usize] & FPSCR_MASK,
            K::VLdr {
                load,
                dbl,
                d,
                rn,
                imm,
            } => {
                let a = self.vaddr(rn).wrapping_add(imm as u32);
                self.vxfer(load, dbl, d, a, op.pc)?;
            }
            K::VLdm {
                load,
                dbl,
                d,
                n,
                rn,
                db,
                wb,
            } => {
                let base = self.vaddr(rn);
                let bytes = 4 * n as u32;
                let start = if db { base.wrapping_sub(bytes) } else { base };
                let step = if dbl { 2 } else { 1 };
                for i in (0..n).step_by(step) {
                    let a = start.wrapping_add(4 * i as u32);
                    self.vxfer(load, dbl, d + i, a, op.pc)?;
                }
                if wb {
                    self.r[rn as usize] = if db { start } else { base.wrapping_add(bytes) };
                }
            }
            _ => return Err(err("INSN_INVALID", op.pc)),
        }
        self.fpscr |= fp.flags;
        Ok(())
    }
}
