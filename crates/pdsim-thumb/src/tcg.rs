//! QEMU's TCG op budget: how many TCG ops Unicorn 2 (QEMU 5) emits for
//! each instruction, so blocks end where `translator_loop` ends them when
//! `tcg_op_buf_full` (4000 ops) is reached.
//!
//! The counts mirror `target/arm/translate.c` and the constant folding of
//! `tcg/tcg-op.c` for an x86-64 host (which ops are native, and which
//! extract/deposit shapes are single ops there), with Unicorn's additions:
//! a block hook at the start of every block and an exit-request check
//! after every guest memory access outside IT blocks.

use crate::decode::{Alu, Am, Dsp, Ext, Idx, Misc, MulL, Op, Opnd, QOp, Rm, Sf, Sh, K, NOREG, V2, V3};

/// `tcg_op_buf_full`: translation stops after the instruction that takes
/// the op count to this.
pub const MAX_OPS: u32 = 4000;

/// `TCG_MAX_INSNS`.
pub const MAX_INSNS: usize = 512;

/// The ops of the block being translated so far, and the state the next
/// instruction's ops depend on.
#[derive(Debug, Clone)]
pub struct Budget {
    pub ops: u32,
    /// ITSTATE before the next instruction.
    it: u8,
    /// CONTROL.FPCA before the next instruction.
    fpca: bool,
}

/// ITSTATE after an instruction in an IT block.
pub fn it_advance(it: u8) -> u8 {
    if it & 7 == 0 {
        0
    } else {
        (it & 0xe0) | ((it << 1) & 0x1f)
    }
}

impl Budget {
    /// A block starting with ITSTATE `it` and CONTROL.FPCA `fpca`.
    pub fn new(it: u8, fpca: bool) -> Budget {
        // gen_tb_start, Unicorn's block hook and the exit check; restoring
        // the IT bits when the block starts inside an IT block.
        let ops = 9 + if it != 0 { 2 } else { 0 };
        Budget { ops, it, fpca }
    }

    /// Whether translation stops after the instruction just added.
    pub fn full(&self) -> bool {
        self.ops >= MAX_OPS
    }

    /// Adds `op` and returns its op count.
    pub fn add(&mut self, op: &Op) -> u32 {
        let in_it = self.it & 0xf != 0;
        let mut n = 1; // insn_start
        if in_it && !matches!(op.k, K::Bkpt) {
            n += cond_ops(self.it >> 4);
        }
        if is_fp(&op.k) && !self.fpca {
            // vfp_access_check: a new FP context.
            n += 6;
            self.fpca = true;
        }
        n += body(&op.k, in_it);
        self.it = match op.k {
            K::It(v) => v,
            _ if in_it => it_advance(self.it),
            _ => 0,
        };
        self.ops += n;
        n
    }
}

/// A floating-point instruction (one that runs `vfp_access_check`).
pub(crate) fn is_fp(k: &K) -> bool {
    matches!(
        k,
        K::V3 { .. }
            | K::V2 { .. }
            | K::VFix { .. }
            | K::VMovI { .. }
            | K::VCmp { .. }
            | K::VMovR { .. }
            | K::VMov2 { .. }
            | K::Vmrs { .. }
            | K::Vmsr { .. }
            | K::VLdr { .. }
            | K::VLdm { .. }
    )
}

/// `arm_skip_unless`: the condition test, branch and label.
fn cond_ops(cond: u8) -> u32 {
    match cond >> 1 {
        0..=3 => 3, // eq/ne cs/cc mi/pl vs/vc
        4 => 5,     // hi/ls
        5 => 4,     // ge/lt
        6 => 7,     // gt/le
        _ => 0,     // al
    }
}

/// `tcg_gen_andi_i32` from a different temp.
fn andi(mask: u32) -> u32 {
    match mask {
        0 | 0xff | 0xffff => 1,
        _ => 2,
    }
}

/// `tcg_gen_{shl,shr,sar,rot}i_i32` in place.
fn shifti(n: u32) -> u32 {
    if n == 0 {
        0
    } else {
        2
    }
}

