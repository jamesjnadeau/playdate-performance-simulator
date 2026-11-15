//! Executing integer instructions: data processing and flags, loads and
//! stores, branches and IT, multiply and divide, bitfields, DSP and SIMD.
//! Floating point is in `vfp.rs`.

use crate::decode::{Alu, Am, Dsp, Ext, Idx, Misc, MulL, Op, Opnd, QOp, Sf, Sh, Sz, K, NOREG};
use crate::mem::Fault;
use crate::{err, Exit, ThumbCpu};

/// AddWithCarry: result, carry out, overflow.
#[inline]
fn add_c(a: u32, b: u32, c: bool) -> (u32, bool, bool) {
    let (s1, c1) = a.overflowing_add(b);
    let (r, c2) = s1.overflowing_add(c as u32);
    let v = ((a ^ r) & (b ^ r)) >> 31 != 0;
    (r, c1 | c2, v)
}

/// Shift_C. `n` is the amount (already decoded: 32 for an immediate LSR or
/// ASR #0, 1 for RRX); a register shift passes the bottom byte of `rs`.
#[inline]
pub(crate) fn shift_c(x: u32, t: Sh, n: u32, c: bool) -> (u32, bool) {
    if n == 0 {
        return (x, c);
    }
    match t {
        Sh::Lsl if n < 32 => (x << n, (x >> (32 - n)) & 1 != 0),
        Sh::Lsl if n == 32 => (0, x & 1 != 0),
        Sh::Lsl => (0, false),
        Sh::Lsr if n < 32 => (x >> n, (x >> (n - 1)) & 1 != 0),
        Sh::Lsr if n == 32 => (0, x >> 31 != 0),
        Sh::Lsr => (0, false),
        Sh::Asr if n < 32 => (((x as i32) >> n) as u32, (x >> (n - 1)) & 1 != 0),
        Sh::Asr => (((x as i32) >> 31) as u32, x >> 31 != 0),
        Sh::Ror => {
            let r = x.rotate_right(n % 32);
            (r, r >> 31 != 0)
        }
        Sh::Rrx => ((c as u32) << 31 | x >> 1, x & 1 != 0),
    }
}

/// Signed saturation to `bits` (1..=32): the value and whether it saturated.
fn ssat(x: i64, bits: u32) -> (u32, bool) {
    let max = (1i64 << (bits - 1)) - 1;
    let min = -(1i64 << (bits - 1));
    if x > max {
        (max as u32, true)
    } else if x < min {
        (min as u32, true)
    } else {
        (x as u32, false)
    }
}

/// Unsigned saturation to `bits` (0..=31).
fn usat(x: i64, bits: u32) -> (u32, bool) {
    let max = (1i64 << bits) - 1;
    if x > max {
        (max as u32, true)
    } else if x < 0 {
        (0, true)
    } else {
        (x as u32, false)
    }
}

/// The signed bottom (`top == false`) or top half of `x`.
#[inline]
fn half(x: u32, top: bool) -> i32 {
    if top {
        (x as i32) >> 16
    } else {
        x as i16 as i32
    }
}

/// Parallel add/subtract: `kind` is S, Q, SH (0..=2) or U, UQ, UH (4..=6);
/// `f` is ADD8, ADD16, ASX, -, SUB8, SUB16, SAX. Returns the result and,
/// for S and U, the new GE bits.
fn parallel(kind: u8, f: u8, n: u32, m: u32) -> (u32, Option<u8>) {
    let signed = kind < 4;
    let w: u32 = if f == 0 || f == 4 { 8 } else { 16 };
    let lanes = 32 / w;
    let mask = (1u32 << w) - 1;
    let get = |x: u32, i: u32| -> i64 {
        let v = (x >> (i * w)) & mask;
        if signed {
            ((v << (32 - w)) as i32 >> (32 - w)) as i64
        } else {
            v as i64
        }
    };
    let mut res = 0u32;
    let mut ge = 0u8;
    for i in 0..lanes {
        // (n lane, m lane, add?)
        let (mi, add) = match f {
            0 | 1 => (i, true),
            4 | 5 => (i, false),
            2 => (1 - i, i == 1), // ASX: lo = n.lo - m.hi, hi = n.hi + m.lo
            _ => (1 - i, i == 0), // SAX: lo = n.lo + m.hi, hi = n.hi - m.lo
        };
        let (a, b) = (get(n, i), get(m, mi));
        let r = if add { a + b } else { a - b };
        let (out, ok) = match kind {
            0 => (r as u32, r >= 0),
            4 => (r as u32, if add { r >= 1 << w } else { r >= 0 }),
            1 => (ssat(r, w).0, false),
            5 => (usat(r, w).0, false),
            _ => ((r >> 1) as u32, false),
        };
        res |= (out & mask) << (i * w);
        if ok {
            ge |= (if w == 8 { 1 } else { 3 }) << (i * w / 8);
        }
    }
    (res, (kind & 3 == 0).then_some(ge))
}

