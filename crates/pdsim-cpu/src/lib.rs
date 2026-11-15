//! The CPU interface `pdsim-arm` runs C games on.
//!
//! A backend (Unicorn, or the `pdsim-thumb` interpreter) emulates a Cortex-M7
//! in Thumb mode. The emulator calls back into [`Hooks`] at the start of each
//! basic block in writable (code) memory and before each instruction in
//! read-only executable memory (the API trampolines). Hooks get the CPU and
//! the caller's state `S` separately, so a hook can run game code again
//! (nested [`Cpu::emu_start`]).
//!
//! This crate has no dependencies, so a backend can implement [`Cpu`]
//! without depending on `pdsim-arm`.

/// A register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reg {
    /// `r0`..=`r12`.
    R(u8),
    Sp,
    Lr,
    Pc,
    /// Single-precision VFP register `s0`..=`s31`.
    S(u8),
    Fpscr,
    /// The application status flags (APSR: N, Z, C, V, Q and GE).
    Cpsr,
}

/// An emulation fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuError {
    /// What went wrong, in the wording of the crash message
    /// (`C game crashed: {kind} at pc ...`).
    pub kind: String,
    /// The PC when it happened.
    pub pc: u32,
}

impl std::fmt::Display for CpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at pc {:#010x}", self.kind, self.pc)
    }
}

impl std::error::Error for CpuError {}

/// An emulated Cortex-M7 in Thumb mode.
pub trait Cpu<S> {
    /// Maps `size` bytes at `addr`, readable and executable, and writable
    /// when `writable`. Read-only memory is where trampolines live:
    /// [`Hooks::on_trampoline`] runs before each instruction there.
    fn mem_map(&mut self, addr: u32, size: u32, writable: bool) -> Result<(), CpuError>;
    fn mem_read(&self, addr: u32, buf: &mut [u8]) -> Result<(), CpuError>;
    /// Writes memory, including read-only memory.
    fn mem_write(&mut self, addr: u32, data: &[u8]) -> Result<(), CpuError>;
    fn reg_read(&self, r: Reg) -> u32;
    fn reg_write(&mut self, r: Reg, v: u32);
    /// Runs from `begin` (Thumb bit set) until PC == `until` or `emu_stop`. Re-entrant: may be
    /// called from inside `Hooks::on_trampoline`.
    fn emu_start(
        &mut self,
        begin: u32,
        until: u32,
        state: &mut S,
        hooks: &dyn Hooks<S>,
    ) -> Result<(), CpuError>;
    /// Stops the innermost `emu_start` at the end of the current block.
    fn emu_stop(&mut self);
}

/// Callbacks from a running [`Cpu`].
pub trait Hooks<S> {
    /// At the start of each basic block in writable memory, before it runs.
    fn on_block(&self, cpu: &mut dyn Cpu<S>, state: &mut S, addr: u32, size: u32);
    /// Before each instruction in read-only memory. Usually sets PC to
    /// return from the trampoline.
    fn on_trampoline(&self, cpu: &mut dyn Cpu<S>, state: &mut S, addr: u32);
}