/// `tcg_gen_extract_i32`.
fn extract(ofs: u32, len: u32) -> u32 {
    if ofs + len == 32 {
        2
    } else if ofs == 0 {
        andi((1u32 << len) - 1)
    } else if (ofs, len) == (8, 8) {
        1
    } else if ofs + len == 16 || ofs + len == 8 || len == 16 || len == 8 {
        3
    } else {
        4
    }
}

/// `tcg_gen_sextract_i32`.
fn sextract(ofs: u32, len: u32) -> u32 {
    if ofs + len == 32 {
        2
    } else if (ofs == 0 && (len == 8 || len == 16)) || (ofs, len) == (8, 8) {
        1
    } else if ofs + len == 16 || ofs + len == 8 || len == 16 || len == 8 {
        3
    } else {
        4
    }
}

/// `tcg_gen_deposit_i32` into a different temp.
fn deposit(ofs: u32, len: u32) -> u32 {
    match (ofs, len) {
        (_, 32) => 0,
        (0, 8) | (8, 8) | (0, 16) => 1,
        _ if ofs + len == 32 || ofs == 0 => 3,
        _ => andi((1u32 << len) - 1) + 2 + 2 + 1,
    }
}

/// `gen_arm_shift_im`.
fn shift_im(sh: Sh, n: u8, flags: bool) -> u32 {
    let n = n as u32;
    let out = |ofs: u32| if flags { extract(ofs, 1) } else { 0 };
    match sh {
        Sh::Lsl if n == 0 => 0,
        Sh::Lsl => out(32 - n) + 2,
        Sh::Lsr if n == 0 || n == 32 => 2 * flags as u32 + 1,
        Sh::Lsr | Sh::Ror => out(n - 1) + 2,
        Sh::Asr => out(if n == 0 { 31 } else { n - 1 }) + 2,
        Sh::Rrx => 2 + out(0) + 2 + 1,
    }
}

/// `gen_arm_shift_reg`.
fn shift_reg(sh: Sh, flags: bool) -> u32 {
    match (flags, sh) {
        (true, _) => 1,
        (_, Sh::Lsl | Sh::Lsr) => 7,
        (_, Sh::Asr) => 4,
        _ => 3,
    }
}

/// A guest load or store: the address extension, the access and (outside
/// IT blocks) Unicorn's exit-request check.
fn mem(in_it: bool) -> u32 {
    2 + if in_it { 0 } else { 3 }
}

/// `op_addr_ri_pre` (with the stack-limit check) for `am`.
fn ri_pre(am: &Am) -> u32 {
    let (p, w) = (am.idx != Idx::Post, am.idx != Idx::Off);
    let ofs = am.imm;
    let mut n = if am.rn == 15 || !p || ofs == 0 { 1 } else { 2 };
    if am.rn == 13 && w {
        n += 1 + if ofs < 0 { 2 } else { 0 };
    }
    n
}

/// `op_addr_ri_post` with `off` added to the post-index offset.
fn ri_post(am: &Am, off: i32) -> u32 {
    match am.idx {
        Idx::Off => 0,
        Idx::Pre => shifti(off as u32) + 1,
        Idx::Post => shifti(am.imm.wrapping_add(off) as u32) + 1,
    }
}

/// `op_addr_rr_pre` (Thumb only has offset addressing with a register).
fn rr_pre(am: &Am) -> u32 {
    1 + 1 + shifti(am.sh as u32) + 1
}

/// The address computation of a load or store, before and after.
fn ldst(am: &Am) -> u32 {
    if am.rm == NOREG {
        ri_pre(am) + ri_post(am, 0)
    } else {
        rr_pre(am)
    }
}

