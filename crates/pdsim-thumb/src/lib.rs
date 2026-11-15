//! A Cortex-M7 (ARMv7E-M Thumb-2 + FPv5 single precision) interpreter.
//!
//! [`ThumbCpu`] implements [`pdsim_cpu::Cpu`] and is meant to behave like
//! the Unicorn backend in everything a game can observe: register and
//! memory results, flags, FPSCR, which blocks `on_block` sees (QEMU's
//! translation blocks), and the kind and PC of every fault (Unicorn's
//! `uc_error` names).
//!
//! Code is decoded once into [`decode::Block`]s, cached per code region and
//! dropped when the game (or [`Cpu::mem_write`]) writes to its page.
//! Only single precision exists, and there are no exceptions, interrupts,
//! MPU or privilege levels: a fault ends `emu_start` with an error.

pub mod decode;
mod exec;
mod mem;
pub mod tcg;
mod vfp;

use decode::Block;
use mem::Mem;
use pdsim_cpu::{Cpu, CpuError, Hooks, Reg, Region};
use std::rc::Rc;

/// Why execution left the instruction stream.
pub(crate) enum Exit {
    /// A fault, which `emu_start` returns.
    Fault(CpuError),
    /// WFI: `emu_start` returns `Ok` with PC after it.
    Halt,
}

pub(crate) fn err(kind: &str, pc: u32) -> Exit {
    Exit::Fault(CpuError {
        kind: kind.to_string(),
        pc,
    })
}

/// The interpreter's CPU state and memory.
pub struct ThumbCpu {
    /// `r0`..`r15`. While an instruction runs, `r[15]` holds its address
    /// plus 4 (what reading PC gives).
    pub(crate) r: [u32; 16],
    pub(crate) n: bool,
    pub(crate) z: bool,
    pub(crate) c: bool,
    pub(crate) v: bool,
    pub(crate) q: bool,
    pub(crate) ge: u8,
    /// ITSTATE (`firstcond:mask`), 0 outside an IT block.
    pub(crate) it: u8,
    /// Thumb state; clear after an interworking branch to an even address.
    pub(crate) thumb: bool,
    pub(crate) s: [u32; 32],
    pub(crate) fpscr: u32,
    /// CONTROL.FPCA: set by the first floating-point instruction, which
    /// also starts FPSCR from zero (QEMU's lazy FP state).
    pub(crate) fpca: bool,
    /// The translator's view of FPCA in the running block: set by the
    /// first floating-point instruction even when its IT condition fails,
    /// so a later one in the same block does not create a new FP context
    /// (QEMU clears `v7m_new_fp_ctxt_needed` at translation time).
    pub(crate) tb_fpca: bool,
    pub(crate) primask: bool,
    pub(crate) faultmask: bool,
    pub(crate) basepri: u8,
    /// The exclusive monitor: address and value of the last LDREX.
    pub(crate) excl: Option<(u32, u32)>,
    /// The running instruction is inside an IT block.
    pub(crate) in_it: bool,
    /// A store fault inside an IT block, not yet reported. Unicorn checks
    /// for a failed store only after memory accesses outside IT blocks:
    /// the failed store is skipped, and the fault ends the run after the
    /// next access outside an IT block in the same block (at that
    /// instruction), or else at the end of the block (at the next PC).
    pub(crate) deferred: Option<mem::Fault>,
    pub(crate) mem: Mem,
    /// `until` of the innermost `emu_start`.
    until: u32,
    /// `emu_stop` was called.
    stop: bool,
}

impl Default for ThumbCpu {
    fn default() -> Self {
        Self::new()
    }
}

/// Addresses the M profile never executes from (Peripheral and Device
/// memory): fetching there is an exception, mapped or not.
fn is_xn(pc: u32) -> bool {
    (0x4000_0000..0x6000_0000).contains(&pc) || pc >= 0xa000_0000
}

