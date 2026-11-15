//! Thumb / Thumb-2 decoding into [`Op`]s, and splitting code into
//! [`Block`]s the way QEMU (Unicorn) splits it into translation blocks.
//!
//! The decoder follows the encoding tables of the Armv7-M Architecture
//! Reference Manual (A5.2 16-bit, A5.3 32-bit, A6/A7 floating point): one
//! `match` on the top five bits of the first halfword, then a sub-decoder
//! per table. Encodings the M7 doesn't have (and the ones QEMU chooses to
//! UNDEF) decode to [`K::Undef`], which faults with `INSN_INVALID` when it
//! runs.

use crate::mem::{Mem, PAGE};
use crate::tcg::{Budget, MAX_INSNS};
use pdsim_cpu::CpuError;

/// When a data-processing instruction sets the flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sf {
    No,
    Yes,
    /// 16-bit encodings: only outside an IT block.
    OutIt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sh {
    Lsl,
    Lsr,
    Asr,
    Ror,
    Rrx,
}

impl Sh {
    fn from_bits(t: u32) -> Sh {
        [Sh::Lsl, Sh::Lsr, Sh::Asr, Sh::Ror][t as usize & 3]
    }
}

/// The second operand of a data-processing instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opnd {
    /// A constant, with the carry out of `ThumbExpandImm_C` if it has one.
    Imm(u32, Option<bool>),
    /// `rm` shifted by a constant.
    Sh(u8, Sh, u8),
    /// `rm` shifted by the bottom byte of `rs`.
    ShR(u8, Sh, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alu {
    And,
    Eor,
    Orr,
    Orn,
    Bic,
    Mov,
    Mvn,
    Add,
    Adc,
    Sub,
    Sbc,
    Rsb,
    Tst,
    Teq,
    Cmp,
    Cmn,
}

impl Alu {
    /// Whether it only sets flags.
    pub fn is_test(self) -> bool {
        matches!(self, Alu::Tst | Alu::Teq | Alu::Cmp | Alu::Cmn)
    }
}

/// Size and signedness of a load or store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sz {
    B,
    Sb,
    H,
    Sh,
    W,
}

/// Addressing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idx {
    /// `[rn, off]`
    Off,
    /// `[rn, off]!`
    Pre,
    /// `[rn], off`
    Post,
}

/// `NOREG` in [`Am::rm`]: an immediate offset.
pub const NOREG: u8 = 0xff;

/// A load/store address: `rn` plus `imm`, or plus `rm << sh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Am {
    pub rn: u8,
    pub rm: u8,
    pub sh: u8,
    pub imm: i32,
    pub idx: Idx,
}

/// Long multiplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulL {
    Smull,
    Umull,
    Smlal,
    Umlal,
    Umaal,
    /// SMLAL<x><y>: top halves of n, m.
    SmlalXy(bool, bool),
    /// SMLALD(X)
    Smlald(bool),
    /// SMLSLD(X)
    Smlsld(bool),
}

/// 32-bit DSP multiplies; `ra == 15` is the non-accumulating form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dsp {
    /// SMLA<x><y> / SMUL<x><y>: top halves of n, m.
    SmlaXy(bool, bool),
    /// SMLAW<y> / SMULW<y>
    SmlaW(bool),
    /// SMLAD(X) / SMUAD(X)
    Smlad(bool),
    /// SMLSD(X) / SMUSD(X)
    Smlsd(bool),
    /// SMMLA(R) / SMMUL(R)
    Smmla(bool),
    /// SMMLS(R)
    Smmls(bool),
    /// USADA8 / USAD8
    Usada8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ext {
    Sxtb,
    Sxth,
    Uxtb,
    Uxth,
    Sxtb16,
    Uxtb16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Misc {
    Clz,
    Rbit,
    Rev,
    Rev16,
    Revsh,
}

/// Saturating add/subtract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QOp {
    Qadd,
    Qsub,
    Qdadd,
    Qdsub,
}

/// Three-register VFP operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V3 {
    Mla,
    Mls,
    Nmla,
    Nmls,
    Mul,
    Nmul,
    Add,
    Sub,
    Div,
    /// Fused multiply-add on (negated?) n and (negated?) d.
    Fma {
        neg_n: bool,
        neg_d: bool,
    },
    MaxNm,
    MinNm,
    /// VSEL<cond>
    Sel(u8),
}

/// A rounding mode for VRINT / VCVT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rm {
    /// To nearest, ties away from zero.
    A,
    /// To nearest, ties to even.
    N,
    /// Towards +infinity.
    P,
    /// Towards -infinity.
    M,
    /// Towards zero.
    Z,
    /// FPSCR.RMode.
    Fpscr,
}

/// Two-register VFP operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V2 {
    Mov,
    Abs,
    Neg,
    Sqrt,
    /// VRINT{A,N,P,M,Z,R}: no Inexact.
    Rint(Rm),
    /// VRINTX: FPSCR rounding, raises Inexact.
    RintX,
    /// Float to integer.
    ToInt {
        signed: bool,
        rm: Rm,
    },
    /// Integer to float.
    FromInt {
        signed: bool,
    },
    /// VCVTB/VCVTT: half (bottom/top) to single.
    FromHalf {
        top: bool,
    },
    /// VCVTB/VCVTT: single to half (bottom/top).
    ToHalf {
        top: bool,
    },
}

