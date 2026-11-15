//! Whole-game differential test: every ELF runs on Unicorn and on the
//! pure-Rust Thumb interpreter, and the two runs must agree block by block.
//!
//! Runs are deterministic, so "lockstep" is done by recording and
//! comparing: two independent [`ArmGame`]s load the same ELF, one per
//! backend, each with its own `Device`. A block probe snapshots every
//! register and the stores made since the previous block at each
//! `on_block`; both games run one frame at a time and the snapshot vectors
//! are compared. The first mismatch is the [`Divergence`].
//!
//! The ELFs are built outside the test (each test skips with a message
//! when its ELF or the SDK is missing):
//! - `make -C examples/c-stress device_bin`, `make -C calibration/bench-c device_bin`
//! - `make -C tests/thumb-corpus` (builds [`SEEDS`])
//!
//! With `PDSIM_REQUIRE_ELFS=1`, a missing ELF or SDK fails the test instead.

use capstone::arch::arm::{ArchExtraMode, ArchMode};
use capstone::prelude::*;
use pdsim_arm::cpu::{Cpu, Reg};
use pdsim_arm::emu::Mem as _;
use pdsim_arm::emu::{ArmState, API_BASE, API_SIZE, CODE_BASE, HEAP_BASE, HEAP_SIZE, STACK_BASE, STACK_SIZE};
use pdsim_arm::{ArmGame, Backend};
use pdsim_core::Calibration;
use pdsim_device::{Device, Game};
use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use Backend::{Thumb, Unicorn};

/// The corpus seeds checked; must match the Makefile's default `SEEDS`.
/// Seed 18 is the one that exposed Unicorn's write-hook IT-state leak.
const SEEDS: [u32; 5] = [0, 1, 2, 3, 18];

/// Every register a snapshot holds.
fn regs() -> Vec<Reg> {
    let mut v: Vec<Reg> = (0..13).map(Reg::R).collect();
    v.extend([Reg::Sp, Reg::Lr, Reg::Pc, Reg::Cpsr, Reg::Fpscr]);
    v.extend((0..32).map(Reg::S));
    v
}

/// The state at the start of a block.
#[derive(Debug, Clone, PartialEq)]
struct Snap {
    addr: u32,
    size: u32,
    regs: Vec<u32>,
    /// Stores since the previous snapshot (made by the previous block and
    /// anything it called).
    writes: Vec<(u32, Vec<u8>)>,
}

/// Where the two backends first disagree.
#[derive(Debug)]
pub struct Divergence {
    /// Index of the first differing snapshot, counted over all frames.
    pub block: u32,
    /// The block that ran just before the difference showed up.
    pub pc: u32,
    /// A differing register: Unicorn's value, then the interpreter's.
    pub reg: Option<(Reg, u32, u32)>,
    /// A differing store, or memory range: address, Unicorn's bytes, then
    /// the interpreter's.
    pub write: Option<(u32, Vec<u8>, Vec<u8>)>,
    /// What differs, then Capstone disassembly of the whole block at `pc`.
    pub disasm: String,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "divergence at block {} (pc {:#010x})", self.block, self.pc)?;
        if let Some((r, u, t)) = self.reg {
            writeln!(f, "  {r:?}: unicorn {u:#010x}, thumb {t:#010x}")?;
        }
        if let Some((a, u, t)) = &self.write {
            writeln!(f, "  write at {a:#010x}: unicorn {u:02x?}, thumb {t:02x?}")?;
        }
        write!(f, "{}", self.disasm)
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn elf(dir: &str) -> PathBuf {
    root().join(dir).join("Source/pdex.elf")
}

fn corpus_elf(seed: u32) -> PathBuf {
    root().join(format!("tests/thumb-corpus/corpus_{seed}.elf"))
}

fn all_elfs() -> Vec<PathBuf> {
    let mut v = vec![elf("examples/c-stress"), elf("calibration/bench-c")];
    v.extend(SEEDS.map(corpus_elf));
    v
}