impl ThumbCpu {
    pub fn new() -> ThumbCpu {
        ThumbCpu {
            r: [0; 16],
            n: false,
            z: true,
            c: false,
            v: false,
            q: false,
            ge: 0,
            it: 0,
            thumb: true,
            s: [0; 32],
            fpscr: 0,
            fpca: false,
            tb_fpca: false,
            primask: false,
            faultmask: false,
            basepri: 0,
            excl: None,
            in_it: false,
            deferred: None,
            mem: Mem::default(),
            until: 0,
            stop: false,
        }
    }

    /// Starts or stops recording game stores (one entry per access).
    pub fn set_write_log(&mut self, on: bool) {
        self.mem.set_log(on);
    }

    /// The game stores recorded since the last call, oldest first.
    pub fn write_log(&mut self) -> Vec<(u32, Vec<u8>)> {
        self.mem.drain_log()
    }

    /// APSR: N, Z, C, V, Q and GE.
    pub(crate) fn apsr(&self) -> u32 {
        (self.n as u32) << 31
            | (self.z as u32) << 30
            | (self.c as u32) << 29
            | (self.v as u32) << 28
            | (self.q as u32) << 27
            | (self.ge as u32) << 16
    }

    pub(crate) fn set_nzcvq(&mut self, v: u32) {
        self.n = v & 1 << 31 != 0;
        self.z = v & 1 << 30 != 0;
        self.c = v & 1 << 29 != 0;
        self.v = v & 1 << 28 != 0;
        self.q = v & 1 << 27 != 0;
    }

    /// The block at `pc` in code area `i`, decoded and cached if needed.
    fn block(&mut self, i: usize, pc: u32) -> Result<Rc<Block>, CpuError> {
        if let Some(b) = self.mem.cached(i, pc) {
            if b.it == self.it && b.fpca == self.fpca {
                return Ok(b);
            }
        }
        let b = Rc::new(decode::block_at(&self.mem, pc, self.it, self.fpca)?);
        self.mem.cache(i, pc, b.clone());
        Ok(b)
    }

    /// A fetch fault at `pc`, which isn't in a code or trampoline region.
    fn fetch_fault(&self, pc: u32) -> CpuError {
        let kind = match self.mem.kind_at(pc) {
            _ if is_xn(pc) => "EXCEPTION",
            Some(Region::Data) => "FETCH_PROT",
            _ => "FETCH_UNMAPPED",
        };
        CpuError {
            kind: kind.to_string(),
            pc,
        }
    }

    /// Reports a deferred store fault at the end of a block.
    fn end_block(&mut self) -> Result<(), Exit> {
        match self.deferred.take() {
            Some(f) => Err(Exit::Fault(f.at(self.r[15]))),
            None => Ok(()),
        }
    }

    /// How an instruction that leaves the block reports a deferred store
    /// fault: a WFI or an exception (SVC, BKPT, ...) reports it instead,
    /// at the PC it left; an invalid instruction replaces it.
    fn deferred_exit(&mut self, e: Exit) -> Exit {
        match (e, self.deferred.take()) {
            (Exit::Halt, Some(f)) => Exit::Fault(f.at(self.r[15])),
            (Exit::Fault(e), Some(f)) if e.kind == "EXCEPTION" => Exit::Fault(f.at(e.pc)),
            (e, _) => e,
        }
    }