/// A decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum K {
    /// UNDEFINED: faults with `INSN_INVALID`.
    Undef,
    Nop,
    /// IT: the new ITSTATE (`firstcond:mask`).
    It(u8),
    Alu {
        op: Alu,
        s: Sf,
        rd: u8,
        rn: u8,
        b: Opnd,
    },
    /// ADR (and ADD/SUB with rn = pc), precomputed.
    Adr {
        rd: u8,
        v: u32,
    },
    Movw {
        rd: u8,
        imm: u16,
    },
    Movt {
        rd: u8,
        imm: u16,
    },
    Mul {
        s: Sf,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    /// MLA / MLS
    Mla {
        sub: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    MulL {
        op: MulL,
        lo: u8,
        hi: u8,
        rn: u8,
        rm: u8,
    },
    Div {
        signed: bool,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    Dsp {
        op: Dsp,
        rd: u8,
        rn: u8,
        rm: u8,
        ra: u8,
    },
    /// SSAT / USAT of `rn` shifted.
    Sat {
        signed: bool,
        rd: u8,
        rn: u8,
        to: u8,
        sh: Sh,
        n: u8,
    },
    Sat16 {
        signed: bool,
        rd: u8,
        rn: u8,
        to: u8,
    },
    Bfx {
        signed: bool,
        rd: u8,
        rn: u8,
        lsb: u8,
        width: u8,
    },
    /// BFI, or BFC when `rn == 15`.
    Bfi {
        rd: u8,
        rn: u8,
        lsb: u8,
        msb: u8,
    },
    /// Extend `rm` rotated right by `rot`, adding `rn` unless it is 15.
    Ext {
        op: Ext,
        rd: u8,
        rn: u8,
        rm: u8,
        rot: u8,
    },
    Misc {
        op: Misc,
        rd: u8,
        rm: u8,
    },
    Sel {
        rd: u8,
        rn: u8,
        rm: u8,
    },
    /// Parallel add/subtract: `kind` is hw2[6:4] (S, Q, SH, U, UQ, UH),
    /// `f` is hw1[6:4] (ADD8, ADD16, ASX, -, SUB8, SUB16, SAX).
    Par {
        kind: u8,
        f: u8,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    Q {
        op: QOp,
        rd: u8,
        rn: u8,
        rm: u8,
    },
    /// PKHBT (`tb == false`) / PKHTB.
    Pkh {
        tb: bool,
        rd: u8,
        rn: u8,
        rm: u8,
        n: u8,
    },
    Ld {
        sz: Sz,
        rt: u8,
        am: Am,
    },
    St {
        sz: Sz,
        rt: u8,
        am: Am,
    },
    /// A PC-relative load, address precomputed.
    LdLit {
        sz: Sz,
        rt: u8,
        addr: u32,
    },
    Ldrd {
        rt: u8,
        rt2: u8,
        am: Am,
    },
    Strd {
        rt: u8,
        rt2: u8,
        am: Am,
    },
    LdrdLit {
        rt: u8,
        rt2: u8,
        addr: u32,
    },
    Ldm {
        rn: u8,
        list: u16,
        wb: bool,
        db: bool,
    },
    Stm {
        rn: u8,
        list: u16,
        wb: bool,
        db: bool,
    },
    Ldrex {
        sz: Sz,
        rt: u8,
        rn: u8,
        imm: u16,
    },
    Strex {
        sz: Sz,
        rd: u8,
        rt: u8,
        rn: u8,
        imm: u16,
    },
    Clrex,
    /// TBB / TBH
    Tb {
        half: bool,
        rn: u8,
        rm: u8,
    },
    B {
        t: u32,
    },
    /// B<cond> (outside IT blocks).
    Bc {
        cond: u8,
        t: u32,
    },
    Bl {
        t: u32,
    },
    /// BLX <imm>: to Arm state.
    BlxI {
        t: u32,
    },
    Bx {
        rm: u8,
    },
    Blx {
        rm: u8,
    },
    Cbz {
        nz: bool,
        rn: u8,
        t: u32,
    },
    Svc,
    Bkpt,
    Wfi,
    /// YIELD and WFE, which Unicorn treats as invalid after the PC moves on.
    Yield,
    Mrs {
        rd: u8,
        sysm: u8,
    },
    Msr {
        rn: u8,
        sysm: u8,
        mask: u8,
    },
    Cps {
        disable: bool,
        i: bool,
        f: bool,
    },
    /// DSB, DMB and ISB (`isb` ends the block).
    Barrier {
        isb: bool,
    },
    V3 {
        op: V3,
        d: u8,
        n: u8,
        m: u8,
    },
    V2 {
        op: V2,
        d: u8,
        m: u8,
    },
    /// VCVT between float and fixed point, in place on `d`.
    VFix {
        to_fixed: bool,
        signed: bool,
        half: bool,
        d: u8,
        fbits: u8,
    },
    VMovI {
        d: u8,
        v: u32,
    },
    /// VCMP(E) with `s[m]`, or with +0 when `m` is `NOREG`.
    VCmp {
        e: bool,
        d: u8,
        m: u8,
    },
    /// VMOV between `rt` and `s[n]`.
    VMovR {
        to_core: bool,
        rt: u8,
        n: u8,
    },
    /// VMOV between `rt`, `rt2` and `s[m]`, `s[m+1]`.
    VMov2 {
        to_core: bool,
        rt: u8,
        rt2: u8,
        m: u8,
    },
    /// VMRS (`rt == 15`: to APSR.NZCV)
    Vmrs {
        rt: u8,
    },
    Vmsr {
        rt: u8,
    },
    /// VLDR / VSTR of `s[d]` (or `s[d..d+2]` when `dbl`).
    VLdr {
        load: bool,
        dbl: bool,
        d: u8,
        rn: u8,
        imm: i32,
    },
    /// VLDM / VSTM of `n` words (`s[d..d+n]`).
    VLdm {
        load: bool,
        dbl: bool,
        d: u8,
        n: u8,
        rn: u8,
        db: bool,
        wb: bool,
    },
}

/// One instruction at `pc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Op {
    pub pc: u32,
    /// 2 or 4.
    pub len: u8,
    pub k: K,
}

/// A basic block, as QEMU translates it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub ops: Vec<Op>,
    /// Bytes from the first instruction to the end of the last.
    pub size: u32,
    /// ITSTATE and CONTROL.FPCA at the start, which QEMU keys its blocks
    /// by and which change how many TCG ops the block takes.
    pub it: u8,
    pub fpca: bool,
}

/// Whether a halfword starts a 32-bit instruction.
#[inline]
pub fn is32(hw1: u16) -> bool {
    hw1 >> 11 >= 0b11101
}

#[inline]
fn bit(x: u32, n: u32) -> bool {
    (x >> n) & 1 != 0
}

#[inline]
fn bits(x: u32, hi: u32, lo: u32) -> u32 {
    (x >> lo) & ((1 << (hi - lo + 1)) - 1)
}

#[inline]
fn sext(x: u32, width: u32) -> u32 {
    (((x << (32 - width)) as i32) >> (32 - width)) as u32
}

fn align4(x: u32) -> u32 {
    x & !3
}

/// DecodeImmShift.
fn imm_shift(t: u32, imm5: u32) -> (Sh, u8) {
    match t {
        0 => (Sh::Lsl, imm5 as u8),
        1 => (Sh::Lsr, if imm5 == 0 { 32 } else { imm5 as u8 }),
        2 => (Sh::Asr, if imm5 == 0 { 32 } else { imm5 as u8 }),
        _ if imm5 == 0 => (Sh::Rrx, 1),
        _ => (Sh::Ror, imm5 as u8),
    }
}

/// ThumbExpandImm_C, with the carry out when the constant is rotated.
fn expand_imm(imm12: u32) -> Opnd {
    if imm12 >> 10 == 0 {
        let b = imm12 & 0xff;
        let v = match (imm12 >> 8) & 3 {
            0 => b,
            1 => b << 16 | b,
            2 => b << 24 | b << 8,
            _ => b * 0x0101_0101,
        };
        Opnd::Imm(v, None)
    } else {
        let v = (0x80 | (imm12 & 0x7f)).rotate_right(imm12 >> 7);
        Opnd::Imm(v, Some(v >> 31 != 0))
    }
}

/// Decodes the instruction at `pc` (`hw2` is ignored for 16-bit ones).
pub fn decode(pc: u32, hw1: u16, hw2: u16) -> Op {
    if is32(hw1) {
        Op {
            pc,
            len: 4,
            k: d32(pc, hw1 as u32, hw2 as u32),
        }
    } else {
        Op {
            pc,
            len: 2,
            k: d16(pc, hw1 as u32),
        }
    }
}

fn d16(pc: u32, i: u32) -> K {
    let r = |s: u32| ((i >> s) & 7) as u8;
    let alu = |op, s, rd, rn, b| K::Alu { op, s, rd, rn, b };
    let imm8 = i & 0xff;
    let am = |rn, imm: u32| Am {
        rn,
        rm: NOREG,
        sh: 0,
        imm: imm as i32,
        idx: Idx::Off,
    };
    match i >> 11 {
        0b00000..=0b00010 => {
            let (t, n) = imm_shift(i >> 11, bits(i, 10, 6));
            alu(Alu::Mov, Sf::OutIt, r(0), 0, Opnd::Sh(r(3), t, n))
        }
        0b00011 => {
            let (rd, rn) = (r(0), r(3));
            let x = bits(i, 8, 6);
            match bits(i, 10, 9) {
                0 => alu(Alu::Add, Sf::OutIt, rd, rn, Opnd::Sh(x as u8, Sh::Lsl, 0)),
                1 => alu(Alu::Sub, Sf::OutIt, rd, rn, Opnd::Sh(x as u8, Sh::Lsl, 0)),
                2 => alu(Alu::Add, Sf::OutIt, rd, rn, Opnd::Imm(x, None)),
                _ => alu(Alu::Sub, Sf::OutIt, rd, rn, Opnd::Imm(x, None)),
            }
        }
        0b00100 => alu(Alu::Mov, Sf::OutIt, r(8), 0, Opnd::Imm(imm8, None)),
        0b00101 => alu(Alu::Cmp, Sf::Yes, 0, r(8), Opnd::Imm(imm8, None)),
        0b00110 => alu(Alu::Add, Sf::OutIt, r(8), r(8), Opnd::Imm(imm8, None)),
        0b00111 => alu(Alu::Sub, Sf::OutIt, r(8), r(8), Opnd::Imm(imm8, None)),
        0b01000 if !bit(i, 10) => {
            let (rdn, rm) = (r(0), r(3));
            let reg = Opnd::Sh(rm, Sh::Lsl, 0);
            let s = Sf::OutIt;
            match bits(i, 9, 6) {
                0 => alu(Alu::And, s, rdn, rdn, reg),
                1 => alu(Alu::Eor, s, rdn, rdn, reg),
                2 => alu(Alu::Mov, s, rdn, 0, Opnd::ShR(rdn, Sh::Lsl, rm)),
                3 => alu(Alu::Mov, s, rdn, 0, Opnd::ShR(rdn, Sh::Lsr, rm)),
                4 => alu(Alu::Mov, s, rdn, 0, Opnd::ShR(rdn, Sh::Asr, rm)),
                5 => alu(Alu::Adc, s, rdn, rdn, reg),
                6 => alu(Alu::Sbc, s, rdn, rdn, reg),
                7 => alu(Alu::Mov, s, rdn, 0, Opnd::ShR(rdn, Sh::Ror, rm)),
                8 => alu(Alu::Tst, Sf::Yes, 0, rdn, reg),
                9 => alu(Alu::Rsb, s, rdn, rm, Opnd::Imm(0, None)),
                10 => alu(Alu::Cmp, Sf::Yes, 0, rdn, reg),
                11 => alu(Alu::Cmn, Sf::Yes, 0, rdn, reg),
                12 => alu(Alu::Orr, s, rdn, rdn, reg),
                13 => K::Mul {
                    s,
                    rd: rdn,
                    rn: rm,
                    rm: rdn,
                },
                14 => alu(Alu::Bic, s, rdn, rdn, reg),
                _ => alu(Alu::Mvn, s, rdn, 0, reg),
            }
        }
        0b01000 => {
            let rdn = (bits(i, 7, 7) << 3 | bits(i, 2, 0)) as u8;
            let rm = bits(i, 6, 3) as u8;
            let reg = Opnd::Sh(rm, Sh::Lsl, 0);
            match bits(i, 9, 8) {
                0 => alu(Alu::Add, Sf::No, rdn, rdn, reg),
                1 => alu(Alu::Cmp, Sf::Yes, 0, rdn, reg),
                2 => alu(Alu::Mov, Sf::No, rdn, 0, reg),
                _ if i & 7 != 0 => K::Undef,
                _ if bit(i, 7) => K::Blx { rm },
                _ => K::Bx { rm },
            }
        }
        0b01001 => K::LdLit {
            sz: Sz::W,
            rt: r(8),
            addr: align4(pc.wrapping_add(4)).wrapping_add(imm8 << 2),
        },
        0b01010 | 0b01011 => {
            let a = Am {
                rn: r(3),
                rm: r(6),
                sh: 0,
                imm: 0,
                idx: Idx::Off,
            };
            let rt = r(0);
            match bits(i, 11, 9) {
                0 => K::St { sz: Sz::W, rt, am: a },
                1 => K::St { sz: Sz::H, rt, am: a },
                2 => K::St { sz: Sz::B, rt, am: a },
                3 => K::Ld {
                    sz: Sz::Sb,
                    rt,
                    am: a,
                },
                4 => K::Ld { sz: Sz::W, rt, am: a },
                5 => K::Ld { sz: Sz::H, rt, am: a },
                6 => K::Ld { sz: Sz::B, rt, am: a },
                _ => K::Ld {
                    sz: Sz::Sh,
                    rt,
                    am: a,
                },
            }
        }
        0b01100..=0b10001 => {
            let imm5 = bits(i, 10, 6);
            let (rt, rn) = (r(0), r(3));
            match i >> 11 {
                0b01100 => K::St {
                    sz: Sz::W,
                    rt,
                    am: am(rn, imm5 << 2),
                },
                0b01101 => K::Ld {
                    sz: Sz::W,
                    rt,
                    am: am(rn, imm5 << 2),
                },
                0b01110 => K::St {
                    sz: Sz::B,
                    rt,
                    am: am(rn, imm5),
                },
                0b01111 => K::Ld {
                    sz: Sz::B,
                    rt,
                    am: am(rn, imm5),
                },
                0b10000 => K::St {
                    sz: Sz::H,
                    rt,
                    am: am(rn, imm5 << 1),
                },
                _ => K::Ld {
                    sz: Sz::H,
                    rt,
                    am: am(rn, imm5 << 1),
                },
            }
        }
        0b10010 => K::St {
            sz: Sz::W,
            rt: r(8),
            am: am(13, imm8 << 2),
        },
        0b10011 => K::Ld {
            sz: Sz::W,
            rt: r(8),
            am: am(13, imm8 << 2),
        },
        0b10100 => K::Adr {
            rd: r(8),
            v: align4(pc.wrapping_add(4)).wrapping_add(imm8 << 2),
        },
        0b10101 => alu(Alu::Add, Sf::No, r(8), 13, Opnd::Imm(imm8 << 2, None)),
        0b10110 | 0b10111 => misc16(pc, i),
        0b11000 if imm8 == 0 => K::Undef,
        0b11000 => K::Stm {
            rn: r(8),
            list: imm8 as u16,
            wb: true,
            db: false,
        },
        0b11001 if imm8 == 0 => K::Undef,
        0b11001 => K::Ldm {
            rn: r(8),
            list: imm8 as u16,
            wb: imm8 & (1 << r(8)) == 0,
            db: false,
        },
        0b11010 | 0b11011 => match bits(i, 11, 8) {
            14 => K::Undef,
            15 => K::Svc,
            cond => K::Bc {
                cond: cond as u8,
                t: pc.wrapping_add(4).wrapping_add(sext(imm8 << 1, 9)),
            },
        },
        0b11100 => K::B {
            t: pc.wrapping_add(4).wrapping_add(sext(bits(i, 10, 0) << 1, 12)),
        },
        _ => K::Undef,
    }
}

/// 1011 xxxx xxxx xxxx: miscellaneous 16-bit instructions.
fn misc16(pc: u32, i: u32) -> K {
    let r = |s: u32| ((i >> s) & 7) as u8;
    match bits(i, 11, 8) {
        0 => K::Alu {
            op: if bit(i, 7) { Alu::Sub } else { Alu::Add },
            s: Sf::No,
            rd: 13,
            rn: 13,
            b: Opnd::Imm(bits(i, 6, 0) << 2, None),
        },
        1 | 3 | 9 | 11 => K::Cbz {
            nz: bit(i, 11),
            rn: r(0),
            t: pc
                .wrapping_add(4)
                .wrapping_add(bits(i, 9, 9) << 6 | bits(i, 7, 3) << 1),
        },
        2 => K::Ext {
            op: [Ext::Sxth, Ext::Sxtb, Ext::Uxth, Ext::Uxtb][bits(i, 7, 6) as usize],
            rd: r(0),
            rn: 15,
            rm: r(3),
            rot: 0,
        },
        4 | 5 => match (i & 0xff) as u16 | (bits(i, 8, 8) as u16) << 14 {
            0 => K::Undef,
            list => K::Stm {
                rn: 13,
                list,
                wb: true,
                db: true,
            },
        },
        6 if bits(i, 7, 5) == 0b011 => K::Cps {
            disable: bit(i, 4),
            i: bit(i, 1),
            f: bit(i, 0),
        },
        10 => {
            let (rd, rm) = (r(0), r(3));
            match bits(i, 7, 6) {
                0 => K::Misc {
                    op: Misc::Rev,
                    rd,
                    rm,
                },
                1 => K::Misc {
                    op: Misc::Rev16,
                    rd,
                    rm,
                },
                3 => K::Misc {
                    op: Misc::Revsh,
                    rd,
                    rm,
                },
                _ => K::Undef,
            }
        }
        12 | 13 => match (i & 0xff) as u16 | (bits(i, 8, 8) as u16) << 15 {
            0 => K::Undef,
            list => K::Ldm {
                rn: 13,
                list,
                wb: true,
                db: false,
            },
        },
        14 => K::Bkpt,
        15 if i & 0xf != 0 => K::It((i & 0xff) as u8),
        15 => match bits(i, 7, 4) {
            1 | 2 => K::Yield,
            3 => K::Wfi,
            _ => K::Nop,
        },
        _ => K::Undef,
    }
}

fn d32(pc: u32, h: u32, l: u32) -> K {
    match h >> 11 {
        0b11101 => {
            if bit(h, 10) {
                coproc(h, l)
            } else if bit(h, 9) {
                dp_shifted(h, l)
            } else if !bit(h, 6) {
                ldm_stm(h, l)
            } else {
                dual_excl(pc, h, l)
            }
        }
        0b11110 => {
            if bit(l, 15) {
                branch_misc(pc, h, l)
            } else if bit(h, 9) {
                plain_imm(pc, h, l)
            } else {
                dp_modified_imm(h, l)
            }
        }
        _ => {
            if bit(h, 10) {
                coproc(h, l)
            } else if !bit(h, 9) {
                ldst_single(pc, h, l)
            } else if !bit(h, 8) {
                dp_register(h, l)
            } else if !bit(h, 7) {
                multiply(h, l)
            } else {
                long_multiply(h, l)
            }
        }
    }
}

fn ldm_stm(h: u32, l: u32) -> K {
    let rn = (h & 15) as u8;
    let (wb, load) = (bit(h, 5), bit(h, 4));
    let db = match bits(h, 8, 7) {
        1 => false,
        2 => true,
        _ => return K::Undef,
    };
    let list = l as u16;
    if (wb && list & (1 << rn) != 0) || list.count_ones() < 2 || rn == 15 {
        return K::Undef;
    }
    if load {
        K::Ldm { rn, list, wb, db }
    } else {
        K::Stm { rn, list, wb, db }
    }
}

fn dual_excl(pc: u32, h: u32, l: u32) -> K {
    let rn = (h & 15) as u8;
    let rt = bits(l, 15, 12) as u8;
    let (p, u, w, load) = (bit(h, 8), bit(h, 7), bit(h, 5), bit(h, 4));
    if !p && !w {
        let rd = bits(l, 11, 8) as u8;
        let imm = ((l & 0xff) << 2) as u16;
        return match (u, load) {
            (false, false) => K::Strex {
                sz: Sz::W,
                rd,
                rt,
                rn,
                imm,
            },
            (false, true) => K::Ldrex {
                sz: Sz::W,
                rt,
                rn,
                imm,
            },
            (true, false) => {
                let sz = match bits(l, 7, 4) {
                    4 => Sz::B,
                    5 => Sz::H,
                    _ => return K::Undef,
                };
                K::Strex {
                    sz,
                    rd: (l & 15) as u8,
                    rt,
                    rn,
                    imm: 0,
                }
            }
            (true, true) => match bits(l, 7, 4) {
                0 | 1 => K::Tb {
                    half: bit(l, 4),
                    rn,
                    rm: (l & 15) as u8,
                },
                4 => K::Ldrex {
                    sz: Sz::B,
                    rt,
                    rn,
                    imm: 0,
                },
                5 => K::Ldrex {
                    sz: Sz::H,
                    rt,
                    rn,
                    imm: 0,
                },
                _ => K::Undef,
            },
        };
    }
    let rt2 = bits(l, 11, 8) as u8;
    let off = ((l & 0xff) << 2) as i32;
    let imm = if u { off } else { -off };
    if rn == 15 && load {
        if w {
            return K::Undef;
        }
        return K::LdrdLit {
            rt,
            rt2,
            addr: align4(pc.wrapping_add(4)).wrapping_add(imm as u32),
        };
    }
    let am = Am {
        rn,
        rm: NOREG,
        sh: 0,
        imm,
        idx: match (p, w) {
            (true, false) => Idx::Off,
            (true, true) => Idx::Pre,
            _ => Idx::Post,
        },
    };
    if load {
        K::Ldrd { rt, rt2, am }
    } else {
        K::Strd { rt, rt2, am }
    }
}

fn dp_shifted(h: u32, l: u32) -> K {
    let op = bits(h, 8, 5);
    let s = bit(h, 4);
    let rn = (h & 15) as u8;
    let rd = bits(l, 11, 8) as u8;
    let rm = (l & 15) as u8;
    let imm5 = bits(l, 14, 12) << 2 | bits(l, 7, 6);
    let t = bits(l, 5, 4);
    if op == 6 {
        if s || bit(l, 4) {
            return K::Undef;
        }
        return K::Pkh {
            tb: bit(l, 5),
            rd,
            rn,
            rm,
            n: imm5 as u8,
        };
    }
    let (sh, n) = imm_shift(t, imm5);
    dp(op, s, rd, rn, Opnd::Sh(rm, sh, n))
}

fn dp_modified_imm(h: u32, l: u32) -> K {
    let imm12 = bits(h, 10, 10) << 11 | bits(l, 14, 12) << 8 | (l & 0xff);
    let op = bits(h, 8, 5);
    if op == 6 {
        return K::Undef;
    }
    dp(
        op,
        bit(h, 4),
        bits(l, 11, 8) as u8,
        (h & 15) as u8,
        expand_imm(imm12),
    )
}

/// The data-processing opcode table shared by the shifted-register and
/// modified-immediate encodings.
fn dp(op: u32, s: bool, rd: u8, rn: u8, b: Opnd) -> K {
    let sf = if s { Sf::Yes } else { Sf::No };
    let test = rd == 15 && s;
    let (op, rd, rn) = match op {
        0 if test => (Alu::Tst, 0, rn),
        0 => (Alu::And, rd, rn),
        1 => (Alu::Bic, rd, rn),
        2 if rn == 15 => (Alu::Mov, rd, 0),
        2 => (Alu::Orr, rd, rn),
        3 if rn == 15 => (Alu::Mvn, rd, 0),
        3 => (Alu::Orn, rd, rn),
        4 if test => (Alu::Teq, 0, rn),
        4 => (Alu::Eor, rd, rn),
        8 if test => (Alu::Cmn, 0, rn),
        8 => (Alu::Add, rd, rn),
        10 => (Alu::Adc, rd, rn),
        11 => (Alu::Sbc, rd, rn),
        13 if test => (Alu::Cmp, 0, rn),
        13 => (Alu::Sub, rd, rn),
        14 => (Alu::Rsb, rd, rn),
        _ => return K::Undef,
    };
    K::Alu { op, s: sf, rd, rn, b }
}

fn plain_imm(pc: u32, h: u32, l: u32) -> K {
    let rn = (h & 15) as u8;
    let rd = bits(l, 11, 8) as u8;
    let imm12 = bits(h, 10, 10) << 11 | bits(l, 14, 12) << 8 | (l & 0xff);
    let imm5 = bits(l, 14, 12) << 2 | bits(l, 7, 6);
    let low5 = (l & 31) as u8;
    match bits(h, 8, 4) {
        0b00000 if rn == 15 => K::Adr {
            rd,
            v: align4(pc.wrapping_add(4)).wrapping_add(imm12),
        },
        0b00000 => K::Alu {
            op: Alu::Add,
            s: Sf::No,
            rd,
            rn,
            b: Opnd::Imm(imm12, None),
        },
        0b01010 if rn == 15 => K::Adr {
            rd,
            v: align4(pc.wrapping_add(4)).wrapping_sub(imm12),
        },
        0b01010 => K::Alu {
            op: Alu::Sub,
            s: Sf::No,
            rd,
            rn,
            b: Opnd::Imm(imm12, None),
        },
        0b00100 => K::Movw {
            rd,
            imm: ((rn as u32) << 12 | imm12) as u16,
        },
        0b01100 => K::Movt {
            rd,
            imm: ((rn as u32) << 12 | imm12) as u16,
        },
        op @ (0b10000 | 0b10010 | 0b11000 | 0b11010) => {
            let signed = op & 0b01000 == 0;
            let asr = op & 0b00010 != 0;
            if asr && imm5 == 0 {
                K::Sat16 {
                    signed,
                    rd,
                    rn,
                    to: (l & 15) as u8 + signed as u8,
                }
            } else {
                K::Sat {
                    signed,
                    rd,
                    rn,
                    to: low5 + signed as u8,
                    sh: if asr { Sh::Asr } else { Sh::Lsl },
                    n: imm5 as u8,
                }
            }
        }
        op @ (0b10100 | 0b11100) => {
            let width = low5 + 1;
            if imm5 as u8 + width > 32 {
                return K::Undef;
            }
            K::Bfx {
                signed: op == 0b10100,
                rd,
                rn,
                lsb: imm5 as u8,
                width,
            }
        }
        0b10110 if low5 < imm5 as u8 => K::Undef,
        0b10110 => K::Bfi {
            rd,
            rn,
            lsb: imm5 as u8,
            msb: low5,
        },
        _ => K::Undef,
    }
}

fn branch_misc(pc: u32, h: u32, l: u32) -> K {
    let s = bit(h, 10);
    let (j1, j2) = (bit(l, 13), bit(l, 11));
    match (bit(l, 14), bit(l, 12)) {
        (false, false) if bits(h, 9, 7) != 7 => {
            let imm = (s as u32) << 20
                | (j2 as u32) << 19
                | (j1 as u32) << 18
                | bits(h, 5, 0) << 12
                | bits(l, 10, 0) << 1;
            K::Bc {
                cond: bits(h, 9, 6) as u8,
                t: pc.wrapping_add(4).wrapping_add(sext(imm, 21)),
            }
        }
        (false, false) => match bits(h, 10, 4) {
            0x38 | 0x39 if !bit(l, 13) => K::Msr {
                rn: (h & 15) as u8,
                sysm: (l & 0xff) as u8,
                mask: bits(l, 11, 10) as u8,
            },
            0x3a if !bit(l, 13) => {
                if bits(l, 10, 8) != 0 {
                    return K::Undef;
                }
                match l & 0xff {
                    1 | 2 => K::Yield,
                    3 => K::Wfi,
                    _ => K::Nop,
                }
            }
            0x3b if !bit(l, 13) => match bits(l, 7, 4) {
                2 => K::Clrex,
                4 | 5 => K::Barrier { isb: false },
                6 => K::Barrier { isb: true },
                _ => K::Undef,
            },
            0x3e | 0x3f if !bit(l, 13) => K::Mrs {
                rd: bits(l, 11, 8) as u8,
                sysm: (l & 0xff) as u8,
            },
            _ => K::Undef,
        },
        (b14, true) => {
            let i1 = !(j1 ^ s);
            let i2 = !(j2 ^ s);
            let imm = (s as u32) << 24
                | (i1 as u32) << 23
                | (i2 as u32) << 22
                | bits(h, 9, 0) << 12
                | bits(l, 10, 0) << 1;
            let t = pc.wrapping_add(4).wrapping_add(sext(imm, 25));
            if b14 {
                K::Bl { t }
            } else {
                K::B { t }
            }
        }
        (true, false) => {
            if bit(l, 0) {
                return K::Undef;
            }
            let i1 = !(j1 ^ s);
            let i2 = !(j2 ^ s);
            let imm = (s as u32) << 24
                | (i1 as u32) << 23
                | (i2 as u32) << 22
                | bits(h, 9, 0) << 12
                | bits(l, 10, 1) << 2;
            K::BlxI {
                t: align4(pc.wrapping_add(4)).wrapping_add(sext(imm, 25)),
            }
        }
    }
}

fn ldst_single(pc: u32, h: u32, l: u32) -> K {
    let rn = (h & 15) as u8;
    let rt = bits(l, 15, 12) as u8;
    let size = bits(h, 6, 5);
    let load = bit(h, 4);
    let signed = bit(h, 8);
    if size == 3 {
        return K::Undef;
    }
    let sz = match (size, signed, load) {
        (0, false, _) => Sz::B,
        (0, true, true) => Sz::Sb,
        (1, false, _) => Sz::H,
        (1, true, true) => Sz::Sh,
        (2, false, _) => Sz::W,
        _ => return K::Undef,
    };
    // Loads of a byte or halfword into pc are preload hints.
    let hint = load && rt == 15 && sz != Sz::W;
    if rn == 15 {
        if !load {
            return K::Undef;
        }
        if hint {
            return K::Nop;
        }
        let off = l & 0xfff;
        let base = align4(pc.wrapping_add(4));
        let addr = if bit(h, 7) {
            base.wrapping_add(off)
        } else {
            base.wrapping_sub(off)
        };
        return K::LdLit { sz, rt, addr };
    }
    let am = if bit(h, 7) {
        Am {
            rn,
            rm: NOREG,
            sh: 0,
            imm: (l & 0xfff) as i32,
            idx: Idx::Off,
        }
    } else if bit(l, 11) {
        let (p, u, w) = (bit(l, 10), bit(l, 9), bit(l, 8));
        if !p && !w {
            return K::Undef;
        }
        let off = (l & 0xff) as i32;
        Am {
            rn,
            rm: NOREG,
            sh: 0,
            imm: if u { off } else { -off },
            idx: match (p, w) {
                (true, false) => Idx::Off,
                (true, true) => Idx::Pre,
                _ => Idx::Post,
            },
        }
    } else if bits(l, 10, 6) == 0 {
        Am {
            rn,
            rm: (l & 15) as u8,
            sh: bits(l, 5, 4) as u8,
            imm: 0,
            idx: Idx::Off,
        }
    } else {
        return K::Undef;
    };
    if hint {
        K::Nop
    } else if load {
        K::Ld { sz, rt, am }
    } else {
        K::St { sz, rt, am }
    }
}

fn dp_register(h: u32, l: u32) -> K {
    if bits(l, 15, 12) != 0xf {
        return K::Undef;
    }
    let op1 = bits(h, 7, 4);
    let op2 = bits(l, 7, 4);
    let rn = (h & 15) as u8;
    let rd = bits(l, 11, 8) as u8;
    let rm = (l & 15) as u8;
    if op1 & 8 == 0 {
        if op2 == 0 {
            return K::Alu {
                op: Alu::Mov,
                s: if op1 & 1 != 0 { Sf::Yes } else { Sf::No },
                rd,
                rn: 0,
                b: Opnd::ShR(rn, Sh::from_bits(op1 >> 1), rm),
            };
        }
        if op2 & 0b1100 != 0b1000 {
            return K::Undef;
        }
        let op = match op1 {
            0 => Ext::Sxth,
            1 => Ext::Uxth,
            2 => Ext::Sxtb16,
            3 => Ext::Uxtb16,
            4 => Ext::Sxtb,
            5 => Ext::Uxtb,
            _ => return K::Undef,
        };
        return K::Ext {
            op,
            rd,
            rn,
            rm,
            rot: (op2 & 3) as u8 * 8,
        };
    }
    if op2 & 8 == 0 {
        let f = op1 & 7;
        if op2 & 3 == 3 || f == 3 || f == 7 {
            return K::Undef;
        }
        return K::Par {
            kind: op2 as u8,
            f: f as u8,
            rd,
            rn,
            rm,
        };
    }
    if op2 & 0b1100 != 0b1000 || op1 & 0b1100 != 0b1000 {
        return K::Undef;
    }
    match (op1 & 3, op2 & 3) {
        (0, o) => K::Q {
            op: [QOp::Qadd, QOp::Qdadd, QOp::Qsub, QOp::Qdsub][o as usize],
            rd,
            rn,
            rm,
        },
        (1, o) => K::Misc {
            op: [Misc::Rev, Misc::Rev16, Misc::Rbit, Misc::Revsh][o as usize],
            rd,
            rm,
        },
        (2, 0) => K::Sel { rd, rn, rm },
        (3, 0) => K::Misc {
            op: Misc::Clz,
            rd,
            rm,
        },
        _ => K::Undef,
    }
}

fn multiply(h: u32, l: u32) -> K {
    if bits(l, 7, 6) != 0 {
        return K::Undef;
    }
    let rn = (h & 15) as u8;
    let ra = bits(l, 15, 12) as u8;
    let rd = bits(l, 11, 8) as u8;
    let rm = (l & 15) as u8;
    let op2 = bits(l, 5, 4);
    let dsp = |op| K::Dsp { op, rd, rn, rm, ra };
    match (bits(h, 6, 4), op2) {
        (0, 0) if ra == 15 => K::Mul {
            s: Sf::No,
            rd,
            rn,
            rm,
        },
        (0, 0) => K::Mla {
            sub: false,
            rd,
            rn,
            rm,
            ra,
        },
        (0, 1) => K::Mla {
            sub: true,
            rd,
            rn,
            rm,
            ra,
        },
        (1, o) => dsp(Dsp::SmlaXy(o & 2 != 0, o & 1 != 0)),
        (2, o @ (0 | 1)) => dsp(Dsp::Smlad(o == 1)),
        (3, o @ (0 | 1)) => dsp(Dsp::SmlaW(o == 1)),
        (4, o @ (0 | 1)) => dsp(Dsp::Smlsd(o == 1)),
        (5, o @ (0 | 1)) => dsp(Dsp::Smmla(o == 1)),
        (6, o @ (0 | 1)) => dsp(Dsp::Smmls(o == 1)),
        (7, 0) => dsp(Dsp::Usada8),
        _ => K::Undef,
    }
}

fn long_multiply(h: u32, l: u32) -> K {
    let rn = (h & 15) as u8;
    let lo = bits(l, 15, 12) as u8;
    let hi = bits(l, 11, 8) as u8;
    let rm = (l & 15) as u8;
    let op2 = bits(l, 7, 4);
    let mull = |op| K::MulL { op, lo, hi, rn, rm };
    match (bits(h, 6, 4), op2) {
        (0, 0) => mull(MulL::Smull),
        (1, 15) if lo == 15 => K::Div {
            signed: true,
            rd: hi,
            rn,
            rm,
        },
        (2, 0) => mull(MulL::Umull),
        (3, 15) if lo == 15 => K::Div {
            signed: false,
            rd: hi,
            rn,
            rm,
        },
        (4, 0) => mull(MulL::Smlal),
        (4, 8..=11) => mull(MulL::SmlalXy(op2 & 2 != 0, op2 & 1 != 0)),
        (4, 12 | 13) => mull(MulL::Smlald(op2 & 1 != 0)),
        (5, 12 | 13) => mull(MulL::Smlsld(op2 & 1 != 0)),
        (6, 0) => mull(MulL::Umlal),
        (6, 6) => mull(MulL::Umaal),
        _ => K::Undef,
    }
}

/// Coprocessor space: floating point (coprocessors 10 and 11) only.
fn coproc(h: u32, l: u32) -> K {
    if bits(l, 11, 9) != 0b101 {
        return K::Undef;
    }
    let dbl = bit(l, 8);
    // Single-precision register numbers: Vx:X.
    let sd = (bits(l, 15, 12) << 1 | bits(h, 6, 6)) as u8;
    let sn = ((h & 15) << 1 | bits(l, 7, 7)) as u8;
    let sm = ((l & 15) << 1 | bits(l, 5, 5)) as u8;
    if bit(h, 12) {
        // The unconditional (Armv8) space: VSEL, VMAXNM, VRINT{A,N,P,M},
        // VCVT{A,N,P,M}.
        if bits(h, 11, 8) != 0b1110 || bit(l, 4) || dbl {
            return K::Undef;
        }
        if !bit(h, 7) {
            if bit(l, 6) {
                return K::Undef;
            }
            const SEL: [u8; 4] = [0, 6, 10, 12]; // EQ, VS, GE, GT
            return K::V3 {
                op: V3::Sel(SEL[bits(h, 5, 4) as usize]),
                d: sd,
                n: sn,
                m: sm,
            };
        }
        return match bits(h, 5, 4) {
            0 => K::V3 {
                op: if bit(l, 6) { V3::MinNm } else { V3::MaxNm },
                d: sd,
                n: sn,
                m: sm,
            },
            3 if bit(l, 6) => {
                let rm = [Rm::A, Rm::N, Rm::P, Rm::M][(h & 3) as usize];
                match bits(h, 3, 2) {
                    2 if !bit(l, 7) => K::V2 {
                        op: V2::Rint(rm),
                        d: sd,
                        m: sm,
                    },
                    3 => K::V2 {
                        op: V2::ToInt {
                            signed: bit(l, 7),
                            rm,
                        },
                        d: sd,
                        m: sm,
                    },
                    _ => K::Undef,
                }
            }
            _ => K::Undef,
        };
    }
    let op1 = bits(h, 9, 4);
    if op1 & 0b110000 == 0b100000 {
        if bit(l, 4) {
            return transfer(h, l, sn);
        }
        if dbl {
            return K::Undef;
        }
        return vfp_dp(h, l, sd, sn, sm);
    }
    if op1 & 0b100000 != 0 || op1 & 0b111110 == 0 {
        return K::Undef;
    }
    if op1 & 0b111110 == 0b000100 {
        // 64-bit transfers.
        if bits(l, 7, 6) != 0 || !bit(l, 4) {
            return K::Undef;
        }
        let m = if dbl {
            if bit(l, 5) {
                return K::Undef;
            }
            ((l & 15) * 2) as u8
        } else {
            if sm == 31 {
                return K::Undef;
            }
            sm
        };
        return K::VMov2 {
            to_core: bit(h, 4),
            rt: bits(l, 15, 12) as u8,
            rt2: (h & 15) as u8,
            m,
        };
    }
    // Extension register load/store.
    let (p, u, w, load) = (bit(h, 8), bit(h, 7), bit(h, 5), bit(h, 4));
    let rn = (h & 15) as u8;
    let imm8 = l & 0xff;
    let d = if dbl {
        if bit(h, 6) {
            return K::Undef;
        }
        (bits(l, 15, 12) * 2) as u8
    } else {
        sd
    };
    if p && !w {
        let off = (imm8 << 2) as i32;
        return K::VLdr {
            load,
            dbl,
            d,
            rn,
            imm: if u { off } else { -off },
        };
    }
    if p == u {
        return K::Undef;
    }
    if dbl && imm8 & 1 != 0 {
        return K::Undef;
    }
    let n = imm8 as u8;
    if n == 0 || d as u32 + n as u32 > 32 || (rn == 15 && w) {
        return K::Undef;
    }
    K::VLdm {
        load,
        dbl,
        d,
        n,
        rn,
        db: p,
        wb: w,
    }
}

/// 8-, 16- and 32-bit transfers between core and VFP registers.
fn transfer(h: u32, l: u32, sn: u8) -> K {
    let rt = bits(l, 15, 12) as u8;
    let to_core = bit(h, 4);
    let a = bits(h, 7, 5);
    if !bit(l, 8) {
        return match a {
            0 => K::VMovR { to_core, rt, n: sn },
            7 if h & 15 == 1 => {
                if to_core {
                    K::Vmrs { rt }
                } else {
                    K::Vmsr { rt }
                }
            }
            _ => K::Undef,
        };
    }
    // VMOV.32 between a core register and a scalar Dn[x].
    if a & 0b110 != 0 || bits(l, 6, 5) != 0 || bit(l, 7) {
        return K::Undef;
    }
    K::VMovR {
        to_core,
        rt,
        n: ((h & 15) * 2 + (a & 1)) as u8,
    }
}

fn vfp_dp(h: u32, l: u32, d: u8, n: u8, m: u8) -> K {
    let opc1 = bits(h, 7, 7) << 2 | bits(h, 5, 4);
    let op = bit(l, 6);
    let v3 = |op| K::V3 { op, d, n, m };
    match opc1 {
        0 => v3(if op { V3::Mls } else { V3::Mla }),
        1 => v3(if op { V3::Nmla } else { V3::Nmls }),
        2 => v3(if op { V3::Nmul } else { V3::Mul }),
        3 => v3(if op { V3::Sub } else { V3::Add }),
        4 if !op => v3(V3::Div),
        5 => v3(V3::Fma {
            neg_n: op,
            neg_d: true,
        }),
        6 => v3(V3::Fma {
            neg_n: op,
            neg_d: false,
        }),
        7 if !op => {
            let imm8 = (h & 15) << 4 | (l & 15);
            let (a, b) = (imm8 >> 7, (imm8 >> 6) & 1);
            let exp = (b ^ 1) << 7 | if b == 1 { 0x7c } else { 0 } | (imm8 >> 4) & 3;
            K::VMovI {
                d,
                v: a << 31 | exp << 23 | (imm8 & 15) << 19,
            }
        }
        7 => {
            let o7 = bit(l, 7);
            let v2 = |op| K::V2 { op, d, m };
            match h & 15 {
                0 => v2(if o7 { V2::Abs } else { V2::Mov }),
                1 => v2(if o7 { V2::Sqrt } else { V2::Neg }),
                2 => v2(V2::FromHalf { top: o7 }),
                3 => v2(V2::ToHalf { top: o7 }),
                4 => K::VCmp { e: o7, d, m },
                5 => K::VCmp { e: o7, d, m: NOREG },
                6 => v2(V2::Rint(if o7 { Rm::Z } else { Rm::Fpscr })),
                7 if !o7 => v2(V2::RintX),
                8 => v2(V2::FromInt { signed: o7 }),
                c @ (10 | 11 | 14 | 15) => {
                    let size = if o7 { 32 } else { 16 };
                    let imm5 = (l & 15) << 1 | bits(l, 5, 5);
                    if imm5 > size {
                        return K::Undef;
                    }
                    K::VFix {
                        to_fixed: c & 4 != 0,
                        signed: c & 1 == 0,
                        half: !o7,
                        d,
                        fbits: (size - imm5) as u8,
                    }
                }
                c @ (12 | 13) => v2(V2::ToInt {
                    signed: c == 13,
                    rm: if o7 { Rm::Z } else { Rm::Fpscr },
                }),
                _ => K::Undef,
            }
        }
        _ => K::Undef,
    }
}

impl K {
    /// Whether QEMU ends the translation block after this instruction.
    pub fn ends_block(&self) -> bool {
        match *self {
            K::B { .. }
            | K::Bc { .. }
            | K::Bl { .. }
            | K::BlxI { .. }
            | K::Bx { .. }
            | K::Blx { .. }
            | K::Cbz { .. }
            | K::Tb { .. }
            | K::Svc
            | K::Bkpt
            | K::Undef
            | K::Wfi
            | K::Yield
            | K::Msr { .. }
            | K::Cps { .. }
            | K::Barrier { isb: true }
            | K::Vmsr { .. } => true,
            K::Alu { op, rd, .. } => rd == 15 && !op.is_test(),
            K::Adr { rd, .. }
            | K::Movw { rd, .. }
            | K::Movt { rd, .. }
            | K::Ld { rt: rd, .. }
            | K::LdLit { rt: rd, .. } => rd == 15,
            K::Ldm { list, .. } => list & 0x8000 != 0,
            _ => false,
        }
    }
}

/// Decodes the block starting at `addr`, which must be in an executable
/// region. It ends after the first instruction that ends a QEMU
/// translation block, before an instruction QEMU would put in the next
/// page's block, or before an instruction that can't be fetched.
pub fn block_at(mem: &Mem, addr: u32, it: u8, fpca: bool) -> Result<Block, CpuError> {
    let page = addr & !(PAGE - 1);
    let mut ops = Vec::new();
    let mut budget = Budget::new(it, fpca);
    let mut pc = addr;
    loop {
        let first = ops.is_empty();
        let fault = || {
            Err(CpuError {
                kind: "FETCH_UNMAPPED".to_string(),
                pc,
            })
        };
        let Some(hw1) = mem.fetch16(pc) else {
            if first {
                return fault();
            }
            break;
        };
        let hw2 = if is32(hw1) {
            match mem.fetch16(pc.wrapping_add(2)) {
                Some(h) => h,
                None if first => return fault(),
                None => break,
            }
        } else {
            0
        };
        let op = decode(pc, hw1, hw2);
        ops.push(op);
        pc = pc.wrapping_add(op.len as u32);
        if op.k.ends_block() {
            break;
        }
        // translator_loop: the op buffer is full, or the instruction limit.
        budget.add(&op);
        if budget.full() || ops.len() >= MAX_INSNS {
            break;
        }
        let off = pc.wrapping_sub(page);
        if off >= PAGE {
            break;
        }
        if off == PAGE - 2 && mem.fetch16(pc).is_some_and(is32) {
            break;
        }
    }
    Ok(Block {
        size: pc.wrapping_sub(addr),
        ops,
        it,
        fpca,
    })
}

/// The single instruction at `addr` (for trampolines, which run one
/// instruction at a time).
pub fn one_at(mem: &Mem, addr: u32) -> Result<Op, CpuError> {
    let fault = || CpuError {
        kind: "FETCH_UNMAPPED".to_string(),
        pc: addr,
    };
    let hw1 = mem.fetch16(addr).ok_or_else(fault)?;
    let hw2 = if is32(hw1) {
        mem.fetch16(addr.wrapping_add(2)).ok_or_else(fault)?
    } else {
        0
    };
    Ok(decode(addr, hw1, hw2))
}
