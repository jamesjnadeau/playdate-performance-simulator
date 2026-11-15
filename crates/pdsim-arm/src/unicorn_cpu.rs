//! [`Cpu`] on Unicorn: a Cortex-M7 in Thumb mode.

use crate::cpu::{Cpu, CpuError, Hooks, Reg};
use unicorn_engine::unicorn_const::{uc_error, Arch, Mode, Prot};
use unicorn_engine::{ArmCpuModel, RegisterARM, Unicorn};

/// What the Unicorn hooks forward to while `emu_start` runs.
struct Ctx<S: 'static> {
    state: *mut S,
    hooks: Option<*const dyn Hooks<S>>,
}

impl<S> Clone for Ctx<S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S> Copy for Ctx<S> {}

type Uc<'u, S> = Unicorn<'u, Ctx<S>>;

/// Unicorn emulating a Cortex-M7.
pub struct UnicornCpu<S: 'static> {
    uc: Uc<'static, S>,
}

/// The engine inside a hook, which gets a `&mut Unicorn` rather than the
/// [`UnicornCpu`] that owns it.
struct InHook<'a, 'u, S: 'static>(&'a mut Uc<'u, S>);

fn err(uc: &Unicorn<'_, impl Sized>, e: uc_error) -> CpuError {
    CpuError {
        kind: format!("{e:?}"),
        pc: uc.reg_read(RegisterARM::PC).unwrap_or(0) as u32,
    }
}

fn reg_id(r: Reg) -> i32 {
    match r {
        Reg::R(n) => {
            assert!(n <= 12, "r{n} is not a general register");
            RegisterARM::R0 as i32 + n as i32
        }
        Reg::Sp => RegisterARM::SP as i32,
        Reg::Lr => RegisterARM::LR as i32,
        Reg::Pc => RegisterARM::PC as i32,
        Reg::S(n) => {
            assert!(n <= 31, "s{n} is not a VFP register");
            RegisterARM::S0 as i32 + n as i32
        }
        Reg::Fpscr => RegisterARM::FPSCR as i32,
        Reg::Cpsr => RegisterARM::APSR as i32,
    }
}

impl<S: 'static> UnicornCpu<S> {
    /// A Cortex-M7 with nothing mapped.
    pub fn new() -> Result<UnicornCpu<S>, CpuError> {
        let ctx = Ctx {
            state: std::ptr::null_mut(),
            hooks: None,
        };
        let mut uc =
            Unicorn::new_with_data(Arch::ARM, Mode::THUMB | Mode::MCLASS, ctx).map_err(|e| CpuError {
                kind: format!("{e:?}"),
                pc: 0,
            })?;
        uc.ctl_set_cpu_model(ArmCpuModel::CORTEX_M7.into())
            .map_err(|e| err(&uc, e))?;
        Ok(UnicornCpu { uc })
    }
}

fn mem_map<S: 'static>(uc: &mut Uc<'_, S>, addr: u32, size: u32, writable: bool) -> Result<(), CpuError> {
    let (begin, end) = (addr as u64, addr as u64 + size as u64 - 1);
    if writable {
        uc.mem_map(addr as u64, size as u64, Prot::ALL)
            .map_err(|e| err(uc, e))?;
        uc.add_block_hook(begin, end, |uc, addr, size| {
            let Ctx { state, hooks } = *uc.get_data();
            if let Some(hooks) = hooks {
                // SAFETY: `state` and `hooks` were set by `emu_start` from
                // references that outlive the emulation they are used in,
                // and are restored before it returns.
                let (state, hooks) = unsafe { (&mut *state, &*hooks) };
                hooks.on_block(&mut InHook(uc), state, addr as u32, size);
            }
        })
        .map_err(|e| err(uc, e))?;
    } else {
        uc.mem_map(addr as u64, size as u64, Prot::READ | Prot::EXEC)
            .map_err(|e| err(uc, e))?;
        uc.add_code_hook(begin, end, |uc, addr, _| {
            let Ctx { state, hooks } = *uc.get_data();
            if let Some(hooks) = hooks {
                // SAFETY: as for the block hook.
                let (state, hooks) = unsafe { (&mut *state, &*hooks) };
                hooks.on_trampoline(&mut InHook(uc), state, addr as u32);
            }
        })
        .map_err(|e| err(uc, e))?;
    }
    Ok(())
}

