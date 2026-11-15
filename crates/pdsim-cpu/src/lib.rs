//! The CPU interface `pdsim-arm` runs C games on.
//!
//! A backend (Unicorn, or the `pdsim-thumb` interpreter) emulates a Cortex-M7
//! in Thumb mode. Memory is mapped as one of three [`Region`] kinds, which
//! fix what may be read, written and executed and which [`Hooks`] run there:
//! [`Hooks::on_block`] at the start of each basic block in [`Region::Code`],
//! and [`Hooks::on_trampoline`] before each instruction in
//! [`Region::Trampoline`]. Fetching an instruction from anywhere else
//! (including [`Region::Data`]) is a fault. Hooks get the CPU and the
//! caller's state `S` separately, so a hook can run game code again (nested
//! [`Cpu::emu_start`]).
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

/// What a mapped region holds; fixes its protections and hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    /// Game code: readable, writable and executable.
    /// [`Hooks::on_block`] runs at the start of every basic block in it.
    Code,
    /// Heap, stack and other data: readable and writable, not executable.
    /// Fetching an instruction from it faults (Unicorn: `FETCH_PROT`).
    /// No hooks run.
    Data,
    /// API trampolines: readable and executable, not writable by the game
    /// (only [`Cpu::mem_write`] writes it). [`Hooks::on_trampoline`] runs
    /// before every instruction in it, except at the `until` address of the
    /// running [`Cpu::emu_start`], where emulation stops first.
    Trampoline,
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
    /// Maps `size` bytes at `addr` as `kind`, with the protections and
    /// hooks [`Region`] describes. Executing outside `Code` and `Trampoline`
    /// regions, and game writes to a `Trampoline`, are faults.
    fn mem_map(&mut self, addr: u32, size: u32, kind: Region) -> Result<(), CpuError>;
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
    /// Starts (`true`) or stops recording the game's stores, for
    /// differential testing. Stopping drops what was recorded. Call it
    /// outside `emu_start`.
    fn set_write_log(&mut self, on: bool);
    /// The game stores recorded since the last call, oldest first: one
    /// `(address, bytes)` per memory access. [`Cpu::mem_write`] is not
    /// recorded. Empty when not recording.
    fn write_log(&mut self) -> Vec<(u32, Vec<u8>)>;
}

/// Callbacks from a running [`Cpu`].
pub trait Hooks<S> {
    /// At the start of each basic block in a [`Region::Code`] region, before
    /// it runs.
    fn on_block(&self, cpu: &mut dyn Cpu<S>, state: &mut S, addr: u32, size: u32);
    /// Before each instruction in a [`Region::Trampoline`] region (never at
    /// the `until` address). Usually sets PC to return from the trampoline.
    fn on_trampoline(&self, cpu: &mut dyn Cpu<S>, state: &mut S, addr: u32);
}
