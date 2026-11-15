//! Instruction semantics of the interpreter. Each test maps a code page at
//! `CODE` and a data page at `DATA`, writes halfwords and runs them until
//! the address after the last one. Expected values follow the Armv7-M ARM
//! pseudocode; `pdsim-arm/tests/thumb_vs_unicorn.rs` checks the same
//! programs against Unicorn.

use pdsim_cpu::{Cpu, CpuError, Hooks, Reg, Region};
use pdsim_thumb::ThumbCpu;

const CODE: u32 = 0x1000_0000;
const DATA: u32 = 0x3000_0000;

#[derive(Default)]
struct St {
    blocks: Vec<(u32, u32)>,
}

struct H;

impl Hooks<St> for H {
    fn on_block(&self, _: &mut dyn Cpu<St>, st: &mut St, addr: u32, size: u32) {
        st.blocks.push((addr, size));
    }
    fn on_trampoline(&self, _: &mut dyn Cpu<St>, _: &mut St, _: u32) {}
}

/// Runs `code` from `CODE` until the address after it; returns the CPU, the
/// result and the blocks reported to `on_block`.
fn run_full(
    code: &[u16],
    until: u32,
    setup: impl FnOnce(&mut ThumbCpu),
) -> (ThumbCpu, Result<(), CpuError>, Vec<(u32, u32)>) {
    let mut cpu = ThumbCpu::new();
    Cpu::<St>::mem_map(&mut cpu, CODE, 0x1_0000, Region::Code).unwrap();
    Cpu::<St>::mem_map(&mut cpu, DATA, 0x1_0000, Region::Data).unwrap();
    let bytes: Vec<u8> = code.iter().flat_map(|h| h.to_le_bytes()).collect();
    Cpu::<St>::mem_write(&mut cpu, CODE, &bytes).unwrap();
    Cpu::<St>::reg_write(&mut cpu, Reg::Sp, DATA + 0x8000);
    setup(&mut cpu);
    let mut st = St::default();
    let r = cpu.emu_start(CODE | 1, until, &mut st, &H);
    (cpu, r, st.blocks)
}

fn run(code: &[u16], setup: impl FnOnce(&mut ThumbCpu)) -> ThumbCpu {
    let (cpu, r, _) = run_full(code, CODE + 2 * code.len() as u32, setup);
    r.unwrap();
    cpu
}

fn rd(cpu: &ThumbCpu, r: Reg) -> u32 {
    Cpu::<St>::reg_read(cpu, r)
}

fn wr(cpu: &mut ThumbCpu, r: Reg, v: u32) {
    Cpu::<St>::reg_write(cpu, r, v)
}

fn nzcv(cpu: &ThumbCpu) -> u32 {
    rd(cpu, Reg::Cpsr) >> 28
}

const Q: u32 = 1 << 27;

#[test]
fn adds_sets_carry_and_overflow() {
    // adds r0, r0, r1
    let cpu = run(&[0x1840], |c| {
        wr(c, Reg::R(0), 0x7fff_ffff);
        wr(c, Reg::R(1), 1);
        wr(c, Reg::Cpsr, 0);
    });
    assert_eq!(rd(&cpu, Reg::R(0)), 0x8000_0000);
    assert_eq!(nzcv(&cpu), 0b1001);
    // 0xffffffff + 1: carry out, no overflow, zero.
    let cpu = run(&[0x1840], |c| {
        wr(c, Reg::R(0), 0xffff_ffff);
        wr(c, Reg::R(1), 1);
    });
    assert_eq!(rd(&cpu, Reg::R(0)), 0);
    assert_eq!(nzcv(&cpu), 0b0110);
}

#[test]
fn adc_sbc_use_and_set_carry() {
    // sbcs r0, r1 ; adcs r2, r3
    let cpu = run(&[0x4188, 0x415a], |c| {
        wr(c, Reg::Cpsr, 0);
        wr(c, Reg::R(0), 10);
        wr(c, Reg::R(1), 3);
        wr(c, Reg::R(2), 0xffff_ffff);
        wr(c, Reg::R(3), 0);
    });
    // C clear: 10 - 3 - 1; no borrow, so C is set for the ADCS.
    assert_eq!(rd(&cpu, Reg::R(0)), 6);
    // 0xffffffff + 0 + 1 = 0 with carry out.
    assert_eq!(rd(&cpu, Reg::R(2)), 0);
    assert_eq!(nzcv(&cpu), 0b0110);
}