/// The SDK and whether `elf` exists; prints why a test is skipped, or
/// fails under `PDSIM_REQUIRE_ELFS=1`.
fn ready(elf: &Path) -> Option<PathBuf> {
    let skip = |why: String| {
        if std::env::var("PDSIM_REQUIRE_ELFS").is_ok_and(|v| v == "1") {
            panic!("{why} (PDSIM_REQUIRE_ELFS=1)");
        }
        eprintln!("{why}; skipping");
    };
    let Some(sdk) = Device::sdk_from_env() else {
        skip("PLAYDATE_SDK_PATH not set".into());
        return None;
    };
    if !elf.exists() {
        skip(format!("{} not built", elf.display()));
        return None;
    }
    Some(sdk)
}

fn load(elf: &Path, sdk: &Path, backend: Backend) -> ArmGame {
    let data = std::env::temp_dir().join(format!("pdsim-lockstep-{}-{backend:?}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    let dev = Device::new(
        Calibration::default_estimates(),
        elf.parent().unwrap(),
        &data,
        Some(sdk.to_path_buf()),
    )
    .shared();
    ArmGame::load(dev, elf, Some(sdk), false, backend)
        .unwrap_or_else(|e| panic!("{} on {backend:?}: {}", elf.display(), e.0))
}

type Snaps = Rc<RefCell<Vec<Snap>>>;

/// Records a [`Snap`] at every block of `g`.
fn record(g: &mut ArmGame) -> Snaps {
    let snaps: Snaps = Rc::default();
    let out = snaps.clone();
    let regs = regs();
    g.set_block_probe(Box::new(move |cpu: &mut dyn Cpu<ArmState>, addr, size| {
        let writes = cpu.write_log();
        let regs = regs.iter().map(|r| cpu.reg_read(*r)).collect();
        out.borrow_mut().push(Snap {
            addr,
            size,
            regs,
            writes,
        });
    }));
    snaps
}

/// The `size`-byte block at `pc`, from `g`'s memory.
fn disasm(g: &mut ArmGame, pc: u32, size: u32) -> String {
    let cs = Capstone::new()
        .arm()
        .mode(ArchMode::Thumb)
        .extra_mode([ArchExtraMode::MClass].iter().copied())
        .build()
        .expect("capstone");
    let code = g.cpu().bytes(pc, size as usize);
    let mut out = String::new();
    match cs.disasm_all(&code, pc as u64) {
        Ok(insns) => {
            for i in insns.iter() {
                let bytes: String = i.bytes().iter().map(|b| format!("{b:02x}")).collect();
                out += &format!(
                    "  {:#010x}: {bytes:<8}  {} {}\n",
                    i.address(),
                    i.mnemonic().unwrap_or(""),
                    i.op_str().unwrap_or("")
                );
            }
        }
        Err(e) => out += &format!("  (capstone: {e})\n"),
    }
    out
}

/// The first difference between two snapshots, as `(reg, write, what)`.
#[allow(clippy::type_complexity)]
fn diff_snap(
    u: &Snap,
    t: &Snap,
) -> Option<(Option<(Reg, u32, u32)>, Option<(u32, Vec<u8>, Vec<u8>)>, String)> {
    if (u.addr, u.size) != (t.addr, t.size) {
        return Some((
            Some((Reg::Pc, u.addr, t.addr)),
            None,
            format!(
                "  block (addr, size): unicorn ({:#010x}, {}), thumb ({:#010x}, {})\n",
                u.addr, u.size, t.addr, t.size
            ),
        ));
    }
    let names = regs();
    if let Some(i) = (0..names.len()).find(|&i| u.regs[i] != t.regs[i]) {
        return Some((Some((names[i], u.regs[i], t.regs[i])), None, String::new()));
    }
    if u.writes != t.writes {
        let n = u.writes.len().max(t.writes.len());
        let i = (0..n).find(|&i| u.writes.get(i) != t.writes.get(i)).unwrap();
        let (ua, ub) = u.writes.get(i).cloned().unwrap_or((0, vec![]));
        let (ta, tb) = t.writes.get(i).cloned().unwrap_or((0, vec![]));
        let addrs = |w: &[(u32, Vec<u8>)]| {
            w.iter()
                .map(|(a, _)| format!("{a:#x}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let what = format!(
            "  store #{i} of {} (unicorn) / {} (thumb): unicorn at {ua:#010x}, thumb at {ta:#010x}\n  \
             unicorn stores: {}\n  thumb stores:   {}\n",
            u.writes.len(),
            t.writes.len(),
            addrs(&u.writes),
            addrs(&t.writes)
        );
        return Some((None, Some((if ua != 0 { ua } else { ta }, ub, tb)), what));
    }
    None
}

/// The first 16-byte chunk of the game's memory that differs.
fn diff_memory(u: &mut ArmGame, t: &mut ArmGame) -> Option<(u32, Vec<u8>, Vec<u8>)> {
    let code_len = u.state().image.bytes.len() as u32;
    for (base, len) in [
        (CODE_BASE, code_len),
        (HEAP_BASE, HEAP_SIZE),
        (STACK_BASE, STACK_SIZE),
        (API_BASE, API_SIZE),
    ] {
        let a = u.cpu().bytes(base, len as usize);
        let b = t.cpu().bytes(base, len as usize);
        if a != b {
            let i = (0..a.len().min(b.len())).find(|&i| a[i] != b[i]).unwrap_or(0) & !15;
            let end = (i + 16).min(a.len()).min(b.len());
            return Some((base + i as u32, a[i..end].to_vec(), b[i..end].to_vec()));
        }
    }
    None
}

fn regs_of(g: &mut ArmGame) -> Vec<u32> {
    let cpu = g.cpu();
    regs().iter().map(|r| cpu.reg_read(*r)).collect()
}

/// Runs `elf` on both backends for `frames` frames and compares them at
/// every block, and all of memory after loading and at the end.
fn lockstep(elf: &Path, frames: u32) -> Result<(), Divergence> {
    let Some(sdk) = ready(elf) else { return Ok(()) };
    let mut u = load(elf, &sdk, Unicorn);
    let mut t = load(elf, &sdk, Thumb);
    let at_load = |block, write, what: &str| Divergence {
        block,
        pc: 0,
        reg: None,
        write,
        disasm: what.to_string(),
    };
    if let Some(w) = diff_memory(&mut u, &mut t) {
        return Err(at_load(0, Some(w), "  memory after kEventInit\n"));
    }
    let su = record(&mut u);
    let st = record(&mut t);
    let mut block = 0u32;
    let mut last_pc = 0u32;
    let mut last_size = 0u32;
    for frame in 0..frames {
        let ru = u.frame(None).map(|_| ()).map_err(|e| e.0);
        let rt = t.frame(None).map(|_| ()).map_err(|e| e.0);
        // The stores after the frame's last block.
        let tail = |g: &mut ArmGame, snaps: &Snaps| {
            let writes = g.cpu().write_log();
            let regs = regs_of(g);
            snaps.borrow_mut().push(Snap {
                addr: u32::MAX,
                size: 0,
                regs,
                writes,
            });
        };
        tail(&mut u, &su);
        tail(&mut t, &st);
        let (vu, vt) = (su.take(), st.take());
        for (a, b) in vu.iter().zip(&vt) {
            if let Some((reg, write, what)) = diff_snap(a, b) {
                let mut disasm = format!("  frame {frame}\n{what}");
                disasm += &disasm_of(&mut u, last_pc, last_size);
                return Err(Divergence {
                    block,
                    pc: last_pc,
                    reg,
                    write,
                    disasm,
                });
            }
            last_pc = a.addr;
            last_size = a.size;
            block += 1;
        }
        if vu.len() != vt.len() || ru != rt {
            let what = format!(
                "  frame {frame}: {} blocks (unicorn) vs {} (thumb); result {ru:?} vs {rt:?}\n",
                vu.len(),
                vt.len()
            );
            return Err(Divergence {
                block,
                pc: last_pc,
                reg: None,
                write: None,
                disasm: what + &disasm_of(&mut u, last_pc, last_size),
            });
        }
        if ru.is_err() {
            break;
        }
    }
    if let Some(w) = diff_memory(&mut u, &mut t) {
        return Err(at_load(block, Some(w), "  memory at the end\n"));
    }
    Ok(())
}

/// The block that ran last (`pc`, `size` bytes), which holds the
/// divergent instruction.
fn disasm_of(g: &mut ArmGame, pc: u32, size: u32) -> String {
    if pc == 0 || pc == u32::MAX {
        String::new()
    } else {
        disasm(g, pc, size)
    }
}

/// `(addr, size)` of every block `on_block` sees in `frames` frames. With
/// `recording` off the write log is off too, so Unicorn runs without its
/// memory write hook, as it does when making reports.
fn block_trace(elf: &Path, frames: u32, backend: Backend, recording: bool) -> Vec<(u32, u32)> {
    let Some(sdk) = ready(elf) else { return vec![] };
    let mut g = load(elf, &sdk, backend);
    let trace: Rc<RefCell<Vec<(u32, u32)>>> = Rc::default();
    let out = trace.clone();
    g.set_block_probe(Box::new(move |cpu: &mut dyn Cpu<ArmState>, a, s| {
        if recording {
            cpu.write_log();
        }
        out.borrow_mut().push((a, s));
    }));
    if !recording {
        g.cpu().set_write_log(false);
    }
    for _ in 0..frames {
        if g.frame(None).is_err() {
            break;
        }
    }
    trace.take()
}

fn check(r: Result<(), Divergence>) {
    if let Err(d) = r {
        panic!("{d}");
    }
}

#[test]
fn lockstep_c_stress() {
    check(lockstep(&elf("examples/c-stress"), 120))
}

#[test]
fn lockstep_bench_c() {
    check(lockstep(&elf("calibration/bench-c"), 60))
}

#[test]
fn lockstep_corpus() {
    for seed in SEEDS {
        check(lockstep(&corpus_elf(seed), 3))
    }
}

#[test]
fn block_trace_matches_unicorn() {
    for e in all_elfs() {
        let u = block_trace(&e, 30, Unicorn, false);
        let t = block_trace(&e, 30, Thumb, false);
        same_trace(&e, ("unicorn", &u), ("thumb", &t));
    }
}

/// Panics at the first block where the two traces differ.
fn same_trace(e: &Path, (na, a): (&str, &[(u32, u32)]), (nb, b): (&str, &[(u32, u32)])) {
    if a != b {
        let i = (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i)).unwrap();
        panic!(
            "{}: block {i} of {} / {}: {na} {:x?}, {nb} {:x?}",
            e.display(),
            a.len(),
            b.len(),
            a.get(i),
            b.get(i)
        );
    }
}

/// Recording (on Unicorn, a memory write hook) changes neither the blocks
/// nor their costs, so the lockstep runs check what reports are made of.
#[test]
fn recording_does_not_change_costing() {
    for e in all_elfs() {
        for b in [Unicorn, Thumb] {
            let plain = block_trace(&e, 30, b, false);
            let recorded = block_trace(&e, 30, b, true);
            same_trace(&e, (&format!("{b:?} plain"), &plain), ("recorded", &recorded));
        }
    }
    let e = elf("examples/c-stress");
    let Some(sdk) = ready(&e) else { return };
    for b in [Unicorn, Thumb] {
        let mut probed = load(&e, &sdk, b);
        let snaps = record(&mut probed);
        let mut plain = load(&e, &sdk, b);
        for _ in 0..3 {
            assert_eq!(
                probed.frame(None).unwrap().cycles,
                plain.frame(None).unwrap().cycles
            );
        }
        assert!(
            snaps.borrow().iter().any(|s| !s.writes.is_empty()),
            "{b:?} recorded no stores"
        );
    }
}
