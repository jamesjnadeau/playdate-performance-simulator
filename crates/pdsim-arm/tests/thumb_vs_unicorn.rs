//! Differential test: the pure-Rust Thumb interpreter against Unicorn.
//!
//! Two parts:
//! - targeted scenarios for the edges a game can observe (block
//!   boundaries, fault kinds and PCs, `until`, `emu_stop`, trampolines,
//!   lazy FP, special registers, `mem_map` errors);
//! - random instruction sequences built from encoding templates that
//!   avoid UNPREDICTABLE forms, comparing registers, flags, FPSCR, the S
//!   registers, memory and the block trace.

// Instruction encodings read better as plain shift-and-or chains.
#![allow(clippy::precedence)]

use pdsim_arm::cpu::*;
use pdsim_arm::Backend;

const C: u32 = 0x1000_0000;
const T: u32 = 0x2000_0000;
const D: u32 = 0x3000_0000;
const SP: u32 = D + 0x8000;
/// Where `r7` points in the random sequences (their memory base).
const BASE: u32 = D + 0x4000;

#[derive(Default)]
struct St {
    blocks: Vec<(u32, u32)>,
    tramp: Vec<u32>,
    stop_at_block: Option<usize>,
}

struct H;

impl Hooks<St> for H {
    fn on_block(&self, cpu: &mut dyn Cpu<St>, st: &mut St, a: u32, s: u32) {
        st.blocks.push((a, s));
        if st.stop_at_block == Some(st.blocks.len()) {
            cpu.emu_stop();
        }
    }
    fn on_trampoline(&self, cpu: &mut dyn Cpu<St>, st: &mut St, a: u32) {
        st.tramp.push(a);
        let lr = cpu.reg_read(Reg::Lr);
        cpu.reg_write(Reg::Pc, lr);
    }
}

/// Everything a run leaves behind that a game could observe.
#[derive(Debug, PartialEq)]
struct Snap {
    result: Result<(), (String, u32)>,
    r: Vec<u32>,
    cpsr: u32,
    fpscr: u32,
    s: Vec<u32>,
    blocks: Vec<(u32, u32)>,
    tramp: Vec<u32>,
    code: Vec<u8>,
    data: Vec<u8>,
}

impl Snap {
    /// The fields that differ, described.
    fn diff(&self, o: &Snap) -> Vec<String> {
        let mut out = Vec::new();
        if self.result != o.result {
            out.push(format!("result {:x?} vs {:x?}", self.result, o.result));
        }
        for i in 0..16 {
            if self.r[i] != o.r[i] {
                out.push(format!("r{i} {:#x} vs {:#x}", self.r[i], o.r[i]));
            }
        }
        if self.cpsr != o.cpsr {
            out.push(format!("cpsr {:#x} vs {:#x}", self.cpsr, o.cpsr));
        }
        if self.fpscr != o.fpscr {
            out.push(format!("fpscr {:#x} vs {:#x}", self.fpscr, o.fpscr));
        }
        for i in 0..32 {
            if self.s[i] != o.s[i] {
                out.push(format!("s{i} {:#x} vs {:#x}", self.s[i], o.s[i]));
            }
        }
        if self.blocks != o.blocks {
            out.push(format!("blocks {:x?} vs {:x?}", self.blocks, o.blocks));
        }
        if self.tramp != o.tramp {
            out.push(format!("tramp {:x?} vs {:x?}", self.tramp, o.tramp));
        }
        for (name, a, b, base) in [("code", &self.code, &o.code, C), ("data", &self.data, &o.data, D)] {
            if let Some(i) = (0..a.len()).find(|&i| a[i] != b[i]) {
                out.push(format!("{name} first differs at {:#x}", base + i as u32));
            }
        }
        out
    }
}

/// A scenario: code at `C`, a setup, and how to run it.
struct Case<'a> {
    code: Vec<u16>,
    setup: &'a dyn Fn(&mut dyn Cpu<St>),
    begin: u32,
    until: u32,
    stop_at_block: Option<usize>,
}