#[test]
fn shifted_register_carry_out() {
    // ands.w r0, r1, r2, lsr #32
    let cpu = run(&[0xea11, 0x0012], |c| {
        wr(c, Reg::Cpsr, 0);
        wr(c, Reg::R(1), 0xffff_ffff);
        wr(c, Reg::R(2), 0x8000_0000);
    });
    assert_eq!(rd(&cpu, Reg::R(0)), 0);
    assert_eq!(nzcv(&cpu), 0b0110, "C is bit 31 of r2");
    let cpu = run(&[0xea11, 0x0012], |c| {
        wr(c, Reg::Cpsr, 0x2000_0000);
        wr(c, Reg::R(1), 0xffff_ffff);
        wr(c, Reg::R(2), 0x7fff_ffff);
    });
    assert_eq!(nzcv(&cpu), 0b0100);
}

#[test]
fn it_block_with_flag_setting_inside() {
    // cmp r0, #0 ; itte eq ; adds.w r1, r2, r3 ; moveq r4, #1 ; movne r5, #1
    let code = [0x2800, 0xbf06, 0xeb12, 0x0103, 0x2401, 0x2501];
    let cpu = run(&code, |c| {
        wr(c, Reg::R(0), 0);
        wr(c, Reg::R(2), 1);
        wr(c, Reg::R(3), 1);
        wr(c, Reg::R(4), 7);
        wr(c, Reg::R(5), 7);
    });
    assert_eq!(rd(&cpu, Reg::R(1)), 2);
    // ADDS cleared Z, so the T slot (EQ) is skipped and the E slot (NE) runs.
    assert_eq!(rd(&cpu, Reg::R(4)), 7);
    assert_eq!(rd(&cpu, Reg::R(5)), 1);
    // Inside the IT block the 16-bit MOVs do not set flags.
    assert_eq!(nzcv(&cpu), 0b0000);
    // With r0 != 0 nothing but the E slot runs.
    let cpu = run(&code, |c| {
        wr(c, Reg::R(0), 1);
        wr(c, Reg::R(1), 9);
        wr(c, Reg::R(4), 7);
        wr(c, Reg::R(5), 7);
    });
    assert_eq!(rd(&cpu, Reg::R(1)), 9);
    assert_eq!(rd(&cpu, Reg::R(4)), 7);
    assert_eq!(rd(&cpu, Reg::R(5)), 1);
}

#[test]
fn ssat_usat_set_q() {
    // ssat r0, #8, r1
    let cpu = run(&[0xf301, 0x0007], |c| {
        wr(c, Reg::Cpsr, 0);
        wr(c, Reg::R(1), 300);
    });
    assert_eq!(rd(&cpu, Reg::R(0)), 127);
    assert_eq!(rd(&cpu, Reg::Cpsr) & Q, Q);
    // usat r2, #8, r3
    let cpu = run(&[0xf383, 0x0208], |c| {
        wr(c, Reg::Cpsr, 0);
        wr(c, Reg::R(3), -5i32 as u32);
    });
    assert_eq!(rd(&cpu, Reg::R(2)), 0);
    assert_eq!(rd(&cpu, Reg::Cpsr) & Q, Q);
    // In range: Q untouched.
    let cpu = run(&[0xf383, 0x0208], |c| {
        wr(c, Reg::Cpsr, 0);
        wr(c, Reg::R(3), 200);
    });
    assert_eq!(rd(&cpu, Reg::R(2)), 200);
    assert_eq!(rd(&cpu, Reg::Cpsr) & Q, 0);
}