    fn run<S>(&mut self, state: &mut S, hooks: &dyn Hooks<S>) -> Result<(), Exit> {
        loop {
            let pc = self.r[15];
            if pc == self.until {
                return Ok(());
            }
            let kind = if is_xn(pc) {
                None
            } else {
                self.mem.find(pc).map(|i| (i, self.mem.area(i).kind))
            };
            if !self.thumb {
                // Arm state: QEMU translates a one-instruction block that
                // is UNDEFINED on M profile.
                return match kind {
                    Some((_, Region::Code)) => {
                        hooks.on_block(self, state, pc, 4);
                        if std::mem::take(&mut self.stop) {
                            return Ok(());
                        }
                        Err(err("INSN_INVALID", pc))
                    }
                    Some((_, Region::Trampoline)) => Err(err("INSN_INVALID", pc)),
                    _ => Err(Exit::Fault(self.fetch_fault(pc))),
                };
            }
            match kind {
                Some((i, Region::Code)) => {
                    let block = self.block(i, pc).map_err(Exit::Fault)?;
                    // A block runs up to `until` (QEMU checks for it while
                    // translating).
                    let until = self.until;
                    let (n, size) = match block.ops.iter().position(|o| o.pc == until) {
                        Some(n) if until != pc => (n, until - pc),
                        _ => (block.ops.len(), block.size),
                    };
                    hooks.on_block(self, state, pc, size);
                    if std::mem::take(&mut self.stop) {
                        self.r[15] = pc;
                        return Ok(());
                    }
                    self.tb_fpca = self.fpca;
                    for op in &block.ops[..n] {
                        self.exec(op).map_err(|e| self.deferred_exit(e))?;
                    }
                    self.end_block()?;
                }
                Some((_, Region::Trampoline)) => {
                    hooks.on_trampoline(self, state, pc);
                    if std::mem::take(&mut self.stop) {
                        return Ok(());
                    }
                    if self.r[15] != pc || !self.thumb {
                        continue;
                    }
                    let op = decode::one_at(&self.mem, pc).map_err(Exit::Fault)?;
                    self.tb_fpca = self.fpca;
                    self.exec(&op).map_err(|e| self.deferred_exit(e))?;
                    self.end_block()?;
                }
                _ => return Err(Exit::Fault(self.fetch_fault(pc))),
            }
        }
    }
}

impl<S> Cpu<S> for ThumbCpu {
    fn mem_map(&mut self, addr: u32, size: u32, kind: Region) -> Result<(), CpuError> {
        self.mem.map(addr, size, kind)
    }

    fn mem_read(&self, addr: u32, buf: &mut [u8]) -> Result<(), CpuError> {
        if self.mem.read(addr, buf) {
            Ok(())
        } else {
            Err(CpuError {
                kind: "READ_UNMAPPED".to_string(),
                pc: self.r[15],
            })
        }
    }

    fn mem_write(&mut self, addr: u32, data: &[u8]) -> Result<(), CpuError> {
        if self.mem.mapped(addr, data.len() as u32) && self.mem.write(addr, data) {
            Ok(())
        } else {
            Err(CpuError {
                kind: "WRITE_UNMAPPED".to_string(),
                pc: self.r[15],
            })
        }
    }

    fn reg_read(&self, r: Reg) -> u32 {
        match r {
            Reg::R(i) => self.r[i as usize],
            Reg::Sp => self.r[13],
            Reg::Lr => self.r[14],
            Reg::Pc => self.r[15],
            Reg::S(i) => self.s[i as usize],
            Reg::Fpscr => self.fpscr,
            Reg::Cpsr => self.apsr(),
        }
    }

    fn reg_write(&mut self, r: Reg, v: u32) {
        match r {
            Reg::R(i) => self.r[i as usize] = v,
            Reg::Sp => self.r[13] = v,
            Reg::Lr => self.r[14] = v,
            Reg::Pc => {
                self.r[15] = v & !1;
                self.thumb = v & 1 != 0;
            }
            Reg::S(i) => self.s[i as usize] = v,
            Reg::Fpscr => self.fpscr = v & vfp::FPSCR_MASK,
            Reg::Cpsr => self.set_nzcvq(v),
        }
    }

    fn emu_start(
        &mut self,
        begin: u32,
        until: u32,
        state: &mut S,
        hooks: &dyn Hooks<S>,
    ) -> Result<(), CpuError> {
        let saved = (self.until, self.stop);
        self.until = until;
        self.stop = false;
        self.deferred = None;
        self.r[15] = begin & !1;
        self.thumb = begin & 1 != 0;
        let r = self.run(state, hooks);
        (self.until, self.stop) = saved;
        match r {
            Ok(()) | Err(Exit::Halt) => Ok(()),
            Err(Exit::Fault(e)) => {
                self.r[15] = e.pc;
                Err(e)
            }
        }
    }

    fn emu_stop(&mut self) {
        self.stop = true;
    }
}