fn emu_start<S: 'static>(
    uc: &mut Uc<'_, S>,
    begin: u32,
    until: u32,
    state: &mut S,
    hooks: &dyn Hooks<S>,
) -> Result<(), CpuError> {
    let outer = *uc.get_data();
    // SAFETY: only erases the lifetime of `hooks`. The pointer is used by the
    // hooks during this call and replaced by `outer` before it returns.
    let hooks: *const (dyn Hooks<S> + '_) = hooks;
    let hooks: *const dyn Hooks<S> = unsafe { std::mem::transmute(hooks) };
    *uc.get_data_mut() = Ctx {
        state: state as *mut S,
        hooks: Some(hooks),
    };
    let r = uc.emu_start(begin as u64, until as u64, 0, 0);
    *uc.get_data_mut() = outer;
    r.map_err(|e| err(uc, e))
}

/// Implements [`Cpu`] for a type that reaches the engine through `$r`
/// (shared) and `$m` (mutable), with `$c` bound to `self`.
macro_rules! impl_cpu {
    ($t:ty, $c:ident => $r:expr, $m:expr) => {
        impl<S: 'static> Cpu<S> for $t {
            fn mem_map(&mut self, addr: u32, size: u32, writable: bool) -> Result<(), CpuError> {
                let $c = self;
                mem_map($m, addr, size, writable)
            }
            fn mem_read(&self, addr: u32, buf: &mut [u8]) -> Result<(), CpuError> {
                let $c = self;
                let uc = $r;
                uc.mem_read(addr as u64, buf).map_err(|e| err(uc, e))
            }
            fn mem_write(&mut self, addr: u32, data: &[u8]) -> Result<(), CpuError> {
                let $c = self;
                let uc = $m;
                uc.mem_write(addr as u64, data).map_err(|e| err(uc, e))
            }
            fn reg_read(&self, r: Reg) -> u32 {
                let $c = self;
                let uc = $r;
                uc.reg_read(reg_id(r)).unwrap_or(0) as u32
            }
            fn reg_write(&mut self, r: Reg, v: u32) {
                let $c = self;
                let uc = $m;
                let _ = uc.reg_write(reg_id(r), v as u64);
            }
            fn emu_start(
                &mut self,
                begin: u32,
                until: u32,
                state: &mut S,
                hooks: &dyn Hooks<S>,
            ) -> Result<(), CpuError> {
                let $c = self;
                emu_start($m, begin, until, state, hooks)
            }
            fn emu_stop(&mut self) {
                let $c = self;
                let uc = $m;
                let _ = uc.emu_stop();
            }
        }
    };
}

impl_cpu!(UnicornCpu<S>, c => &c.uc, &mut c.uc);
impl_cpu!(InHook<'_, '_, S>, c => &*c.0, &mut *c.0);

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct St {
        blocks: u32,
        nested_r0: u32,
    }

    /// A trampoline hook that calls back into emulated code (a nested
    /// `emu_start`) and then returns to the caller by setting PC = LR.
    struct NestingHooks;

    const RET: u32 = 0x2000_0100;

    impl Hooks<St> for NestingHooks {
        fn on_block(&self, _: &mut dyn Cpu<St>, st: &mut St, _: u32, _: u32) {
            st.blocks += 1;
        }
        fn on_trampoline(&self, cpu: &mut dyn Cpu<St>, st: &mut St, addr: u32) {
            assert_eq!(addr, 0x2000_0000, "the hook never runs at `until`");
            let lr = cpu.reg_read(Reg::Lr);
            cpu.reg_write(Reg::R(0), 41);
            cpu.reg_write(Reg::Lr, RET | 1);
            cpu.emu_start(0x1000_0201, RET, st, self).unwrap();
            st.nested_r0 = cpu.reg_read(Reg::R(0));
            cpu.reg_write(Reg::Pc, lr);
        }
    }

    #[test]
    fn nested_emu_start_from_a_hook_returns_to_the_outer_run() {
        let mut cpu = UnicornCpu::<St>::new().unwrap();
        cpu.mem_map(0x1000_0000, 0x1000, true).unwrap();
        cpu.mem_map(0x2000_0000, 0x1000, false).unwrap();
        cpu.mem_map(0x3000_0000, 0x1000, true).unwrap();
        // blx r4 ; nop
        cpu.mem_write(0x1000_0000, &[0xa0, 0x47, 0x00, 0xbf]).unwrap();
        // adds r0, #1 ; bx lr
        cpu.mem_write(0x1000_0200, &[0x01, 0x30, 0x70, 0x47]).unwrap();
        // Read-only memory is still written by mem_write: bx lr ; ... ; nop at RET.
        cpu.mem_write(0x2000_0000, &[0x70, 0x47]).unwrap();
        cpu.mem_write(RET, &[0x00, 0xbf]).unwrap();
        cpu.reg_write(Reg::Sp, 0x3000_0800);
        cpu.reg_write(Reg::R(4), 0x2000_0001);
        cpu.reg_write(Reg::R(0), 7);
        let mut st = St::default();
        cpu.emu_start(0x1000_0001, 0x1000_0002, &mut st, &NestingHooks)
            .unwrap();
        assert_eq!(st.nested_r0, 42);
        // The outer run resumed after the trampoline and stopped at `until`.
        assert_eq!(cpu.reg_read(Reg::Pc), 0x1000_0002);
        // The outer block and the nested block were both reported.
        assert!(st.blocks >= 2, "blocks {}", st.blocks);
    }

    #[test]
    fn faults_report_the_unicorn_error_and_pc() {
        let mut cpu = UnicornCpu::<St>::new().unwrap();
        cpu.mem_map(0x1000_0000, 0x1000, true).unwrap();
        // ldr r0, [r1]
        cpu.mem_write(0x1000_0000, &[0x08, 0x68, 0x00, 0xbf]).unwrap();
        cpu.reg_write(Reg::R(1), 0x5000_0000);
        let e = cpu
            .emu_start(0x1000_0001, 0x1000_0002, &mut St::default(), &NestingHooks)
            .unwrap_err();
        assert_eq!(e.kind, "READ_UNMAPPED");
        assert_eq!(e.pc, 0x1000_0000);
    }
}