#[test]
fn umull_smlal_sdiv_udiv() {
    let code = [
        0xfba2, 0x0103, // umull r0, r1, r2, r3
        0xfbc2, 0x4503, // smlal r4, r5, r2, r3
        0xfb97, 0xf6f8, // sdiv r6, r7, r8
        0xfb9a, 0xf9fb, // sdiv r9, r10, r11
        0xfbb2, 0xfcf3, // udiv r12, r2, r3
    ];
    let cpu = run(&code, |c| {
        wr(c, Reg::R(2), 0xffff_ffff);
        wr(c, Reg::R(3), 2);
        wr(c, Reg::R(4), 1);
        wr(c, Reg::R(5), 0);
        wr(c, Reg::R(7), 100);
        wr(c, Reg::R(8), 0);
        wr(c, Reg::R(10), 0x8000_0000);
        wr(c, Reg::R(11), 0xffff_ffff);
    });
    assert_eq!((rd(&cpu, Reg::R(0)), rd(&cpu, Reg::R(1))), (0xffff_fffe, 1));
    // 1 + (-1 * 2) = -1
    assert_eq!(
        (rd(&cpu, Reg::R(4)), rd(&cpu, Reg::R(5))),
        (0xffff_ffff, 0xffff_ffff)
    );
    assert_eq!(rd(&cpu, Reg::R(6)), 0, "division by zero gives 0");
    assert_eq!(rd(&cpu, Reg::R(9)), 0x8000_0000, "INT_MIN / -1 = INT_MIN");
    assert_eq!(rd(&cpu, Reg::R(12)), 0x7fff_ffff);
}

#[test]
fn ldm_stm_push_pop_writeback() {
    let code = [
        0xc006, // stm r0!, {r1, r2}
        0xe930, 0x0018, // ldmdb r0!, {r3, r4}
        0xb502, // push {r1, lr}
        0xbd10, // pop {r4, pc}
        0x2601, // movs r6, #1 (skipped)
        0x2701, // movs r7, #1 (the return address)
    ];
    let ret = CODE + 12;
    let (cpu, r, blocks) = run_full(&code, ret + 2, |c| {
        wr(c, Reg::R(0), DATA);
        wr(c, Reg::R(1), 0x1111);
        wr(c, Reg::R(2), 0x2222);
        wr(c, Reg::Lr, ret | 1);
    });
    r.unwrap();
    assert_eq!(rd(&cpu, Reg::R(0)), DATA);
    assert_eq!(rd(&cpu, Reg::R(3)), 0x1111);
    assert_eq!(rd(&cpu, Reg::R(4)), 0x1111, "pop {{r4}} loads the pushed r1");
    assert_eq!(rd(&cpu, Reg::Sp), DATA + 0x8000);
    assert_eq!(rd(&cpu, Reg::R(6)), 0);
    assert_eq!(rd(&cpu, Reg::R(7)), 1);
    assert_eq!(rd(&cpu, Reg::Pc), ret + 2);
    assert_eq!(blocks, vec![(CODE, 10), (ret, 2)], "POP {{pc}} ends the block");
}

#[test]
fn tbb_tbh_branch() {
    let tbb = [
        0xe8df, 0xf000, // tbb [pc, r0]
        0x0102, // entries 2 (-> +4 = 0x8) and 1 (-> +2 = 0x6)
        0x2102, // 0x6: movs r1, #2
        0x2201, // 0x8: movs r2, #1
    ];
    let (cpu, r, blocks) = run_full(&tbb, CODE + 10, |c| wr(c, Reg::R(0), 1));
    r.unwrap();
    assert_eq!((rd(&cpu, Reg::R(1)), rd(&cpu, Reg::R(2))), (2, 1));
    assert_eq!(blocks, vec![(CODE, 4), (CODE + 6, 4)]);
    let tbh = [
        0xe8df, 0xf010, // tbh [pc, r0, lsl #1]
        0x0003, 0x0002, // entries 3 (-> 0xa) and 2 (-> 0x8)
        0x2102, // 0x8: movs r1, #2
        0x2201, // 0xa: movs r2, #1
    ];
    let (cpu, r, _) = run_full(&tbh, CODE + 12, |c| wr(c, Reg::R(0), 1));
    r.unwrap();
    assert_eq!((rd(&cpu, Reg::R(1)), rd(&cpu, Reg::R(2))), (2, 1));
    let (cpu, r, _) = run_full(&tbh, CODE + 12, |c| wr(c, Reg::R(0), 0));
    r.unwrap();
    assert_eq!((rd(&cpu, Reg::R(1)), rd(&cpu, Reg::R(2))), (0, 1));
}