impl ThumbCpu {
    #[inline]
    pub(crate) fn cond(&self, c: u8) -> bool {
        let r = match c >> 1 {
            0 => self.z,
            1 => self.c,
            2 => self.n,
            3 => self.v,
            4 => self.c && !self.z,
            5 => self.n == self.v,
            6 => self.n == self.v && !self.z,
            _ => true,
        };
        if c & 1 != 0 && c != 15 {
            !r
        } else {
            r
        }
    }

    #[inline]
    fn reg(&self, r: u8) -> u32 {
        self.r[r as usize]
    }

    #[inline]
    fn operand(&self, b: Opnd) -> (u32, bool) {
        match b {
            Opnd::Imm(v, c) => (v, c.unwrap_or(self.c)),
            Opnd::Sh(rm, t, n) => shift_c(self.reg(rm), t, n as u32, self.c),
            Opnd::ShR(rm, t, rs) => shift_c(self.reg(rm), t, self.reg(rs) & 0xff, self.c),
        }
    }

    /// The address of a load/store and the base writeback value.
    #[inline]
    fn address(&self, am: &Am) -> (u32, Option<u32>) {
        let off = if am.rm == NOREG {
            am.imm as u32
        } else {
            self.reg(am.rm) << am.sh
        };
        let base = self.reg(am.rn);
        let a = base.wrapping_add(off);
        match am.idx {
            Idx::Off => (a, None),
            Idx::Pre => (a, Some(a)),
            Idx::Post => (base, Some(a)),
        }
    }

    /// Finishes a game access: a store fault inside an IT block is
    /// deferred, and a deferred fault is reported after any access outside
    /// one (see [`ThumbCpu::deferred`]).
    pub(crate) fn access<T>(&mut self, r: Result<T, Fault>, store: bool, pc: u32) -> Result<Option<T>, Exit> {
        match r {
            Err(f) if store && self.in_it => {
                self.deferred = Some(f);
                Ok(None)
            }
            // A failed load keeps the pending fault's kind.
            Err(f) => Err(Exit::Fault(
                self.deferred.take().filter(|_| !store).unwrap_or(f).at(pc),
            )),
            Ok(v) => match self.deferred {
                Some(f) if !self.in_it => {
                    self.deferred = None;
                    Err(Exit::Fault(f.at(pc)))
                }
                _ => Ok(Some(v)),
            },
        }
    }

    fn load(&mut self, sz: Sz, a: u32, pc: u32) -> Result<u32, Exit> {
        let v = match sz {
            Sz::B => self.mem.ld8(a),
            Sz::Sb => self.mem.ld8(a).map(|v| v as u8 as i8 as u32),
            Sz::H => self.mem.ld16(a),
            Sz::Sh => self.mem.ld16(a).map(|v| v as u16 as i16 as u32),
            Sz::W => self.mem.ld32(a),
        };
        Ok(self.access(v, false, pc)?.unwrap_or(0))
    }

    fn store(&mut self, sz: Sz, a: u32, v: u32, pc: u32) -> Result<(), Exit> {
        let r = match sz {
            Sz::B | Sz::Sb => self.mem.st8(a, v),
            Sz::H | Sz::Sh => self.mem.st16(a, v),
            Sz::W => self.mem.st32(a, v),
        };
        self.access(r, true, pc).map(drop)
    }