fn body(k: &K, in_it: bool) -> u32 {
    let flags = |s: Sf| s == Sf::Yes || (s == Sf::OutIt && !in_it);
    let ld = mem(in_it);
    match *k {
        K::Undef | K::Nop | K::It(_) | K::Barrier { .. } => 0,
        K::Alu { op, s, rd, rn, b } => {
            let s = flags(s);
            let logic = matches!(
                op,
                Alu::And
                    | Alu::Eor
                    | Alu::Orr
                    | Alu::Orn
                    | Alu::Bic
                    | Alu::Mov
                    | Alu::Mvn
                    | Alu::Tst
                    | Alu::Teq
            );
            let lcc = s && logic;
            let mut n = match b {
                Opnd::Imm(_, c) => 1 + (lcc && c.is_some()) as u32,
                Opnd::Sh(_, sh, k) => 1 + shift_im(sh, k, lcc),
                Opnd::ShR(_, sh, _) => 2 + shift_reg(sh, lcc),
            };
            if !matches!(op, Alu::Mov | Alu::Mvn) {
                n += 1;
            }
            n += match op {
                Alu::Mov => 0,
                Alu::And | Alu::Eor | Alu::Orr | Alu::Bic | Alu::Mvn | Alu::Tst | Alu::Teq => 1,
                Alu::Orn => 2,
                Alu::Add | Alu::Sub | Alu::Rsb => {
                    if s {
                        7
                    } else {
                        1
                    }
                }
                Alu::Cmp | Alu::Cmn => 7,
                Alu::Adc => 2 + 6 * s as u32,
                Alu::Sbc => 4 + 5 * s as u32,
            };
            if lcc {
                n += 2;
            }
            n + match op {
                Alu::Tst | Alu::Teq | Alu::Cmp | Alu::Cmn => 0,
                Alu::Add | Alu::Sub if rd == 13 && rn == 13 => 2,
                Alu::Mov if rd == 13 => 2,
                _ => 1,
            }
        }
        K::Adr { .. } | K::Movw { .. } => 2,
        K::Movt { .. } => 5,
        K::Mul { s, .. } => 4 + 2 * flags(s) as u32,
        K::Mla { .. } => 6,
        K::Div { .. } => 4,
        K::MulL { op, .. } => match op {
            MulL::Smull | MulL::Umull => 5,
            MulL::Smlal | MulL::Umlal => 8,
            MulL::Umaal => 10,
            MulL::SmlalXy(x, y) => 2 + mulxy(x, y) + 2 + 2 + 1 + 2,
            MulL::Smlald(x) | MulL::Smlsld(x) => 26 + 2 * x as u32,
        },
        K::Dsp { op, ra, .. } => {
            let acc = ra != 15;
            match op {
                Dsp::SmlaXy(x, y) => 2 + mulxy(x, y) + 2 * acc as u32 + 1,
                Dsp::SmlaW(_) => 2 + 2 + 1 + 2 * acc as u32 + 1,
                Dsp::Smlad(x) | Dsp::Smlsd(x) => 2 + 2 * x as u32 + 9 + 1 + 2 * acc as u32 + 1,
                Dsp::Smmla(r) => 2 + 1 + 2 * acc as u32 + 3 * r as u32 + 1,
                Dsp::Smmls(r) => 2 + 1 + if acc { 3 } else { 0 } + 3 * r as u32 + 1,
                Dsp::Usada8 => 2 + 1 + 2 * acc as u32 + 1,
            }
        }
        K::Sat { sh, n, .. } => 1 + if sh == Sh::Asr || n != 0 { 2 } else { 0 } + 3,
        K::Sat16 { .. } => 4,
        K::Bfx {
            signed, lsb, width, ..
        } => {
            2 + if signed {
                sextract(lsb as u32, width as u32)
            } else {
                extract(lsb as u32, width as u32)
            }
        }
        K::Bfi { lsb, msb, .. } => {
            let w = msb as u32 + 1 - lsb as u32;
            1 + if w != 32 { 1 + deposit(lsb as u32, w) } else { 0 } + 1
        }
        K::Ext { op, rn, rot, .. } => {
            let b16 = matches!(op, Ext::Sxtb16 | Ext::Uxtb16);
            1 + shifti(rot as u32) + 1 + if rn != 15 { 1 + if b16 { 9 } else { 1 } } else { 0 } + 1
        }
        K::Misc { op, .. } => match op {
            Misc::Clz => 4,
            Misc::Rbit | Misc::Rev => 3,
            Misc::Rev16 => 10,
            Misc::Revsh => 5,
        },
        K::Sel { .. } => 5,
        K::Par { kind, .. } => 4 + if kind & 3 == 0 { 2 } else { 0 },
        K::Q { op, .. } => 4 + matches!(op, QOp::Qdadd | QOp::Qdsub) as u32,
        K::Pkh { tb, n, .. } => 2 + if tb { 2 } else { shifti(n as u32) } + 1 + 1,
        K::Ld { am, .. } => ldst(&am) + ld + 1,
        K::St { am, .. } => ldst(&am) + 1 + ld,
        K::LdLit { .. } => 1 + ld + 1,
        K::Ldrd { am, .. } => ri_pre(&am) + 2 * (ld + 1) + 2 + ri_post(&am, -4),
        K::Strd { am, .. } => ri_pre(&am) + 2 * (1 + ld) + 2 + ri_post(&am, -4),
        K::LdrdLit { .. } => 1 + 2 * (ld + 1) + 2,
        K::Ldm { rn, list, wb, db } | K::Stm { rn, list, wb, db } => {
            let n = list.count_ones();
            let mut c = 1 + if db { 2 } else { 0 } + (rn == 13 && wb) as u32;
            c += n * (ld + 1) + 2 * (n - 1);
            if wb {
                c += if db {
                    if n != 1 {
                        2
                    } else {
                        0
                    }
                } else {
                    2
                } + 1;
            }
            c
        }
        K::Ldrex { imm, .. } => 6 + ld - 2 + shifti(imm as u32),
        K::Strex { imm, .. } => 18 + 2 * (ld - 2) + shifti(imm as u32),
        K::Clrex => 1,
        K::Mrs { .. } => 3,
        // V* costs with FPCA already set.
        K::V3 { op, .. } => match op {
            V3::Mul | V3::Add | V3::Sub | V3::Div | V3::MaxNm | V3::MinNm => 6,
            V3::Nmul => 7,
            V3::Mla => 8,
            V3::Mls | V3::Nmls => 9,
            V3::Nmla => 10,
            V3::Fma { neg_n, neg_d } => 7 + neg_n as u32 + neg_d as u32,
            V3::Sel(c) => match c {
                0 | 6 => 5,
                10 => 6,
                _ => 7,
            },
        },
        K::V2 { op, .. } => match op {
            V2::Mov | V2::Abs | V2::Neg | V2::Sqrt => 3,
            V2::Rint(Rm::Fpscr) | V2::RintX | V2::FromInt { .. } => 5,
            V2::ToInt {
                rm: Rm::Fpscr | Rm::Z,
                ..
            } => 5,
            V2::Rint(_) => 8,
            V2::ToInt { .. } => 9,
            V2::FromHalf { .. } | V2::ToHalf { .. } => 10,
        },
        K::VFix { .. } => 6,
        K::VMovI { .. } | K::VMovR { .. } => 2,
        K::VMov2 { .. } => 4,
        K::VCmp { .. } => 3,
        K::Vmrs { rt } => {
            if rt == 15 {
                5
            } else {
                2
            }
        }
        K::VLdr { rn, imm, .. } => (if rn == 15 || imm == 0 { 1 } else { 2 }) + ld + 1,
        K::VLdm {
            dbl, n, rn, db, wb, ..
        } => {
            let regs = if dbl { n as u32 / 2 } else { n as u32 };
            1 + if db { 2 } else { 0 }
                + (rn == 13 && wb) as u32
                + regs * (ld + 1 + 2)
                + if wb {
                    if db {
                        3
                    } else {
                        1
                    }
                } else {
                    0
                }
        }
        // These end the block, so their own count doesn't matter.
        K::Tb { .. }
        | K::B { .. }
        | K::Bc { .. }
        | K::Bl { .. }
        | K::BlxI { .. }
        | K::Bx { .. }
        | K::Blx { .. }
        | K::Cbz { .. }
        | K::Svc
        | K::Bkpt
        | K::Wfi
        | K::Yield
        | K::Msr { .. }
        | K::Cps { .. }
        | K::Vmsr { .. } => 0,
    }
}

/// `gen_mulxy`.
fn mulxy(x: bool, y: bool) -> u32 {
    1 + x as u32 + 1 + y as u32 + 1
}