#[test]
fn simd_uadd8_and_sel() {
    // uadd8 r0, r1, r2 ; sel r3, r4, r5
    let cpu = run(&[0xfa81, 0xf042, 0xfaa4, 0xf385], |c| {
        wr(c, Reg::R(1), 0xff01_ff01);
        wr(c, Reg::R(2), 0x0101_0101);
        wr(c, Reg::R(4), 0x1111_1111);
        wr(c, Reg::R(5), 0x2222_2222);
    });
    assert_eq!(rd(&cpu, Reg::R(0)), 0x0002_0002);
    assert_eq!((rd(&cpu, Reg::Cpsr) >> 16) & 0xf, 0b1010);
    assert_eq!(rd(&cpu, Reg::R(3)), 0x1122_1122);
}

#[test]
fn vfp_nan_and_default_nan() {
    // vadd.f32 s0, s1, s2
    let cpu = run(&[0xee30, 0x0a81], |c| {
        wr(c, Reg::S(1), 0x7fc0_0001);
        wr(c, Reg::S(2), 1.0f32.to_bits());
    });
    assert_eq!(rd(&cpu, Reg::S(0)), 0x7fc0_0001);
    // An sNaN is quieted and raises Invalid Operation.
    let cpu = run(&[0xee30, 0x0a81], |c| {
        wr(c, Reg::S(1), 1.0f32.to_bits());
        wr(c, Reg::S(2), 0x7f80_0001);
    });
    assert_eq!(rd(&cpu, Reg::S(0)), 0x7fc0_0001);
    assert_eq!(rd(&cpu, Reg::Fpscr) & 1, 1);
    // vmsr fpscr, r0 (DN) ; vadd.f32 s0, s1, s2
    let cpu = run(&[0xeee1, 0x0a10, 0xee30, 0x0a81], |c| {
        wr(c, Reg::R(0), 1 << 25);
        wr(c, Reg::S(1), 0x7fc0_0001);
        wr(c, Reg::S(2), 1.0f32.to_bits());
    });
    assert_eq!(rd(&cpu, Reg::S(0)), 0x7fc0_0000);
}

#[test]
fn vcvt_saturates_and_rounds() {
    let code = [
        0xeebc, 0x0ae0, // vcvt.u32.f32 s0, s1
        0xeebc, 0x1ae1, // vcvt.u32.f32 s2, s3
        0xeebd, 0x2a62, // vcvtr.s32.f32 s4, s5
    ];
    let cpu = run(&code, |c| {
        wr(c, Reg::S(1), (-1.5f32).to_bits());
        wr(c, Reg::S(3), 5e9f32.to_bits());
        wr(c, Reg::S(5), 2.5f32.to_bits());
    });
    assert_eq!(rd(&cpu, Reg::S(0)), 0);
    assert_eq!(rd(&cpu, Reg::S(2)), 0xffff_ffff);
    assert_eq!(rd(&cpu, Reg::S(4)), 2, "ties to even");
    // Invalid Operation (saturation) and Inexact (2.5 -> 2).
    assert_eq!(rd(&cpu, Reg::Fpscr) & 0x1f, 0x11);
}

#[test]
fn fz_flushes_denormals() {
    // vmsr fpscr, r0 ; vmul.f32 s0, s1, s2
    let code = [0xeee1, 0x0a10, 0xee20, 0x0a81];
    let cpu = run(&code, |c| {
        wr(c, Reg::R(0), 1 << 24);
        wr(c, Reg::S(1), 1);
        wr(c, Reg::S(2), 2.0f32.to_bits());
    });
    assert_eq!(rd(&cpu, Reg::S(0)), 0);
    assert_eq!(rd(&cpu, Reg::Fpscr) & 0x80, 0x80, "Input Denormal");
    let cpu = run(&code, |c| {
        wr(c, Reg::R(0), 0);
        wr(c, Reg::S(1), 1);
        wr(c, Reg::S(2), 2.0f32.to_bits());
    });
    assert_eq!(rd(&cpu, Reg::S(0)), 2);
}