    /// Writes a value loaded into `rt`; a load into PC is an interworking
    /// branch.
    #[inline]
    fn set_loaded(&mut self, rt: u8, v: u32, npc: &mut u32) {
        if rt == 15 {
            self.thumb = v & 1 != 0;
            *npc = v & !1;
        } else {
            self.r[rt as usize] = v;
        }
    }

    fn set_nz(&mut self, v: u32) {
        self.n = v >> 31 != 0;
        self.z = v == 0;
    }

    /// Runs one instruction. On return `r[15]` holds the next PC (the
    /// faulting PC on a fault).
    pub(crate) fn exec(&mut self, op: &Op) -> Result<(), Exit> {
        let pc = op.pc;
        let mut npc = pc.wrapping_add(op.len as u32);
        self.r[15] = pc.wrapping_add(4);
        let it = self.it;
        let in_it = it & 0xf != 0;
        self.in_it = in_it;
        // QEMU creates a new FP context (FPSCR from FPDSCR, which is zero)
        // in the first floating-point instruction translated with FPCA
        // clear, behind that instruction's IT condition.
        let new_fp_ctxt = crate::tcg::is_fp(&op.k) && !std::mem::replace(&mut self.tb_fpca, true);
        if in_it {
            self.it = if it & 7 == 0 {
                0
            } else {
                (it & 0xe0) | ((it << 1) & 0x1f)
            };
            if !matches!(op.k, K::Bkpt) && !self.cond(it >> 4) {
                self.r[15] = npc;
                return Ok(());
            }
        }
        match op.k {
            K::Undef => return Err(err("INSN_INVALID", pc)),
            K::Nop | K::Barrier { .. } => {}
            K::It(v) => self.it = v,
            K::Alu { op: a, s, rd, rn, b } => {
                let setf = match s {
                    Sf::No => false,
                    Sf::Yes => true,
                    Sf::OutIt => !in_it,
                };
                let (bv, sc) = self.operand(b);
                let x = self.reg(rn);
                let (res, c, v) = match a {
                    Alu::And | Alu::Tst => (x & bv, sc, self.v),
                    Alu::Eor | Alu::Teq => (x ^ bv, sc, self.v),
                    Alu::Orr => (x | bv, sc, self.v),
                    Alu::Orn => (x | !bv, sc, self.v),
                    Alu::Bic => (x & !bv, sc, self.v),
                    Alu::Mov => (bv, sc, self.v),
                    Alu::Mvn => (!bv, sc, self.v),
                    Alu::Add | Alu::Cmn => add_c(x, bv, false),
                    Alu::Adc => add_c(x, bv, self.c),
                    Alu::Sub | Alu::Cmp => add_c(x, !bv, true),
                    Alu::Sbc => add_c(x, !bv, self.c),
                    Alu::Rsb => add_c(!x, bv, true),
                };
                if !a.is_test() {
                    if rd == 15 {
                        npc = res & !1;
                    } else {
                        self.r[rd as usize] = res;
                    }
                }
                if setf {
                    self.set_nz(res);
                    self.c = c;
                    self.v = v;
                }
            }
            K::Adr { rd, v } => self.r[rd as usize] = v,
            K::Movw { rd, imm } => self.r[rd as usize] = imm as u32,
            K::Movt { rd, imm } => self.r[rd as usize] = (self.reg(rd) & 0xffff) | (imm as u32) << 16,
            K::Mul { s, rd, rn, rm } => {
                let v = self.reg(rn).wrapping_mul(self.reg(rm));
                self.r[rd as usize] = v;
                if s == Sf::Yes || (s == Sf::OutIt && !in_it) {
                    self.set_nz(v);
                }
            }
            K::Mla { sub, rd, rn, rm, ra } => {
                let p = self.reg(rn).wrapping_mul(self.reg(rm));
                let a = self.reg(ra);
                self.r[rd as usize] = if sub { a.wrapping_sub(p) } else { a.wrapping_add(p) };
            }
            K::MulL {
                op: m,
                lo,
                hi,
                rn,
                rm,
            } => {
                let (n, mm) = (self.reg(rn), self.reg(rm));
                let acc = (self.reg(hi) as u64) << 32 | self.reg(lo) as u64;
                let s64 = |x: u32| x as i32 as i64;
                let r: u64 = match m {
                    MulL::Smull => (s64(n) * s64(mm)) as u64,
                    MulL::Umull => n as u64 * mm as u64,
                    MulL::Smlal => acc.wrapping_add((s64(n) * s64(mm)) as u64),
                    MulL::Umlal => acc.wrapping_add(n as u64 * mm as u64),
                    MulL::Umaal => n as u64 * mm as u64 + self.reg(lo) as u64 + self.reg(hi) as u64,
                    MulL::SmlalXy(nt, mt) => {
                        acc.wrapping_add((half(n, nt) as i64 * half(mm, mt) as i64) as u64)
                    }
                    MulL::Smlald(x) | MulL::Smlsld(x) => {
                        let mm = if x { mm.rotate_right(16) } else { mm };
                        let p1 = half(n, false) as i64 * half(mm, false) as i64;
                        let p2 = half(n, true) as i64 * half(mm, true) as i64;
                        let p = if matches!(m, MulL::Smlald(_)) {
                            p1 + p2
                        } else {
                            p1 - p2
                        };
                        acc.wrapping_add(p as u64)
                    }
                };
                self.r[lo as usize] = r as u32;
                self.r[hi as usize] = (r >> 32) as u32;
            }
            K::Div { signed, rd, rn, rm } => {
                let (n, m) = (self.reg(rn), self.reg(rm));
                self.r[rd as usize] = if m == 0 {
                    0
                } else if signed {
                    (n as i32).wrapping_div(m as i32) as u32
                } else {
                    n / m
                };
            }
            K::Dsp {
                op: d,
                rd,
                rn,
                rm,
                ra,
            } => {
                let (n, m) = (self.reg(rn), self.reg(rm));
                let acc = (ra != 15).then(|| self.reg(ra));
                let r = match d {
                    Dsp::SmlaXy(nt, mt) => self.acc_q(half(n, nt) as i64 * half(m, mt) as i64, acc),
                    Dsp::SmlaW(mt) => self.acc_q((n as i32 as i64 * half(m, mt) as i64) >> 16, acc),
                    Dsp::Smlad(x) | Dsp::Smlsd(x) => {
                        let m = if x { m.rotate_right(16) } else { m };
                        let p1 = half(n, false) as i64 * half(m, false) as i64;
                        let p2 = half(n, true) as i64 * half(m, true) as i64;
                        let p = if matches!(d, Dsp::Smlad(_)) {
                            p1 + p2
                        } else {
                            p1 - p2
                        };
                        self.acc_q(p, acc)
                    }
                    Dsp::Smmla(round) | Dsp::Smmls(round) => {
                        let p = (n as i32 as i64).wrapping_mul(m as i32 as i64);
                        let a = (acc.unwrap_or(0) as i64) << 32;
                        let mut r = if matches!(d, Dsp::Smmla(_)) {
                            a.wrapping_add(p)
                        } else {
                            a.wrapping_sub(p)
                        };
                        if round {
                            r = r.wrapping_add(0x8000_0000);
                        }
                        (r >> 32) as u32
                    }
                    Dsp::Usada8 => {
                        let mut s = acc.unwrap_or(0);
                        for i in 0..4 {
                            let a = (n >> (8 * i)) & 0xff;
                            let b = (m >> (8 * i)) & 0xff;
                            s = s.wrapping_add(a.abs_diff(b));
                        }
                        s
                    }
                };
                self.r[rd as usize] = r;
            }
            K::Sat {
                signed,
                rd,
                rn,
                to,
                sh,
                n,
            } => {
                let x = shift_c(self.reg(rn), sh, n as u32, false).0 as i32 as i64;
                let (r, q) = if signed {
                    ssat(x, to as u32)
                } else {
                    usat(x, to as u32)
                };
                self.r[rd as usize] = r;
                self.q |= q;
            }
            K::Sat16 { signed, rd, rn, to } => {
                let x = self.reg(rn);
                let mut r = 0;
                for i in 0..2 {
                    let h = half(x, i == 1) as i64;
                    let (v, q) = if signed {
                        ssat(h, to as u32)
                    } else {
                        usat(h, to as u32)
                    };
                    self.q |= q;
                    r |= (v & 0xffff) << (16 * i);
                }
                self.r[rd as usize] = r;
            }
            K::Bfx {
                signed,
                rd,
                rn,
                lsb,
                width,
            } => {
                let x = self.reg(rn) >> lsb;
                let w = width as u32;
                self.r[rd as usize] = if w == 32 {
                    x
                } else if signed {
                    (((x << (32 - w)) as i32) >> (32 - w)) as u32
                } else {
                    x & ((1 << w) - 1)
                };
            }
            K::Bfi { rd, rn, lsb, msb } => {
                let w = (msb - lsb) as u32 + 1;
                let mask = (if w == 32 { u32::MAX } else { (1 << w) - 1 }) << lsb;
                let src = if rn == 15 { 0 } else { self.reg(rn) << lsb };
                self.r[rd as usize] = (self.reg(rd) & !mask) | (src & mask);
            }
            K::Ext {
                op: e,
                rd,
                rn,
                rm,
                rot,
            } => {
                let x = self.reg(rm).rotate_right(rot as u32);
                let add = if rn == 15 { 0 } else { self.reg(rn) };
                self.r[rd as usize] = match e {
                    Ext::Sxtb => add.wrapping_add(x as u8 as i8 as u32),
                    Ext::Sxth => add.wrapping_add(x as u16 as i16 as u32),
                    Ext::Uxtb => add.wrapping_add(x & 0xff),
                    Ext::Uxth => add.wrapping_add(x & 0xffff),
                    Ext::Sxtb16 | Ext::Uxtb16 => {
                        let ext = |b: u32| {
                            if e == Ext::Sxtb16 {
                                b as u8 as i8 as u32
                            } else {
                                b & 0xff
                            }
                        };
                        let lo = add.wrapping_add(ext(x)) & 0xffff;
                        let hi = (add >> 16).wrapping_add(ext(x >> 16)) & 0xffff;
                        hi << 16 | lo
                    }
                };
            }
            K::Misc { op: o, rd, rm } => {
                let x = self.reg(rm);
                self.r[rd as usize] = match o {
                    Misc::Clz => x.leading_zeros(),
                    Misc::Rbit => x.reverse_bits(),
                    Misc::Rev => x.swap_bytes(),
                    Misc::Rev16 => (x & 0xff00_ff00) >> 8 | (x & 0x00ff_00ff) << 8,
                    Misc::Revsh => (x as u16).swap_bytes() as i16 as u32,
                };
            }
            K::Sel { rd, rn, rm } => {
                let mut mask = 0u32;
                for i in 0..4 {
                    if self.ge & (1 << i) != 0 {
                        mask |= 0xff << (8 * i);
                    }
                }
                self.r[rd as usize] = (self.reg(rn) & mask) | (self.reg(rm) & !mask);
            }
            K::Par { kind, f, rd, rn, rm } => {
                let (r, ge) = parallel(kind, f, self.reg(rn), self.reg(rm));
                self.r[rd as usize] = r;
                if let Some(ge) = ge {
                    self.ge = ge;
                }
            }
            K::Q { op: o, rd, rn, rm } => {
                let m = self.reg(rm) as i32 as i64;
                let mut n = self.reg(rn) as i32 as i64;
                if matches!(o, QOp::Qdadd | QOp::Qdsub) {
                    let (d, q) = ssat(2 * n, 32);
                    self.q |= q;
                    n = d as i32 as i64;
                }
                let r = if matches!(o, QOp::Qadd | QOp::Qdadd) {
                    m + n
                } else {
                    m - n
                };
                let (r, q) = ssat(r, 32);
                self.q |= q;
                self.r[rd as usize] = r;
            }
            K::Pkh { tb, rd, rn, rm, n } => {
                let (x, m) = (self.reg(rn), self.reg(rm));
                self.r[rd as usize] = if tb {
                    let s = shift_c(m, Sh::Asr, if n == 0 { 32 } else { n as u32 }, false).0;
                    (x & 0xffff_0000) | (s & 0xffff)
                } else {
                    (x & 0xffff) | ((m << n) & 0xffff_0000)
                };
            }
            K::Ld { sz, rt, am } => {
                let (a, wb) = self.address(&am);
                let v = self.load(sz, a, pc)?;
                if let Some(w) = wb {
                    self.r[am.rn as usize] = w;
                }
                self.set_loaded(rt, v, &mut npc);
            }
            K::St { sz, rt, am } => {
                let (a, wb) = self.address(&am);
                self.store(sz, a, self.reg(rt), pc)?;
                if let Some(w) = wb {
                    self.r[am.rn as usize] = w;
                }
            }
            K::LdLit { sz, rt, addr } => {
                let v = self.load(sz, addr, pc)?;
                self.set_loaded(rt, v, &mut npc);
            }
            K::Ldrd { rt, rt2, am } => {
                let (a, wb) = self.address(&am);
                let v = self.load(Sz::W, a, pc)?;
                self.r[rt as usize] = v;
                let v2 = self.load(Sz::W, a.wrapping_add(4), pc)?;
                self.r[rt2 as usize] = v2;
                if let Some(w) = wb {
                    self.r[am.rn as usize] = w;
                }
            }
            K::LdrdLit { rt, rt2, addr } => {
                let v = self.load(Sz::W, addr, pc)?;
                self.r[rt as usize] = v;
                let v2 = self.load(Sz::W, addr.wrapping_add(4), pc)?;
                self.r[rt2 as usize] = v2;
            }
            K::Strd { rt, rt2, am } => {
                let (a, wb) = self.address(&am);
                self.store(Sz::W, a, self.reg(rt), pc)?;
                self.store(Sz::W, a.wrapping_add(4), self.reg(rt2), pc)?;
                if let Some(w) = wb {
                    self.r[am.rn as usize] = w;
                }
            }
            K::Ldm { rn, list, wb, db } => {
                let count = list.count_ones();
                let base = self.reg(rn);
                let mut a = if db { base.wrapping_sub(4 * count) } else { base };
                let mut loaded_base = None;
                for i in 0..16u8 {
                    if list & (1 << i) == 0 {
                        continue;
                    }
                    let v = self.load(Sz::W, a, pc)?;
                    if i == rn {
                        loaded_base = Some(v);
                    } else {
                        self.set_loaded(i, v, &mut npc);
                    }
                    a = a.wrapping_add(4);
                }
                if wb {
                    self.r[rn as usize] = if db { base.wrapping_sub(4 * count) } else { a };
                }
                if let Some(v) = loaded_base {
                    self.r[rn as usize] = v;
                }
            }
            K::Stm { rn, list, wb, db } => {
                let count = list.count_ones();
                let base = self.reg(rn);
                let start = if db { base.wrapping_sub(4 * count) } else { base };
                let mut a = start;
                for i in 0..16u8 {
                    if list & (1 << i) != 0 {
                        self.store(Sz::W, a, self.reg(i), pc)?;
                        a = a.wrapping_add(4);
                    }
                }
                if wb {
                    self.r[rn as usize] = if db { start } else { a };
                }
            }
            K::Ldrex { sz, rt, rn, imm } => {
                let a = self.reg(rn).wrapping_add(imm as u32);
                if a & (size_of(sz) - 1) != 0 {
                    return Err(err("EXCEPTION", pc));
                }
                let v = self.load(sz, a, pc)?;
                self.excl = Some((a, v));
                self.r[rt as usize] = v;
            }
            K::Strex { sz, rd, rt, rn, imm } => {
                let a = self.reg(rn).wrapping_add(imm as u32);
                // QEMU compares with the monitor first: only a store that
                // would happen checks alignment.
                let r = match self.excl.take() {
                    Some((ea, ev)) if ea == a => {
                        if a & (size_of(sz) - 1) != 0 {
                            self.excl = Some((ea, ev));
                            return Err(err("EXCEPTION", pc));
                        }
                        let old = self.load(sz, a, pc)?;
                        let keep = (1u64 << (8 * size_of(sz))) - 1;
                        if old as u64 == ev as u64 & keep {
                            self.store(sz, a, self.reg(rt), pc)?;
                        }
                        (old != ev) as u32
                    }
                    _ => 1,
                };
                self.r[rd as usize] = r;
            }
            K::Clrex => self.excl = None,
            K::Tb { half, rn, rm } => {
                let base = self.reg(rn);
                let off = if half {
                    self.load(Sz::H, base.wrapping_add(self.reg(rm) << 1), pc)?
                } else {
                    self.load(Sz::B, base.wrapping_add(self.reg(rm)), pc)?
                };
                npc = pc.wrapping_add(4).wrapping_add(off * 2);
            }
            K::B { t } => npc = t,
            K::Bc { cond, t } => {
                if self.cond(cond) {
                    npc = t;
                }
            }
            K::Bl { t } => {
                self.r[14] = npc | 1;
                npc = t;
            }
            K::BlxI { t } => {
                self.r[14] = npc | 1;
                self.thumb = false;
                npc = t;
            }
            K::Bx { rm } => {
                let v = self.reg(rm);
                self.thumb = v & 1 != 0;
                npc = v & !1;
            }
            K::Blx { rm } => {
                let v = self.reg(rm);
                self.r[14] = npc | 1;
                self.thumb = v & 1 != 0;
                npc = v & !1;
            }
            K::Cbz { nz, rn, t } => {
                if (self.reg(rn) != 0) == nz {
                    npc = t;
                }
            }
            K::Svc => return Err(err("EXCEPTION", npc)),
            K::Bkpt => return Err(err("EXCEPTION", pc)),
            K::Yield => return Err(err("INSN_INVALID", npc)),
            K::Wfi => {
                self.r[15] = npc;
                return Err(Exit::Halt);
            }
            K::Mrs { rd, sysm } => {
                self.r[rd as usize] = match sysm {
                    0..=3 => self.apsr() & 0xf80f_0000,
                    8 => self.r[13],
                    16 => self.primask as u32,
                    17 | 18 => self.basepri as u32,
                    19 => self.faultmask as u32,
                    20 => (self.fpca as u32) << 2,
                    _ => 0,
                };
            }
            K::Msr { rn, sysm, mask } => {
                let v = self.reg(rn);
                match sysm {
                    0..=3 => {
                        if mask & 2 != 0 {
                            self.set_nzcvq(v);
                        }
                        if mask & 1 != 0 {
                            self.ge = ((v >> 16) & 0xf) as u8;
                        }
                    }
                    8 => self.r[13] = v,
                    16 => self.primask = v & 1 != 0,
                    17 => self.basepri = v as u8,
                    18 => {
                        let b = v as u8;
                        if b != 0 && (b < self.basepri || self.basepri == 0) {
                            self.basepri = b;
                        }
                    }
                    19 => self.faultmask = v & 1 != 0,
                    20 => self.fpca = v & 4 != 0,
                    _ => {}
                }
            }
            K::Cps { disable, i, f } => {
                if i {
                    self.primask = disable;
                }
                if f {
                    self.faultmask = disable;
                }
            }
            _ => {
                if new_fp_ctxt {
                    self.fpca = true;
                    self.fpscr = 0;
                }
                self.exec_vfp(op)?
            }
        }
        self.r[15] = npc;
        Ok(())
    }

    /// Accumulates a DSP product into `acc` (if any); sets Q when the sum
    /// overflows 32 bits.
    fn acc_q(&mut self, p: i64, acc: Option<u32>) -> u32 {
        let r = p + acc.map_or(0, |a| a as i32 as i64);
        if r != r as i32 as i64 {
            self.q = true;
        }
        r as u32
    }
}

fn size_of(sz: Sz) -> u32 {
    match sz {
        Sz::B | Sz::Sb => 1,
        Sz::H | Sz::Sh => 2,
        Sz::W => 4,
    }
}