impl<'a> Case<'a> {
    fn new(code: &[u16], setup: &'a dyn Fn(&mut dyn Cpu<St>)) -> Case<'a> {
        Case {
            code: code.to_vec(),
            setup,
            begin: C | 1,
            until: C + 2 * code.len() as u32,
            stop_at_block: None,
        }
    }

    fn run(&self, backend: Backend) -> Snap {
        let mut cpu = backend.build::<St>().unwrap();
        cpu.mem_map(C, 0x10000, Region::Code).unwrap();
        cpu.mem_map(T, 0x1000, Region::Trampoline).unwrap();
        cpu.mem_map(D, 0x10000, Region::Data).unwrap();
        let b: Vec<u8> = self.code.iter().flat_map(|h| h.to_le_bytes()).collect();
        cpu.mem_write(C, &b).unwrap();
        cpu.reg_write(Reg::Sp, SP);
        (self.setup)(&mut *cpu);
        let mut st = St {
            stop_at_block: self.stop_at_block,
            ..St::default()
        };
        let result = cpu
            .emu_start(self.begin, self.until, &mut st, &H)
            .map_err(|e| (e.kind, e.pc));
        let mut code = vec![0; 0x10000];
        cpu.mem_read(C, &mut code).unwrap();
        let mut data = vec![0; 0x10000];
        cpu.mem_read(D, &mut data).unwrap();
        Snap {
            result,
            r: (0..13)
                .map(Reg::R)
                .chain([Reg::Sp, Reg::Lr, Reg::Pc])
                .map(|r| cpu.reg_read(r))
                .collect(),
            cpsr: cpu.reg_read(Reg::Cpsr),
            fpscr: cpu.reg_read(Reg::Fpscr),
            s: (0..32).map(|i| cpu.reg_read(Reg::S(i))).collect(),
            blocks: st.blocks,
            tramp: st.tramp,
            code,
            data,
        }
    }

    /// The differences between the backends, if any.
    fn compare(&self) -> Option<String> {
        let u = self.run(Backend::Unicorn);
        let t = self.run(Backend::Thumb);
        let d = u.diff(&t);
        (!d.is_empty()).then(|| format!("(unicorn vs thumb) {}", d.join("; ")))
    }
}

fn nothing(_: &mut dyn Cpu<St>) {}

#[test]
fn targeted_scenarios_match_unicorn() {
    let mut bad = Vec::new();
    let mut check = |name: &str, case: Case| {
        if let Some(d) = case.compare() {
            bad.push(format!("{name}: {d}"));
        }
    };
    let nop = 0xbf00u16;
    let with = |pre: usize, insn: &[u16]| {
        let mut v = vec![nop; pre];
        v.extend(insn);
        v.extend(vec![nop; 100]);
        v
    };

    // Page boundaries: QEMU never lets a block cross into the next 1 KiB
    // page.
    check("nops", Case::new(&vec![nop; 3000], &nothing));
    check("b.w at 1022", Case::new(&with(511, &[0xf000, 0xb800]), &nothing));
    for pre in [509, 510, 511, 2045, 2046, 2047] {
        check(
            &format!("mov.w after {pre}"),
            Case::new(&with(pre, &[0xea4f, 0x0001]), &nothing),
        );
    }

    // Instructions that end blocks or stop the run.
    for (name, insn) in [
        ("udf", &[0xde00u16][..]),
        ("svc", &[0xdf00]),
        ("bkpt", &[0xbe00]),
        ("yield", &[0xbf10]),
        ("wfi", &[0xbf30]),
        ("wfe", &[0xbf20]),
        ("sev", &[0xbf40]),
        ("blx imm", &[0xf000, 0xe800]),
        ("dsb isb", &[0xf3bf, 0x8f4f, 0xf3bf, 0x8f6f]),
        ("vmrs", &[0xeef1, 0x0a10]),
        ("it", &[0xbf08, 0x2001]),
        ("cps", &[0xb672]),
        ("msr", &[0xf380, 0x8800]),
        ("mrs", &[0xf3ef, 0x8000]),
        ("clrex", &[0xf3bf, 0x8f2f]),
        ("pld", &[0xf891, 0xf000]),
        ("mrs primask", &[0xf3ef, 0x8010]),
        ("mrs control", &[0xf3ef, 0x8014]),
        ("msr primask", &[0xf380, 0x8810]),
        ("dbg", &[0xf3af, 0x80f0]),
        ("nop.w", &[0xf3af, 0x8000]),
        ("vadd.f64", &[0xee30, 0x0b01]),
        ("vcvt.f64.f32", &[0xeeb7, 0x0ac0]),
        ("vmov.f64 imm", &[0xeeb7, 0x0b00]),
        ("bkpt in failing it", &[0xbf18, 0xbe00]),
        ("udf in failing it", &[0xbf18, 0xde00]),
    ] {
        check(name, Case::new(&with(1, insn), &nothing));
    }

    // Branches and interworking.
    check(
        "bx even",
        Case::new(&[nop, 0x4708, nop, nop], &|c| c.reg_write(Reg::R(1), C + 4)),
    );
    check(
        "pop pc even",
        Case::new(&[0xbd00, nop], &|c| {
            c.mem_write(SP, &(C + 2).to_le_bytes()).unwrap()
        }),
    );
    check(
        "bx even to data",
        Case::new(&[0x4708, nop], &|c| c.reg_write(Reg::R(1), D)),
    );
    check(
        "bx even unmapped",
        Case::new(&[0x4708, nop], &|c| c.reg_write(Reg::R(1), 0x5000_0000)),
    );
    check(
        "bx unmapped",
        Case::new(&[nop, 0x4708, nop], &|c| c.reg_write(Reg::R(1), 0x5000_0001)),
    );
    check(
        "bx even tramp",
        Case::new(&[0x4708, nop], &|c| {
            c.reg_write(Reg::R(1), T);
            c.reg_write(Reg::Lr, C | 3);
        }),
    );
    check(
        "bx odd tramp",
        Case::new(&[0x4708, nop, nop], &|c| {
            c.reg_write(Reg::R(1), T | 1);
            c.reg_write(Reg::Lr, C | 3);
        }),
    );
    check(
        "blx tramp",
        Case::new(&[0x4788, nop, nop], &|c| c.reg_write(Reg::R(1), T | 1)),
    );
    check(
        "fetch straddling the end",
        Case::new(&[nop, 0x4708], &|c| c.reg_write(Reg::R(1), (C + 0x10000 - 2) | 1)),
    );
    check(
        "ldr pc",
        Case::new(&[0xf8d1, 0xf000, nop], &|c| {
            c.reg_write(Reg::R(1), D);
            c.mem_write(D, &(C + 5).to_le_bytes()).unwrap();
        }),
    );
    check(
        "add pc",
        Case::new(&[0x4487, nop, nop, nop], &|c| c.reg_write(Reg::R(0), 0)),
    );
    check(
        "mov pc",
        Case::new(&[0x4687, nop, nop, nop], &|c| c.reg_write(Reg::R(0), C + 6)),
    );
    for a in [
        0x0u32,
        0x2000_1000,
        0x3fff_f000,
        0x4100_0000,
        0x6000_0000,
        0x9fff_f000,
        0xa000_0000,
        0xe000_0000,
        0xf000_0000,
        0xffff_f000,
    ] {
        check(
            &format!("bx {a:#x}"),
            Case::new(&[0x4708, nop], &move |c| c.reg_write(Reg::R(1), a | 1)),
        );
        check(
            &format!("ldr {a:#x}"),
            Case::new(&[0x6808, nop], &move |c| c.reg_write(Reg::R(1), a)),
        );
        check(
            &format!("str {a:#x}"),
            Case::new(&[0x6008, nop], &move |c| c.reg_write(Reg::R(1), a)),
        );
    }

    // Memory faults and alignment.
    for (name, insn, r1) in [
        ("ldr unaligned", &[0x6808u16][..], D + 1),
        ("ldrh unaligned", &[0x8808], D + 1),
        ("ldrd unaligned", &[0xe9d1, 0x0200], D + 2),
        ("ldm unaligned", &[0xc905], D + 2),
        ("ldrex unaligned", &[0xe851, 0x0f00], D + 2),
        ("vldr unaligned", &[0xed91, 0x0a00], D + 2),
        ("str tramp", &[0x6008], T),
        ("str unmapped", &[0x6008], 0x5000_0000),
        ("ldr straddle", &[0x6808], D + 0xfffe),
        ("ldr into code", &[0x6808], C),
        ("str into code", &[0x6008], C + 0x100),
    ] {
        check(
            name,
            Case::new(&with(1, insn), &move |c| c.reg_write(Reg::R(1), r1)),
        );
    }
    // Self-modifying code: a store into the next block.
    check(
        "smc",
        Case::new(&[0x8008, 0xe7ff, 0xde00, nop], &|c| {
            c.reg_write(Reg::R(0), nop as u32);
            c.reg_write(Reg::R(1), C + 4);
        }),
    );

    // Exclusives (QEMU's monitor).
    check(
        "strex ok",
        Case::new(&[0xe851, 0x0f00, 0xe841, 0x0200, nop], &|c| {
            c.reg_write(Reg::R(1), D);
            c.reg_write(Reg::R(2), 7);
        }),
    );
    check(
        "strex no monitor",
        Case::new(&[0xe841, 0x0200, nop], &|c| {
            c.reg_write(Reg::R(1), D);
            c.reg_write(Reg::R(2), 7);
        }),
    );
    check(
        "strex other address",
        Case::new(&[0xe851, 0x0f00, 0xe843, 0x0200, nop], &|c| {
            c.reg_write(Reg::R(1), D);
            c.reg_write(Reg::R(3), D + 4);
        }),
    );

    // Registers and special registers.
    check(
        "cpsr write",
        Case::new(&[nop], &|c| c.reg_write(Reg::Cpsr, 0xffff_ffff)),
    );
    check(
        "cpsr ge then mrs",
        Case::new(&[0xf3ef, 0x8000, nop], &|c| c.reg_write(Reg::Cpsr, 0x000f_0000)),
    );
    check(
        "ge via msr",
        Case::new(&[0xf380, 0x8c00, nop], &|c| c.reg_write(Reg::R(0), 0xf80f_0000)),
    );
    check(
        "ge via uadd8",
        Case::new(&[0xfa81, 0xf042, nop], &|c| {
            c.reg_write(Reg::R(1), 0xff00_ff00);
            c.reg_write(Reg::R(2), 0x0100_0100);
        }),
    );
    check(
        "fpscr write",
        Case::new(&[nop], &|c| c.reg_write(Reg::Fpscr, 0xffff_ffff)),
    );
    check(
        "fpscr lazy",
        Case::new(&[0xee30, 0x0a81, nop], &|c| c.reg_write(Reg::Fpscr, 0x0300_0000)),
    );
    check(
        "vmrs first",
        Case::new(&[0xeef1, 0x0a10, nop], &|c| c.reg_write(Reg::Fpscr, 0x0300_0000)),
    );
    check(
        "vmov first",
        Case::new(&[0xee10, 0x0a10, nop], &|c| c.reg_write(Reg::Fpscr, 0x0300_0000)),
    );
    check(
        "vmsr",
        Case::new(&[0xeee1, 0x0a10, 0xee30, 0x0a81, nop], &|c| {
            c.reg_write(Reg::R(0), 0xffff_ffff)
        }),
    );
    check(
        "mov sp odd",
        Case::new(&[0x4685, nop], &|c| c.reg_write(Reg::R(0), D + 3)),
    );
    check(
        "vsqrt -1",
        Case::new(&[0xeeb1, 0x0ae0, 0xee10, 0x0a10, nop], &|c| {
            c.reg_write(Reg::S(1), 0xbf80_0000)
        }),
    );

    // `until`, `emu_stop` and the start address.
    let mut case = Case::new(&[nop, 0xe7fd], &nothing);
    case.until = C + 100;
    case.stop_at_block = Some(3);
    check("stop in block hook", case);
    let mut case = Case::new(&[nop, nop, nop, 0xe7fb], &nothing);
    case.until = C + 4;
    check("until mid block", case);
    let mut case = Case::new(&[nop], &nothing);
    case.until = C;
    check("until at start", case);
    let mut case = Case::new(&[nop, nop], &nothing);
    case.begin = C;
    check("even begin", case);

    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn mem_map_errors_match_unicorn() {
    let calls = |b: Backend| {
        let mut c = b.build::<St>().unwrap();
        let mut buf = [0u8; 4];
        let e = |r: Result<(), CpuError>| r.map_err(|e| e.kind);
        vec![
            e(c.mem_map(0x1000_0400, 0x1000, Region::Data)),
            e(c.mem_map(0x1000_0000, 0x400, Region::Data)),
            e(c.mem_map(0x1000_0000, 0, Region::Data)),
            e(c.mem_map(0x1000_0000, 0x2000, Region::Data)),
            e(c.mem_map(0x1000_1000, 0x2000, Region::Data)),
            e(c.mem_read(0x2000_0000, &mut buf)),
            e(c.mem_read(0x1000_1ffe, &mut buf)),
            e(c.mem_write(0x2000_0000, &buf)),
            e(c.mem_map(0x1000_2000, 0x1000, Region::Code)),
            e(c.mem_read(0x1000_1ffe, &mut buf)),
        ]
    };
    assert_eq!(calls(Backend::Unicorn), calls(Backend::Thumb));
}

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn n(&mut self, k: u32) -> u32 {
        (self.next() % k as u64) as u32
    }
    fn b(&mut self) -> bool {
        self.next() & 1 != 0
    }
    fn pick<X: Copy>(&mut self, s: &[X]) -> X {
        s[self.n(s.len() as u32) as usize]
    }
    /// A low register other than the base `r7`.
    fn lo(&mut self) -> u16 {
        self.n(7) as u16
    }
    /// A data register for 32-bit encodings (not r7, sp or pc).
    fn reg(&mut self) -> u16 {
        self.pick(&[0, 1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12])
    }
    /// Two different data registers.
    fn reg2(&mut self) -> (u16, u16) {
        let a = self.reg();
        loop {
            let b = self.reg();
            if b != a {
                return (a, b);
            }
        }
    }
    fn int(&mut self) -> u32 {
        match self.n(4) {
            0 => self.pick(&[
                0,
                1,
                2,
                31,
                32,
                0xff,
                0xffff,
                0x7fff_ffff,
                0x8000_0000,
                0xffff_ffff,
            ]),
            1 => self.n(64),
            _ => self.next() as u32,
        }
    }
    fn float(&mut self) -> u32 {
        let r = self.next() as u32;
        let sign = r & 0x8000_0000;
        match self.n(6) {
            0 => {
                sign | self.pick(&[
                    0,
                    0x3f80_0000,
                    0x3f00_0000,
                    0x7f80_0000,
                    0x7fc0_0000,
                    0x7f80_0001,
                    0x7fa0_0000,
                    0x7f7f_ffff,
                    0x0080_0000,
                    0x0000_0001,
                    0x007f_ffff,
                    0x4f00_0000,
                    0x4f80_0000,
                    0x3fc0_0000,
                    0x4020_0000,
                    0x4b00_0000,
                ])
            }
            // Denormals.
            1 => sign | (r & 0x7f_ffff),
            // Near one, so results stay normal and exact cases appear.
            2 => sign | (0x3f00_0000 + (r & 0x01ff_ffff)),
            // Small integers.
            3 => (self.n(200) as i32 - 100) as f32 as u32 ^ sign,
            _ => r,
        }
    }
}

fn t32(a: u16, b: u16) -> Vec<u16> {
    vec![a, b]
}

/// A 16-bit Thumb data-processing or memory instruction (no branches).
fn gen16(g: &mut Rng) -> Vec<u16> {
    let (rd, rm, rn) = (g.lo(), g.lo(), g.lo());
    vec![match g.n(16) {
        14 => 0x4800 | rd << 8 | g.n(256) as u16,
        15 => 0x5000 | (g.n(8) as u16) << 9 | rm << 6 | 7 << 3 | rd,
        0 => g.n(3) as u16 * 0x800 | (g.n(32) as u16) << 6 | rm << 3 | rd,
        1 => 0x1800 | (g.n(4) as u16) << 9 | rm << 6 | rn << 3 | rd,
        2 => 0x2000 | (g.n(4) as u16) << 11 | rd << 8 | g.n(256) as u16,
        3 | 4 => 0x4000 | (g.n(16) as u16) << 6 | rm << 3 | rd,
        5 => {
            // ADD/CMP/MOV with a high register (CMP needs one).
            let op = g.n(3) as u16;
            let (a, b) = (g.reg(), g.reg());
            let (a, b) = if op == 1 && a < 8 && b < 8 {
                (a, 8 + g.n(5) as u16)
            } else {
                (a, b)
            };
            0x4400 | op << 8 | (a >> 3) << 7 | b << 3 | (a & 7)
        }
        6 => {
            g.pick(&[0x6000u16, 0x7000, 0x8000]) | (g.b() as u16) << 11 | (g.n(32) as u16) << 6 | 7 << 3 | rd
        }
        7 => 0x9000 | (g.b() as u16) << 11 | rd << 8 | g.n(256) as u16,
        8 => g.pick(&[0xa000u16, 0xa800]) | rd << 8 | g.n(256) as u16,
        9 => 0xb200 | (g.n(4) as u16) << 6 | rm << 3 | rd,
        10 => 0xba00 | g.pick(&[0u16, 1, 3]) << 6 | rm << 3 | rd,
        11 => {
            let list = 1 + g.n(127) as u16;
            if g.b() {
                0xb400 | (g.b() as u16) << 8 | list
            } else {
                0xbc00 | list
            }
        }
        12 => 0xc000 | (g.b() as u16) << 11 | 7 << 8 | (1 + g.n(127) as u16),
        _ => 0xb000 | (g.b() as u16) << 7 | g.n(8) as u16,
    }]
}

/// A 32-bit integer instruction.
fn gen32(g: &mut Rng) -> Vec<u16> {
    let (rd, rn, rm) = (g.reg(), g.reg(), g.reg());
    match g.n(24) {
        0 | 1 => {
            // Data processing, modified immediate.
            let op = g.pick(&[0u16, 1, 2, 3, 4, 8, 10, 11, 13, 14]);
            let s = g.b() as u16;
            let (rd, s) = if s == 1 && [0, 4, 8, 13].contains(&op) && g.n(3) == 0 {
                (15, 1)
            } else {
                (rd, s)
            };
            let rn = if (op == 2 || op == 3) && g.b() { 15 } else { rn };
            let i = g.n(4096) as u16;
            t32(
                0xf000 | (i >> 11) << 10 | op << 5 | s << 4 | rn,
                (i >> 8 & 7) << 12 | rd << 8 | (i & 0xff),
            )
        }
        2 | 3 => {
            // Data processing, shifted register.
            let op = g.pick(&[0u16, 1, 2, 3, 4, 6, 8, 10, 11, 13, 14]);
            let s = if op == 6 { 0 } else { g.b() as u16 };
            let (rd, s) = if s == 1 && [0, 4, 8, 13].contains(&op) && g.n(3) == 0 {
                (15, 1)
            } else {
                (rd, s)
            };
            let rn = if (op == 2 || op == 3) && g.b() { 15 } else { rn };
            let imm = g.n(32) as u16;
            let ty = if op == 6 {
                g.pick(&[0u16, 2])
            } else {
                g.n(4) as u16
            };
            t32(
                0xea00 | op << 5 | s << 4 | rn,
                (imm >> 2) << 12 | rd << 8 | (imm & 3) << 6 | ty << 4 | rm,
            )
        }
        4 => {
            // ADDW, SUBW, MOVW, MOVT.
            let i = g.n(4096) as u16;
            let (h, rn) = match g.n(4) {
                0 => (0xf200, rn),
                1 => (0xf2a0, rn),
                2 => (0xf240, g.n(16) as u16),
                _ => (0xf2c0, g.n(16) as u16),
            };
            t32(
                h | (i >> 11) << 10 | rn,
                (i >> 8 & 7) << 12 | rd << 8 | (i & 0xff),
            )
        }
        5 => {
            // SSAT, USAT (and the 16-bit forms).
            let sh = g.b() as u16;
            let imm = g.n(32) as u16;
            let (imm, sh) = if sh == 1 && imm == 0 { (1, 1) } else { (imm, sh) };
            match g.n(3) {
                0 => t32(
                    0xf300 | sh << 5 | rn,
                    (imm >> 2) << 12 | rd << 8 | (imm & 3) << 6 | g.n(32) as u16,
                ),
                1 => t32(
                    0xf380 | sh << 5 | rn,
                    (imm >> 2) << 12 | rd << 8 | (imm & 3) << 6 | g.n(32) as u16,
                ),
                _ => t32(g.pick(&[0xf320u16, 0xf3a0]) | rn, rd << 8 | g.n(16) as u16),
            }
        }
        6 => {
            // SBFX, UBFX, BFI, BFC.
            let lsb = g.n(32) as u16;
            let (hw1, last) = match g.n(4) {
                0 => (0xf340 | rn, g.n(32 - lsb as u32) as u16),
                1 => (0xf3c0 | rn, g.n(32 - lsb as u32) as u16),
                2 => (0xf360 | rn, lsb + g.n(32 - lsb as u32) as u16),
                _ => (0xf36f, lsb + g.n(32 - lsb as u32) as u16),
            };
            t32(hw1, (lsb >> 2) << 12 | rd << 8 | (lsb & 3) << 6 | last)
        }
        7 => t32(
            0xfa00 | (g.n(4) as u16) << 5 | (g.b() as u16) << 4 | rn,
            0xf000 | rd << 8 | rm,
        ),
        8 => {
            // Extend (and add).
            let rn = if g.b() { 15 } else { rn };
            t32(
                0xfa00 | (g.n(6) as u16) << 4 | rn,
                0xf080 | rd << 8 | (g.n(4) as u16) << 4 | rm,
            )
        }
        9 | 10 => {
            // Parallel add and subtract.
            let op1 = g.pick(&[0u16, 1, 2, 4, 5, 6]);
            t32(
                0xfa80 | op1 << 4 | rn,
                0xf000 | rd << 8 | (g.b() as u16) << 6 | (g.n(3) as u16) << 4 | rm,
            )
        }
        11 => {
            // QADD.., REV.., SEL, CLZ.
            let (op1, op2) = g.pick(&[
                (0u16, 0u16),
                (0, 1),
                (0, 2),
                (0, 3),
                (1, 0),
                (1, 1),
                (1, 2),
                (1, 3),
                (2, 0),
                (3, 0),
            ]);
            let rn = if op1 == 1 || op1 == 3 { rm } else { rn };
            t32(0xfa80 | op1 << 4 | rn, 0xf080 | rd << 8 | op2 << 4 | rm)
        }
        12 | 13 => {
            // 32-bit multiplies and multiply-accumulates.
            let (op1, op2) = g.pick(&[
                (0u16, 0u16),
                (0, 1),
                (1, 0),
                (1, 1),
                (1, 2),
                (1, 3),
                (2, 0),
                (2, 1),
                (3, 0),
                (3, 1),
                (4, 0),
                (4, 1),
                (5, 0),
                (5, 1),
                (6, 0),
                (6, 1),
                (7, 0),
            ]);
            let ra = if op1 != 6 && g.n(3) == 0 { 15 } else { g.reg() };
            t32(0xfb00 | op1 << 4 | rn, ra << 12 | rd << 8 | op2 << 4 | rm)
        }
        14 => {
            // Long multiplies and divides.
            let (op1, op2) = g.pick(&[
                (0u16, 0u16),
                (1, 15),
                (2, 0),
                (3, 15),
                (4, 0),
                (4, 8),
                (4, 9),
                (4, 10),
                (4, 11),
                (4, 12),
                (4, 13),
                (5, 12),
                (5, 13),
                (6, 0),
                (6, 6),
            ]);
            let (lo, hi) = g.reg2();
            let lo = if op2 == 15 { 15 } else { lo };
            t32(0xfb80 | op1 << 4 | rn, lo << 12 | hi << 8 | op2 << 4 | rm)
        }
        15 | 16 => {
            // Loads and stores off r7, immediate offsets.
            let size = g.n(3) as u16;
            let load = g.b() as u16;
            let sign = (load == 1 && size < 2 && g.b()) as u16;
            let hw1 = 0xf800 | sign << 8 | load << 4 | 7;
            if g.b() {
                t32(hw1 | 0x80 | size << 5, rd << 12 | g.n(4096) as u16)
            } else {
                let (p, w) = g.pick(&[(1u16, 0u16), (1, 1), (0, 1)]);
                let u = if p == 1 && w == 0 { 0 } else { g.b() as u16 };
                t32(
                    hw1 | size << 5,
                    rd << 12 | 0x800 | p << 10 | u << 9 | w << 8 | g.n(256) as u16,
                )
            }
        }
        17 => {
            // LDRD, STRD.
            let (a, b) = g.reg2();
            let (p, w) = g.pick(&[(1u16, 0u16), (1, 1), (0, 1)]);
            let load = g.b() as u16;
            t32(
                0xe840 | p << 8 | (g.b() as u16) << 7 | w << 5 | load << 4 | 7,
                a << 12 | b << 8 | g.n(256) as u16,
            )
        }
        18 => {
            // LDM, STM off r7.
            let load = g.b() as u16;
            let mut list = 0;
            while (list as u32).count_ones() < 2 {
                list = (g.next() as u16) & 0x1f7f & if load == 1 { 0x5f7f } else { 0x1f7f };
            }
            let w = g.b() as u16;
            t32(g.pick(&[0xe880u16, 0xe900]) | w << 5 | load << 4 | 7, list)
        }
        19 => {
            // Exclusives off r7.
            let (a, b) = g.reg2();
            let imm = g.n(4) as u16;
            match g.n(6) {
                0 | 1 => t32(0xe857, a << 12 | 0xf00 | imm),
                2 | 3 => t32(0xe847, a << 12 | b << 8 | imm),
                4 => t32(0xe8d7, a << 12 | 0xf00 | g.pick(&[0x4fu16, 0x5f])),
                _ => t32(0xe8c7, a << 12 | 0xf00 | g.pick(&[0x40u16, 0x50]) | b),
            }
        }
        20 => {
            // MRS, MSR APSR, barriers, CLREX.
            match g.n(4) {
                0 => t32(0xf3ef, 0x8000 | rd << 8),
                1 => t32(0xf380 | rn, 0x8000 | (1 + g.n(3) as u16) << 10),
                2 => t32(0xf3bf, g.pick(&[0x8f4fu16, 0x8f5f, 0x8f2f])),
                _ => t32(0xf3af, 0x8000),
            }
        }
        21 => t32(0xfa90 | rn, 0xf0f0 | rd << 8 | rm),
        22 => {
            // Literal loads.
            let size = g.pick(&[0u16, 1, 2]);
            let sign = (size < 2 && g.b()) as u16;
            t32(
                0xf81f | sign << 8 | (g.b() as u16) << 7 | size << 5,
                rd << 12 | g.n(1024) as u16,
            )
        }
        _ => {
            // Register offsets off r7, with a small index.
            let size = g.n(3) as u16;
            let load = g.b() as u16;
            let sign = (load == 1 && size < 2 && g.b()) as u16;
            let rm = g.lo();
            t32(
                0xf800 | sign << 8 | size << 5 | load << 4 | 7,
                rd << 12 | (g.n(4) as u16) << 4 | rm,
            )
        }
    }
}

/// A single-precision VFP instruction; `in_it` leaves out the ones that
/// can't be conditional.
fn genfp(g: &mut Rng, in_it: bool) -> Vec<u16> {
    let (d, n, m) = (g.n(32) as u16, g.n(32) as u16, g.n(32) as u16);
    let (vd, dd) = (d >> 1, d & 1);
    let (vn, nn) = (n >> 1, n & 1);
    let (vm, mm) = (m >> 1, m & 1);
    let three = |o1: u16, o2: u16, o3: u16| {
        t32(
            0xee00 | o1 << 7 | dd << 6 | o2 << 4 | vn,
            vd << 12 | 0xa00 | nn << 7 | o3 << 6 | mm << 5 | vm,
        )
    };
    let two = |opc2: u16, op: u16| t32(0xeeb0 | dd << 6 | opc2, vd << 12 | 0xa40 | op << 7 | mm << 5 | vm);
    let k = if in_it { g.n(10) } else { g.n(13) };
    match k {
        0 | 1 => {
            let (o1, o2, o3) = g.pick(&[
                (0u16, 0u16, 0u16),
                (0, 0, 1),
                (0, 1, 0),
                (0, 1, 1),
                (0, 2, 0),
                (0, 2, 1),
                (0, 3, 0),
                (0, 3, 1),
                (1, 0, 0),
                (1, 1, 0),
                (1, 1, 1),
                (1, 2, 0),
                (1, 2, 1),
            ]);
            three(o1, o2, o3)
        }
        2 => {
            let (opc2, op) = g.pick(&[
                (0u16, 0u16),
                (0, 1),
                (1, 0),
                (1, 1),
                (2, 0),
                (2, 1),
                (3, 0),
                (3, 1),
                (4, 0),
                (4, 1),
                (6, 0),
                (6, 1),
                (7, 0),
                (8, 0),
                (8, 1),
                (12, 0),
                (12, 1),
                (13, 0),
                (13, 1),
            ]);
            two(opc2, op)
        }
        3 => t32(0xeeb5 | dd << 6, vd << 12 | 0xa40 | (g.b() as u16) << 7),
        4 => {
            // Fixed-point conversions.
            // Fraction bits must not exceed the size (UNPREDICTABLE).
            let opc2 = 0b1010 | (g.b() as u16) << 2 | g.b() as u16;
            let sx = g.b() as u16;
            let imm = g.n(if sx == 1 { 32 } else { 17 }) as u16;
            t32(
                0xeeb0 | dd << 6 | opc2,
                vd << 12 | 0xa40 | sx << 7 | (imm & 1) << 5 | imm >> 1,
            )
        }
        5 => {
            let imm = g.n(256) as u16;
            t32(0xeeb0 | dd << 6 | imm >> 4, vd << 12 | 0xa00 | (imm & 15))
        }
        6 => {
            // VMOV core <-> single, and two of each.
            let rt = g.reg();
            if g.b() {
                t32(0xee00 | (g.b() as u16) << 4 | vn, rt << 12 | 0xa10 | nn << 7)
            } else {
                let (a, b) = g.reg2();
                let m = g.n(31) as u16;
                t32(
                    0xec40 | (g.b() as u16) << 4 | b,
                    a << 12 | 0xa10 | (m & 1) << 5 | m >> 1,
                )
            }
        }
        7 => match g.n(3) {
            0 => t32(0xeef1, 0xfa10),
            1 => t32(0xeef1, g.reg() << 12 | 0xa10),
            _ => t32(0xeee1, 12 << 12 | 0xa10),
        },
        8 => t32(
            0xed00 | (g.b() as u16) << 7 | dd << 6 | (g.b() as u16) << 4 | 7,
            vd << 12 | 0xa00 | g.n(256) as u16,
        ),
        9 => {
            // VLDM, VSTM, VPUSH, VPOP.
            let count = 1 + g.n(32 - d as u32) as u16;
            let load = g.b() as u16;
            let (p, u, w, rn) = g.pick(&[
                (0u16, 1u16, 0u16, 7u16),
                (0, 1, 1, 7),
                (1, 0, 1, 7),
                (1, 0, 1, 13),
                (0, 1, 1, 13),
            ]);
            t32(
                0xec00 | p << 8 | u << 7 | dd << 6 | w << 5 | load << 4 | rn,
                vd << 12 | 0xa00 | count,
            )
        }
        10 => t32(
            0xfe00 | dd << 6 | (g.n(4) as u16) << 4 | vn,
            vd << 12 | 0xa00 | nn << 7 | mm << 5 | vm,
        ),
        11 => t32(
            0xfe80 | dd << 6 | vn,
            vd << 12 | 0xa00 | nn << 7 | (g.b() as u16) << 6 | mm << 5 | vm,
        ),
        _ => {
            let rm = g.n(4) as u16;
            if g.b() {
                t32(0xfeb8 | dd << 6 | rm, vd << 12 | 0xa40 | mm << 5 | vm)
            } else {
                t32(
                    0xfebc | dd << 6 | rm,
                    vd << 12 | 0xa40 | (g.b() as u16) << 7 | mm << 5 | vm,
                )
            }
        }
    }
}

fn gen_plain(g: &mut Rng, in_it: bool) -> Vec<u16> {
    match g.n(10) {
        0..=2 => gen16(g),
        3..=6 => gen32(g),
        _ => genfp(g, in_it),
    }
}

/// A random straight-line program (with forward skips and IT blocks).
fn program(g: &mut Rng) -> Vec<u16> {
    let mut out = Vec::new();
    // Start from random FP controls: rounding mode, FZ, DN, AHP.
    out.extend([0xeee1, 12 << 12 | 0xa10]);
    let len = 4 + g.n(24);
    for _ in 0..len {
        match g.n(16) {
            0 => {
                // IT block.
                let cond = g.n(14) as u16;
                let mask = 1 + g.n(15) as u16;
                out.push(0xbf00 | cond << 4 | mask);
                for _ in 0..4 - mask.trailing_zeros() {
                    out.extend(gen_plain(g, true));
                }
            }
            1 => {
                // A forward branch over the next instruction.
                let next = gen_plain(g, false);
                let skip = next.len() as u16;
                match g.n(4) {
                    0 => out.push(0xd000 | (g.n(14) as u16) << 8 | (skip - 1)),
                    1 => out.push(0xe000 | (skip - 1)),
                    2 => out.push(0xb100 | (g.b() as u16) << 11 | (skip - 1) << 3 | g.lo()),
                    _ => out.extend([0xf000, 0xf800 | skip]),
                }
                out.extend(next);
            }
            _ => out.extend(gen_plain(g, false)),
        }
    }
    out
}

fn random_state(g: &mut Rng, cpu: &mut dyn Cpu<St>) {
    for r in [0, 1, 2, 3, 4, 5, 6, 8, 9, 10, 11] {
        let v = g.int();
        cpu.reg_write(Reg::R(r), v);
    }
    cpu.reg_write(Reg::R(7), BASE);
    let fpscr = (g.next() as u32 & 0x07c0_0000) | if g.n(4) == 0 { g.next() as u32 & 0x9f } else { 0 };
    cpu.reg_write(Reg::R(12), fpscr);
    cpu.reg_write(Reg::Lr, C | 1);
    cpu.reg_write(Reg::Cpsr, (g.next() as u32) & 0xf800_0000);
    for i in 0..32 {
        let v = g.float();
        cpu.reg_write(Reg::S(i), v);
    }
    let mut data = vec![0u8; 0x10000];
    for w in data.chunks_mut(4) {
        let v = if g.n(4) == 0 { g.float() } else { g.next() as u32 };
        w.copy_from_slice(&v.to_le_bytes());
    }
    cpu.mem_write(D, &data).unwrap();
}

#[test]
fn random_programs_match_unicorn() {
    let var = |k: &str| std::env::var(k).ok().and_then(|s| s.parse::<u64>().ok());
    let runs = var("PDSIM_FUZZ_RUNS").unwrap_or(3000);
    // `PDSIM_FUZZ_SEED=n` runs one seed and finds the first instruction
    // where the backends part.
    let seeds = match var("PDSIM_FUZZ_SEED") {
        Some(s) => s..=s,
        None => 1..=runs,
    };
    let one = seeds.start() == seeds.end();
    let mut bad = Vec::new();
    for seed in seeds {
        let mut g = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let code = program(&mut g);
        let state = g.next();
        let setup = move |c: &mut dyn Cpu<St>| random_state(&mut Rng(state | 1), c);
        if let Some(d) = Case::new(&code, &setup).compare() {
            let hex: Vec<String> = code.iter().map(|h| format!("{h:04x}")).collect();
            bad.push(format!("seed {seed}: {d}\n  code {}", hex.join(" ")));
            if one {
                bad.push(first_divergence(&code, &setup));
            }
            if bad.len() >= 10 {
                break;
            }
        }
    }
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
}

/// Runs ever longer prefixes of `code` and describes the first
/// instruction after which the backends differ, with the state before it.
fn first_divergence(code: &[u16], setup: &dyn Fn(&mut dyn Cpu<St>)) -> String {
    let mut before = Case::new(&[], setup).run(Backend::Unicorn);
    let mut i = 0;
    while i < code.len() {
        let len = if code[i] >> 11 >= 0b11101 { 2 } else { 1 };
        let mut case = Case::new(code, setup);
        case.until = C + 2 * (i + len) as u32;
        let reached = case.run(Backend::Unicorn).r[15] == case.until;
        if let Some(d) = case.compare().filter(|_| reached) {
            let insn: Vec<String> = code[i..i + len].iter().map(|h| format!("{h:04x}")).collect();
            return format!(
                "first differs after {} at {:#x}: {d}\n  before: r {:x?}\n  cpsr {:#x} fpscr {:#x}\n  s {:x?}",
                insn.join(" "),
                C + 2 * i as u32,
                before.r,
                before.cpsr,
                before.fpscr,
                before.s
            );
        }
        before = case.run(Backend::Unicorn);
        i += len;
    }
    "no prefix differs".to_string()
}

#[test]
/// Unicorn defers a store fault inside an IT block: the store is skipped
/// and the fault ends the run after the next access outside an IT block,
/// at an exception, or at the end of the block.
fn deferred_store_faults_match_unicorn() {
    let nop = 0xbf00u16;
    let movs3 = 0x2301u16; // movs r3,#1
    let b0 = 0xe7ffu16; // b next
    for (name, code, until) in [
        (
            "it str; movs; nop; b; nop",
            vec![0xbf08u16, 0x6008, movs3, nop, b0, nop, nop],
            14u32,
        ),
        (
            "same until mid",
            vec![0xbf08u16, 0x6008, movs3, nop, nop, nop, nop, nop, nop],
            8,
        ),
        (
            "it strd",
            vec![0xbf08u16, 0xe9c1, 0x0200, movs3, nop, b0, nop],
            14,
        ),
        (
            "it vstr d",
            vec![0xbf08u16, 0xed81, 0x0b00, movs3, nop, b0, nop],
            14,
        ),
        (
            "it str; ldr r4,[r7]",
            vec![0xbf08u16, 0x6008, movs3, 0x683c, nop, b0, nop],
            14,
        ),
        (
            "it str; str r3,[r7]; ",
            vec![0xbf08u16, 0x6008, movs3, 0x603b, 0x607b, b0, nop],
            14,
        ),
        (
            "it str; push",
            vec![0xbf08u16, 0x6008, movs3, 0xb408, nop, b0, nop],
            14,
        ),
        (
            "it str; it ldr; ldr",
            vec![0xbf08u16, 0x6008, 0xbf08, 0x683c, movs3, 0x687d, b0, nop],
            16,
        ),
        (
            "it str; wfi",
            vec![0xbf08u16, 0x6008, movs3, 0xbf30, nop, nop],
            12,
        ),
        (
            "it str; svc",
            vec![0xbf08u16, 0x6008, movs3, 0xdf00, nop, nop],
            12,
        ),
        (
            "it str; udf",
            vec![0xbf08u16, 0x6008, movs3, 0xde00, nop, nop],
            12,
        ),
        (
            "it str; b far",
            vec![0xbf08u16, 0x6008, movs3, 0xe002, nop, nop, nop, nop, nop],
            18,
        ),
        (
            "it str; it ldr fault",
            vec![0xbf08u16, 0x6008, 0xbf08, 0x6808, movs3, nop],
            12,
        ),
        ("itt str; str ok", vec![0xbf04u16, 0x6008, 0x603b, movs3, nop], 10),
        (
            "it str; bkpt",
            vec![0xbf08u16, 0x6008, movs3, 0xbe00, nop, nop],
            12,
        ),
        (
            "it str; ldrex unaligned",
            vec![0xbf08u16, 0x6008, movs3, 0x1c7f, 0xe857, 0x4f00, nop, nop],
            16,
        ),
        (
            "it str; ldr fault",
            vec![0xbf08u16, 0x6008, movs3, 0x680c, nop, nop],
            12,
        ),
        (
            "itt str; str trampoline",
            vec![0xbf04u16, 0x6008, 0x6028, movs3, nop],
            10,
        ),
        (
            "it str; str fault",
            vec![0xbf08u16, 0x6008, movs3, 0x6028, nop, nop],
            12,
        ),
        (
            "it str; vldr",
            vec![0xbf08u16, 0x6008, movs3, 0xed97, 0x0a00, nop, nop],
            14,
        ),
        (
            "it str; it svc",
            vec![0xbf08u16, 0x6008, 0xbf08, 0xdf00, nop, nop],
            12,
        ),
        (
            "it str; tbb",
            vec![0xbf08u16, 0x6008, 0xe8d7, 0xf003, nop, nop, nop, nop],
            16,
        ),
    ] {
        for a in [0x5000_0000u32, 0x5000_0001] {
            let set = move |c: &mut dyn Cpu<St>| {
                c.reg_write(Reg::R(1), a);
                c.reg_write(Reg::R(7), D + 0x100);
                c.reg_write(Reg::R(5), T);
                c.reg_write(Reg::Sp, D + 0x200);
                c.reg_write(Reg::Cpsr, 0x4000_0000);
            };
            let mut case = Case::new(&code, &set);
            case.until = C + until;
            let s = case.run(Backend::Unicorn);
            let t = case.run(Backend::Thumb);
            let d = s.diff(&t);
            assert!(d.is_empty(), "{name} at {a:#x}: {d:#?}");
        }
    }
}