#[test]
fn vcmp_vmrs_sets_apsr() {
    // vcmp.f32 s0, s1 ; vmrs APSR_nzcv, fpscr
    let code = [0xeeb4, 0x0a60, 0xeef1, 0xfa10];
    let cpu = run(&code, |c| {
        wr(c, Reg::S(0), f32::NAN.to_bits());
        wr(c, Reg::S(1), 1.0f32.to_bits());
    });
    assert_eq!(nzcv(&cpu), 0b0011);
    assert_eq!(rd(&cpu, Reg::Fpscr) & 1, 0, "VCMP is quiet for a qNaN");
    let cpu = run(&code, |c| {
        wr(c, Reg::S(0), 1.0f32.to_bits());
        wr(c, Reg::S(1), 2.0f32.to_bits());
    });
    assert_eq!(nzcv(&cpu), 0b1000);
    let cpu = run(&code, |c| {
        wr(c, Reg::S(0), 2.0f32.to_bits());
        wr(c, Reg::S(1), 2.0f32.to_bits());
    });
    assert_eq!(nzcv(&cpu), 0b0110);
}

#[test]
fn unknown_instruction_crashes_like_unicorn() {
    let (cpu, r, blocks) = run_full(&[0xde00, 0xbf00], CODE + 4, |_| {});
    let e = r.unwrap_err();
    assert_eq!(e.kind, "INSN_INVALID");
    assert_eq!(e.pc, CODE);
    assert_eq!(rd(&cpu, Reg::Pc), CODE);
    assert_eq!(blocks, vec![(CODE, 2)], "the block ends at the UDF");
}

#[test]
fn unmapped_read_is_a_crash() {
    // nop ; ldr r0, [r1]
    let (_, r, _) = run_full(&[0xbf00, 0x6808], CODE + 4, |c| wr(c, Reg::R(1), 0));
    let e = r.unwrap_err();
    assert_eq!(e.kind, "READ_UNMAPPED");
    assert_eq!(e.pc, CODE + 2);
}

#[test]
fn store_fault_in_it_block_is_reported_at_block_end() {
    // it eq ; str r0, [r1] (unmapped) ; movs r3, #1 ; nop ; b.n next
    let code = [0xbf08, 0x6008, 0x2301, 0xbf00, 0xe7ff, 0xbf00];
    let (cpu, r, _) = run_full(&code, CODE + 12, |c| {
        wr(c, Reg::R(1), 0x5000_0000);
        wr(c, Reg::Cpsr, 1 << 30);
    });
    let e = r.unwrap_err();
    assert_eq!(e.kind, "WRITE_UNMAPPED");
    assert_eq!(e.pc, CODE + 10, "after the block that holds the store");
    assert_eq!(rd(&cpu, Reg::R(3)), 1, "the rest of the block ran");
}

#[test]
fn store_fault_in_it_block_is_reported_at_next_access() {
    // it eq ; str r0, [r1] (unmapped) ; movs r3, #1 ; ldr r4, [r7] ; nop
    let code = [0xbf08, 0x6008, 0x2301, 0x683c, 0xbf00];
    let (cpu, r, _) = run_full(&code, CODE + 10, |c| {
        wr(c, Reg::R(1), 0x5000_0000);
        wr(c, Reg::R(4), 7);
        wr(c, Reg::R(7), DATA);
        wr(c, Reg::Cpsr, 1 << 30);
    });
    let e = r.unwrap_err();
    assert_eq!(e.kind, "WRITE_UNMAPPED");
    assert_eq!(e.pc, CODE + 6, "at the load");
    assert_eq!(rd(&cpu, Reg::R(4)), 7, "the load wrote nothing");
}
